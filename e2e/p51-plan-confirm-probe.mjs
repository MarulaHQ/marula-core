// p51-dispatch-surface — the `plan.failed` reachability probe.
//
// Purpose: measure, on the running stack, the premise the lane's second
// decision rests on — that the plan's Confirm button is disabled for exactly
// the conditions `confirmPlan` would otherwise have to report as a failure, so
// no user-reachable gesture reaches a failed confirm. Remediation item 18 read
// the branch as unreachable from the code; this re-reads it from the rendered
// control.
//
// It records, per posture: the primary's label and `disabled` state, whether
// the removed `[data-testid=plan-fail]` notice is present anywhere, and the
// queue size before the plan and after a forced click on the disabled control,
// so "nothing is booked" is a measurement rather than a claim.
//
// Non-purpose: it does not decide the ruling. It reports what the control does.
//
// Usage: node e2e/p51-plan-confirm-probe.mjs
import { chromium } from "playwright";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const OUT = resolve("design/qa-p51-dispatch-surface/captures");
mkdirSync(OUT, { recursive: true });

const envPath = resolve("e2e/.e2e-stack.env");
if (!existsSync(envPath)) throw new Error("e2e/.e2e-stack.env is missing. Run scripts/e2e-stack.sh up first.");
const env = Object.fromEntries(
  readFileSync(envPath, "utf8")
    .split("\n")
    .filter((line) => line.includes("="))
    .map((line) => [line.slice(0, line.indexOf("=")), line.slice(line.indexOf("=") + 1)]),
);
const WEB = process.env.E2E_WEB_URL ?? env.E2E_WEB_URL ?? "http://localhost:3000";
const API = (process.env.E2E_API_URL ?? env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1600, height: 994 } });

const res = await fetch(`${API}/api/v1/auth/login`, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ email: "owner@demo.local", password: "demo-password-123" }),
});
const tokens = await res.json();
await ctx.request.post(`${API}/api/v1/auth/login`, { data: { email: "owner@demo.local", password: "demo-password-123" } });
const auth = { authorization: `Bearer ${tokens.access_token}` };
await ctx.request.put(`${API}/api/v1/settings/user`, { data: { key: "tours.disabled", value: true }, headers: auth });
await ctx.request.put(`${API}/api/v1/settings/ai/decision/features`, {
  data: { enabled: true, idempotency_key: `features:probe:${String(Date.now())}` },
  headers: auth,
});
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
    idempotency_key: `grant:probe:${String(Date.now())}`,
  },
  headers: auth,
});
await ctx.addInitScript(
  ([raw, exp]) => {
    window.localStorage.setItem("marula-tokens", raw);
    window.localStorage.setItem("marula-tokens-exp", exp);
  },
  [JSON.stringify(tokens), String(Date.now() + tokens.expires_in * 1000 - 30_000)],
);

const page = await ctx.newPage();
await page.goto(`${WEB}/dispatch`, { waitUntil: "domcontentloaded", timeout: 60_000 });
await page.waitForSelector('[data-testid="dispatch-board-root"]', { timeout: 30_000 });

/** Everything this probe asserts, read from the rendered DOM in one go. */
const READ = () => {
  const primary = document.querySelector('[data-testid="plan-panel"] .dp-btn.primary');
  return {
    primaryText: primary ? (primary.textContent ?? "").trim() : null,
    primaryDisabled: primary ? primary.disabled : null,
    planPanelOpen: !!document.querySelector('[data-testid="plan-panel"]'),
    planFailNotice: document.querySelectorAll('[data-testid="plan-fail"]').length,
    // Queue cards are only drawn while the plan panel is closed, so a booked
    // plan shows up here as a smaller queue once the panel is gone.
    queueCards: document.querySelectorAll(".dp-qcard").length,
  };
};

const record = { generated: new Date().toISOString(), postures: [] };
const say = async (id, what) => {
  const snap = await page.evaluate(READ);
  record.postures.push({ id, what, ...snap });
  console.log(`${id.padEnd(10)} primary="${String(snap.primaryText)}" disabled=${String(snap.primaryDisabled)} panel=${String(snap.planPanelOpen)} failNotice=${snap.planFailNotice} queue=${snap.queueCards}`);
  return snap;
};

const beforePlan = await say("board", "the board at rest, before any plan");

await page.getByText("Plan with AI").first().click({ timeout: 10_000 });
await page.getByText("Everything in the unassigned queue", { exact: false }).first().click({ timeout: 10_000 });
await page.getByText("Propose a plan", { exact: false }).first().click({ timeout: 10_000 });
await page.waitForSelector('[data-testid="plan-panel"]', { timeout: 20_000 });
await say("clean", "a plan with no conflict and no undecided overlap");

/** Nudge row 1 earlier until the primary names an overlap. */
for (let i = 0; i < 24; i += 1) {
  const d = await page.evaluate(READ);
  if (d.primaryText !== null && d.primaryText.includes("overlap")) break;
  await page.getByLabel("15 minutes earlier").first().click({ timeout: 4_000 }).catch(() => {});
}
const overlapped = await say("overlap", "a proposed row nudged onto a placed job");

// Decide it with one of the pack's three reasons.
await page
  .getByRole("button", { name: "Remote in parallel" })
  .first()
  .click({ timeout: 10_000 })
  .catch(() => {});
await say("decided", "the overlap given one of the three reasons");

/** Move row 1 onto the technician who is off, then later until it lands in it. */
await page.locator('[data-testid="plan-panel"] select.dp-sel').first().selectOption({ label: "Jordan T." }).catch(() => {});
for (let i = 0; i < 24; i += 1) {
  const d = await page.evaluate(READ);
  if (d.primaryText !== null && d.primaryText.includes("conflict")) break;
  await page.getByLabel("15 minutes later").first().click({ timeout: 4_000 }).catch(() => {});
}
const blocked = await say("blocked", "a row moved into Jordan T.'s 14:00-18:00 absence and nudged into it");

// The canary: force a pointer event on the disabled control. A real user cannot
// do this (a disabled button dispatches no click), so whatever happens here is
// the probe's own artefact — recorded, not relied on.
await page.locator('[data-testid="plan-panel"] .dp-btn.primary').click({ force: true, timeout: 5_000 }).catch(() => {});
await page.waitForTimeout(400);
const afterForced = await say("forced-click", "a forced pointer event on the disabled Confirm");

// Book nothing: dismiss whatever is left and read the queue.
if (afterForced.planPanelOpen) {
  await page.getByRole("button", { name: "Discard" }).first().click({ timeout: 5_000 }).catch(() => {});
  await page.waitForTimeout(200);
}
const afterDismiss = await say("dismissed", "the plan discarded, back at the board");

record.verdict = {
  overlapReachable: overlapped.primaryDisabled === true && String(overlapped.primaryText).includes("overlap"),
  conflictReachable: blocked.primaryDisabled === true && String(blocked.primaryText).includes("conflict"),
  failNoticeEverRendered: record.postures.some((p) => p.planFailNotice > 0),
  queueUnchangedFromStart: beforePlan.queueCards === afterDismiss.queueCards,
  queueBefore: beforePlan.queueCards,
  queueAfter: afterDismiss.queueCards,
};
console.log(`\n${JSON.stringify(record.verdict, null, 2)}`);
writeFileSync(resolve(OUT, "plan-confirm-probe.json"), `${JSON.stringify(record, null, 2)}\n`);
await ctx.close();
await browser.close();
