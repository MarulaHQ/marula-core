// p51-dispatch-surface — the footer capture and measurement leg.
//
// Purpose: prove, on the running stack, that the Dispatch surface draws the
// pack's footer (DESIGN-NOTES.md §B2) and that it wears the template's own
// strip root. The audit that ranked this lane asked "is a footer present" by
// reading the rendered DOM for `.m-surface-footer` (or the pack's `#ft`/`.ft2`)
// and answered no on all 28 driven dispatch states; this leg re-asks that exact
// question, per state, and records the geometry beside the answer so a reader
// can compare the strip with the pack's own 52px footer.
//
// It drives the states the remediation names (item 14: board, week, queue,
// plan, travel, planning), one screenshot each at the pack's 1600x994, plus a
// 1:1 band of the footer itself. Every image is captured at full size; nothing
// is downscaled.
//
// Non-purpose: it does not score the pack against the app. It measures the
// footer region and the density control; the rest of the board is the audit's
// ground.
//
// Usage: node e2e/p51-dispatch-footer.mjs
import { chromium } from "playwright";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const LANE = "design/qa-p51-dispatch-surface";
const OUT = resolve(LANE, "captures");
mkdirSync(OUT, { recursive: true });

const envPath = resolve("e2e/.e2e-stack.env");
if (!existsSync(envPath)) {
  throw new Error("e2e/.e2e-stack.env is missing. Run scripts/e2e-stack.sh up first (never set the ports by hand).");
}
const env = Object.fromEntries(
  readFileSync(envPath, "utf8")
    .split("\n")
    .filter((line) => line.includes("="))
    .map((line) => [line.slice(0, line.indexOf("=")), line.slice(line.indexOf("=") + 1)]),
);
const WEB = process.env.E2E_WEB_URL ?? env.E2E_WEB_URL ?? "http://localhost:3000";
const API = (process.env.E2E_API_URL ?? env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");

const STAFF_EMAIL = "owner@demo.local";
const STAFF_PASSWORD = "demo-password-123";
const VIEWPORT = { width: 1600, height: 994 };

// The AI grant is an ORG setting, so it persists for the rest of the run once
// granted: the no-grant states are driven first so their legend is the pack's
// four-swatch bar, and the granted states come last.
const states = [
  { id: "01-board-rest", route: "/dispatch", theme: "light", anchor: '[data-testid="dispatch-board-root"]' },
  { id: "01-board-rest-dark", route: "/dispatch", theme: "dark", anchor: '[data-testid="dispatch-board-root"]' },
  { id: "10-week-view", route: "/dispatch", theme: "light", anchor: '[data-testid="dispatch-board-root"]', actions: ["clickText:Week"] },
  { id: "11-unassigned-queue", route: "/dispatch", theme: "light", anchor: '[data-testid="dispatch-board-root"]', actions: ["clickText:Unassigned"] },
  { id: "08-density-compact", route: "/dispatch", theme: "light", anchor: '[data-testid="dispatch-board-root"]', density: "Compact" },
  { id: "09-density-dense", route: "/dispatch", theme: "light", anchor: '[data-testid="dispatch-board-root"]', density: "Dense" },
  { id: "07-collisions-banner", route: "/dispatch?demo=collision", theme: "light", anchor: '[data-testid="dispatch-board-root"]' },
  { id: "18-planning-loaded", route: "/dispatch/planning", theme: "light", anchor: '[data-testid="planning-loaded"]' },
  { id: "20-travel-drafts", route: "/dispatch/travel", theme: "light", anchor: '[data-testid="travel-root"]' },
  { id: "02-board-ai-granted", route: "/dispatch", theme: "light", ai: true, anchor: '[data-testid="dispatch-board-root"]' },
  {
    id: "13-plan-review",
    route: "/dispatch",
    theme: "light",
    ai: true,
    anchor: '[data-testid="dispatch-board-root"]',
    actions: ["clickText:Day", "clickText:Plan with AI", "clickTextAny:Everything in the unassigned queue", "clickText:Propose a plan"],
  },
];

/* The AI grants are org settings, not user settings (`p47-conformance.mjs`). */
async function setAiFeatures(ctx, tokens, enabled) {
  await ctx.request.put(`${API}/api/v1/settings/ai/decision/features`, {
    data: { enabled, idempotency_key: `features:p51:${String(Date.now())}:${String(enabled)}` },
    headers: { authorization: `Bearer ${tokens.access_token}` },
  });
}

async function enableAi(ctx, tokens) {
  const auth = { authorization: `Bearer ${tokens.access_token}` };
  await setAiFeatures(ctx, tokens, true);
  const roles = await ctx.request.get(`${API}/api/v1/roles`, { headers: auth });
  let roleId = null;
  if (roles.ok()) {
    const body = await roles.json();
    const rows = Array.isArray(body) ? body : (body.items ?? body.roles ?? []);
    roleId = rows[0]?.id ?? null;
  }
  await ctx.request.put(`${API}/api/v1/settings/ai/grants`, {
    data: {
      surface_key: "decision_dispatch",
      grantee_kind: "role",
      role_id: roleId ?? undefined,
      idempotency_key: `grant:p51:${Date.now()}`,
    },
    headers: auth,
  });
}

async function staffContext(browser, state) {
  const ctx = await browser.newContext({
    viewport: VIEWPORT,
    colorScheme: state.theme === "dark" ? "dark" : "light",
  });
  const res = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: STAFF_EMAIL, password: STAFF_PASSWORD }),
  });
  if (!res.ok) throw new Error(`staff login failed (${res.status})`);
  const tokens = await res.json();
  const login = await ctx.request.post(`${API}/api/v1/auth/login`, {
    data: { email: STAFF_EMAIL, password: STAFF_PASSWORD },
  });
  if (!login.ok()) throw new Error(`context login failed (${login.status()})`);
  await ctx.request.put(`${API}/api/v1/settings/user`, {
    data: { key: "tours.disabled", value: true },
    headers: { authorization: `Bearer ${tokens.access_token}` },
  });
  await ctx.addInitScript(
    ([raw, exp, th]) => {
      window.localStorage.setItem("marula-tokens", raw);
      window.localStorage.setItem("marula-tokens-exp", exp);
      window.localStorage.setItem("marula-theme", th === "dark" ? "dark" : "light");
      window.localStorage.setItem("marula-theme-preset", "marula");
    },
    [JSON.stringify(tokens), String(Date.now() + tokens.expires_in * 1000 - 30_000), state.theme],
  );
  return { ctx, tokens };
}

