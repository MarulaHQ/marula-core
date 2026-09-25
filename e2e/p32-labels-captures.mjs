// Wave 32 lane 5 `p32-labels-scanning` — vision-review captures.
//
// Drives the REAL app against the live e2e stack (never mocks), staging
// scratch data through /api/v1 so the screenshots show true states:
//   1. Settings → Labels & scanning with a selected template and its
//      server-rendered live preview (QR + Code128 at physical size).
//   2. The ticket Money tab immediately after a synthesized keyboard-wedge
//      burst: the scan chip, the typed kind, and the filled cost line.
//   3. A device asset detail with the label print panel open.
//
// Each surface is captured in light and dark. Run with the stack env
// exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p32-labels-captures.mjs
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = "design/qa-p32-labels-scanning/captures";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const BOARD_ID = "6d657269-6469-616e-0000-00000000003c";
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

// --- scratch data -----------------------------------------------------------
const sku = `CAP-SCAN-${STAMP}`;
const item = await api(`/stock-items`, "POST", {
  name: `Captured scan part ${STAMP}`,
  kind: "consumable",
  sku,
  unit: "each",
  currency: "USD",
  idempotency_key: crypto.randomUUID(),
});
const location = await api(`/stock-locations`, "POST", {
  name: `Capture shelf ${STAMP}`,
  kind: "warehouse",
  code: `CAP-${STAMP}`,
});
await api(`/stock/receive`, "POST", {
  item_id: item.id,
  location_id: location.id,
  qty_milli: 1000,
  unit_cost_minor: 2450,
  idempotency_key: crypto.randomUUID(),
});
// A template so the designer + asset print panel have real content.
await api(`/label-templates`, "POST", {
  name: `Capture serialized unit ${STAMP}`,
  purpose: "serialized_unit",
  size_preset: "zebra_2x1",
  barcode_token: "serial",
  include_qr: true,
  fields: ["name", "serial", "location"],
  is_default: true,
  idempotency_key: crypto.randomUUID(),
});
await api(`/label-templates`, "POST", {
  name: `Capture asset tag ${STAMP}`,
  purpose: "device_asset",
  size_preset: "dymo_multipurpose",
  barcode_token: "tag",
  include_qr: true,
  fields: ["name", "tag"],
  is_default: true,
  idempotency_key: crypto.randomUUID(),
});
const ticket = await api(`/tickets`, "POST", {
  board_id: BOARD_ID,
  title: `Capture scan-to-parts ${STAMP}`,
  priority: "p3",
  idempotency_key: crypto.randomUUID(),
});
const client = await api(`/clients`, "POST", { name: `Capture client ${STAMP}` });
const asset = await api(`/clients/${client.id}/assets`, "POST", {
  tag: `CAP-ASSET-${STAMP}`,
  name: `Captured loan device ${STAMP}`,
  kind: "device",
});

const failures = [];
for (const theme of ["light", "dark"]) {
  const browser = await chromium.launch({ args: ["--no-proxy-server"] });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  // The app authenticates through the httpOnly session cookie, so the login
  // must go through THIS context's request jar (same as the E2E harness).
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
  try {
    // 1. Labels settings + designer preview.
    await page.goto(`${WEB}/settings/labels`, { waitUntil: "networkidle" });
    const rows = page.getByTestId("label-template-row");
    await rows.first().waitFor({ timeout: 30_000 });
    await rows
      .filter({ hasText: "Capture serialized unit" })
      .first()
      .click();
    await page.getByTestId("label-preview-surface").waitFor({ timeout: 20_000 });
    await page.waitForTimeout(600);
    await page.screenshot({ path: join(OUT, `labels-designer-${theme}.png`) });

    // 2. Ticket money tab right after a keyboard-wedge scan.
    await page.goto(`${WEB}/tickets/classic/${ticket.id}`, { waitUntil: "networkidle" });
    await page.getByTestId("detail-tab-money").click();
    await page.getByTestId("scan-bar").waitFor({ timeout: 30_000 });
    const desc = page.locator("#cl-desc");
    await desc.fill("note ");
    await desc.pressSequentially(sku, { delay: 8 });
    await desc.press("Enter");
    await page.getByTestId("scan-chip-summary").waitFor({ timeout: 20_000 });
    await page.locator("#cl-unit").waitFor({ timeout: 15_000 });
    await page.waitForTimeout(600);
    await page.screenshot({ path: join(OUT, `ticket-scan-${theme}.png`) });

    // 3. Asset detail with the label print panel open.
    await page.goto(`${WEB}/assets/${asset.id}`, { waitUntil: "networkidle" });
    await page.getByTestId("label-print-button").click();
    await page.getByTestId("label-print-panel").waitFor({ timeout: 20_000 });
    await page.getByTestId("label-preview-surface").waitFor({ timeout: 20_000 });
    await page.waitForTimeout(600);
    await page.screenshot({ path: join(OUT, `asset-label-print-${theme}.png`) });
  } catch (err) {
    failures.push(`${theme}: ${err.message ?? err}`);
  }
  await ctx.close();
  await browser.close();
}

await ctx0.dispose();
if (failures.length > 0) {
  console.error("capture misses:", failures.join(" | "));
  process.exitCode = 1;
} else {
  console.log(`captures written to ${OUT}`);
}
