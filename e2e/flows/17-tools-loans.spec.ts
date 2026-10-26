// Flow 17 — Tools and loans close-out (Wave 32 lane 4): a tool checked out
// to the caller and checked back in with a condition, a client loan issued
// from stock and returned, and a kit whose member readiness shows before a
// checkout. Scratch units are staged through the API per test.
import { expect, test } from "../helpers/harness";
import { localDatePlus } from "../helpers/dates";
import { authedFetch } from "../helpers/api";
import type { Locator, Page } from "@playwright/test";

async function pickSelect(
  page: Page,
  trigger: string | Locator,
  option: string,
): Promise<void> {
  const control = typeof trigger === "string" ? page.getByTestId(trigger) : trigger;
  await control.click();
  await page.getByRole("option", { name: option }).click();
}

interface StockItem {
  id: string;
  name: string;
  unit: string;
}

interface Location {
  id: string;
  name: string;
}

interface Client {
  id: string;
  name: string;
}

/** Stages a serialized tool item with one in-stock unit at the location. */
async function stageTool(post: ReturnType<typeof authedFetch>, run: number): Promise<{ item: StockItem; location: Location; serial: string }> {
  const item = await post<StockItem>("/stock-items", {
    name: `E2E Drill ${run}`,
    kind: "serialized",
    unit: "each",
    currency: "USD",
    is_tool: true,
  });
  const location = await post<Location>("/stock-locations", {
    name: `E2E Dock ${run}`,
    kind: "warehouse",
  });
  const serial = `SN-E2E-${run}`;
  await post("/stock/receive", {
    item_id: item.id,
    location_id: location.id,
    qty_milli: 1000,
    serial_numbers: [serial],
    note: "e2e staging",
  });
  return { item, location, serial };
}

function tomorrow(): string {
  return localDatePlus(1);
}

test("check a tool out to me and check it back in", async ({ authedPage, ownerTokens }) => {
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });
  const { item, location, serial } = await stageTool(post, Date.now() % 1000000);

  await authedPage.goto("/stock/tools");
  await authedPage.getByTestId("tools-checkout-open").click();
  await pickSelect(authedPage, "tools-checkout-from", location.name);
  await authedPage.getByTestId("tools-checkout-due").fill(tomorrow());
  // The dialog opens with one item row (row testids are keys, not indexes).
  await pickSelect(
    authedPage,
    authedPage.locator('[id^="checkout-item-"]').first(),
    item.name,
  );
  await authedPage.getByTestId("tools-checkout-save").click();
  await expect(authedPage.getByTestId("tools-notice")).toBeVisible({ timeout: 20_000 });

  // Who-has-what flips the unit to checked out with the due phrase.
  const row = authedPage
    .locator('[data-testid^="tools-row-"]')
    .filter({ hasText: serial });
  await expect(row).toContainText("checked out", { timeout: 20_000 });
  await expect(row).toContainText("tomorrow");

  // Check-in takes a condition and returns the unit to stock.
  await row.locator('[data-testid^="tools-checkin-"]').click();
  await authedPage.locator('[data-testid^="tools-checkin-condition-"]').first().click();
  await authedPage.getByRole("option", { name: "good" }).click();
  await authedPage.getByTestId("tools-checkin-save").click();
  await expect(row).toContainText("In stock", { timeout: 20_000 });
});

