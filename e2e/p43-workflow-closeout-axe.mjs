// Wave 43 lane 18 `p43-workflow-closeout-ui` — focused axe scan of the surfaces
// this lane added: the run ledger, the run detail (the trace table plus the
// retry controls), and the rule editor with the registry-driven trigger picker
// open. Reuses the house axe version (@axe-core/playwright) and WCAG tags and
// writes machine-readable results into the lane evidence dir.
// `/automations/runs` is also registered in e2e/helpers/axe.ts SURFACES so the
// full matrix covers it; the detail route cannot be a static matrix row because
// it needs a run id.
//
// The run id is a REAL execution from `e2e/p43-workflow-closeout-drive.mjs`
// (the worker is booted by hand — the sanctioned entry point does not include
// it). Nothing is inserted into the database.
//   set -a; . e2e/.e2e-stack.env; set +a
//   node e2e/p43-workflow-closeout-axe.mjs
import { chromium } from "playwright";
import { AxeBuilder } from "@axe-core/playwright";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/+$/, "");
const OUT = "design/qa-p43-workflow-closeout-ui";
const TRACES = `${OUT}/traces`;
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
mkdirSync(OUT, { recursive: true });

const report = JSON.parse(readFileSync(`${TRACES}/drive-report.json`, "utf8"));
const failedRunId = report.runs.mixed?.run_id;
const parkedRunId = report.runs.parked?.run_id;
if (!failedRunId) throw new Error("drive report has no failed run — run the driver first");

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

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
const page = await context.newPage();
await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1500);
await page.fill('input[type="email"]', EMAIL);
await page.fill('input[type="password"]', PASSWORD);
await page.locator('button[type="submit"]').click();
await page.waitForURL(/tickets|today|dashboard/, { timeout: 45_000 });

const scans = [];
const scan = async (label, path, ready) => {
  await page.goto(`${WEB}${path}`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(3500);
  if (ready) await page.locator(ready).first().waitFor({ state: "visible", timeout: 25_000 }).catch(() => {});
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  const violations = results.violations.map((v) => ({
    id: v.id,
    impact: v.impact,
    help: v.help,
    nodes: v.nodes.map((n) => n.target),
  }));
  const critical = violations.filter((v) => v.impact === "critical");
  scans.push({ label, url: page.url(), violations });
  console.log(`[axe] ${label}: ${violations.length} violation(s), ${critical.length} critical`);
  for (const v of violations) console.log(`  - ${v.impact} ${v.id}: ${v.help} (${v.nodes.length} node(s))`);
};

await scan("run-ledger", "/automations/runs", "text=Workflow runs");
await scan("run-detail-failed", `/automations/runs/${failedRunId}`, "text=Workflow runs");
// The parked run wears the status "waiting" banner, so its contrast is scanned
// rather than only measured.
if (parkedRunId) await scan("run-detail-parked", `/automations/runs/${parkedRunId}`, "text=Workflow runs");
await scan("run-not-found", "/automations/runs/00000000-0000-0000-0000-000000000000", "text=Run not found");
await scan("automations-rule-editor", "/automations", "text=Automation rules");
// The rule editor with the trigger picker open — the surface this lane changed.
await page.getByTestId("automation-new-rule").first().click().catch(() => {});
await page.waitForTimeout(700);
const open = await page.locator("#ar-event").isVisible().catch(() => false);
if (open) {
  await page.locator("#ar-event").click();
  await page.waitForTimeout(500);
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  const violations = results.violations.map((v) => ({
    id: v.id,
    impact: v.impact,
    help: v.help,
    nodes: v.nodes.map((n) => n.target),
  }));
  const critical = violations.filter((v) => v.impact === "critical");
  scans.push({ label: "automations-trigger-picker-open", url: page.url(), violations });
  console.log(`[axe] automations-trigger-picker-open: ${violations.length} violation(s), ${critical.length} critical`);
}

writeFileSync(
  `${OUT}/axe-results.json`,
  JSON.stringify({ generated: new Date().toISOString(), scans }, null, 2),
);

const total = scans.reduce((n, s) => n + s.violations.length, 0);
const criticals = scans.reduce(
  (n, s) => n + s.violations.filter((v) => v.impact === "critical").length,
  0,
);
console.log(`[axe] done — ${scans.length} scan(s), ${total} violation(s), ${criticals} critical → ${OUT}/axe-results.json`);
await browser.close();
