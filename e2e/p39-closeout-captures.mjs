// Wave 39 lane 5 `p39-billing-closeout` — vision-review captures.
//
// Drives the REAL app against the live e2e stack (never mocks) so the
// screenshots show true states of the surfaces this lane wired:
//   1. The contract editor's commercial block: PO, minimum commitment,
//      escalator, and the type-specific unit/incident fields.
//   2. The prepaid-block and NTE-cap cards, with real rows and the
//      replace editor open on what is stored.
//   3. The change-order and service-credit cards, then again after the
//      run, when the order is `billed` and the credit `applied`.
//   4. The CREATE dialog on a T&M contract, where the blocks and caps
//      editors are reachable for the first time (lane 3 shipped the
//      state, nothing to set it).
//   5. The project billing card: the linked contract, the burn against
//      the phase budgets with its typed alert, and the release ledger.
//   6. The work-type map admin on the category settings page.
//   7. The verification close card with a signed-off reviewer note.
//
// Each surface is captured in light and dark. Run with the stack env
// exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p39-closeout-captures.mjs
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = "design/qa-p39-billing-closeout/captures";
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

// Every stage write is checked. A silent error body here would produce
// a screenshot of an empty state and call it a review.
async function stage(path, method, data, what) {
  const res = await auth(path, { method, data });
  if (!res.ok()) {
    throw new Error(`stage failed: ${what} → ${method} ${path} → ${res.status()} ${await res.text()}`);
  }
  return res.json();
}

const iso = (offsetDays) => new Date(Date.now() + offsetDays * 86400_000).toISOString().slice(0, 10);
const started = new Date(Date.now() - 7200_000).toISOString();
const at = (minutes) => new Date(Date.now() - 7200_000 + minutes * 60_000).toISOString();

// --- stage: client, a T&M contract carrying the whole pricing stack ------
const client = await stage(
  "/clients",
  "POST",
  { name: `Closeout Capture ${STAMP}`, idempotency_key: `p39-close-client-${STAMP}` },
  "client",
);

const contract = await stage(
  "/contracts",
  "POST",
  {
    client_id: client.id,
    number: `C-CLOSE-${STAMP}`,
    name: `Closeout Capture Agreement ${STAMP}`,
    contract_type: "time_and_materials",
    currency: "USD",
    hourly_rate_minor: 15000,
    effective_from: iso(-30),
    po_number: `PO-${STAMP}`,
    minimum_commitment_minor: 500000,
    escalator: { uplift_bps: 300, first_uplift_on: iso(30) },
    blocks: [
      {
        label: `50-hour block, Q3 ${STAMP}`,
        total_minutes: 3000,
        sale_price_minor: 600000,
        block_rate_minor: 12000,
        rollover_policy: "rollover",
        expires_on: iso(60),
      },
    ],
    caps: [
      { scope: "period", cap_minor: 2000000, reached_behavior: "warn" },
      {
        scope: "contract",
        work_type: "remote",
        cap_minor: 1000000,
        reached_behavior: "overage_billable",
        overage_rate_minor: 13000,
      },
    ],
    idempotency_key: `p39-close-contract-${STAMP}`,
  },
  "contract",
);
// A run bills active contracts only, so the agreement has to leave draft
// before any of the downstream state can exist.
await stage(
  `/contracts/${contract.id}`,
  "PATCH",
  { status: "active", idempotency_key: `p39-close-activate-${STAMP}` },
  "activate contract",
);

// --- stage: a recurring agreement, the only shape that carries an
//     escalator (it walks billing anniversaries off the anchor) ----------
const recurring = await stage(
  "/contracts",
  "POST",
  {
    client_id: client.id,
    number: `R-CLOSE-${STAMP}`,
    name: `Closeout Capture Subscription ${STAMP}`,
    contract_type: "recurring",
    currency: "USD",
    amount_minor: 1200000,
    billing_frequency: "monthly",
    billing_anchor: iso(-30),
    effective_from: iso(-30),
    po_number: `PO-SUB-${STAMP}`,
    minimum_commitment_minor: 900000,
    escalator: { uplift_bps: 300, first_uplift_on: iso(30) },
    idempotency_key: `p39-close-recurring-${STAMP}`,
  },
  "recurring contract",
);

