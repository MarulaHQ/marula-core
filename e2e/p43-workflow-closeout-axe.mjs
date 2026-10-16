// Wave 43 lane 18 `p43-workflow-closeout-ui` — focused axe scan of the surfaces
// this lane added: the run ledger, the run detail (the trace table plus the
// retry controls), and the rule editor with the registry-driven trigger picker
// open. Reuses the house axe version (@axe-core/playwright) and WCAG tags and
// writes machine-readable results into the lane evidence dir.
// `/automations/runs` is also registered in e2e/helpers/axe.ts SURFACES so the
// full matrix covers it; the detail route cannot be a static matrix row because
// it needs a run id.
//
// The run rows are written with `psql` first, for the same reason the capture
// spec states: the stack does not boot the worker, so nothing executes and an
// empty ledger would leave the table unscanned.
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p43-workflow-closeout-axe.mjs
import { chromium } from "playwright";
import { AxeBuilder } from "@axe-core/playwright";
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/+$/, "");
const DB = process.env.DATABASE_URL ?? "";
const OUT = "design/qa-p43-workflow-closeout-ui";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
mkdirSync(OUT, { recursive: true });

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

const psql = (sql) => execFileSync("psql", [DB, "-v", "ON_ERROR_STOP=1", "-tAc", sql], { encoding: "utf8" }).trim();
const orgId = psql("SELECT org_id FROM automation_rules ORDER BY created_at LIMIT 1");
if (!orgId) throw new Error("no org in the demo database");

const q = (s) => `'${String(s).replace(/'/g, "''")}'`;
const runId = randomUUID();
const stepId = randomUUID();
psql(
  `INSERT INTO workflow_runs (id, org_id, rule_name, rule_version, rule_definition, catalogue_version, status, trigger_kind, error_code, error, started_at, finished_at, created_at, updated_at) ` +
    `VALUES (${q(runId)}, ${q(orgId)}, 'Axe fixture workflow', 1, '{}'::jsonb, '1', 'failed', 'event', 'node_dispatch_failed', 'The step was refused.', now() - interval '5 minutes', now() - interval '5 minutes', now() - interval '5 minutes', now() - interval '5 minutes');` +
    `INSERT INTO workflow_run_steps (id, org_id, run_id, step_index, node_id, node_label, attempt, status, input, error_code, error, started_at, finished_at) ` +
    `VALUES (${q(stepId)}, ${q(orgId)}, ${q(runId)}, 1, 'action.http', 'Call an HTTP endpoint', 1, 'failed', '{}'::jsonb, 'node_dispatch_failed', 'The step was refused.', now() - interval '5 minutes', now() - interval '5 minutes');`,
);

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
const page = await context.newPage();
await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1200);
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
await scan("run-detail", `/automations/runs/${runId}`, "text=Workflow runs");
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
  scans.push({ label: "automations-trigger-picker-open", url: page.url(), violations });
  console.log(`[axe] automations-trigger-picker-open: ${violations.length} violation(s)`);
}

writeFileSync(
  `${OUT}/axe-results.json`,
  JSON.stringify({ generated: new Date().toISOString(), scans }, null, 2),
);

const total = scans.reduce((n, s) => n + s.violations.length, 0);
console.log(`[axe] done — ${scans.length} scan(s), ${total} violation(s) in ${OUT}/axe-results.json`);
await browser.close();
