// Flow 19 — context-embedded stock workflows (Wave 32 lane 6
// `p32-stock-flows-context`).
//
// Two flagship journeys, minimum-switching as the bar:
//   A. Ticket-side — the parts & shipping panel completes use / ship to
//      client (carrier + tracking + label print) / return to stock without
//      a /stock visit. The scan leg is a SYNTHESIZED keyboard-wedge burst
//      typed into another field: it must stage the item, strip itself out
//      of that field, and never record a movement on its own.
//   B. Stockroom-side — scan an item on /stock and attach it to a chosen
//      ticket from the picker, without opening the ticket; receive on a
//      PO with scan-to-receive and dispatch straight from the receipt.
//
// Scratch state is staged per run through the API (unique stamps), so the
// flow is order-independent and re-runnable on a dirty demo org.
import { expect, test } from "../helpers/harness";
import { authedFetch, DEMO_SERVICE_DESK_BOARD_ID } from "../helpers/api";
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

interface Ticket {
  id: string;
  number: number;
  title: string;
}

test("the ticket's parts & shipping panel completes use, ship with a label, and return in place", async ({
  authedPage,
  ownerTokens,
}) => {
  const get = authedFetch(ownerTokens.access_token);
  const post = (path: string, body?: unknown) =>
    get(path, { method: "POST", body });
  const run = Date.now() % 1_000_000;

  const item = await post<StockItem>("/stock-items", {
    name: `E2E flow6 part ${run}`,
    kind: "consumable",
    sku: `E2E-FLOW6-${run}`,
    unit: "each",
    currency: "USD",
    idempotency_key: crypto.randomUUID(),
  });
  const location = await post<Location>("/stock-locations", {
    name: `E2E flow6 shelf ${run}`,
    kind: "warehouse",
    code: `F6-${run}`,
  });
  await post("/stock/receive", {
    item_id: item.id,
    location_id: location.id,
    qty_milli: 25_000,
    unit_cost_minor: 500,
    idempotency_key: crypto.randomUUID(),
  });
  const ticket = await post<Ticket>("/tickets", {
    board_id: DEMO_SERVICE_DESK_BOARD_ID,
    title: `E2E flow6 ticket ${run}`,
    priority: "p3",
    idempotency_key: crypto.randomUUID(),
  });

  await authedPage.goto(`/tickets/${ticket.id}`);
  await authedPage.getByTestId("wb-parts-used").click();
  await expect(authedPage.getByTestId("movement-scan")).toBeVisible({ timeout: 30_000 });

  // Keyboard-wedge burst while the cursor sits in the item filter: the
  // scan stages the item, strips its characters back out of the filter,
  // and records nothing.
  const foreign = authedPage.getByTestId("movement-item-filter");
  await foreign.fill("note ");
  await foreign.pressSequentially(`E2E-FLOW6-${run}`, { delay: 8 });
  await foreign.press("Enter");
  await expect(authedPage.getByTestId("scan-chip-summary")).toContainText(item.name, {
    timeout: 20_000,
  });
  await expect(foreign).toHaveValue("note ");
  await expect(authedPage.getByTestId("wb-parts-receipt")).toHaveCount(0);

  // Use on the job.
  await pickSelect(authedPage, "movement-from-location", location.name);
  await authedPage.getByTestId("movement-save").click();
  await expect(authedPage.getByTestId("wb-parts-receipt")).toContainText("Used 1", {
    timeout: 20_000,
  });
  await expect(authedPage.getByTestId("wb-row-parts").first()).toContainText(item.name, {
    timeout: 20_000,
  });

  // Ship to the client: carrier + tracking, then print the label from the
  // same panel.
  await authedPage.getByTestId("wb-parts-used").click();
  await authedPage.getByTestId("movement-kind-ship-out").click();
  await pickSelect(authedPage, "movement-item", item.name);
  await pickSelect(authedPage, "movement-from-location", location.name);
  await authedPage.getByTestId("movement-carrier").fill("UPS");
  await authedPage.getByTestId("movement-tracking").fill(`1Z-FLOW6-${run}`);
  await authedPage.getByTestId("movement-save").click();
  const receipt = authedPage.getByTestId("wb-parts-receipt");
  await expect(receipt).toContainText("Shipped 1", { timeout: 20_000 });
  await expect(receipt).toContainText(`1Z-FLOW6-${run}`);
  await receipt.getByTestId("label-print-button").click();
  await expect(authedPage.getByTestId("label-print-panel")).toBeVisible({ timeout: 20_000 });

  // Return / restock from the ticket (the record lands on this timeline).
  await authedPage.getByTestId("wb-parts-used").click();
  await authedPage.getByTestId("movement-kind-receive").click();
  await pickSelect(authedPage, "movement-item", item.name);
  await pickSelect(authedPage, "movement-to-location", location.name);
  await authedPage.getByTestId("movement-save").click();
  await expect(authedPage.getByTestId("wb-parts-receipt")).toContainText("Returned 1", {
    timeout: 20_000,
  });
});

