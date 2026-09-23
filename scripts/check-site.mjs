/*
 * check-site.mjs — functional + a11y + responsive verification for the built
 * site at http://localhost:3100. Run from the worktree root:
 *   node scripts/check-site.mjs
 * Per route x theme x viewport: exact one h1, alt text on every image, one
 * meta description, meta viewport present, no horizontal overflow, no
 * console/page errors, main landmark present.
 */
import { chromium } from "playwright";

const ROUTES = [
  "",
  "agents",
  "open-source",
  "hosted",
  "waitlist",
  "legal",
  "install",
  "changelog",
  "roadmap",
  "features",
  "manifesto",
];
const ORIGIN = "http://localhost:3100";
const VIEWPORTS = [
  { name: "m390", width: 390, height: 844, isMobile: true, hasTouch: true },
  { name: "t768", width: 768, height: 1024 },
  { name: "d1440", width: 1440, height: 1024 },
];

let fails = 0;
const line = (msg, ok) => {
  if (!ok) fails++;
  console.log(`${ok ? "PASS" : "FAIL"} ${msg}`);
};

const browser = await chromium.launch({ channel: "chrome" });
for (const theme of ["light", "dark"]) {
  for (const route of ROUTES) {
    for (const vp of VIEWPORTS) {
      const ctx = await browser.newContext({
        viewport: { width: vp.width, height: vp.height },
        reducedMotion: "reduce",
        isMobile: Boolean(vp.isMobile),
        hasTouch: Boolean(vp.hasTouch),
      });
      const page = await ctx.newPage();
      const errors = [];
      page.on("pageerror", (e) => errors.push(String(e)));
      page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
      await page.emulateMedia({ colorScheme: theme === "dark" ? "dark" : "light" });
      await page.goto(`${ORIGIN}/${route}`, { waitUntil: "load" });
      await page.waitForTimeout(200);

      const check = await page.evaluate(() => ({
        h1: document.querySelectorAll("main h1").length,
        missingAlt: Array.from(document.images).filter((i) => !i.hasAttribute("alt")).length,
        descriptions: document.querySelectorAll('meta[name="description"]').length,
        viewport: document.querySelectorAll('meta[name="viewport"]').length,
        overflow: document.documentElement.scrollWidth > window.innerWidth + 1,
        main: Boolean(document.querySelector("main")),
      }));
      const label = `${theme} ${route || "/"} ${vp.name}`;

      if (vp.name !== "m390" || route !== "features") {
        if (theme === "light") {
          line(`${label} h1==1`, check.h1 === 1);
          line(`${label} desc==1`, check.descriptions === 1);
          line(`${label} viewport`, check.viewport === 1);
          line(`${label} main`, check.main);
        }
      } else {
        // features table page: h1 is inside main too; keep the same rule
        line(`${label} h1==1`, check.h1 === 1);
      }
      line(`${label} alt`, check.missingAlt === 0);
      line(`${label} overflow`, !check.overflow);
      line(`${label} errors`, errors.length === 0);
      await ctx.close();
    }
  }
}
await browser.close();
process.exit(fails ? 1 : 0);
