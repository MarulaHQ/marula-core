// Flow 16 — Stock close-out (Wave 32 lane 4): item creation, receiving
// through the one movement dialog, the low-stock banner fed by thresholds,
// the availability check refusing an over-consumption, a quote line pulled
// from stock with its availability read, and the ticket parts-used flow
// that consumes stock and lands the movement on the ticket timeline.
import { expect, test } from "../helpers/harness";
import { authedFetch, firstDemoTicket } from "../helpers/api";
import type { Page } from "@playwright/test";

async function pickSelect(page: Page, testid: string, option: string): Promise<void> {
  await page.getByTestId(testid).click();
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

test("create an item, receive stock, threshold it into the low banner, see the ledger", async ({
  authedPage,
  ownerTokens,
}) => {
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });
  const run = Date.now() % 1000000;

  const itemName = `E2E Splices ${run}`;
  const item = await post<StockItem>("/stock-items", {
    name: itemName,
    kind: "consumable",
    unit: "each",
    currency: "USD",
    is_tool: false,
  });
  const location = await post<Location>("/stock-locations", {
    name: `E2E Dock ${run}`,
    kind: "warehouse",
  });

  await authedPage.goto("/stock");
  await authedPage.getByTestId("stock-new-item").click();
  await authedPage.getByTestId("stock-name").fill(`E2E Anchors ${run}`);
  await authedPage.getByTestId("stock-create").click();
  await expect(
    authedPage.locator('[data-testid^="stock-row-"]').filter({ hasText: `E2E Anchors ${run}` }),
  ).toBeVisible({ timeout: 20_000 });

  // Receive four into the first location through the movement dialog.
  await authedPage.getByTestId(`stock-receive-${item.id}`).click();
  await pickSelect(authedPage, "movement-to-location", location.name);
  await authedPage.getByTestId("movement-qty").fill("4");
  await authedPage.getByTestId("movement-save").click();
  await expect(authedPage.getByTestId("stock-movement-recorded")).toBeVisible({ timeout: 20_000 });

  // The item detail shows the level, the ledger row, and takes thresholds.
  await authedPage
    .locator('[data-testid^="stock-row-"]')
    .filter({ hasText: itemName })
    .getByRole("link", { name: itemName })
    .click();
  await expect(authedPage.getByTestId("stock-detail-name")).toContainText(itemName);
  await expect(authedPage.locator('[data-testid^="stock-onhand-row-"]').first()).toContainText("4 each");
  await expect(authedPage.getByTestId("stock-ledger-table")).toContainText("Received");

  await authedPage.locator('[data-testid^="stock-thresholds-"]').first().click();
  await authedPage.getByTestId("stock-threshold-min").fill("10");
  await authedPage.getByTestId("stock-threshold-reorder").fill("10");
  await authedPage.getByTestId("stock-threshold-save").click();
  await expect(
    authedPage.locator('[data-testid^="stock-onhand-row-"]').first(),
  ).toContainText("low", { timeout: 20_000 });

  // The ledger row carries the same threshold state the automations read: the
  // low pill on the row (the p41 template replaced the old top-of-page banner
  // with the row state + the "At reorder point" shortcut).
  await authedPage.goto("/stock");
  await expect(
    authedPage.locator('[data-testid^="stock-row-"]').filter({ hasText: itemName }),
  ).toContainText("Low stock", { timeout: 20_000 });
});