test("a stockroom wedge burst stages the item and the picker attaches it to a ticket", async ({
  authedPage,
  ownerTokens,
}) => {
  const get = authedFetch(ownerTokens.access_token);
  const post = (path: string, body?: unknown) => get(path, { method: "POST", body });
  const run = Date.now() % 1_000_000;

  const item = await post<StockItem>("/stock-items", {
    name: `E2E dispatch part ${run}`,
    kind: "consumable",
    sku: `E2E-DISPATCH-${run}`,
    unit: "each",
    currency: "USD",
    idempotency_key: crypto.randomUUID(),
  });
  const location = await post<Location>("/stock-locations", {
    name: `E2E dispatch van ${run}`,
    kind: "van",
    code: `D6-${run}`,
  });
  await post("/stock/receive", {
    item_id: item.id,
    location_id: location.id,
    qty_milli: 10_000,
    idempotency_key: crypto.randomUUID(),
  });
  const ticket = await post<Ticket>("/tickets", {
    board_id: DEMO_SERVICE_DESK_BOARD_ID,
    title: `E2E dispatch ticket ${run}`,
    priority: "p3",
    idempotency_key: crypto.randomUUID(),
  });

  await authedPage.goto("/stock");
  await expect(authedPage.getByTestId("stock-dispatch-scan")).toBeVisible({ timeout: 30_000 });

  // A burst into the board search routes to the dispatch scan and opens
  // the movement form with the item staged. The dispatch scan input is scoped:
  // /stock renders two scan bars (dispatch + the movement form's).
  const search = authedPage.getByTestId("stock-dispatch-scan").getByTestId("scan-input");
  await search.fill("cab");
  await search.pressSequentially(`E2E-DISPATCH-${run}`, { delay: 8 });
  await search.press("Enter");
  await expect(authedPage.getByTestId("scan-chip-summary")).toContainText(item.name, {
    timeout: 20_000,
  });
  // The burst resolves into a chip and the query is consumed: the staged
  // identity rides the chip, so the input clears rather than keeping the
  // raw code as leftover text.
  await expect(search).toHaveValue("");
  await expect(authedPage.getByTestId("movement-ticket")).toBeVisible({ timeout: 20_000 });

  // Attach to a ticket straight from the stockroom — the ticket is never
  // opened in the browser; the ledger carries the anchor.
  await authedPage.getByTestId("movement-ticket").click();
  await authedPage.getByRole("option", { name: new RegExp(ticket.title) }).click();
  await pickSelect(authedPage, "movement-from-location", location.name);
  await authedPage.getByTestId("movement-save").click();
  await expect(authedPage.getByTestId("stock-movement-recorded")).toBeVisible({
    timeout: 20_000,
  });

  const movements = await get<{ stock_item_id: string; ticket_id: string | null }[]>(
    `/tickets/${ticket.id}/stock-movements`,
  );
  expect(movements.some((m) => m.stock_item_id === item.id && m.ticket_id === ticket.id)).toBe(true);
});

test("scan-to-receive stages a PO line and dispatches straight to a ticket", async ({
  authedPage,
  ownerTokens,
}) => {
  const get = authedFetch(ownerTokens.access_token);
  const post = (path: string, body?: unknown) => get(path, { method: "POST", body });
  const run = Date.now() % 1_000_000;

  const vendor = await post<{ id: string; name: string }>("/vendors", {
    name: `E2E flow6 vendor ${run}`,
    default_currency: "USD",
  });
  const item = await post<StockItem>("/stock-items", {
    name: `E2E flow6 received ${run}`,
    kind: "consumable",
    sku: `E2E-RECEIVE-${run}`,
    unit: "each",
    currency: "USD",
    idempotency_key: crypto.randomUUID(),
  });
  const location = await post<Location>("/stock-locations", {
    name: `E2E flow6 dock ${run}`,
    kind: "warehouse",
    code: `R6-${run}`,
  });
  const order = await post<{ order: { id: string } }>("/purchase-orders", {
    vendor_id: vendor.id,
    lines: [
      {
        description: item.name,
        qty_ordered_milli: 2_000,
        unit_cost_minor: 900,
        stock_item_id: item.id,
      },
    ],
    idempotency_key: crypto.randomUUID(),
  });
  await post(`/purchase-orders/${order.order.id}/order`, {
    idempotency_key: crypto.randomUUID(),
  });
  const ticket = await post<Ticket>("/tickets", {
    board_id: DEMO_SERVICE_DESK_BOARD_ID,
    title: `E2E receive ticket ${run}`,
    priority: "p3",
    idempotency_key: crypto.randomUUID(),
  });

  await authedPage.goto(`/purchasing/${order.order.id}`);
  await authedPage.getByTestId("purchasing-receive").click();
  await expect(authedPage.getByTestId("purchasing-receive-scan")).toBeVisible({ timeout: 30_000 });

  // Scan-to-receive: the line stages at the scanned quantity (1), never
  // the pre-filled remaining, and commits only when the human submits.
  await pickSelect(authedPage, "purchasing-receive-location", location.name);
  const scanInput = authedPage.getByTestId("scan-input");
  await scanInput.fill(`E2E-RECEIVE-${run}`);
  await scanInput.press("Enter");
  const qty = authedPage.locator('[data-testid^="purchasing-receive-qty-"]').first();
  await expect(qty).toHaveValue("1", { timeout: 20_000 });

  await authedPage.getByTestId("purchasing-receive-submit").click();
  const dispatchStrip = authedPage.getByTestId("purchasing-received-dispatch");
  await expect(dispatchStrip).toBeVisible({ timeout: 20_000 });

  // Immediately dispatch the received goods to a ticket.
  await authedPage.getByTestId(`purchasing-dispatch-${item.id}`).click();
  await authedPage.getByTestId("movement-ticket").click();
  await authedPage.getByRole("option", { name: new RegExp(ticket.title) }).click();
  await pickSelect(authedPage, "movement-from-location", location.name);
  await authedPage.getByTestId("movement-save").click();
  await expect(authedPage.getByTestId("purchasing-action-notice")).toContainText("Dispatched", {
    timeout: 20_000,
  });

  const movements = await get<{ stock_item_id: string; ticket_id: string | null }[]>(
    `/tickets/${ticket.id}/stock-movements`,
  );
  expect(movements.some((m) => m.stock_item_id === item.id && m.ticket_id === ticket.id)).toBe(true);
});
