// Wave 43 lane 16 `p43-workflow-canvas` — vision-review captures for the
// workflow canvas, drawn against the REAL e2e stack (never mocks):
//   A  /automations — the ledger with the "Open the canvas" header action and
//      the per-row "Open in canvas" toggle.
//   B  /automations/canvas — a saved workflow document: laid-out node cards,
//      the header (Canvas/Steps toggle, Save), the validation strip, the
//      add-step library, the schema-driven inspector.
//   C  /automations/canvas?rule=<id> — the view toggle promoting a simple rule
//      (derive), with the ticket-writing step surfaced as needs-a-value.
//   D  the Steps view (same document, read as a list).
// Captured light + dark (plus a 375px mobile pass and a keyboard-focus pass)
// into design/qa-p43-workflow-canvas/captures/, reviewed in <=8-image batches
// per the lane mandate.
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p43-workflow-canvas-captures.mjs
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/+$/, "");
const OUT = "design/qa-p43-workflow-canvas/captures";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const STAMP = Date.now() % 1_000_000;

mkdirSync(OUT, { recursive: true });

// --- API: login (durable-gate retry) ------------------------------------------
let token = null;
for (let attempt = 0; attempt < 5; attempt += 1) {
  const login = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: EMAIL, password: PASSWORD }),
  });
  if (login.ok) {
    token = (await login.json()).access_token;
    break;
  }
  await new Promise((r) => setTimeout(r, 4000));
}
if (!token) throw new Error("owner login failed through the auth gate");
const auth = (path, init = {}) =>
  fetch(`${API}/api/v1${path}`, {
    ...init,
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
      ...(init.headers ?? {}),
    },
  });
const json = async (res) => (res.ok ? res.json() : null);

// A validated document so the canvas opens on a clean graph.
const document = {
  version: "1",
  nodes: [
    { id: "cond", node_type: "field.equals", params: { field: "priority", value: "p1" } },
    { id: "wait", node_type: "flow.delay", params: { duration: "5m" } },
    { id: "stop", node_type: "flow.stop", params: {} },
  ],
  edges: [
    { from: "cond", to: "wait" },
    { from: "wait", to: "stop" },
  ],
  loop_max_depth: 1,
};
const docRes = await auth("/workflows/documents", {
  method: "POST",
  body: JSON.stringify({
    name: `Capture workflow (${STAMP})`,
    trigger_event: "ticket.created",
    definition: document,
  }),
});
const savedDoc = await json(docRes);
console.log(`[capture] workflow document: ${docRes.status} ${savedDoc?.id ?? ""}`);

// A simple rule to promote through the view toggle (shows the needs-a-value state).
const ruleRes = await auth("/automation-rules", {
  method: "POST",
  body: JSON.stringify({
    name: `Capture canvas rule (${STAMP})`,
    board_id: null,
    subject_kind: "ticket",
    trigger_event: "ticket.created",
    conditions: [{ type: "priority_is", priority: "p1" }],
    actions: [{ type: "set_priority", priority: "p2" }],
  }),
});
const captureRuleId = (await json(ruleRes))?.id ?? null;
console.log(`[capture] simple rule: ${captureRuleId}`);

// --- browser -------------------------------------------------------------------
const browser = await chromium.launch();
let shots = 0;
const shot = async (page, name) => {
  await page.screenshot({ path: `${OUT}/${name}.png` });
  shots += 1;
  console.log(`  [shot] ${name}`);
};

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

