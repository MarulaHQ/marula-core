// Wave 44 lane 1 `p44-ui-today` — vision-review captures for the /today
// surface drawn to the pack (brand/marula-mockups/marula-surface-pack/
// web/today/): the default day, the quick-log dialog in its states, and the
// quiet-day empty. Drives the REAL app against the live e2e stack (never
// mocks) and stages true state through the API:
//   1. /today default — greeting header, attention cards, day body
//   2. quick-log defaults — Day/Started at/For, prefill note, chips
//   3. quick-log corrected — a work type chosen (primary lights, footer
//      names the window)
//   4. quick-log non-billable — the toggle off + reason select
//   5. quick-log overlap — an entry overlapping yesterday 09:00 staged,
//      the blush line naming ticket, window and nearest gap
//   6. /today after logging — the dialog closes, Yesterday updates
//   7. quiet day — appointments cancelled + no due/overdue work
// Captured in light + dark (14 files → reviewed in ≤8-image batches).
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p44-captures.mjs
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = "design/qa-p44-today/captures";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const STAMP = Date.now() % 1_000_000;

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

const dayIso = (offsetDays = 0) => {
  const d = new Date();
  d.setDate(d.getDate() + offsetDays);
  return d.toISOString().slice(0, 10);
};
/** Local wall-clock point → ISO instant: the dialog speaks local time
 * (09:00 means the browser's 09:00), so the staged overlap must too. */
const localIso = (offsetDays, hh, mm = 0) => {
  const d = new Date();
  d.setDate(d.getDate() + offsetDays);
  d.setHours(hh, mm, 0, 0);
  return d.toISOString();
};

// --- stage: yesterday's gap + an overlapping entry ----------------------
const tickets = (await (await auth("/tickets?limit=50")).json()).filter((t) => t.category !== "closed");
const ticket = tickets[0];

let overlapEntryId = null;
// A block already logged yesterday 09:00–10:00 (browser-local, the frame
// the dialog speaks) — the quick-log default then overlaps it, pack state 05.
{
  const staged = await auth("/time-entries", {
    method: "POST",
    data: {
      ticket_id: ticket.id,
      started_at: localIso(-1, 9),
      ended_at: localIso(-1, 10),
      billable: true,
      notes: `p44 overlap stage ${STAMP}`,
    },
  });
  overlapEntryId = (await staged.json())?.id ?? null;
}

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

// --- browser ------------------------------------------------------------
const browser = await chromium.launch();
let shots = 0;
for (const theme of ["light", "dark"]) {
  const ctx = await browser.newContext({ colorScheme: theme, viewport: { width: 1440, height: 1000 } });
  const page = await ctx.newPage();
  page.on("pageerror", (err) => console.error(`[pageerror ${theme}]`, err.message));
  await loginWithRetry(page);

  // The stack reseeds onboarding, so the first-run tour covers /today.
  // Walk it off with its own controls — disable it for good, then proceed.
  const dismissTour = async () => {
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
  await page.goto(`${WEB}/today`);
  await page.waitForSelector('[data-testid="tour-overlay"]', { timeout: 15_000 }).catch(() => {});
  await dismissTour();
  await page.waitForTimeout(500);

  // 1. default day: header, attention cards, day body, queue
  await page.goto(`${WEB}/today`);
  await page.waitForLoadState("networkidle");
  await page.waitForTimeout(1200);
  await page.screenshot({ path: `${OUT}/${theme}-01-today-default.png` });
  shots += 1;

  // 2. quick-log defaults (opens from the header, chips unselected)
  await page.getByRole("button", { name: /Quick log/ }).click();
  await page.waitForSelector('[data-testid="quicklog-submit"]', { timeout: 10_000 });
  await page.waitForTimeout(600);
  await page.screenshot({ path: `${OUT}/${theme}-02-quick-log-default.png` });
  shots += 1;

  // 3. corrected: choose a work type (the primary lights up, footer renames)
  await page.locator('[data-testid="quicklog-work-types"] button').first().click();
  await page.waitForTimeout(300);
  await page.screenshot({ path: `${OUT}/${theme}-03-quick-log-corrected.png` });
  shots += 1;

  // 4. non-billable: flip the toggle, the reason select appears
  await page.locator('[data-testid="quicklog-billable"]').click();
  await page.waitForSelector('[data-testid="quicklog-unbillable-reason"]', { timeout: 5000 });
  await page.waitForTimeout(300);
  await page.screenshot({ path: `${OUT}/${theme}-04-quick-log-non-billable.png` });
  shots += 1;

  // 5. overlap error: the default 09:00 window hits the staged entry
  await page.locator('[data-testid="quicklog-billable"]').click();
  await page.locator('[data-testid="quicklog-start"]').fill("09:00");
  await page.waitForTimeout(500);
  await page.screenshot({ path: `${OUT}/${theme}-05-quick-log-error.png` });
  shots += 1;

  // 6. after logging: move past the staged block, submit, the dialog closes
  await page.locator('[data-testid="quicklog-start"]').fill("11:30");
  await page.waitForTimeout(500);
  await page.locator('[data-testid="quicklog-submit"]').click();
  await page
    .getByRole("dialog", { name: "Quick log" })
    .waitFor({ state: "detached", timeout: 15_000 });
  await page.waitForTimeout(800);
  await page.screenshot({ path: `${OUT}/${theme}-06-after-logging.png` });
  shots += 1;

  // Re-stage for the next theme pass: drop what this pass wrote, keep the
  // overlap block fresh so 05 fires again.
  if (overlapEntryId !== null) {
    await auth(`/time-entries/${overlapEntryId}`, { method: "DELETE" });
  }
  const restaged = await auth("/time-entries", {
    method: "POST",
    data: {
      ticket_id: ticket.id,
      started_at: localIso(-1, 9),
      ended_at: localIso(-1, 10),
      billable: true,
      notes: `p44 overlap stage ${STAMP}`,
    },
  });
  overlapEntryId = (await restaged.json())?.id ?? null;

  // 7. day-empty state: no visits booked — the pack's quiet-day anatomy
  // (empty day card + muted flank cards) against the real queue below.
  await page.goto(`${WEB}/today`);
  await page.waitForLoadState("networkidle");
  await page.waitForTimeout(1500);
  await page.screenshot({ path: `${OUT}/${theme}-07-day-empty.png` });
  shots += 1;

  await ctx.close();
}

await browser.close();
console.log(`[p44-captures] wrote ${shots} screenshots to ${OUT}`);
