// Flow 15 — labels + scanning (Wave 32 lane 5 `p32-labels-scanning`).
//
// Two legs:
//   1. A SYNTHESIZED keyboard-wedge burst (fast keystrokes, suffix Enter)
//      typed while the cursor sits in another form field. The scan bar must
//      resolve it, fill the parts line, keep focus where it was, swallow the
//      Enter so no form submits, and clean the burst out of the field it
//      leaked into.
//   2. The label designer: preview renders through the core's one renderer
//      (real QR + Code128 SVG at physical millimetres) and the browser
//      print target receives the print document.
//
// Scratch state is staged through the API with per-run unique codes, so the
// flow is order-independent and re-runnable on a dirty demo org.
import { expect, test } from "../helpers/harness";
import { authedFetch, DEMO_SERVICE_DESK_BOARD_ID } from "../helpers/api";

interface Ticket {
  id: string;
}

test("a keyboard-wedge burst fills a ticket parts line without submitting or stealing focus", async ({
  authedPage,
  ownerTokens,
}) => {
  const get = authedFetch(ownerTokens.access_token);
  const stamp = Date.now() % 1_000_000;
  const sku = `E2E-SCAN-${stamp}`;
  const item = await get<{ id: string; name: string }>(`/stock-items`, {
    method: "POST",
    body: {
      name: `E2E scan part ${stamp}`,
      kind: "consumable",
      sku,
      unit: "each",
      currency: "USD",
      idempotency_key: crypto.randomUUID(),
    },
  });
  // A real receipt with a locked cost, so the scan proves it carries the
  // item's last-known unit cost into the line (the no-cost path is covered
  // by unit tests). Costs land on the item through the ledger, not at create.
  const location = await get<{ id: string }>(`/stock-locations`, {
    method: "POST",
    body: { name: `E2E scan shelf ${stamp}`, kind: "warehouse", code: `SH-${stamp}` },
  });
  await get(`/stock/receive`, {
    method: "POST",
    body: {
      item_id: item.id,
      location_id: location.id,
      qty_milli: 1000,
      unit_cost_minor: 2450,
      idempotency_key: crypto.randomUUID(),
    },
  });
  const ticket = await get<Ticket>(`/tickets`, {
    method: "POST",
    body: {
      board_id: DEMO_SERVICE_DESK_BOARD_ID,
      title: `E2E scan-to-parts ${stamp}`,
      priority: "p3",
      idempotency_key: crypto.randomUUID(),
    },
  });

  // The money tab lives on the classic workspace route.
  await authedPage.goto(`/tickets/classic/${ticket.id}`);
  await authedPage.getByTestId("detail-tab-money").click();
  const scanBar = authedPage.getByTestId("scan-bar");
  await expect(scanBar).toBeVisible({ timeout: 30_000 });

  // Park the cursor in the description field, then let the "scanner" type
  // its burst into it (that is what a HID wedge does to a focused field).
  const foreign = authedPage.locator("#cl-desc");
  await foreign.fill("note ");
  // Fast burst timing: ≤10 ms gaps read as a scanner, not as typing.
  await foreign.pressSequentially(sku, { delay: 8 });
  await foreign.press("Enter");

  // Resolved, routed, and filled — with focus still in the field the scan
  // landed in, and the injected burst stripped back out of it.
  await expect(authedPage.getByTestId("scan-chip-summary")).toContainText(item.name, {
    timeout: 20_000,
  });
  await expect(foreign).toHaveValue(item.name, { timeout: 20_000 });
  await expect(authedPage.locator("#cl-qty")).toHaveValue("1");
  await expect(authedPage.locator("#cl-unit")).toHaveValue("24.50");
  await expect(authedPage.getByTestId("ticket-parts-scan-note")).toContainText(
    "review, then add it",
  );
  // The focus law: focus never moved out of the field the scan landed in.
  await expect(foreign).toBeFocused();

  // The Enter did NOT submit: the cost-line table has no row yet.
  await expect(
    authedPage.locator("tbody tr").filter({ hasText: item.name }),
  ).toHaveCount(0);

  // Committing is the human's move — and it lands the line.
  await authedPage.getByRole("button", { name: "Add line" }).click();
  await expect(
    authedPage.locator("tbody tr").filter({ hasText: item.name }).first(),
  ).toBeVisible({ timeout: 20_000 });
});

test("label designer previews a real label and the print target gets the print document", async ({
  authedPage,
}) => {
  // Count print calls from the label print iframe (addInitScript runs in
  // child frames too, so the iframe's own window.print is stubbed).
  await authedPage.addInitScript(() => {
    const w = window as unknown as { __labelPrintCalls?: number };
    w.print = () => {
      const top = window.top as unknown as { __labelPrintCalls?: number };
      if (top !== null && top !== undefined) top.__labelPrintCalls = (top.__labelPrintCalls ?? 0) + 1;
    };
  });

  const stamp = Date.now() % 1_000_000;
  await authedPage.goto("/settings/labels");
  await expect(authedPage.getByTestId("label-new")).toBeVisible({ timeout: 30_000 });
  await authedPage.getByTestId("label-new").click();

  await authedPage.getByTestId("label-name").fill(`E2E serialized unit ${stamp}`);

  // Preview renders through the core: label surface at physical size, a QR
  // image and a Code128 band.
  const preview = authedPage.getByTestId("label-preview-surface");
  await expect(preview).toBeVisible({ timeout: 20_000 });
  await expect(authedPage.getByTestId("label-qr")).toBeVisible();
  await expect(authedPage.getByTestId("label-barcode")).toBeVisible();
  const box = await preview.boundingBox();
  // 50.8 × 25.4 mm at CSS 96 dpi ≈ 192 × 96 px — assert the aspect, not a
  // browser-version-specific pixel count.
  expect(box).not.toBeNull();
  expect((box?.width ?? 0) / (box?.height ?? 1)).toBeCloseTo(2, 0);

  // The browser print target receives the label document (@page sized).
  await authedPage.getByTestId("label-print-test").click();
  await expect
    .poll(async () =>
      authedPage.evaluate(
        () => (window as unknown as { __labelPrintCalls?: number }).__labelPrintCalls ?? 0,
      ),
    )
    .toBe(1);
  const srcdoc = await authedPage.evaluate(() => {
    const frame = document.querySelector("iframe[aria-hidden='true']") as HTMLIFrameElement | null;
    return frame?.srcdoc ?? "";
  });
  expect(srcdoc).toContain("@page { size: 50.8mm 25.4mm; margin: 0; }");
  expect(srcdoc).toContain("data:image/svg+xml");

  // Raw ZPL is honest on web: the button says the desktop app owns it.
  await authedPage.getByTestId("label-send-zpl").click();
  await expect(authedPage.getByTestId("label-status")).toContainText("desktop", {
    timeout: 15_000,
  });

  // Saving lands the template in the list.
  await authedPage.getByTestId("label-save").click();
  await expect(authedPage.getByTestId("label-template-list")).toContainText(
    `E2E serialized unit ${stamp}`,
    { timeout: 20_000 },
  );
});