test("issue a loan to a client from stock and return it", async ({ authedPage, ownerTokens }) => {
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });
  const get = authedFetch(ownerTokens.access_token);
  const run = Date.now() % 1000000;
  const { item, location, serial } = await stageTool(post, run);
  const clients = await get<Client[]>("/clients");
  const client = clients[0];

  await authedPage.goto("/stock/loans");
  await authedPage.getByTestId("loans-issue-open").click();
  await pickSelect(authedPage, "loans-issue-client", client.name);
  await authedPage.getByTestId("loans-issue-expected").fill(tomorrow());
  await pickSelect(
    authedPage,
    authedPage.locator('[id^="loan-item-"]').first(),
    item.name,
  );
  await pickSelect(
    authedPage,
    authedPage.locator('[id^="loan-serial-"]').first(),
    serial,
  );
  await authedPage.getByTestId("loans-issue-agreement").fill(`AGR-${run}`);
  await authedPage.getByTestId("loans-issue-save").click();
  await expect(authedPage.getByTestId("loans-notice")).toBeVisible({ timeout: 20_000 });

  // The row lands on the board with its due state (the template's STATE
  // column carries the due phrase — "Due tomorrow" — not the raw "open").
  const row = authedPage
    .locator('[data-testid^="loans-row-"]')
    .filter({ hasText: item.name });
  await expect(row).toBeVisible({ timeout: 20_000 });
  await expect(row).toContainText("tomorrow");

  // The return flow takes a destination (stock-sourced) and a condition.
  await row.locator('[data-testid^="loans-return-"]').click();
  await pickSelect(authedPage, "loans-return-location", location.name);
  await authedPage.locator('[data-testid^="loans-return-condition-"]').first().click();
  await authedPage.getByRole("option", { name: "good" }).click();
  await authedPage.getByTestId("loans-return-save").click();
  await expect(row).toContainText("Returned", { timeout: 20_000 });
});

test("a kit shows member readiness before the checkout", async ({ authedPage, ownerTokens }) => {
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });
  const run = Date.now() % 1000000;

  // Two tool units in stock become one kit; the readiness read counts them.
  const first = await stageTool(post, run);
  const secondItem = await post<StockItem>("/stock-items", {
    name: `E2E Scope ${run}`,
    kind: "serialized",
    unit: "each",
    currency: "USD",
    is_tool: true,
  });
  const secondSerial = `SN-E2E-B-${run}`;
  await post("/stock/receive", {
    item_id: secondItem.id,
    location_id: first.location.id,
    qty_milli: 1000,
    serial_numbers: [secondSerial],
    note: "e2e staging",
  });

  // The kit is built in the browser: create, then add both members.
  await authedPage.goto("/stock/tools");
  await authedPage.getByTestId("tools-kit-new").click();
  await authedPage.getByTestId("tools-kit-name").fill(`E2E Kit ${run}`);
  await authedPage.getByTestId("tools-kit-save").click();
  const kitRow = authedPage.getByTestId("tools-kits-table").getByRole("row").filter({ hasText: `E2E Kit ${run}` });
  await expect(kitRow).toBeVisible({ timeout: 20_000 });
  await kitRow.locator('[data-testid^="tools-kit-manage-"]').click();
  const memberRows = authedPage.locator('[data-testid^="tools-kit-member-row-"]');
  await pickSelect(authedPage, "tools-kit-member-item", first.item.name);
  await authedPage.getByTestId("tools-kit-member-add").click();
  await expect(memberRows.filter({ hasText: first.item.name })).toBeVisible({ timeout: 20_000 });
  await pickSelect(authedPage, "tools-kit-member-item", secondItem.name);
  await authedPage.getByTestId("tools-kit-member-add").click();
  await expect(memberRows.filter({ hasText: secondItem.name })).toBeVisible({ timeout: 20_000 });

  // Close the members dialog; its overlay would swallow the header click.
  await authedPage.keyboard.press("Escape");
  await expect(authedPage.getByTestId("tools-checkout-open")).toBeEnabled({ timeout: 20_000 });

  // Opening the checkout with the kit selected shows readiness, not error.
  await authedPage.getByTestId("tools-checkout-open").click();
  await pickSelect(authedPage, "tools-checkout-kit", `E2E Kit ${run}`);
  const readiness = authedPage.getByTestId("tools-checkout-kit-readiness");
  await expect(readiness).toBeVisible({ timeout: 20_000 });
  await expect(readiness).toContainText(first.item.name);
  await expect(readiness).toContainText(secondItem.name);
});
