// Rasterize the canonical colour lockup SVGs to PNGs for native (no SVG
// runtime in RN). Source of truth: apps/web/public/lockup-color{,-on-dark}.svg
// (byte copies of the kit's lockups, per design/qa-p41-brand-assets).
import { chromium } from "playwright";
import { readFileSync } from "node:fs";
const scale = 3;
const b = await chromium.launch();
const page = await b.newPage();
for (const variant of ["color", "color-on-dark"]) {
  const svg = readFileSync(`apps/web/public/lockup-${variant}.svg`, "utf8");
  await page.setContent(`<style>*{margin:0}html,body{background:transparent}</style>${svg}`);
  await page.waitForTimeout(300);
  const el = await page.$("svg");
  await el.screenshot({
    path: `native/assets/lockup-${variant}.png`,
    omitBackground: true,
    scale: "device",
  });
  console.log(`wrote native/assets/lockup-${variant}.png`);
}
await b.close();
