// Wave 32 lane 8 `p32-today-day-pack` — vision-review captures.
//
// Drives the REAL app against the live e2e stack (never mocks), staging
// scratch data through /api/v1 so the screenshots show true states: the /today
// day pack split into trip segments (an office return re-opens a collect
// point), the consolidated grab list with per-line provenance (pin, org rule),
// where-to-grab sources, availability with holders and due dates, a
// "Not stocked" hole, the tomorrow preview for a direct-to-site start, the
// ticket pin dialog with its strip, the toolbox config surface, and the
// mobile viewport mirror.
//
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p32-day-pack-captures.mjs
import { chromium, request } from "playwright";
import { mkdirSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = process.env.CAPTURE_OUT ?? "design/qa-p32-today-day-pack/captures";
const THEMES = (process.env.CAPTURE_THEMES ?? "light,dark").split(",");
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const STAMP = Date.now() % 1_000_000;
// The desktop app's embedded API rejects seeded refresh tokens (its own
// login works), so desktop runs log in through the form instead.
const UI_LOGIN = process.env.CAPTURE_LOGIN === "ui";
const BOARD_ID = "6d657269-6469-616e-0000-00000000003c";

mkdirSync(OUT, { recursive: true });

const ctx0 = await request.newContext();
const login = await ctx0.post(`${API}/api/v1/auth/login`, {
  data: { email: EMAIL, password: PASSWORD },
});
if (!login.ok()) throw new Error(`login failed: ${login.status()}`);
const tokens = await login.json();

async function api(path, method, body) {
  const res = await ctx0.fetch(`${API}/api/v1${path}`, {
    method,
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${tokens.access_token}`,
    },
    data: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok()) throw new Error(`${method} ${path} → ${res.status()}: ${await res.text()}`);
  return res.headers()["content-type"]?.includes("json") ? res.json() : undefined;
}

const day = new Date().toISOString().slice(0, 10);
const tomorrow = new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString().slice(0, 10);

// --- scratch data -----------------------------------------------------------
const me = await api(`/me`, "GET");
const users = await api(`/org/users?limit=50`, "GET");
const other = (users.users ?? []).find((user) => user.id !== me.user_id);

const location = await api(`/stock-locations`, "POST", {
  name: `Capture pack shelf ${STAMP}`,
  kind: "warehouse",
});

const ends = await api(`/stock-items`, "POST", {
  name: `Capture pack RJ45 ends ${STAMP}`,
  kind: "consumable",
  sku: `P8-ENDS-${STAMP}`,
  unit: "each",
  currency: "USD",
  idempotency_key: crypto.randomUUID(),
});
await api(`/stock/receive`, "POST", {
  item_id: ends.id,
  location_id: location.id,
  qty_milli: 25_000,
  idempotency_key: crypto.randomUUID(),
});

// No stock anywhere: the honest "Not stocked" hole.
const jacks = await api(`/stock-items`, "POST", {
  name: `Capture pack keystone jacks ${STAMP}`,
  kind: "consumable",
  sku: `P8-JACKS-${STAMP}`,
  unit: "each",
  currency: "USD",
  idempotency_key: crypto.randomUUID(),
});

// A tool whose only unit is with someone else: "None available" plus holder.
const crimper = await api(`/stock-items`, "POST", {
  name: `Capture pack crimper ${STAMP}`,
  kind: "serialized",
  is_tool: true,
  sku: `P8-CRIMP-${STAMP}`,
  unit: "each",
  currency: "USD",
  idempotency_key: crypto.randomUUID(),
});
await api(`/stock/receive`, "POST", {
  item_id: crimper.id,
  location_id: location.id,
  qty_milli: 1000,
  serial_numbers: [`P8-${STAMP}`],
  idempotency_key: crypto.randomUUID(),
});
let checkoutId = null;
if (other !== undefined) {
  const checkout = await api(`/tool-checkouts`, "POST", {
    holder_user_id: other.id,
    from_location_id: location.id,
    due_at: new Date(Date.now() + 2 * 24 * 60 * 60 * 1000).toISOString(),
    items: [{ stock_item_id: crimper.id }],
    idempotency_key: crypto.randomUUID(),
  });
  checkoutId = checkout.checkout.id;
}

// Rule-provenance line: the board recommends a label roll on every ticket.
const labels = await api(`/stock-items`, "POST", {
  name: `Capture pack label roll ${STAMP}`,
  kind: "consumable",
  sku: `P8-LABELS-${STAMP}`,
  unit: "box",
  currency: "USD",
  idempotency_key: crypto.randomUUID(),
});
await api(`/stock/receive`, "POST", {
  item_id: labels.id,
  location_id: location.id,
  qty_milli: 6_000,
  idempotency_key: crypto.randomUUID(),
});
let ruleId = null;

const ticket = await api(`/tickets`, "POST", {
  board_id: BOARD_ID,
  title: `Capture day pack ticket ${STAMP}`,
  priority: "p3",
  idempotency_key: crypto.randomUUID(),
});

const kit = await api(`/tool-kits`, "POST", {
  name: `Capture pack field kit ${STAMP}`,
  code: `P8-KIT-${STAMP}`,
  idempotency_key: crypto.randomUUID(),
});
await api(`/tool-kits/${kit.id}/members`, "POST", {
  stock_item_id: crimper.id,
  qty_milli: 1000,
  idempotency_key: crypto.randomUUID(),
});

const rule = await api(`/day-pack/rules`, "POST", {
  scope_kind: "board",
  scope_id: BOARD_ID,
  target_kind: "item",
  stock_item_id: labels.id,
  qty_milli: 1000,
  note: `Board default ${STAMP}`,
  idempotency_key: crypto.randomUUID(),
});
ruleId = rule.rule.id;

const pickEnds = await api(`/day-pack/picks`, "POST", {
  ticket_id: ticket.id,
  stock_item_id: ends.id,
  qty_milli: 30000,
  note: `Capture pack ${STAMP}`,
  idempotency_key: crypto.randomUUID(),
});
const pickJacks = await api(`/day-pack/picks`, "POST", {
  ticket_id: ticket.id,
  stock_item_id: jacks.id,
  qty_milli: 2000,
  idempotency_key: crypto.randomUUID(),
});

const clients = await api(`/clients?limit=1`, "GET");
const client = clients[0];
const siteA = await api(`/clients/${client.id}/sites`, "POST", {
  name: `Capture pack North ${STAMP}`,
  latitude: 52.5,
  longitude: 13.4,
});
const siteB = await api(`/clients/${client.id}/sites`, "POST", {
  name: `Capture pack South ${STAMP}`,
  latitude: 52.52,
  longitude: 13.45,
});

const start = `${day}T06:00:00.000Z`;
await api(`/dispatch/journey-start`, "PUT", {
  user_id: me.user_id,
  date: day,
  kind: "site",
  site_id: siteA.id,
  label: `Capture start ${STAMP}`,
});
await api(`/dispatch/journey-start`, "PUT", {
  user_id: me.user_id,
  date: tomorrow,
  kind: "site",
  site_id: siteA.id,
  label: `Capture start ${STAMP}`,
});
const appointmentA = await api(`/appointments`, "POST", {
  title: `Capture pack A ${STAMP}`,
  ticket_id: ticket.id,
  client_id: client.id,
  site_id: siteA.id,
  kind: "on_site",
  starts_at: start,
  ends_at: `${day}T07:00:00.000Z`,
});
const appointmentB = await api(`/appointments`, "POST", {
  title: `Capture pack B ${STAMP}`,
  ticket_id: ticket.id,
  client_id: client.id,
  site_id: siteB.id,
  kind: "on_site",
  starts_at: `${day}T07:00:00.000Z`,
  ends_at: `${day}T08:00:00.000Z`,
});
// Office return: pin today's start at the office, then re-save the second
// stop so its recorded leg leaves the office. Trip 2 opens at base.
await api(`/dispatch/journey-start`, "PUT", {
  user_id: me.user_id,
  date: day,
  kind: "office",
  label: `Capture office ${STAMP}`,
  address: `Capture office ${STAMP}`,
  latitude: 52.3,
  longitude: 13.2,
});
await api(`/appointments/${appointmentB.id}`, "PATCH", {
  title: `Capture pack B ${STAMP} (resaved)`,
});
const tomorrowStop = await api(`/appointments`, "POST", {
  title: `Capture pack tomorrow ${STAMP}`,
  ticket_id: ticket.id,
  client_id: client.id,
  site_id: siteA.id,
  kind: "on_site",
  starts_at: `${tomorrow}T06:00:00.000Z`,
  ends_at: `${tomorrow}T07:00:00.000Z`,
});

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
    ([apiBase, tokenPair, mode, uiLogin]) => {
      if (apiBase) window.__MARULA_API_URL__ = apiBase;
      if (!uiLogin) {
        window.localStorage.setItem("meridian-tokens", tokenPair);
        window.localStorage.setItem("meridian-tokens-exp", String(Date.now() + 3_600_000));
      }
      window.localStorage.setItem("meridian-theme", mode);
      window.sessionStorage.setItem("meridian-tour-session", "1");
    },
    [API, JSON.stringify(tokens), theme, UI_LOGIN],
  );
  async function uiLogin() {
    // The login form re-mounts under the auth store's first paint, so a
    // single fill can land on a detached node. Three attempts, each on a
    // fresh load, with keyboard Enter (no held element handle).
    for (let attempt = 0; attempt < 4; attempt += 1) {
      await page.goto(`${WEB}/login`, { waitUntil: "load" });
      await page.waitForTimeout(1500);
      // The context login already set the httpOnly session cookie, so the
      // app can restore itself and /login forwards to the app shell.
      if (!page.url().includes("/login")) {
        await page.goto(`${WEB}/today`, { waitUntil: "load" });
        await page.waitForTimeout(1500);
        return;
      }
      try {
        await page.locator('input[type="email"]').fill(EMAIL, { timeout: 8_000 });
        await page.locator('input[type="password"]').fill(PASSWORD, { timeout: 8_000 });
        await page.keyboard.press("Enter");
        await page.waitForURL(/\/today/, { timeout: 20_000 });
        await page.waitForTimeout(2000);
        return;
      } catch {
        await page.waitForTimeout(1200);
      }
    }
    throw new Error("ui login failed after 4 attempts");
  }

  async function shot(name, { fullPage = false } = {}) {
    shotIndex += 1;
    await page.waitForTimeout(300);
    await page.screenshot({
      path: `${OUT}/${String(shotIndex).padStart(2, "0")}-${name}-${theme}.png`,
      fullPage,
    });
  }

  async function shotElement(pageRef, locator, name) {
    shotIndex += 1;
    await locator.scrollIntoViewIfNeeded();
    await pageRef.waitForTimeout(250);
    await locator.screenshot({
      path: `${OUT}/${String(shotIndex).padStart(2, "0")}-${name}-${theme}.png`,
    });
  }

  try {
    if (UI_LOGIN && !page.url().includes("/today")) await uiLogin();

    // 1. The day pack: trip segments with the office chip and the counts.
    await page.goto(`${WEB}/today`, { waitUntil: "load" });
    const panel = page.getByTestId("day-pack");
    await panel.waitFor({ timeout: 30_000 });
    await page.getByTestId("day-pack-office-chip").first().waitFor({ timeout: 30_000 });
    await shotElement(page, panel, "day-pack-trips");

    // 2. Trip one: the consolidated grab lines with provenances, sources,
    // availability, holders, and the honest "Not stocked" hole.
    await shotElement(page, page.getByTestId("day-pack-trip-1"), "day-pack-lines");

    // 3. Trip two: the office return re-opens a collect point.
    await shotElement(page, page.getByTestId("day-pack-trip-2"), "day-pack-office-return");

    // 4. Tomorrow's list tonight (direct-to-site start).
    await shotElement(page, page.getByTestId("day-pack-tomorrow"), "day-pack-tomorrow");

    // 5. The ticket's pins strip and the pin dialog.
    await page.goto(`${WEB}/tickets/${ticket.id}`, { waitUntil: "load" });
    await page.getByTestId("wb-day-pack-pins").waitFor({ timeout: 30_000 });
    await page.getByTestId("wb-pin-day-pack").click();
    await page.getByTestId("day-pack-pick-dialog").waitFor({ timeout: 30_000 });
    await shot("day-pack-pin-dialog");
    await page.keyboard.press("Escape");

    // 6. The toolbox config surface with the live rule row.
    await page.goto(`${WEB}/settings/toolboxes`, { waitUntil: "load" });
    await page.getByTestId("dispatch-rules").waitFor({ timeout: 30_000 });
    await shot("toolbox-config", { fullPage: true });

    // 7. The mobile mirror of the panel.
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto(`${WEB}/today`, { waitUntil: "load" });
    await page.getByTestId("day-pack").waitFor({ timeout: 30_000 });
    await shot("day-pack-mobile");
  } catch (error) {
    failures.push(`${theme}: ${String(error)}`);
  } finally {
    await browser.close();
  }
}

// --- restore -----------------------------------------------------------------
for (const id of [appointmentA.id, appointmentB.id, tomorrowStop.id]) {
  await api(`/appointments/${id}`, "PATCH", { status: "cancelled" }).catch(() => undefined);
}
for (const id of [pickEnds.pick.id, pickJacks.pick.id]) {
  await api(`/day-pack/picks/${id}`, "DELETE").catch(() => undefined);
}
if (ruleId !== null) {
  await api(`/day-pack/rules/${ruleId}`, "DELETE").catch(() => undefined);
}
if (checkoutId !== null) {
  await api(`/tool-checkouts/${checkoutId}/checkin`, "POST", { lines: [] }).catch(
    () => undefined,
  );
}
if (kit?.id) {
  await api(`/tool-kits/${kit.id}`, "PATCH", { active: false }).catch(() => undefined);
}
for (const id of [ends.id, jacks.id, crimper.id, labels.id]) {
  await api(`/stock-items/${id}/archive`, "POST", {}).catch(() => undefined);
}

console.log(`captures: ${shotIndex} files in ${OUT}`);
if (failures.length > 0) {
  console.error(failures.join("\n"));
  process.exit(1);
}