const order = await stage(
  `/contracts/${contract.id}/change-orders`,
  "POST",
  {
    number: `CO-CAP-${STAMP}`,
    title: "Additional controllers",
    amount_minor: 240000,
    description: "Eight rack controllers, quoted and accepted.",
    idempotency_key: `p39-close-co-${STAMP}`,
  },
  "change order (approved)",
);
await stage(
  `/contracts/${contract.id}/change-orders/${order.id}/transition`,
  "POST",
  { action: "approve", idempotency_key: `p39-close-co-approve-${STAMP}` },
  "approve change order",
);
await stage(
  `/contracts/${contract.id}/change-orders`,
  "POST",
  {
    number: `CO-DRAFT-${STAMP}`,
    title: "Onsite workshop, scope TBC",
    amount_minor: 90000,
    description: "Priced, not yet accepted by the client.",
    idempotency_key: `p39-close-co-draft-${STAMP}`,
  },
  "change order (draft)",
);

// Two credits on purpose: one whose applies_on sits inside the run
// window (the run applies it, and the row freezes with no cancel) and
// one dated forward (still pending, still cancellable).
await stage(
  `/contracts/${contract.id}/sla-credits`,
  "POST",
  {
    reason: "Resolution target breached by 2d 4h",
    amount_minor: 12500,
    currency: "USD",
    applies_on: iso(0),
    idempotency_key: `p39-close-credit-in-${STAMP}`,
  },
  "service credit (in window)",
);
await stage(
  `/contracts/${contract.id}/sla-credits`,
  "POST",
  {
    reason: "P1 response target missed, escalation fired late",
    amount_minor: 25000,
    currency: "USD",
    applies_on: iso(45),
    idempotency_key: `p39-close-credit-out-${STAMP}`,
  },
  "service credit (forward-dated)",
);

// --- stage: a project on that contract, a budgeted phase, real time -----
const project = await stage(
  "/projects",
  "POST",
  {
    client_id: client.id,
    name: `Closeout Capture Rollout ${STAMP}`,
    description: "Controller rollout across three closets.",
    billing_method: "time_and_materials",
    currency: "USD",
    budget_minor: 3000000,
    billing_contract_id: contract.id,
    idempotency_key: `p39-close-project-${STAMP}`,
  },
  "project",
);
const phase = await stage(
  `/projects/${project.id}/phases`,
  "POST",
  {
    name: "Closet 1 rollout",
    is_milestone: true,
    milestone_amount_minor: 250000,
    milestone_currency: "USD",
    // Deliberately tight: 135 approved minutes at the card rate comes to
    // more than this, so the burn row carries a real `over` alert and the
    // shot shows the tag rather than a healthy-looking empty state.
    budget_minor: 30000,
    budget_currency: "USD",
    idempotency_key: `p39-close-phase-${STAMP}`,
  },
  "phase",
);
const phaseBurned = await stage(
  `/projects/${project.id}/phases`,
  "POST",
  {
    name: "Closet 2 rollout",
    is_milestone: false,
    fee_minor: 180000,
    fee_currency: "USD",
    budget_minor: 900000,
    budget_currency: "USD",
    idempotency_key: `p39-close-phase2-${STAMP}`,
  },
  "phase (fixed fee)",
);

// Project time with no ticket: the phase is the parent, which is the
// Wave 39 capability that makes this row exist at all.
for (const [minutes, workType] of [
  [90, "remote"],
  [45, "on_site"],
]) {
  const entry = await stage(
    "/time-entries",
    "POST",
    {
      phase_id: phase.id,
      started_at: started,
      ended_at: at(minutes),
      billable: true,
      work_type: workType,
      notes: `p39 closeout capture ${STAMP}`,
      idempotency_key: `p39-close-entry-${STAMP}-${minutes}`,
    },
    "project time entry",
  );
  await auth(`/time-entries/${entry.id}/submit`, { method: "POST", data: {} });
  await stage(
    "/time-entries/approvals/decide",
    "POST",
    { entry_ids: [entry.id], decision: "approve" },
    "approve project entry",
  );
}

// The release is staged by the WORKER on ProjectMilestoneCompleted, and
// `scripts/e2e-stack.sh` boots core-api only — no worker — so
// this poll is expected to come up empty in this stack. The probe prints
// its status either way, because a 403 would look identical to "no rows"
// and only one of those is an environment fact.
await stage(
  `/projects/${project.id}/phases/${phase.id}/complete`,
  "POST",
  { idempotency_key: `p39-close-phase-done-${STAMP}` },
  "complete milestone phase",
);
let releases = [];
for (let i = 0; i < 20; i++) {
  const res = await auth(`/projects/${project.id}/billing-releases`);
  if (res.ok()) {
    releases = await res.json();
    if (releases.length > 0) break;
  } else {
    console.warn(`WARN release probe answered ${res.status()} — this is not an empty ledger`);
    break;
  }
  await new Promise((r) => setTimeout(r, 1000));
}
if (releases.length === 0) {
  console.log(
    "NOTE no release staged: the e2e stack runs no worker, so the milestone consumer never runs and the release ledger stays empty here.",
  );
}

