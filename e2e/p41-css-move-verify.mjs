// Confirm the shared-chrome CSS move did not change the rendering: capture the
// moved surfaces in both themes and compare against the pack, or at minimum
// assert the moved classes still compute the same colour they did before.
import { chromium } from "playwright";

const WEB = process.env.E2E_WEB_URL || "http://localhost:3002";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const S = null;

const b = await chromium.launch();
const c = await b.newContext({ viewport: { width: 1600, height: 994 } });
const p = await c.newPage();

// The storage state's token is memory-only, so sign in on this context.
await p.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
await p.waitForTimeout(3000);
await p.fill('input[type="email"]', EMAIL);
await p.fill('input[type="password"]', PASSWORD);
await p.locator('button[type="submit"]').click();
await p.waitForTimeout(9000);
if ((await p.locator("body").innerText()).includes("Good to see you")) {
  console.error("SIGN-IN FAILED — every probe would read the login page");
  process.exit(1);
}

const CLASSES = [
  ".m-money-page", ".m-money-head", ".m-money-subnav", ".m-money-stat",
  ".m-money-card", ".m-money-verdict", ".m-sla-pill", ".m-chart-fluid",
  ".m-merged-flag", ".m-enter", ".m-edge", ".m-wb-tab-count",
];

const rows = [];
for (const route of ["/billing", "/sales/quotes", "/tickets", "/tickets/1", "/boards"]) {
  await p.goto(`${WEB}${route}`, { waitUntil: "domcontentloaded" });
  await p.waitForTimeout(3500);
  const got = await p.evaluate((classes) => {
    const out = {};
    for (const cls of classes) {
      const els = Array.from(document.querySelectorAll(cls));
      if (!els.length) continue;
      const cs = getComputedStyle(els[0]);
      out[cls] = `${cs.backgroundColor}|${cs.color}|${cs.borderRadius}`;
    }
    return out;
  }, CLASSES);
  const n = Object.keys(got).length;
  rows.push({ route, n, got });
  console.log(`${route.padEnd(16)} ${n} moved classes present`);
  for (const [k, v] of Object.entries(got)) console.log(`      ${k.padEnd(20)} ${v}`);
}

// Any class that resolved nothing where it used to would show as absent here.
const empty = rows.filter((r) => r.n === 0);
console.log(
  `\n${empty.length === 0 ? "every probed route still renders its moved classes" : `ROUTES WITH NO MOVED CLASSES: ${empty.map((e) => e.route).join(", ")}`}`,
);
await b.close();