async function runAction(page, action) {
  const [verb, ...rest] = action.split(":");
  const arg = rest.join(":");
  try {
    if (verb === "clickText") {
      await page.getByText(arg, { exact: false }).first().click({ timeout: 10_000 });
    } else if (verb === "clickTextAny") {
      await page.getByText(arg, { exact: false }).first().click({ timeout: 10_000 });
    } else if (verb === "wait") {
      await page.waitForTimeout(Number(arg));
    }
    return true;
  } catch {
    return false;
  }
}

/** The measurement the audit's footer question resolves to, plus the geometry. */
const MEASURE = () => {
  const surface =
    document.querySelector('[data-testid="dispatch-board-root"]') ??
    document.querySelector('[data-testid="travel-root"]') ??
    document.querySelector('[data-testid="planning-loaded"]') ??
    document.querySelector(".dp") ??
    document.body;
  const f = document.querySelector(".m-surface-footer");
  if (!f) return { footerPresent: false };
  const st = getComputedStyle(f);
  const r = f.getBoundingClientRect();
  const h1 = document.querySelector("h1");
  const dens = f.querySelector(".m-density-switch");
  return {
    footerPresent: true,
    inTheSurface: surface.contains(f),
    root: f.className,
    testid: f.getAttribute("data-testid"),
    box: {
      x: Math.round(r.left),
      y: Math.round(r.top),
      w: Math.round(r.width),
      h: Math.round(r.height),
    },
    fitsViewport: r.bottom <= window.innerHeight + 1 && r.top >= 0,
    alignsWithPageTitle: h1 ? Math.round(r.left) === Math.round(h1.getBoundingClientRect().left) : null,
    height: st.height,
    padding: st.padding,
    borderTopWidth: st.borderTopWidth,
    gap: st.columnGap,
    color: st.color,
    fontSize: st.fontSize,
    legend: [...f.querySelectorAll(".dp-legend > span")].map((s) => s.textContent.trim()),
    density: dens
      ? {
          present: true,
          className: dens.className,
          steps: [...dens.querySelectorAll('[role="radio"]')].map((b) => ({
            label: b.getAttribute("aria-label"),
            checked: b.getAttribute("aria-checked"),
          })),
        }
      : { present: false },
    keys: [...f.querySelectorAll(".dp-kbd")].map((k) => k.textContent.trim()),
  };
};

const browser = await chromium.launch();
const manifest = { generated: new Date().toISOString(), viewport: VIEWPORT, states: [] };

for (const state of states) {
  const { ctx, tokens } = await staffContext(browser, state);
  await setAiFeatures(ctx, tokens, state.ai === true);
  if (state.ai) await enableAi(ctx, tokens);
  const page = await ctx.newPage();
  const consoleErrors = [];
  page.on("console", (m) => {
    if (m.type() === "error") consoleErrors.push(m.text());
  });
  await page.goto(`${WEB}${state.route}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
  if (state.anchor) {
    await page.waitForSelector(state.anchor, { timeout: 30_000 }).catch(() => {});
  }
  for (const action of state.actions ?? []) await runAction(page, action);
  if (state.density) {
    await page.getByRole("radio", { name: state.density }).click({ timeout: 10_000 }).catch(() => {});
  }
  await page.waitForTimeout(500);

  const measured = await page.evaluate(MEASURE);
  const full = resolve(OUT, `${state.id}--${state.theme}-1600x994.png`);
  await page.screenshot({ path: full });
  // The footer band, 1:1, so the delivered strip is legible without a crop tool.
  const box = measured.footerPresent ? measured.box : null;
  if (box) {
    const band = resolve(OUT, `${state.id}--footer-1x.png`);
    await page.screenshot({
      path: band,
      clip: { x: 0, y: Math.max(0, box.y - 4), width: VIEWPORT.width, height: Math.min(box.h + 8, VIEWPORT.height - box.y) },
    });
  }
  manifest.states.push({ ...state, file: full, density: state.density ?? null, measured, consoleErrors });
  const verdict = measured.footerPresent ? (measured.fitsViewport ? "present" : "present-but-clipped") : "ABSENT";
  console.log(`${state.id.padEnd(24)} ${verdict.padEnd(22)} h=${measured.height ?? "-"} padding=${measured.padding ?? "-"} border=${measured.borderTopWidth ?? "-"} align=${measured.alignsWithPageTitle ?? "-"} legend=${(measured.legend ?? []).length} keys=${(measured.keys ?? []).length}`);
  await ctx.close();
}

writeFileSync(resolve(OUT, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
await browser.close();

const absent = manifest.states.filter((s) => !s.measured.footerPresent);
const clipped = manifest.states.filter((s) => s.measured.footerPresent && !s.measured.fitsViewport);
console.log(`\n${manifest.states.length} states driven; ${absent.length} without the footer root; ${clipped.length} clipped.`);
if (absent.length) process.exitCode = 1;
