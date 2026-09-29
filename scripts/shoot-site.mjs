/*
 * shoot-site.mjs — full-page captures of the built site for visual review.
 * Run from the worktree root with the dev server on :3100 running.
 * Outputs design/qa-p28-website-brief/site-captures/<route>--<vp>--<theme>.png
 */
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const OUT = new URL("../design/qa-p28-website-brief/site-captures/", import.meta.url).pathname;
mkdirSync(OUT, { recursive: true });

const ROUTES = ["", "agents", "open-source", "hosted", "waitlist", "legal", "changelog"];
const VIEWPORTS = [
  { name: "desktop-1440", width: 1440, height: 1024 },
  { name: "mobile-390", width: 390, height: 844, dsf: 2, mobile: true },
];
const ORIGIN = "http://localhost:3100";

const browser = await chromium.launch({ channel: "chrome" });
for (const vp of VIEWPORTS) {
  for (const route of ROUTES) {
    for (const theme of ["light", "dark"]) {
      const ctx = await browser.newContext({
        viewport: { width: vp.width, height: vp.height },
        deviceScaleFactor: vp.dsf ?? 1,
        isMobile: Boolean(vp.mobile),
        hasTouch: Boolean(vp.mobile),
        reducedMotion: "reduce", // deterministic captures; scroll-reveals excluded
      });
      const page = await ctx.newPage();
      await page.emulateMedia({ colorScheme: theme === "dark" ? "dark" : "light" });
      await page.goto(`${ORIGIN}/${route}`, { waitUntil: "load" });
      await page.evaluate((t) => {
        localStorage.setItem("marula-theme", t);
        document.documentElement.dataset.theme = t;
      }, theme);
      const navSpans = await page.locator(".pill-nav .links a").count();
      await page.waitForTimeout(400);
      const name = `${route.replace(/\/$/, "") || "home"}--${vp.name}--${theme}`;
      // fullPage mobile captures on long pages lose fidelity past ~8k px; the
      // seg-shooter covers those; here desktop gets fullPage, mobile top only
      if (vp.name === "desktop-1440") {
        await page.screenshot({ path: `${OUT}${name}.png`, fullPage: true });
      } else {
        await page.screenshot({ path: `${OUT}${name}.png` });
      }
      console.log(`shot ${name} navLinks=${navSpans}`);
      await ctx.close();
    }
  }
}
await browser.close();