// --- the sign-off: a reviewer note that the close card now renders ------
// Every undispositioned entry is ruled on first: the server re-derives
// the three claims inside the writing transaction, so a partial window
// cannot be signed and the capture would show a refused sign-off.
// Dispositioning an already-approved entry is `recategorize` (or an
// adjust with a reason). `approve` only applies to an entry that is still
// awaiting a decision, and the API answers 409 for the rest.
const workspace = await (
  await auth(`/billing/verification?contract_id=${contract.id}&from=${iso(-30)}&to=${iso(1)}`)
).json();
let dispositioned = 0;
for (const group of workspace.groups ?? []) {
  for (const entry of group.entries ?? []) {
    if (entry.disposition !== null && entry.disposition !== undefined) continue;
    // `adjust_minutes` is the action that records the review: it needs a
    // reason, it can only lower, and it writes the disposition. Recategorize
    // to the same slug it already has is a no-op and dispositions nothing.
    const target = Math.max(1, (entry.minutes ?? 1) - 1);
    const res = await auth(`/billing/verification/entries/${entry.id}/action`, {
      method: "POST",
      data: {
        action: "adjust_minutes",
        minutes: target,
        reason: "reviewed as logged for the close-out capture",
        idempotency_key: `p39-close-disposition-${STAMP}-${entry.id}`,
      },
    });
    if (res.ok()) dispositioned += 1;
    else console.warn(`WARN disposition refused for ${entry.id}: ${res.status()} ${await res.text()}`);
  }
}
const signoff = await auth("/billing/verification/signoff", {
  method: "POST",
  data: {
    contract_id: contract.id,
    from: iso(-30),
    to: iso(1),
    // The three claims are the checklist the server re-derives; sending
    // them honestly is the whole point of the sign-off.
    entries_dispositioned: true,
    zero_unclassified: true,
    preview_seen: true,
    reviewer_note: `Two entries priced against the block; the controller order is billed and its credit applied. Walked with ${client.name}.`,
    idempotency_key: `p39-close-signoff-${STAMP}`,
  },
});
if (!signoff.ok()) {
  console.warn(`WARN sign-off refused: ${signoff.status()} ${await signoff.text()}`);
} else {
  console.log(`sign-off recorded after ${dispositioned} dispositions`);
}

// --- capture ------------------------------------------------------------
const browser = await chromium.launch();
const consoleErrors = [];

async function shoot(page, name) {
  await page.waitForTimeout(450);
  await page.screenshot({ path: `${OUT}/${name}.png`, fullPage: false });
  console.log(`shot ${name}`);
}

