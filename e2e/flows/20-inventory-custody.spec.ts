// Flow 20 — the inventory custody surfaces: where a unit is, gear rented in,
// a client's e-waste, and a credit that names the line it corrects.
//
// Every fixture is staged through the API so the UI is exercised against real
// rows, and each test leaves the org in a state the next one can ignore:
// names and tags carry a run suffix, and nothing depends on another test's
// leftovers.
import { expect, test } from "../helpers/harness";
import { authedFetch } from "../helpers/api";
import type { Page } from "@playwright/test";

interface StockItem {
  id: string;
  name: string;
}

interface Location {
  id: string;
  name: string;
}

interface Client {
  id: string;
  name: string;
}

async function pickOption(page: Page, trigger: ReturnType<Page["getByTestId"]>, option: string): Promise<void> {
  await trigger.click();
  await page.getByRole("option", { name: option }).click();
}

test("the custody view counts the unit it just received", async ({ authedPage, ownerTokens }) => {
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });
  const run = Date.now() % 1000000;
  const get = authedFetch(ownerTokens.access_token);
  const custodyOf = async (): Promise<{ serials: number; assets: number }> => {
    const rows = await get<{ custody_class: string; serials: number; assets: number }[]>(
      "/inventory/custody",
    );
    const sale = rows.find((row) => row.custody_class === "sale_stock");
    return { serials: sale?.serials ?? 0, assets: sale?.assets ?? 0 };
  };
  // The demo org carries history, so the assertion is relative to what this
  // unit adds.
  const before = await custodyOf();

  const item = await post<StockItem>("/stock-items", {
    name: `E2E Custody ${run}`,
    kind: "serialized",
    unit: "each",
    currency: "USD",
  });
  const location = await post<Location>("/stock-locations", {
    name: `E2E Custody Dock ${run}`,
    kind: "warehouse",
  });
  await post("/stock/receive", {
    item_id: item.id,
    location_id: location.id,
    qty_milli: 1000,
    serial_numbers: [`CUSTODY-${run}`],
  });

  await authedPage.goto("/stock/custody");
  await expect(
    authedPage.getByRole("heading", { name: "Custody" }),
  ).toBeVisible({ timeout: 20_000 });
  // The classes table is first; the ledger beneath it also says "Sale stock".
  const saleRow = authedPage
    .locator("table")
    .first()
    .getByRole("row", { name: /Sale stock/ });
  await expect(saleRow).toBeVisible();
  // Serials, assets, total — the received unit is one serial and no asset.
  await expect(saleRow.locator("td").nth(1)).toHaveText(String(before.serials + 1));
  await expect(saleRow.locator("td").nth(3)).toHaveText(
    String(before.serials + before.assets + 1),
  );
});

test("a vendor rental is recorded and returned from its register", async ({ authedPage }) => {
  const run = Date.now() % 1000000;
  const tag = `VR-E2E-${run}`;

  await authedPage.goto("/stock/vendor-rentals");
  await authedPage.getByTestId("vendor-rentals-new").click();
  await authedPage.getByTestId("vendor-rentals-vendor").fill("E2E Supplier");
  await authedPage.getByTestId("vendor-rentals-tag").fill(tag);
  await authedPage.getByTestId("vendor-rentals-name").fill("Loaned switch");
  await authedPage.getByTestId("vendor-rentals-cost").fill("120.00");
  await authedPage.getByTestId("vendor-rentals-save").click();

  const row = authedPage.getByRole("row", { name: new RegExp(tag) });
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.getByRole("button", { name: "Return" }).click();
  await authedPage.getByTestId("vendor-rentals-return-save").click();

  // The register defaults to open rentals, so the returned row is read under
  // the Returned filter.
  await pickOption(authedPage, authedPage.getByLabel("Vendor rental status"), "Returned");
  await expect(authedPage.getByRole("row", { name: new RegExp(tag) })).toContainText(/returned/i, {
    timeout: 20_000,
  });
});

test("a client's e-waste is received, disposed and closed", async ({ authedPage, ownerTokens }) => {
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });
  const run = Date.now() % 1000000;
  const client = await post<Client>("/clients", { name: `E2E Ewaste ${run}` });

  await authedPage.goto("/stock/ewaste");
  await authedPage.getByTestId("ewaste-new").click();
  await pickOption(authedPage, authedPage.getByTestId("ewaste-client"), client.name);
  await authedPage.getByTestId("ewaste-received-by").fill("E2E Driver");
  await authedPage.getByTestId("ewaste-line-category").fill("Laptop");
  await authedPage.getByTestId("ewaste-line-weight").fill("2000");
  await authedPage.getByRole("button", { name: "Add line" }).click();
  await authedPage.getByTestId("ewaste-save").click();

  const row = authedPage.getByRole("row", { name: new RegExp(client.name) });
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.getByRole("button", { name: "Open" }).click();

  // The whole 2 kg leaves as a resale, which needs no certificate.
  await authedPage.getByRole("button", { name: /Dispose 2,000 g/ }).click();
  await pickOption(authedPage, authedPage.getByTestId("ewaste-pathway"), "Resold");
  await authedPage.getByTestId("ewaste-dispose-save").click();
  await expect(authedPage.getByText("held 0 g")).toBeVisible({ timeout: 20_000 });

  await authedPage.getByTestId("ewaste-close").click();
  await expect(authedPage.getByRole("dialog")).toContainText("closed", { timeout: 20_000 });
});

test("a credit note names the contract line it corrects", async ({ authedPage, ownerTokens }) => {
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });
  const run = Date.now() % 1000000;
  const client = await post<Client>("/clients", { name: `E2E Credit ${run}` });
  const contract = await post<{ id: string }>("/contracts", {
    client_id: client.id,
    number: `C-E2E-${run}`,
    name: "E2E goodwill agreement",
    contract_type: "time_and_materials",
    status: "active",
    currency: "USD",
    effective_from: "2026-01-01",
    hourly_rate_minor: 12_000,
  });

  await authedPage.goto("/billing/credit-notes");
  await authedPage.getByTestId("issue-credit-note").click();
  await pickOption(authedPage, authedPage.getByLabel("Credit note client"), client.name);
  await authedPage.locator("#cn-total").fill("6.00");
  await authedPage.locator("#cn-currency").fill("USD");

  await pickOption(authedPage, authedPage.getByLabel("Line source kind"), "Contract line");
  await authedPage.getByLabel("Line source id").fill(contract.id);
  await authedPage.getByLabel("Line description").fill(`Goodwill ${run}`);
  await authedPage.getByLabel("Line amount").fill("6.00");
  await authedPage.getByRole("button", { name: "Add line" }).click();
  await authedPage.getByRole("button", { name: "Issue", exact: true }).click();

  // The note appears; opening it shows the corrected line with its source.
  const numberButton = authedPage.getByRole("button", { name: /^CRN-/ }).first();
  await expect(numberButton).toBeVisible({ timeout: 20_000 });
  await numberButton.click();
  await expect(authedPage.getByText("Corrected lines")).toBeVisible({ timeout: 10_000 });
  await expect(authedPage.getByText(`Goodwill ${run}`)).toBeVisible();
});
