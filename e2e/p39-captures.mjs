// Wave 39 lane 1 `p39-time-categories` — vision-review captures.
//
// Drives the REAL app against the live e2e stack (never mocks) so the
// screenshots show true states of the categorization backbone:
//   1. Today → quick-log dialog: the category picker fed by the org
//      taxonomy, the billable toggle, and the typed unbillable reason
//      select that only appears once time is marked non-billable.
//   2. Today with an audited entry: the read-only billing-audit chip
//      above the form, proving it renders as a suggestion (clicking it
//      prefills, it never writes).
//   3. Approvals queue: a row carrying its category label, its
//      non-billable reason badge, and its per-row audit chip.
//   4. Settings → time capture defaults: the ordered taxonomy the
//      capture surfaces read.
//
// Each surface is captured in light and dark (8 files total, one batch).
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p39-captures.mjs
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = "design/qa-p39-time-categories/captures";
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

// --- stage: a category the admin just created, + an audited entry --------
const cats = await (await auth("/time-categories?include_retired=true")).json();
const hasE2e = cats.categories.some((c) => c.slug === `capture_${STAMP}`);
if (!hasE2e) {
  await auth("/time-categories", {
    method: "POST",
    data: {
      slug: `capture_${STAMP}`,
      label: "Field Service",
      description: "On-site reactive work",
      default_billable: true,
      rate_card_affinity: true,
      idempotency_key: `p39-capture-cat-${STAMP}`,
    },
  });
}

const tickets = (await (await auth("/tickets?limit=50")).json()).filter((t) => t.category !== "closed");
const ticket = tickets[0];
const started = new Date(Date.now() - 7200_000).toISOString();
const ended = new Date(Date.now() - 3600_000).toISOString();

const audited = await (
  await auth("/time-entries", {
    method: "POST",
    data: { ticket_id: ticket.id, started_at: started, ended_at: ended, billable: true, work_type: "remote", notes: `p39 capture ${STAMP}` },
  })
).json();

// A non-billable warranty entry: the approvals row then shows BOTH the
// category label and the typed reason badge.
const warranty = await (
  await auth("/time-entries", {
    method: "POST",
    data: {
      ticket_id: ticket.id,
      started_at: new Date(Date.now() - 14_400_000).toISOString(),
      ended_at: new Date(Date.now() - 10_800_000).toISOString(),
      billable: false,
      work_type: "warranty",
      unbillable_reason: "warranty",
      notes: `p39 warranty capture ${STAMP}`,
    },
  })
).json();

// The queue only lists SUBMITTED work, so submit both staged entries —
// this is the same transition flow 05 exercises.
for (const entry of [audited, warranty]) {
  await auth(`/time-entries/${entry.id}/submit`, { method: "POST", data: {} });
}

// --- browser -------------------------------------------------------------
const browser = await chromium.launch();
let shots = 0;
for (const theme of ["light", "dark"]) {
  const ctx = await browser.newContext({ colorScheme: theme, viewport: { width: 1440, height: 1000 } });
  const page = await ctx.newPage();
  await page.goto(`${WEB}/login`);
  await page.fill('input[type="email"]', EMAIL);
  await page.fill('input[type="password"]', PASSWORD);
  await page.click('button[type="submit"]');
  await page.waitForURL(/tickets|today|dashboard/, { timeout: 45_000 });

  const shoot = async (name) => {
    await page.waitForTimeout(1200);
    await page.screenshot({ path: `${OUT}/${name}.png`, fullPage: false });
    shots += 1;
  };

  // 1 + 2: Today → quick-log with the category picker open.
  await page.goto(`${WEB}/today`);
  await page.waitForLoadState("networkidle");
  const logBtn = page.getByRole("button", { name: /log time/i }).first();
  if (await logBtn.isVisible().catch(() => false)) {
    await logBtn.click();
    await page.getByTestId("quicklog-category").first().waitFor({ state: "visible", timeout: 20_000 });
    await shoot(`today-quicklog-category-${theme}`);

    // Mark the time non-billable: the typed reason select must appear.
    await page.locator("#log-billable").selectOption("no");
    await page.getByTestId("quicklog-unbillable-reason").first().waitFor({ state: "visible", timeout: 10_000 });
    await page.getByTestId("quicklog-unbillable-reason").first().selectOption("warranty");
    await shoot(`today-quicklog-unbillable-${theme}`);
    await page.keyboard.press("Escape");
  }

  // 3: approvals queue — category label, reason badge, per-row audit chip.
  await page.goto(`${WEB}/approvals`);
  await page.waitForLoadState("networkidle");
  await page.waitForTimeout(2500);
  await shoot(`approvals-queue-chips-${theme}`);

  // 4: settings — the admin taxonomy surface: order, billing semantics,
  //    rate-card affinity, entry counts, retire.
  await page.goto(`${WEB}/settings/time-categories`);
  await page.waitForLoadState("networkidle");
  await page
    .getByTestId("time-categories-settings")
    .waitFor({ state: "visible", timeout: 30_000 });
  await shoot(`settings-time-categories-${theme}`);

  await ctx.close();
}
await browser.close();
console.log(`[p39-captures] wrote ${shots} captures to ${OUT}`);