test("the availability check refuses an over-consumption before the write", async ({
  authedPage,
  ownerTokens,
}) => {
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });
  const run = Date.now() % 1000000;

  const item = await post<StockItem>("/stock-items", {
    name: `E2E Terms ${run}`,
    kind: "consumable",
    unit: "each",
    currency: "USD",
    is_tool: false,
  });
  const location = await post<Location>("/stock-locations", {
    name: `E2E Dock ${run}`,
    kind: "warehouse",
  });
  await post("/stock/receive", {
    item_id: item.id,
    location_id: location.id,
    qty_milli: 2000,
    note: "e2e staging",
  });

  await authedPage.goto("/stock");
  await authedPage.getByTestId(`stock-use-${item.id}`).click();
  await pickSelect(authedPage, "movement-from-location", location.name);
  await expect(authedPage.getByTestId("movement-available")).toContainText("2 each");
  await authedPage.getByTestId("movement-qty").fill("5");
  await authedPage.getByTestId("movement-save").click();
  await expect(authedPage.getByTestId("movement-error")).toContainText("Only 2 each", { timeout: 20_000 });

  // Within availability the same dialog writes cleanly.
  await authedPage.getByTestId("movement-qty").fill("1");
  await authedPage.getByTestId("movement-save").click();
  await expect(authedPage.getByTestId("stock-movement-recorded")).toBeVisible({ timeout: 20_000 });
});

test("a quote line pulls from stock and shows the availability read", async ({ authedPage, ownerTokens }) => {
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });
  const get = authedFetch(ownerTokens.access_token);
  const run = Date.now() % 1000000;

  const itemName = `E2E Mounts ${run}`;
  const item = await post<StockItem>("/stock-items", {
    name: itemName,
    kind: "consumable",
    unit: "each",
    currency: "USD",
    is_tool: false,
  });
  const location = await post<Location>("/stock-locations", {
    name: `E2E Dock ${run}`,
    kind: "warehouse",
  });
  await post("/stock/receive", {
    item_id: item.id,
    location_id: location.id,
    qty_milli: 3000,
    note: "e2e staging",
  });
  const clients = await get<Client[]>("/clients");

  await authedPage.goto("/sales/quotes");
  await authedPage.getByTestId("quotes-new").click();
  await pickSelect(authedPage, "quote-client", clients[0].name);
  await pickSelect(authedPage, "quote-line-stock-0", itemName);
  await expect(authedPage.getByTestId("quote-line-availability-0")).toContainText("3 each on hand");

  const line = authedPage.getByTestId("quote-line-0");
  await authedPage.getByTestId("quote-line-price-0").fill("25");
  await authedPage.getByTestId("quote-save").click();
  await expect(authedPage.getByTestId("quote-save")).toBeHidden({ timeout: 20_000 });
  await expect(
    authedPage.getByTestId("quotes-table").getByRole("row").filter({ hasText: clients[0].name }).first(),
  ).toBeVisible({ timeout: 20_000 });
});

test("parts used on a ticket consumes stock and lands on the timeline", async ({ authedPage, ownerTokens }) => {
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });
  const run = Date.now() % 1000000;

  const itemName = `E2E Fuses ${run}`;
  const item = await post<StockItem>("/stock-items", {
    name: itemName,
    kind: "consumable",
    unit: "each",
    currency: "USD",
    is_tool: false,
  });
  const location = await post<Location>("/stock-locations", {
    name: `E2E Dock ${run}`,
    kind: "warehouse",
  });
  await post("/stock/receive", {
    item_id: item.id,
    location_id: location.id,
    qty_milli: 5000,
    note: "e2e staging",
  });
  const ticket = await firstDemoTicket(ownerTokens.access_token);

  await authedPage.goto(`/tickets/${ticket.id}`);
  await authedPage.getByTestId("wb-parts-used").click();
  await pickSelect(authedPage, "movement-item", itemName);
  await pickSelect(authedPage, "movement-from-location", location.name);
  await authedPage.getByTestId("movement-qty").fill("2");
  await authedPage.getByTestId("movement-save").click();
  await expect(authedPage.getByTestId("wb-parts-notice")).toBeVisible({ timeout: 20_000 });

  // The Parts chip narrows the unified stream to the movement row.
  await authedPage.getByTestId("wb-filter-parts").click();
  await expect(authedPage.getByTestId("wb-row-parts").first()).toContainText(itemName, { timeout: 20_000 });
  await expect(authedPage.getByTestId("wb-row-parts").first()).toContainText("−2 each");
});
