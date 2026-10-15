// Wave 43 lane 16 `p43-workflow-canvas` — focused axe scan of the canvas,
// since a node graph is the most keyboard/hostile surface the product has.
// Reuses the house axe version (@axe-core/playwright) and WCAG tags; writes
// machine-readable results to the lane evidence dir. The surface is also
// registered in e2e/helpers/axe.ts SURFACES so the full matrix covers it.
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p43-workflow-canvas-axe.mjs
import { chromium } from "playwright";
import { AxeBuilder } from "@axe-core/playwright";
import { mkdirSync, writeFileSync } from "node:fs";

const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/+$/, "");
const OUT = "design/qa-p43-workflow-canvas";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
mkdirSync(OUT, { recursive: true });

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
const page = await context.newPage();
await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1200);
await page.fill('input[type="email"]', EMAIL);
await page.fill('input[type="password"]', PASSWORD);
await page.locator('button[type="submit"]').click();
await page.waitForURL(/tickets|today|dashboard/, { timeout: 45_000 });
await page.goto(`${WEB}/automations/canvas`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(4000);

const results = await new AxeBuilder({ page })
  .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
  .analyze();

const summary = results.violations.map((v) => ({
  id: v.id,
  impact: v.impact,
  help: v.help,
  nodes: v.nodes.map((n) => n.target),
}));
writeFileSync(
  `${OUT}/axe-canvas.json`,
  JSON.stringify({ url: page.url(), violations: summary }, null, 2),
);
const critical = summary.filter((v) => v.impact === "critical");
console.log(`[axe] canvas: ${summary.length} violation(s), ${critical.length} critical`);
for (const v of summary) console.log(`  - ${v.impact} ${v.id}: ${v.help} (${v.nodes.length} node(s))`);
await browser.close();
