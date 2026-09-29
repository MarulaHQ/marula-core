// Wave 32 lane 6 `p32-stock-flows-context` — vision-review captures.
//
// Drives the REAL app against the live e2e stack (never mocks), staging
// scratch data through /api/v1 so the screenshots show true states: the
// ticket's parts and shipping panel with a scan bar, a recorded use, a
// shipped receipt with carrier/tracking and the label print panel, the
// stockroom dispatch scan plus ticket picker, the tools scan bar, and the
// purchase-order receive dialog with scan-to-receive and the post-receipt
// dispatch strip.
//
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p32-stock-flows-captures.mjs
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = process.env.CAPTURE_OUT ?? "design/qa-p32-stock-flows-context/captures";
const THEMES = (process.env.CAPTURE_THEMES ?? "light,dark").split(",");
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const STAMP = Date.now() % 1_000_000;
const BOARD_ID = "6d657269-6469-616e-0000-00000000003c";

mkdirSync(OUT, { recursive: true });

const ctx0 = await (await import("playwright")).request.newContext();
const login = await ctx0.post(`${API}/api/v1/auth/login`, { data: { email: EMAIL, password: PASSWORD } });
if (!login.ok()) throw new Error(`login failed: ${login.status()}`);
const tokens = await login.json();

async function api(path, method, body) {
  const res = await ctx0.fetch(`${API}/api/v1${path}`, {
    method,
    headers: { "content-type": "application/json", authorization: `Bearer ${tokens.access_token}` },
    data: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok()) throw new Error(`${method} ${path} → ${res.status()}: ${await res.text()}`);
  return res.headers()["content-type"]?.includes("json") ? res.json() : undefined;
}

// --- scratch data -----------------------------------------------------------
const location = await api(`/stock-locations`, "POST", {
  name: `Capture flow6 shelf ${STAMP}`,
  kind: "warehouse",
  code: `F6C-${STAMP}`,
});
const item = await api(`/stock-items`, "POST", {
  name: `Capture flow6 cable ${STAMP}`,
  kind: "consumable",
  sku: `F6-CAP-${STAMP}`,
  unit: "each",
  currency: "USD",
  idempotency_key: crypto.randomUUID(),
});
await api(`/stock/receive`, "POST", {
  item_id: item.id,
  location_id: location.id,
  qty_milli: 25_000,
  unit_cost_minor: 450,
  idempotency_key: crypto.randomUUID(),
});
// A default item label, so the shipping receipt's label panel shows a real
// rendered label rather than the honest "no template" state.
await api(`/label-templates`, "POST", {
  name: `Capture flow6 item label ${STAMP}`,
  purpose: "stock_item",
  size_preset: "zebra_2x1",
  barcode_token: "sku",
  include_qr: true,
  fields: ["name", "sku"],
  is_default: true,
  idempotency_key: crypto.randomUUID(),
});
const ticket = await api(`/tickets`, "POST", {
  board_id: BOARD_ID,
  title: `Capture flow6 ticket ${STAMP}`,
  priority: "p3",
  idempotency_key: crypto.randomUUID(),
});
const vendor = await api(`/vendors`, "POST", {
  name: `Capture flow6 vendor ${STAMP}`,
  default_currency: "USD",
});
const order = await api(`/purchase-orders`, "POST", {
  vendor_id: vendor.id,
  lines: [
    {
      description: item.name,
      qty_ordered_milli: 5_000,
      unit_cost_minor: 450,
      stock_item_id: item.id,
    },
  ],
  idempotency_key: crypto.randomUUID(),
});
await api(`/purchase-orders/${order.order.id}/order`, "POST", {
  idempotency_key: crypto.randomUUID(),
});

async function pickSelect(page, trigger, option) {
  await trigger.click();
  await page.getByRole("option", { name: option }).click();
}

let shotIndex = 0;
const failures = [];
for (const theme of THEMES) {
  const browser = await chromium.launch({ args: ["--no-proxy-server"] });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  {
    let ok = false;
    for (let attempt = 0; attempt < 20 && !ok; attempt += 1) {
      const res = await ctx.request.post(`${API}/api/v1/auth/login`, {
        data: { email: EMAIL, password: PASSWORD },
      });
      ok = res.ok();
      if (!ok) await new Promise((resolve) => setTimeout(resolve, 3000));
    }
    if (!ok) throw new Error("context login failed");
  }
  const page = await ctx.newPage();
  await page.addInitScript(
    ([apiBase, tokenPair, mode]) => {
      window.__MARULA_API_URL__ = apiBase;
      window.localStorage.setItem("marula-tokens", tokenPair);
      window.localStorage.setItem("marula-tokens-exp", String(Date.now() + 3_600_000));
      window.localStorage.setItem("marula-theme", mode);
      // The seed resets per-user tours, so without this the capture carries
      // the onboarding popover. The session cap (tours.ts) honors a preset
      // count — the shots show the app, not step 1 of the tour.
      window.sessionStorage.setItem("marula-tour-session", "1");
    },
    [API, JSON.stringify(tokens), theme],
  );

  async function shot(name, { fullPage = false } = {}) {
    shotIndex += 1;
    await page.waitForTimeout(400);
    await page.screenshot({
      path: `${OUT}/${String(shotIndex).padStart(2, "0")}-${name}-${theme}.png`,
      fullPage,
    });
  }

  try {
    // 1. The ticket's parts & shipping panel (scan bar + kind tabs + search).
    await page.goto(`${WEB}/tickets/${ticket.id}`, { waitUntil: "load" });
    await page.getByTestId("wb-parts-used").click();
    await page.getByTestId("movement-scan").waitFor({ timeout: 30_000 });
    await shot("ticket-parts-panel");

    // 2. A recorded use: receipt on the ticket, movement on the timeline.
    await pickSelect(page, page.getByTestId("movement-item"), item.name);
    await pickSelect(page, page.getByTestId("movement-from-location"), location.name);
    await page.getByTestId("movement-save").click();
    await page.getByTestId("wb-parts-receipt").waitFor({ timeout: 30_000 });
    await page.getByTestId("wb-filter-parts").click();
    await page.getByTestId("wb-row-parts").first().waitFor({ timeout: 30_000 });
    await shot("ticket-parts-receipt");

    // 3. Ship to client with carrier + tracking, then the label panel.
    await page.getByTestId("wb-parts-used").click();
    await page.getByTestId("movement-kind-ship-out").click();
    await pickSelect(page, page.getByTestId("movement-item"), item.name);
    await pickSelect(page, page.getByTestId("movement-from-location"), location.name);
    await page.getByTestId("movement-carrier").fill("UPS");
    await page.getByTestId("movement-tracking").fill(`1Z-CAP-${STAMP}-${theme}`);
    await page.getByTestId("movement-save").click();
    await page.getByTestId("wb-parts-receipt").waitFor({ timeout: 30_000 });
    await page.getByTestId("wb-parts-receipt").getByTestId("label-print-button").click();
    await page.getByTestId("label-print-panel").waitFor({ timeout: 30_000 });
    await shot("ticket-ship-label");

    // 4. The stockroom dispatch board and its dialog with the ticket picker.
    await page.goto(`${WEB}/stock`, { waitUntil: "load" });
    await page.getByTestId("stock-dispatch-scan").waitFor({ timeout: 30_000 });
    await shot("stock-dispatch-board");
    await page.getByTestId("stock-dispatch").click();
    await page.getByTestId("movement-ticket").waitFor({ timeout: 30_000 });
    await pickSelect(page, page.getByTestId("movement-item"), item.name);
    await pickSelect(page, page.getByTestId("movement-ticket"), new RegExp(ticket.title));
    await shot("stock-dispatch-dialog");
    await page.keyboard.press("Escape");

    // 5. Item detail with the print + dispatch actions.
    await page.goto(`${WEB}/stock/${item.id}`, { waitUntil: "load" });
    await page.getByTestId("stock-ledger-table").waitFor({ timeout: 30_000 });
    await shot("stock-item-detail");

    // 6. Locations with the per-location label print.
    await page.goto(`${WEB}/stock/locations`, { waitUntil: "load" });
    await page.getByText(location.name).first().waitFor({ state: "attached", timeout: 30_000 });
    await shot("stock-locations-labels", { fullPage: true });

    // 7. Tools with the scan-to-checkout/check-in bar.
    await page.goto(`${WEB}/stock/tools`, { waitUntil: "load" });
    await page.getByTestId("tools-scan").waitFor({ timeout: 30_000 });
    await shot("stock-tools-scan");

    // 8. Purchase-order receiving: the scan bar, then the post-receipt
    //    dispatch strip.
    await page.goto(`${WEB}/purchasing/${order.order.id}`, { waitUntil: "load" });
    await page.getByTestId("purchasing-receive").click();
    await page.getByTestId("purchasing-receive-scan").waitFor({ timeout: 30_000 });
    await pickSelect(page, page.getByTestId("purchasing-receive-location"), location.name);
    await page.getByTestId("scan-input").fill(`F6-CAP-${STAMP}`);
    await page.getByTestId("scan-input").press("Enter");
    await page.locator('[data-testid^="purchasing-receive-qty-"]').first().waitFor({
      timeout: 30_000,
    });
    await shot("purchasing-receive-scan");
    await page.getByTestId("purchasing-receive-submit").click();
    await page.getByTestId("purchasing-received-dispatch").waitFor({ timeout: 30_000 });
    await shot("purchasing-receipt-dispatch");

    // 9. Mobile mirror at 375×812 (Wave 33 reads this grammar).
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto(`${WEB}/tickets/${ticket.id}`, { waitUntil: "load" });
    await page.getByTestId("wb-parts-used").click();
    await page.getByTestId("movement-scan").waitFor({ timeout: 30_000 });
    await shot("mobile-ticket-panel");
    await page.goto(`${WEB}/stock`, { waitUntil: "load" });
    await page.getByTestId("stock-dispatch-scan").waitFor({ timeout: 30_000 });
    await shot("mobile-stock-dispatch");
  } catch (error) {
    failures.push(`${theme}: ${error.message}`);
  }
  await browser.close();
}

if (failures.length > 0) {
  console.error(`CAPTURE FAILURES:\n${failures.join("\n")}`);
  process.exit(1);
}
console.log(`captures written to ${OUT} (${shotIndex} files)`);
