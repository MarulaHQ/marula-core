#!/usr/bin/env node
/**
 * p47-geometry-probe — the row's box against its ACTUAL wrapper.
 *
 * The main harness walks up from the row to the first ancestor with
 * `overflow: auto | scroll` to find the scroll container. That skips an
 * `overflow: hidden` wrapper, which is exactly the wrapper the kit `Card`
 * renders (`packages/ui/src/kit/ui/card.tsx:6`). So the main harness reported
 * "no overflow" on a table that is in fact clipped.
 *
 * This probe finds the nearest ancestor with ANY non-visible overflow-x, so it
 * sees the clipping wrapper, and compares the table's right edge with it.
 *
 * Usage: node scripts/p47-geometry-probe.mjs
 */

import { chromium } from "playwright";
import { readFileSync, existsSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..");
const OUT = resolve(ROOT, "design/qa-p47-rendered-states");

const envPath = resolve(ROOT, "e2e/.e2e-stack.env");
if (!existsSync(envPath)) throw new Error("e2e/.e2e-stack.env missing — boot the stack first");
const env = Object.fromEntries(
  readFileSync(envPath, "utf8")
    .split("\n")
    .filter((l) => l.includes("="))
    .map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]),
);
const WEB = process.env.E2E_WEB_URL ?? env.E2E_WEB_URL ?? "http://localhost:3000";

const ROUTES = [
  "/approvals", "/billing", "/billing/credit-notes", "/tickets", "/clients", "/assets",
  "/stock", "/stock/tools", "/inventory", "/purchasing", "/purchasing/vendors", "/contracts",
  "/projects", "/audit", "/provenance", "/documents", "/kb", "/sales/quotes", "/sales/catalog",
  "/dispatch", "/today", "/settings/team", "/settings/tags", "/settings/boards",
  "/settings/api-keys", "/reports", "/scorecards", "/alerts", "/imports", "/passwords",
];

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1600, height: 994 } });
const page = await context.newPage();
await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded", timeout: 60_000 });
await page.fill("#login-email", "owner@demo.local", { timeout: 60_000 });
await page.fill("#login-password", "demo-password-123", { timeout: 60_000 });
await page.click('[data-testid="login-submit"]', { timeout: 60_000 });
await page.waitForFunction(() => !location.pathname.startsWith("/login"), null, { timeout: 60_000 });

const measure = () =>
  page.evaluate(() => {
    const t = document.querySelector("table");
    if (!t) return { ok: false, why: "no table on the route" };
    let anc = t.parentElement;
    let wrapper = null;
    while (anc && anc !== document.body) {
      const cs = getComputedStyle(anc);
      if (cs.overflowX !== "visible") {
        wrapper = anc;
        break;
      }
      anc = anc.parentElement;
    }
    const w = wrapper ?? t.parentElement;
    const rc = (e) => {
      const x = e.getBoundingClientRect();
      return { x: Math.round(x.x), right: Math.round(x.right), w: Math.round(x.width) };
    };
    const cells = [...t.querySelectorAll("tbody tr:first-child > td")].map((c, i) => ({
      i,
      right: Math.round(c.getBoundingClientRect().right),
    }));
    return {
      ok: true,
      table: rc(t),
      wrapper: {
        tag: w.tagName,
        cls: String(w.className).slice(0, 60),
        ...rc(w),
        scrollWidth: w.scrollWidth,
        clientWidth: w.clientWidth,
        overflowX: getComputedStyle(w).overflowX,
      },
      viewportWidth: window.innerWidth,
      documentScrollWidth: document.documentElement.scrollWidth,
      lastCellRight: cells.length ? cells[cells.length - 1].right : null,
      cellsPastWrapper: cells.filter((c) => c.right > rc(w).right + 1).length,
      tableOverflowsWrapper: rc(t).right > rc(w).right + 1,
      overflowHidden: t.clientWidth < t.scrollWidth || w.scrollWidth > w.clientWidth + 1,
    };
  });

const results = {};
for (const route of ROUTES) {
  await page.goto(`${WEB}${route}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
  await page.waitForTimeout(1100);
  const r = await measure();
  results[route] = r;
  if (!r.ok) {
    console.log(`  · ${route.padEnd(24)} ${r.why}`);
    continue;
  }
  const flag = r.tableOverflowsWrapper ? `  ← TABLE PAST WRAPPER by ${r.table.right - r.wrapper.right}px (wrapper overflow-x: ${r.wrapper.overflowX})` : "";
  console.log(
    `  · ${route.padEnd(24)} table.right=${String(r.table.right).padEnd(5)} wrapper.right=${String(r.wrapper.right).padEnd(5)} sw/cw=${r.wrapper.scrollWidth}/${r.wrapper.clientWidth} pastWrapper=${r.cellsPastWrapper}${flag}`,
  );
}

writeFileSync(resolve(OUT, "geometry.json"), JSON.stringify(results, null, 2) + "\n");
console.log("wrote design/qa-p47-rendered-states/geometry.json");
await context.close();
await browser.close();
