// Wave 44 lane 8 `p44-ui-insight` — vision-review captures for the six
// Insight surfaces (/reports, /dashboards, /scorecards, /provenance,
// /audit, /search). The area's pack has no dedicated states, so the
// data-surface template (web/tickets) and the Dispatch Planning cards
// govern, per DERIVATION-GUIDE.md. Drives the REAL app against the live
// e2e stack (never mocks) and only reads existing state:
//   01 /reports builder tab (default)
//   02 /reports saved tab — table + one toolbar row + 52px footer
//   03 /reports schedules tab
//   04 /dashboards default presets — the Planning-card composition
//   05 /dashboards widget picker open
//   06 /scorecards default window (30d) — own card + team cards
//   07 /scorecards 7d window — the window IS the view control
//   08 /provenance default trail
//   09 /provenance with a filter clause in the chip row
//   10 /audit default ledger
//   11 /audit one row expanded — structured detail
//   12 /search resting under two characters
//   13 /search grouped results
//   14 /search no-results — points at the query
// Captured in light + dark (28 files, reviewed in ≤8-image batches).
//
// Anti-fake-evidence (p41 lesson): after every navigation the surface is
// checked for load-error copy before the shutter fires; a capture that
// shows an error state without being intended as one is logged loudly and
// flagged, so an error can never pass as content.
//
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p44-insight-captures.mjs
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = "design/qa-p44-ui-insight/captures";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";

mkdirSync(OUT, { recursive: true });

const req = await (await import("playwright")).request.newContext();
const login = await req.post(`${API}/api/v1/auth/login`, { data: { email: EMAIL, password: PASSWORD } });
if (!login.ok()) throw new Error(`login failed: ${login.status()}`);
const { access_token: token } = await login.json();
const auth = (path, init = {}) =>
  req.fetch(`${API}/api/v1${path}`, {
    ...init,
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json", ...(init.headers ?? {}) },
  });

const problems = [];
/** Fails the run loudly if the surface is wearing a load error. */
const assertNoLoadError = async (page, label) => {
  const text = (await page.locator("body").innerText()).toLowerCase();
  const hit = /could not load|did not load|couldn't load|unmatched (get|post|put|patch)/.exec(text);
  if (hit !== null) {
    const line = `[STATE-ERROR] ${label}: surface shows load-error copy "${hit[0]}"`;
    console.error(line);
    problems.push(line);
  }
};

/** Login with the durable-gate retry the e2e helpers use: the auth gate
 * may throttle retries after earlier passes, so back off and re-enter. */
const loginWithRetry = async (page) => {
  for (let attempt = 0; attempt < 4; attempt += 1) {
    await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
    const submit = page.locator('button[type="submit"]');
    await submit.waitFor({ state: "visible", timeout: 30_000 });
    await page.waitForTimeout(1500);
    await page.fill('input[type="email"]', EMAIL);
    await page.fill('input[type="password"]', PASSWORD);
    await submit.click();
    try {
      await page.waitForURL(/tickets|today|dashboard/, { timeout: 45_000 });
      return;
    } catch {
      await page.waitForTimeout(5000);
    }
  }
  throw new Error("login failed after retries (auth gate may be throttling)");
};

