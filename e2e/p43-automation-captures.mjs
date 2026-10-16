// Wave 43 lane 17b `p43-automation-builder` — vision-review captures.
//
// Drives the REAL /automations against the live e2e stack (never mocks):
// the ledger, the rebuilt clause-row editor (a complete rule, an any-of set,
// and a row whose offending token is named), the plain-English read-back,
// and the test run rendered as a per-ticket diff of what WOULD change. The
// seeded demo org has AI off, so the captures also record the honest
// degraded state: no AI affordances at all, and the deterministic read-back
// as the survivor.
//
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p43-automation-captures.mjs
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = process.env.CAPTURE_OUT ?? "design/qa-p43-automation-builder/captures";
const THEMES = (process.env.CAPTURE_THEMES ?? "light,dark").split(",");
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

// --- scratch rules -----------------------------------------------------------
// Staged through the same route the editor saves by, so the screenshots show
// rules that arrived the way real rules arrive — not fixtures injected into a
// render.

// A complete rule: an any-of condition, a presence condition, and three
// effects. Aimed at P2/P3 so the diff shows real `~ priority` changes, and
// the P1 matches it also catches show the honest `=` no-change line.
const completeRule = await api(`/automation-rules`, "POST", {
  name: `Captured: escalate high work ${STAMP}`,
  board_id: null,
  subject_kind: "ticket",
  trigger_event: "ticket.created",
  conditions: [
    { type: "priority_in", priorities: ["p2", "p3"] },
    { type: "assignee_is_set" },
  ],
  actions: [
    { type: "set_priority", priority: "p1" },
    { type: "add_internal_comment", body: "Escalated automatically." },
  ],
  idempotency_key: crypto.randomUUID(),
});

// A rule that matches everything — the state the ledger names honestly and
// offers a one-click fix for.
await api(`/automation-rules`, "POST", {
  name: `Captured: loose rule ${STAMP}`,
  board_id: null,
  subject_kind: "ticket",
  trigger_event: "ticket.updated",
  conditions: [],
  actions: [{ type: "add_internal_comment", body: "Touched." }],
  idempotency_key: crypto.randomUUID(),
});

// --- captures ----------------------------------------------------------------

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
      window.sessionStorage.setItem("marula-tour-session", "1");
    },
    [API, JSON.stringify(tokens), theme],
  );

  async function shot(name, { fullPage = false } = {}) {
    shotIndex += 1;
    await page.waitForTimeout(500);
    await page.screenshot({
      path: `${OUT}/${String(shotIndex).padStart(2, "0")}-${name}-${theme}.png`,
      fullPage,
    });
  }

  try {
    // 1. The ledger: both rules, the ticket-less badge column, and the loose
    //    rule's inline "matches every ticket · Add one" verdict in its row.
    await page.goto(`${WEB}/automations`, { waitUntil: "load" });
    await page.getByTestId("automation-rules-table").waitFor({ timeout: 30_000 });
    await shot("01-automations-ledger", { fullPage: true });

    // 2. The editor over the complete rule: two clause rows, a set chip and a
    //    presence phrase, the read-back sentence derived from them.
    await page.getByRole("button", { name: completeRule.name }).click();
    await page.getByTestId("automation-read-back").waitFor({ timeout: 30_000 });
    await shot("02-editor-clause-rows");
    await page.keyboard.press("Escape");

    // 3. The starters: the four cases an MSP writes on its first afternoon.
    await page.getByTestId("automation-new-rule").click();
    await page.getByTestId("automation-starters").waitFor({ timeout: 30_000 });
    await shot("03-editor-starters");
    await page.keyboard.press("Escape");

    // 4. A row whose offending token is NAMED: an any-of row with nothing
    //    selected cannot match, and the message says which token is wrong —
    //    the value, not the row.
    await page.getByTestId("automation-new-rule").click();
    await page.getByTestId("automation-starters").waitFor({ timeout: 30_000 });
    await page.getByRole("button", { name: "Triage what is still open" }).click();
    await page.getByTestId("automation-add-condition").waitFor({ timeout: 30_000 });
    await page.getByTestId("automation-add-condition").click();
    // The fresh row is "priority is any of P1". Emptying the set is the
    // condition that can never match, and the one the grammar must name.
    // The last four checkboxes in the dialog are the fresh row's priority
    // set; the starter's own status row sits above it.
    await page.getByRole("checkbox").nth(-4).uncheck();
    await page.getByRole("checkbox").nth(-3).uncheck();
    await page.getByTestId("automation-read-back").waitFor({ timeout: 30_000 });
    await shot("04-editor-validation");
    // 5. The action side names its own gap: an internal note with no body.
    await page.getByLabel("Add action").click();
    await page.getByRole("option", { name: "Add internal note" }).click();
    await page.getByRole("button", { name: "Add action" }).last().click();
    await page.getByText("Internal note needs a body.").scrollIntoViewIfNeeded();
    await shot("05-action-validation");
    await page.keyboard.press("Escape");

    // 6. The test run: what WOULD change, ticket by ticket.
    await page.goto(`${WEB}/automations`, { waitUntil: "load" });
    await page.getByTestId("automation-rules-table").waitFor({ timeout: 30_000 });
    await page.getByTestId(`automation-test-run-${completeRule.id}`).click();
    await page.getByTestId("automation-test-run-result").waitFor({ timeout: 30_000 });
    // The result scrolls with the ledger card. Put its own top edge at the
    // top of the card for the header line, then walk down to the per-ticket
    // lines.
    await page.getByTestId("automation-test-run-result").evaluate((el) =>
      el.scrollIntoView({ block: "start" }),
    );
    await page.waitForTimeout(300);
    await shot("06-test-run-summary");
    await page.mouse.wheel(0, 300);
    await page.waitForTimeout(300);
    await shot("07-test-run-diff");
  } catch (error) {
    failures.push(`${theme}: ${error?.message ?? error}`);
  }

  await browser.close();
}

if (failures.length > 0) {
  console.error(`CAPTURE FAILURES:\n${failures.join("\n")}`);
  process.exitCode = 1;
} else {
  console.log(`captured ${shotIndex} states to ${OUT}`);
}
