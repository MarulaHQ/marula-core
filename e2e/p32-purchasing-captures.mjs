// Wave 32 lane 4 `p32-purchasing-ui` — vision-review captures.
//
// Drives the REAL app against the live e2e stack (never mocks), staging
// scratch data through /api/v1 so the screenshots show true states: a
// low-stock banner fed by real thresholds, a receipt with its ledger rows,
// a checked-out tool with a due date, an open loan, parts used on a ticket
// timeline, and a project cost rollup fed by a posted bill.
//
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p32-purchasing-captures.mjs
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = "design/qa-p32-purchasing-ui/captures";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const STAMP = Date.now() % 1_000_000;

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

const me = await api(`/me`, "GET");

// --- scratch data -----------------------------------------------------------
const location = await api(`/stock-locations`, "POST", {
  name: `Capture dock ${STAMP}`,
  kind: "warehouse",
  code: `CAP-${STAMP}`,
});
const itemA = await api(`/stock-items`, "POST", {
  name: `Capture RJ45 stock ${STAMP}`,
  kind: "consumable",
  unit: "each",
  currency: "USD",
  is_tool: false,
  idempotency_key: crypto.randomUUID(),
});
await api(`/stock/receive`, "POST", {
  item_id: itemA.id,
  location_id: location.id,
  qty_milli: 3000,
  unit_cost_minor: 1250,
  idempotency_key: crypto.randomUUID(),
});
const itemLow = await api(`/stock-items`, "POST", {
  name: `Capture splices low ${STAMP}`,
  kind: "consumable",
  unit: "each",
  currency: "USD",
  is_tool: false,
  idempotency_key: crypto.randomUUID(),
});
await api(`/stock/receive`, "POST", {
  item_id: itemLow.id,
  location_id: location.id,
  qty_milli: 4000,
  idempotency_key: crypto.randomUUID(),
});
await api(`/stock/levels/thresholds`, "POST", {
  item_id: itemLow.id,
  location_id: location.id,
  min_qty_milli: 10000,
  reorder_point_milli: 10000,
  idempotency_key: crypto.randomUUID(),
});
const tool = await api(`/stock-items`, "POST", {
  name: `Capture torque drill ${STAMP}`,
  kind: "serialized",
  unit: "each",
  currency: "USD",
  is_tool: true,
  idempotency_key: crypto.randomUUID(),
});
const serial = `SN-CAP-${STAMP}`;
await api(`/stock/receive`, "POST", {
  item_id: tool.id,
  location_id: location.id,
  qty_milli: 1000,
  serial_numbers: [serial],
  idempotency_key: crypto.randomUUID(),
});
const serials = await api(`/stock-items/${tool.id}/serials?status=in_stock`, "GET");
const serialId = serials[0]?.id;
const tool2 = await api(`/stock-items`, "POST", {
  name: `Capture inspection cam ${STAMP}`,
  kind: "serialized",
  unit: "each",
  currency: "USD",
  is_tool: true,
  idempotency_key: crypto.randomUUID(),
});
await api(`/stock/receive`, "POST", {
  item_id: tool2.id,
  location_id: location.id,
  qty_milli: 1000,
  serial_numbers: [`SN-CAP-B-${STAMP}`],
  idempotency_key: crypto.randomUUID(),
});
const serials2 = await api(`/stock-items/${tool2.id}/serials?status=in_stock`, "GET");
const serial2Id = serials2[0]?.id;

const vendor = await api(`/vendors`, "POST", {
  name: `Capture vendor ${STAMP}`,
  default_currency: "USD",
});
const vendorLinks = await api(`/vendors/${vendor.id}/links`, "GET");
const projects = await api(`/projects`, "GET");
const projectId = projects[0]?.id;
const order = await api(`/purchase-orders`, "POST", {
  vendor_id: vendor.id,
  currency: "USD",
  project_id: projectId ?? null,
  lines: [
    { description: `Capture patch panel ${STAMP}`, qty_ordered_milli: 2000, unit_cost_minor: 4250, stock_item_id: itemA.id },
  ],
  idempotency_key: crypto.randomUUID(),
});
const orderId = order.order.id;
await api(`/purchase-orders/${orderId}/order`, "POST", {}, "POST");
await api(`/purchase-orders/${orderId}/receive`, "POST", {
  location_id: location.id,
  lines: [{ line_id: order.lines[0].id, qty_milli: 2000 }],
  carrier: "UPS",
  tracking_ref: `1Z-CAP-${STAMP}`,
  idempotency_key: crypto.randomUUID(),
});
await api(`/purchase-orders/${orderId}/links`, "POST", {
  kind: "order_portal",
  url: `https://portal.example.test/orders/${STAMP}`,
  label: "Vendor portal",
  idempotency_key: crypto.randomUUID(),
});
await api(`/purchase-orders/${orderId}/bill`, "PUT", {
  amount_minor: 8500,
  bill_number: `BILL-${STAMP}`,
  currency: "USD",
  idempotency_key: crypto.randomUUID(),
});
await api(`/purchase-orders/${orderId}/bill/post`, "POST", {}, "POST");

const clients = await api(`/clients`, "GET");
const client = clients[0];
const tomorrow = new Date(Date.now() + 86_400_000);
const dueDate = `${tomorrow.getFullYear()}-${String(tomorrow.getMonth() + 1).padStart(2, "0")}-${String(tomorrow.getDate()).padStart(2, "0")}`;
const dueEnd = new Date(`${dueDate}T23:59:59`).toISOString();
await api(`/tool-checkouts`, "POST", {
  holder_user_id: me.user_id,
  from_location_id: location.id,
  due_at: dueEnd,
  items: [{ stock_item_id: tool.id, serial_id: serialId }],
  idempotency_key: crypto.randomUUID(),
});
await api(`/client-loans`, "POST", {
  client_id: client.id,
  expected_return_at: dueDate,
  items: [{ stock_item_id: tool2.id, serial_id: serial2Id }],
  agreement_ref: `AGR-${STAMP}`,
  idempotency_key: crypto.randomUUID(),
});