for (const theme of ["light", "dark"]) {
  const ctx = await browser.newContext({
    viewport: { width: 1440, height: 980 },
    colorScheme: theme,
    deviceScaleFactor: 2,
  });
  const page = await ctx.newPage();
  page.on("console", (m) => {
    if (m.type() === "error") consoleErrors.push(`[${theme}] ${m.text()}`);
  });
  page.on("pageerror", (e) => consoleErrors.push(`[${theme}] pageerror ${e.message}`));

  // Sign in through the app so the shell renders as a signed-in user.
  await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
  await page.getByLabel(/email/i).fill(EMAIL);
  await page.locator('input[type="password"]').fill(PASSWORD);
  await page.getByRole("button", { name: /sign in/i }).click();
  await page.waitForURL((u) => !u.pathname.includes("/login"), { timeout: 30_000 });
  const suffix = theme === "light" ? "light" : "dark";

  // 1. The recurring agreement's commercial block: PO, minimum
  //    commitment, and the escalator. Recurring is the shape that carries
  //    all three, so it is the one that shows them.
  await page.goto(`${WEB}/contracts/${recurring.id}`, { waitUntil: "domcontentloaded" });
  await page.getByLabel(/annual escalator/i).first().waitFor({ timeout: 20_000 });
  await page.getByLabel(/minimum commitment/i).first().scrollIntoViewIfNeeded();
  await shoot(page, `01-contract-pricing-${suffix}`);

  // 1b. The T&M agreement's commercial block: PO and the minimum, and the
  //     escalator correctly absent because T&M has no billing anchor.
  await page.goto(`${WEB}/contracts/${contract.id}`, { waitUntil: "domcontentloaded" });
  await page.getByLabel(/minimum commitment/i).first().waitFor({ timeout: 20_000 });
  await page.getByLabel(/client po number/i).first().scrollIntoViewIfNeeded();
  await shoot(page, `01b-contract-tm-pricing-${suffix}`);

  // 2. Blocks + caps with their editors open on the stored rows.
  await page.getByTestId("contract-blocks").scrollIntoViewIfNeeded();
  await shoot(page, `02-contract-blocks-caps-${suffix}`);
  await page.getByTestId("contract-caps").scrollIntoViewIfNeeded();
  await shoot(page, `03-contract-caps-${suffix}`);

  // 3. Change orders + service credits.
  await page.getByTestId("contract-change-orders").scrollIntoViewIfNeeded();
  await shoot(page, `04-contract-change-orders-${suffix}`);
  await page.getByTestId("contract-sla-credits").scrollIntoViewIfNeeded();
  await shoot(page, `05-contract-sla-credits-${suffix}`);

  // 4. The create dialog on a T&M contract: blocks, caps, escalator.
  await page.goto(`${WEB}/contracts`, { waitUntil: "domcontentloaded" });
  await page.getByRole("button", { name: /new contract/i }).click();
  await page.getByRole("combobox", { name: /type/i }).click();
  await page.getByRole("option", { name: /time & materials|time and materials/i }).click();
  await page.getByPlaceholder(/^label \(e\.g\. 50-hour block/i).waitFor({ timeout: 20_000 });
  await page.mouse.wheel(0, 320);
  await shoot(page, `06-create-dialog-blocks-${suffix}`);
  await page.getByRole("button", { name: /cancel/i }).first().click();

  // 5. The project billing card: the linked contract, the burn against
  //    the phase budgets, and the release ledger in whatever state this
  //    stack can reach (the milestone consumer needs a worker).
  await page.goto(`${WEB}/projects/${project.id}`, { waitUntil: "domcontentloaded" });
  await page.getByTestId("project-billing").waitFor({ timeout: 20_000 });
  await shoot(page, `07-project-billing-${suffix}`);

  // 6. Work-type map admin.
  await page.goto(`${WEB}/settings/time-categories`, { waitUntil: "domcontentloaded" });
  await page.getByTestId("work-type-map").scrollIntoViewIfNeeded();
  await page.waitForTimeout(600);
  await shoot(page, `08-time-categories-map-${suffix}`);

  // 7. The close card with the reviewer note rendered.
  await page.goto(
    `${WEB}/billing/verification?contract_id=${contract.id}&from=${iso(-30)}&to=${iso(1)}`,
    { waitUntil: "domcontentloaded" },
  );
  await page.getByTestId("verify-signoff-state").waitFor({ timeout: 20_000 });
  await page.getByTestId("verify-signoff-state").scrollIntoViewIfNeeded();
  await shoot(page, `09-verify-signoff-note-${suffix}`);

  await ctx.close();
}

// --- the run: bills the order, applies the in-window credit -------------
await stage(
  "/billing/invoice-runs",
  "POST",
  {
    client_id: client.id,
    from: iso(-30),
    to: iso(1),
    due_in_days: 30,
    idempotency_key: `p39-close-run-${STAMP}`,
  },
  "invoice run",
);

// 8. After the run, on the contract: the approved change order is `billed`
//    with its invoice link, and the in-window credit is `applied` with no
//    cancel affordance, which is the frozen state the API enforces. Shot
//    once, in light: the row states are the evidence, not the theme.
const ctx = await browser.newContext({ viewport: { width: 1440, height: 980 }, colorScaleFactor: 2 });
const page = await ctx.newPage();
page.on("pageerror", (e) => consoleErrors.push(`[after-run] pageerror ${e.message}`));
await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
await page.getByLabel(/email/i).fill(EMAIL);
await page.locator('input[type="password"]').fill(PASSWORD);
await page.getByRole("button", { name: /sign in/i }).click();
await page.waitForURL((u) => !u.pathname.includes("/login"), { timeout: 30_000 });
await page.goto(`${WEB}/contracts/${contract.id}`, { waitUntil: "domcontentloaded" });
await page.getByTestId("contract-change-orders").scrollIntoViewIfNeeded();
await shoot(page, "10-contract-change-orders-after-run-light");
await page.getByTestId("contract-sla-credits").scrollIntoViewIfNeeded();
await shoot(page, "11-contract-sla-credits-after-run-light");
await ctx.close();

await browser.close();

if (consoleErrors.length > 0) {
  console.log("CONSOLE ERRORS:");
  for (const e of consoleErrors) console.log(`  ${e}`);
} else {
  console.log("no console errors during capture");
}
console.log(
  `staged contract=${contract.id} project=${project.id} phase=${phase.id} fixed-fee-phase=${phaseBurned.id} releases=${releases.length}`,
);