/** The stack reseeds onboarding, so the first-run tour covers /today. */
const dismissTour = async (page) => {
  for (let step = 0; step < 10; step += 1) {
    const overlay = page.locator('[data-testid="tour-overlay"]');
    if (!(await overlay.isVisible().catch(() => false))) return;
    const stop = page.getByRole("button", { name: /don't show me tours/i }).first();
    const done = page.getByRole("button", { name: /^done$/i }).first();
    const next = page.getByRole("button", { name: /next/i }).first();
    for (const target of [stop, done, next]) {
      if (await target.isVisible().catch(() => false)) {
        await target.click({ timeout: 3000 }).catch(() => {});
        break;
      }
    }
    await page.waitForTimeout(500);
  }
};

const browser = await chromium.launch();
let shots = 0;
const shot = async (page, theme, name) => {
  await page.screenshot({ path: `${OUT}/${theme}-${name}.png` });
  shots += 1;
  console.log(`[p44-insight] ${theme} · ${name}`);
};

for (const theme of ["light", "dark"]) {
  const ctx = await browser.newContext({ colorScheme: theme, viewport: { width: 1440, height: 1000 } });
  const page = await ctx.newPage();
  page.on("pageerror", (err) => console.error(`[pageerror ${theme}]`, err.message));
  await loginWithRetry(page);

  await page.goto(`${WEB}/today`);
  await page.waitForSelector('[data-testid="tour-overlay"]', { timeout: 15_000 }).catch(() => {});
  await dismissTour(page);
  await page.waitForTimeout(500);

  // 01 /reports builder tab (the default) — the frame around the form.
  await page.goto(`${WEB}/reports`);
  await page.waitForLoadState("networkidle");
  await page.getByRole("tab", { name: "Builder" }).waitFor({ timeout: 15_000 });
  await page.waitForTimeout(900);
  await assertNoLoadError(page, "01 reports builder");
  await shot(page, theme, "01-reports-builder");

  // 02 /reports saved tab — the TL table: one toolbar row, 16px card, 52px footer.
  await page.getByRole("tab", { name: "Saved reports" }).click();
  await page.waitForTimeout(1200);
  await assertNoLoadError(page, "02 reports saved");
  await shot(page, theme, "02-reports-saved");

  // 03 /reports schedules tab.
  await page.getByRole("tab", { name: "Schedules" }).click();
  await page.waitForTimeout(1200);
  await assertNoLoadError(page, "03 reports schedules");
  await shot(page, theme, "03-reports-schedules");

  // 04 /dashboards default — presets on the toolbar row, Planning-card grid.
  await page.goto(`${WEB}/dashboards`);
  await page.waitForLoadState("networkidle");
  await page.getByRole("tablist", { name: "Saved arrangements" }).waitFor({ timeout: 15_000 });
  await page.waitForTimeout(1200);
  await assertNoLoadError(page, "04 dashboards default");
  await shot(page, theme, "04-dashboards-default");

  // 05 /dashboards widget picker open.
  await page.getByRole("button", { name: /add widget/i }).click();
  await page.waitForSelector('[aria-controls="widget-picker"][aria-expanded="true"]', { timeout: 10_000 });
  await page.waitForTimeout(600);
  await assertNoLoadError(page, "05 dashboards picker");
  await shot(page, theme, "05-dashboards-picker");

  // 06 /scorecards 30d — own card + the manager's team cards.
  await page.goto(`${WEB}/scorecards`);
  await page.waitForLoadState("networkidle");
  await page.locator('section[aria-label="Your scorecard"]').waitFor({ timeout: 15_000 });
  await page.waitForTimeout(1000);
  await assertNoLoadError(page, "06 scorecards 30d");
  await shot(page, theme, "06-scorecards-30d");

  // 07 /scorecards 7d — the window IS the view control: switching re-queries.
  await page.getByRole("tab", { name: "Last 7 days" }).click();
  await page.waitForTimeout(1500);
  await assertNoLoadError(page, "07 scorecards 7d");
  await shot(page, theme, "07-scorecards-7d");

  // 08 /provenance default trail.
  await page.goto(`${WEB}/provenance`);
  await page.waitForLoadState("networkidle");
  await page.getByRole("tablist", { name: "Provenance views" }).waitFor({ timeout: 15_000 });
  await page.locator("tbody tr").first().waitFor({ timeout: 15_000 });
  await page.waitForTimeout(800);
  await assertNoLoadError(page, "08 provenance default");
  await shot(page, theme, "08-provenance-default");

  // 09 /provenance with a filter clause — the chip row + its result. The
  // builder's default field is Entity, so the clause is API-valid by
  // construction (the Field option only appears beside an entity one).
  await page.getByRole("button", { name: /add filter/i }).click();
  const builder = page.getByRole("dialog", { name: /add provenance filters/i });
  await builder.waitFor({ timeout: 10_000 });
  await builder.locator("input.m-input").first().fill("no-such-entity");
  await page.keyboard.press("Enter");
  await page.waitForTimeout(1200);
  await assertNoLoadError(page, "09 provenance filtered");
  await shot(page, theme, "09-provenance-filtered");

  // 10 /audit default ledger.
  await page.goto(`${WEB}/audit`);
  await page.waitForLoadState("networkidle");
  await page.locator('[data-testid^="audit-row-"]').first().waitFor({ timeout: 15_000 });
  await page.waitForTimeout(800);
  await assertNoLoadError(page, "10 audit default");
  await shot(page, theme, "10-audit-default");

  // 11 /audit expanded row — the structured detail in place.
  await page.locator('[data-testid^="audit-row-"]').first().click();
  await page.getByText("Event id:").first().waitFor({ timeout: 10_000 });
  await page.waitForTimeout(500);
  await assertNoLoadError(page, "11 audit expanded");
  await shot(page, theme, "11-audit-expanded");

  // 12 /search resting — under two characters, the resting state.
  await page.goto(`${WEB}/search`);
  await page.waitForLoadState("networkidle");
  await page.getByRole("searchbox", { name: "Search" }).waitFor({ timeout: 15_000 });
  await page.waitForTimeout(800);
  await assertNoLoadError(page, "12 search resting");
  await shot(page, theme, "12-search-resting");

  // 13 /search grouped results.
  await page.getByRole("searchbox", { name: "Search" }).fill("demo");
  await page.waitForTimeout(1500);
  await assertNoLoadError(page, "13 search results");
  await shot(page, theme, "13-search-results");

  // 14 /search no-results — the state points at the query.
  await page.getByRole("searchbox", { name: "Search" }).fill("zzqqxx-none");
  await page.waitForTimeout(1500);
  await assertNoLoadError(page, "14 search no-results");
  await shot(page, theme, "14-search-no-results");

  await ctx.close();
}

await browser.close();
if (problems.length > 0) {
  console.error(`[p44-insight] ${problems.length} capture(s) show load errors — NOT content:`);
  for (const p of problems) console.error(`  ${p}`);
  process.exitCode = 2;
}
console.log(`[p44-insight] wrote ${shots} screenshots to ${OUT}`);
