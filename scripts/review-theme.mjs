/**
 * Theme review harness for nex-n2.5-pro (UI review agent).
 * Captures responsive + light/dark screenshots against the mockup reference.
 * Usage: npx playwright test scripts/review-theme.mjs --project=chromium
 *        or: node -r ts-node/register scripts/review-theme.mjs (if ts-node available)
 */
import { chromium } from "playwright";

const VIEWPORTS = [
  { name: "desktop", width: 1440, height: 900 },
  { name: "laptop", width: 1024, height: 768 },
  { name: "tablet", width: 768, height: 1024 },
  { name: "narrow", width: 480, height: 900 },
];

const MODES = ["light", "dark"];
const OUTPUT_DIR = "design/qa-tangerine";

async function capture() {
  for (const mode of MODES) {
    for (const vp of VIEWPORTS) {
      const browser = await chromium.launch({ headless: true });
      const context = await browser.newContext({ viewport: { width: vp.width, height: vp.height } });
      const page = await context.newPage();

      // Open the mockup reference (for side-by-side comparison context)
      // The web app is expected at localhost:3000; adjust if different.
      const appUrl = process.env.APP_URL || "http://localhost:3000";
      const mockupPath = "file://" + process.cwd() + "/design/mockups/meridian-ticket-workbench-html-mockup.html";

      await page.goto(appUrl, { waitUntil: "networkidle" });
      // Toggle dark mode via data-theme if the app supports it; otherwise rely on CSS media.
      // The Tangerine preset uses `.dark` / `data-theme="dark"` (see globals.css).
      await page.evaluate((dark) => {
        if (dark) document.documentElement.classList.add("dark");
        else document.documentElement.classList.remove("dark");
      }, mode === "dark");
      await page.waitForTimeout(200); // allow theme transition (≤120ms per motion law + buffer)

      // Capture: full page + viewport-cropped (header, workspace, agent panel)
      await page.screenshot({ path: `${OUTPUT_DIR}/${mode}-${vp.name}-full.png`, fullPage: true });
      await page.screenshot({ path: `${OUTPUT_DIR}/${mode}-${vp.name}-viewport.png`, fullPage: false });

      // Capture mockup reference for comparison
      const mockPage = await context.newPage();
      await mockPage.goto(mockupPath, { waitUntil: "networkidle" });
      await mockPage.screenshot({ path: `${OUTPUT_DIR}/mockup-ref-${vp.name}.png`, fullPage: false });

      await browser.close();
      console.log(`Captured ${mode}/${vp.name}`);
    }
  }
  console.log("Review capture complete. See design/qa-tangerine/");
  console.log("Run: node -e \"console.log('Compare: ' + require('fs').readdirSync('design/qa-tangerine').filter(f=>f.endsWith('.png')).join(', '))\"");
}

capture().catch((e) => {
  console.error("Theme review capture failed:", e);
  process.exit(1);
});