const BOARD_ID = "6d657269-6469-616e-0000-00000000003c";
const ticket = await api(`/tickets`, "POST", {
  board_id: BOARD_ID,
  title: `Capture parts used ${STAMP}`,
  priority: "p3",
  idempotency_key: crypto.randomUUID(),
});
await api(`/stock/consume`, "POST", {
  item_id: itemA.id,
  location_id: location.id,
  qty_milli: 3000,
  ticket_id: ticket.id,
  note: "Terminated the patch panel drops",
  idempotency_key: crypto.randomUUID(),
});

async function pickSelect(page, trigger, option) {
  await trigger.click();
  await page.getByRole("option", { name: option }).click();
}

let shotIndex = 0;
const failures = [];
for (const theme of ["light", "dark"]) {
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
      window.__MERIDIAN_API_URL__ = apiBase;
      window.localStorage.setItem("meridian-tokens", tokenPair);
      window.localStorage.setItem("meridian-tokens-exp", String(Date.now() + 3_600_000));
      window.localStorage.setItem("meridian-theme", mode);
    },
    [API, JSON.stringify(tokens), theme],
  );

  async function shot(name) {
    shotIndex += 1;
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${OUT}/${String(shotIndex).padStart(2, "0")}-${name}-${theme}.png`, fullPage: false });
  }

  try {
    // 1. PO browser.
    await page.goto(`${WEB}/purchasing`, { waitUntil: "load" });
    await page.getByTestId("purchasing-table").waitFor({ timeout: 30_000 });
    await shot("purchasing-browser");

    // 2. PO detail (receipts, deeplink, bill).
    await page.locator('[data-testid^="purchasing-open-"]').first().click();
    await page.getByTestId("purchasing-order-number").waitFor({ timeout: 30_000 });
    await page.getByTestId("purchasing-receipts").waitFor({ timeout: 30_000 });
    await shot("purchasing-order-detail");

    // 3. Vendors.
    await page.goto(`${WEB}/purchasing/vendors`, { waitUntil: "load" });
    await page.getByTestId("purchasing-vendors-table").waitFor({ timeout: 30_000 });
    await shot("purchasing-vendors");

    // 4. Stock board with the low banner.
    await page.goto(`${WEB}/stock`, { waitUntil: "load" });
    await page.getByTestId("stock-low-banner").waitFor({ timeout: 30_000 });
    await shot("stock-board");

    // 5. Movement dialog over the board.
    await page.locator('[data-testid^="stock-receive-"]').first().click();
    await page.getByTestId("movement-item").waitFor({ timeout: 30_000 });
    await shot("stock-movement-dialog");
    await page.keyboard.press("Escape");

    // 6. Item detail: levels, thresholds, ledger.
    await page.goto(`${WEB}/stock/${itemA.id}`, { waitUntil: "load" });
    await page.getByTestId("stock-ledger-table").waitFor({ timeout: 30_000 });
    await shot("stock-item-detail");

    // 7. Tools checkout board.
    await page.goto(`${WEB}/stock/tools`, { waitUntil: "load" });
    await page.getByTestId("tools-table").waitFor({ timeout: 30_000 });
    await shot("stock-tools");

    // 8. Loan register.
    await page.goto(`${WEB}/stock/loans`, { waitUntil: "load" });
    await page.getByTestId("loans-table").waitFor({ timeout: 30_000 });
    await shot("stock-loans");

    // 9. Quote builder with the availability read.
    await page.goto(`${WEB}/sales/quotes`, { waitUntil: "load" });
    await page.getByTestId("quotes-new").click();
    await pickSelect(page, page.getByTestId("quote-client"), client.name);
    await pickSelect(
      page,
      page.locator('[data-testid^="quote-line-stock-"]').first(),
      itemA.name,
    );
    await page.locator('[data-testid^="quote-line-availability-"]').first().waitFor({ timeout: 30_000 });
    await shot("quote-pull-from-stock");
    await page.keyboard.press("Escape");

    // 10. Ticket timeline with the Parts row.
    await page.goto(`${WEB}/tickets/${ticket.id}`, { waitUntil: "load" });
    await page.getByTestId("wb-filter-parts").waitFor({ timeout: 30_000 });
    await page.getByTestId("wb-filter-parts").click();
    await page.getByTestId("wb-row-parts").first().waitFor({ timeout: 30_000 });
    await shot("ticket-parts-timeline");

    // 11. Project cost rollup.
    if (projectId !== undefined) {
      await page.goto(`${WEB}/projects/${projectId}`, { waitUntil: "load" });
      await page.getByTestId("project-cost-rollup").waitFor({ timeout: 30_000 });
      await shot("project-cost-rollup");
    }

    // 12. Mobile stock board.
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto(`${WEB}/stock`, { waitUntil: "load" });
    await page.getByTestId("stock-low-banner").waitFor({ timeout: 30_000 });
    await shot("mobile-stock-board");
    await page.goto(`${WEB}/stock/tools`, { waitUntil: "load" });
    await page.getByTestId("tools-table").waitFor({ timeout: 30_000 });
    await shot("mobile-stock-tools");
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
