#!/usr/bin/env node
/**
 * projects-capture — screenshot the plan and timeline surfaces.
 *
 *   node scripts/projects-capture.mjs <output-dir> [--project <id>] [--only plan|timeline]
 *
 * Runs against an ALREADY RUNNING web app; it never builds, never boots a
 * stack and never writes to the database. Point it at the app with
 * `MARULA_WEB_URL` (default `http://localhost:3001`) and sign in with
 * `MARULA_EMAIL` / `MARULA_PASSWORD` (defaults are the demo owner).
 *
 * Every capture is 1440, 1280 or 1024 wide at 1040 tall — the three widths the
 * design pack draws the timeline at — in both themes, because a chart is the
 * surface where a dark-mode regression hides in a gridline.
 */
import { chromium } from "playwright";
import { mkdir } from "node:fs/promises";
import path from "node:path";

const OUT_DIR = process.argv[2];
if (OUT_DIR === undefined || OUT_DIR === "") {
  console.error("usage: node scripts/projects-capture.mjs <output-dir> [--project <id>] [--only plan|timeline]");
  process.exit(1);
}

const argv = process.argv.slice(3);
const flag = (name) => {
  const index = argv.indexOf(name);
  return index === -1 ? null : argv[index + 1] ?? null;
};

const WEB = (process.env.MARULA_WEB_URL ?? process.env.BASE_URL ?? "http://localhost:3001").replace(/\/+$/, "");
const EMAIL = process.env.MARULA_EMAIL ?? "owner@demo.local";
const PASSWORD = process.env.MARULA_PASSWORD ?? "demo-password-123";
const ONLY = flag("--only");
const WIDTHS = [1440, 1280, 1024];
const THEMES = ["neutral", "dark"];
const HEIGHT = 1040;

async function signIn(context) {
  const page = await context.newPage();
  await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
  const email = page.locator('input[type="email"]');
  if ((await email.count()) === 0) {
    // Already signed in through the shared session cookie.
    return page;
  }
  await email.fill(EMAIL);
  await page.locator('input[type="password"]').fill(PASSWORD);
  await page.locator('button[type="submit"]').click();
  await page.waitForURL((url) => !new URL(url).pathname.startsWith("/login"), { timeout: 45_000 });
  return page;
}

async function resolveProjectId(page) {
  const explicit = flag("--project") ?? process.env.MARULA_PROJECT_ID ?? null;
  if (explicit !== null && explicit !== "") return explicit;
  await page.goto(`${WEB}/projects`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1500);
  const link = page.locator('a[href^="/projects/"]').filter({ hasNot: page.locator('a[href="/projects"]') }).first();
  await link.waitFor({ timeout: 20_000 });
  const href = await link.getAttribute("href");
  if (href === null) throw new Error("no project link on /projects; pass --project <id>");
  return href.split("/")[2];
}

async function settle(page, surface) {
  // The plan surface was rebuilt on its own shell, so wait for the plan grid
  // itself rather than the old record-tab strip; the timeline waits for the
  // chart or its table.
  const marker =
    surface === "timeline"
      ? '[data-testid="gantt-chart"], [data-testid="gantt-list"], [data-testid="gantt-table"]'
      : '[data-slot="plan-table"], [data-testid="project-schedule"], [data-testid="plan-loading"]';
  await page.locator(marker).first().waitFor({ timeout: 30_000 });
  // The chart draws after its reads, its dependency edges and the computed
  // chain land; one quiet moment is the honest wait.
  await page.waitForLoadState("networkidle").catch(() => {});
  await page.waitForTimeout(600);
}

const main = async () => {
  const dir = path.resolve(OUT_DIR);
  await mkdir(dir, { recursive: true });
  const browser = await chromium.launch();
  const written = [];

  for (const theme of THEMES) {
    const context = await browser.newContext({
      viewport: { width: 1440, height: HEIGHT },
      deviceScaleFactor: 1,
      colorScheme: theme === "dark" ? "dark" : "light",
    });
    const page = await signIn(context);
    const projectId = await resolveProjectId(page);

    for (const width of WIDTHS) {
      await page.setViewportSize({ width, height: HEIGHT });
      const surfaces = ONLY === null ? ["plan", "timeline"] : [ONLY];
      for (const surface of surfaces) {
        const url = surface === "timeline" ? `${WEB}/projects/${projectId}/timeline` : `${WEB}/projects/${projectId}`;
        await page.goto(url, { waitUntil: "domcontentloaded" });
        // The mode is an attribute on <html>; the app writes it on boot, so set
        // it again after navigation rather than fighting the bootstrap.
        await page.evaluate((value) => {
          document.documentElement.dataset.theme = value;
        }, theme);
        await settle(page, surface);
        await page.evaluate((value) => {
          document.documentElement.dataset.theme = value;
        }, theme);
        const file = path.join(dir, `${surface}-${theme}-${width}.png`);
        await page.screenshot({ path: file, fullPage: false });
        written.push(file);
        console.log(`captured ${path.relative(process.cwd(), file)}`);
      }
    }
    await context.close();
  }

  await browser.close();
  console.log(`\n${written.length} captures in ${path.relative(process.cwd(), dir)}`);
  console.log(`base url ${WEB}`);
};

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
