// p34-verdicts-closeout — vision capture for the four new UI surfaces.
//
// Captures the planning page (forecast + allocations + work-next) and the
// project page (Gantt) in BOTH themes, in ONE browser context per viewport
// so the two themes are comparable frame for frame.
//
// Batch discipline: this script produces 6 images. The review that consumes
// them is expected to read them in ≤8-image batches and NEVER more than 30 in
// one request; 6 is comfortably inside both limits.
import { chromium } from "playwright";
import { mkdir, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { resolve } from "node:path";

const OUT = resolve(
  process.argv[2] ?? "design/qa-p34-verdicts-closeout/captures/ui",
);
const WEB = process.env.E2E_WEB_URL ?? "http://localhost:3000";
const API = (process.env.E2E_API_URL ?? "http://localhost:28480/").replace(
  /\/+$/,
  "",
);
const EMAIL = process.env.E2E_DEMO_EMAIL ?? "owner@demo.local";
const PASSWORD = process.env.E2E_DEMO_PASSWORD ?? "demo-password-123";

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const browser = await chromium.launch();
const manifest = [];

// One context per theme, so `prefers-color-scheme` and the app's stored
// theme agree and a screenshot is not half-light/half-dark.
for (const scheme of ["light", "dark"]) {
  const context = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
    colorScheme: scheme,
    deviceScaleFactor: 2,
  });
  const page = await context.newPage();
  const problems = [];
  page.on("console", (m) => {
    if (m.type() === "error") problems.push(m.text().slice(0, 200));
  });
  page.on("pageerror", (e) => problems.push(`pageerror: ${String(e).slice(0, 200)}`));

  // Sign in through the real form: a token injected into storage would skip
  // the very hydration this capture is meant to exercise.
  await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
  await page.fill('input[type="email"]', EMAIL);
  await page.fill('input[type="password"]', PASSWORD);
  await page.click('button[type="submit"]');
  // Wait for the LOGIN CARD to disappear rather than for a URL change: the
  // app's post-login navigation is a client-side push, and waiting on the
  // URL can miss a transition that already happened.
  try {
    await page.waitForFunction(
      () => !document.querySelector('[data-testid="login-card"]'),
      { timeout: 30_000 },
    );
  } catch {
    // Record WHY rather than aborting the run: a login that silently fails
    // is exactly the kind of thing a screenshot pass exists to surface.
    const err = await page
      .locator('[data-testid="login-error"]')
      .first()
      .textContent()
      .catch(() => null);
    await page.screenshot({
      path: resolve(OUT, `LOGIN-FAILED-${scheme}.png`),
      fullPage: true,
    });
    throw new Error(
      `login did not complete (${scheme})` +
        (err ? `: ${err.trim()}` : "") +
        ` — screenshot at LOGIN-FAILED-${scheme}.png`,
    );
  }
  await page.waitForTimeout(800);

  const shot = async (name, path, opts = {}) => {
    await page.goto(`${WEB}${path}`, { waitUntil: "networkidle", timeout: 60_000 });
    // Let the panels finish their fetches and render.
    await page.waitForTimeout(1200);
    if (opts.scrollToHeading) {
      // Scrolled by TEXT inside the browser, because `text=` is a Playwright
      // selector and `document.querySelector` does not understand it.
      await page.evaluate((heading) => {
        const el = [...document.querySelectorAll("h1, h2, h3")].find((n) =>
          (n.textContent ?? "").includes(heading),
        );
        el?.scrollIntoView({ block: "center" });
      }, opts.scrollToHeading);
      await page.waitForTimeout(500);
    }
    const file = resolve(OUT, `${name}.png`);
    await page.screenshot({ path: file, fullPage: opts.fullPage ?? false });
    manifest.push({ name, path, scheme, file, consoleErrors: [...problems] });
    problems.length = 0;
    console.log(`  captured ${name} (${scheme})`);
  };

  // 1 + 2: the planning page, top (the three new panels) and the horizon
  // below them, so the new work is comparable with the panel it sits beside.
  await shot(`planning-top-${scheme}`, "/dispatch/planning");
  await shot(
    `planning-full-${scheme}`,
    "/dispatch/planning",
    { fullPage: true },
  );

  // 3: the project page, scrolled to the Gantt.
  const projectId = process.env.P34_PROJECT_ID;
  if (projectId) {
    await shot(`gantt-${scheme}`, `/projects/${projectId}`, {
      scrollToHeading: "Timeline",
    });
  } else {
    console.log("  (no P34_PROJECT_ID — skipping the Gantt shot)");
  }

  await context.close();
}

await browser.close();
await writeFile(resolve(OUT, "manifest.json"), JSON.stringify(manifest, null, 2));
console.log(`\n[vision] ${manifest.length} captures -> ${OUT}`);
for (const m of manifest) {
  if (m.consoleErrors.length) {
    console.log(`  ⚠ ${m.name}: ${m.consoleErrors.length} console error(s)`);
    for (const e of m.consoleErrors.slice(0, 3)) console.log(`      ${e}`);
  }
}