const dismissTour = async (page) => {
  for (let step = 0; step < 8; step += 1) {
    const overlay = page.locator('[data-testid="tour-overlay"]');
    if (!(await overlay.isVisible().catch(() => false))) return;
    for (const name of [/don't show me tours/i, /^done$/i, /next/i]) {
      const target = page.getByRole("button", { name }).first();
      if (await target.isVisible().catch(() => false)) {
        await target.click({ timeout: 3000 }).catch(() => {});
        break;
      }
    }
    await page.waitForTimeout(300);
  }
  await page.keyboard.press("Escape").catch(() => {});
};

const gotoStaff = async (page, path, marker) => {
  for (let attempt = 0; attempt < 3; attempt += 1) {
    await page.goto(`${WEB}${path}`, { waitUntil: "domcontentloaded" }).catch(() => {});
    await page.waitForTimeout(2500);
    if (!page.url().includes("/login")) {
      if (marker) {
        await page.locator(marker).first().waitFor({ state: "visible", timeout: 25_000 }).catch(() => {});
      }
      await page.waitForTimeout(700);
      return;
    }
    await loginWithRetry(page);
  }
  throw new Error(`could not reach ${path} (repeatedly bounced to /login)`);
};

for (const theme of ["light", "dark"]) {
  const ctx = await browser.newContext({
    colorScheme: theme,
    viewport: { width: 1440, height: 1000 },
    reducedMotion: theme === "dark" ? "reduce" : "no-preference",
  });
  const page = await ctx.newPage();
  page.on("pageerror", (err) => console.error(`[pageerror ${theme}]`, err.message));
  await loginWithRetry(page);
  await dismissTour(page);

  // --- A: the ledger with the canvas toggle ----------------------------------
  await gotoStaff(page, "/automations", "text=Automation rules");
  await page.waitForTimeout(800);
  await shot(page, `${theme}-01-automations-with-canvas-toggle`);

  // --- B: the saved workflow on the canvas -----------------------------------
  await gotoStaff(page, "/automations/canvas", "text=Workflows");
  await page.waitForTimeout(2500); // catalogue + document load + ELK layout
  await shot(page, `${theme}-02-canvas-loaded`);

  // Select a step → the schema-driven inspector.
  const firstNode = page.locator(".react-flow__node").first();
  const nodeReady = await firstNode
    .waitFor({ state: "visible", timeout: 20_000 })
    .then(() => true)
    .catch(() => false);
  if (nodeReady) {
    await firstNode.click({ force: true, timeout: 5000 }).catch(() => {});
    await page.waitForTimeout(600);
    await shot(page, `${theme}-03-canvas-inspector`);
  } else {
    console.log(`  [miss] no node card rendered for ${theme}`);
  }

  // The add-step library (palette from the registry).
  await page.getByRole("button", { name: "Add step" }).first().click().catch(() => {});
  await page.waitForTimeout(600);
  await shot(page, `${theme}-04-canvas-library`);
  await page.keyboard.press("Escape").catch(() => {});

  // The Steps view (same document).
  await page.getByRole("button", { name: "Steps", exact: true }).first().click().catch(() => {});
  await page.waitForTimeout(600);
  await shot(page, `${theme}-05-steps-view`);

  // --- C: promoting a simple rule (the view toggle) ---------------------------
  if (captureRuleId) {
    await gotoStaff(page, `/automations/canvas?rule=${captureRuleId}`, "text=Workflows");
    await page.waitForTimeout(2500);
    await shot(page, `${theme}-06-canvas-promoted-rule`);
  }

  await ctx.close();
}

// --- keyboard pass: focus lands on a node, operable without a pointer ---------
{
  const ctx = await browser.newContext({ colorScheme: "light", viewport: { width: 1440, height: 1000 } });
  const page = await ctx.newPage();
  await loginWithRetry(page);
  await dismissTour(page);
  await gotoStaff(page, "/automations/canvas", "text=Workflows");
  await page.waitForTimeout(2500);
  // Put focus inside the graph, then drive it with the keyboard only: Tab
  // walks the nodes/edges, arrow keys move the focused node.
  await page.locator(".react-flow__pane").click({ position: { x: 30, y: 420 } }).catch(() => {});
  await page.waitForTimeout(300);
  for (let i = 0; i < 8; i += 1) {
    await page.keyboard.press("Tab");
    await page.waitForTimeout(140);
    const onNode = await page.evaluate(() =>
      (document.activeElement?.className ?? "").toString().includes("react-flow__node"),
    );
    if (onNode) break;
  }
  await shot(page, "keyboard-01-node-focused");
  const posBefore = await page.evaluate(
    () => document.querySelector(".react-flow__node.selected")?.getAttribute("style") ?? null,
  );
  await page.keyboard.press("Enter");
  await page.waitForTimeout(200);
  for (let i = 0; i < 8; i += 1) await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowDown");
  await page.waitForTimeout(250);
  const posAfter = await page.evaluate(
    () => document.querySelector(".react-flow__node.selected")?.getAttribute("style") ?? null,
  );
  console.log(`  [keyboard] selected node style before/after: ${posBefore} -> ${posAfter}`);
  await shot(page, "keyboard-02-node-moved-by-arrows");
  await ctx.close();
}

// --- mobile pass ---------------------------------------------------------------
{
  const ctx = await browser.newContext({ colorScheme: "light", viewport: { width: 375, height: 780 } });
  const page = await ctx.newPage();
  await loginWithRetry(page);
  await dismissTour(page);
  await gotoStaff(page, "/automations", "text=Automation rules");
  await shot(page, "mobile-01-automations-375");
  await ctx.close();
}

await browser.close();
console.log(`[capture] done — ${shots} captures in ${OUT}`);
