#!/usr/bin/env node
/**
 * p47-clip-probe — measure whether a row's cell content is clipped.
 *
 * The main harness measures the row's box against its container. It does not
 * measure the content INSIDE a cell. This probe does: per cell, clientWidth
 * against scrollWidth, the computed overflow, and whether a text-overflow
 * ellipsis is declared. A cell whose scrollWidth exceeds its clientWidth is
 * clipping its content, and with no ellipsis the reader sees a hard cut.
 *
 * Usage: node scripts/p47-clip-probe.mjs [--surface approvals,billing,...]
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
  ["approvals", "/approvals"],
  ["billing", "/billing"],
  ["tickets", "/tickets"],
  ["clients", "/clients"],
  ["contracts", "/contracts"],
  ["projects", "/projects"],
  ["provenance", "/provenance"],
  ["audit", "/audit"],
];

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1600, height: 994 } });
const page = await context.newPage();
await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded", timeout: 60_000 });
await page.fill("#login-email", "owner@demo.local", { timeout: 60_000 });
await page.fill("#login-password", "demo-password-123", { timeout: 60_000 });
await page.click('[data-testid="login-submit"]', { timeout: 60_000 });
await page.waitForFunction(() => !location.pathname.startsWith("/login"), null, { timeout: 60_000 });

// The table's width depends on the viewport, so a clip that is absent at 1600
// can be present at a laptop width. Measure at three.
const WIDTHS = [1600, 1440, 1280];

const results = {};
for (const width of WIDTHS) {
  await page.setViewportSize({ width, height: 994 });
  for (const [key, path] of ROUTES) {
    await page.goto(`${WEB}${path}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
    await page.waitForTimeout(1100);
    const r = await page.evaluate((p) => {
    const rows = [...document.querySelectorAll("table tbody tr, [data-testid$='-row']")].filter((r) => r.querySelector("td"));
    if (!rows.length) return { ok: false, why: "no row with cells" };
    // Scan EVERY row: the focal row fitted, and the reported clip was on a row
    // the single-row probe never looked at.
    const all = [];
    rows.forEach((row, ri) => {
      [...row.querySelectorAll(":scope > td")].forEach((c, i) => {
        const cs = getComputedStyle(c);
        if (c.scrollWidth > c.clientWidth + 1) {
          all.push({
            row: ri,
            cell: i,
            clientWidth: c.clientWidth,
            scrollWidth: c.scrollWidth,
            overflowX: cs.overflowX,
            textOverflow: cs.textOverflow,
            whiteSpace: cs.whiteSpace,
            text: (c.textContent ?? "").trim().slice(0, 60),
          });
        }
      });
    });
    return {
      ok: true,
      path: p,
      rowsScanned: rows.length,
      clipped: all,
      clippedCells: all.length,
      clippedWithoutEllipsis: all.filter((c) => c.textOverflow !== "ellipsis").length,
    };
  }, path);
    results[`${key}@${width}`] = r;
    console.log(
      `  · ${key.padEnd(12)} ${String(width).padEnd(5)} rows=${r.ok ? r.rowsScanned : "-"} clipped=${r.ok ? r.clippedCells : "-"} without-ellipsis=${r.ok ? r.clippedWithoutEllipsis : "-"}`,
    );
  }
}

writeFileSync(resolve(OUT, "clip-probe.json"), JSON.stringify(results, null, 2) + "\n");
console.log("wrote design/qa-p47-rendered-states/clip-probe.json");
await context.close();
await browser.close();
