import { chromium } from "playwright";
import { readFileSync } from "node:fs";
const css = readFileSync(new URL("./app.css", import.meta.url), "utf8");
const browser = await chromium.launch({ args: ["--no-proxy-server"] });
const page = await browser.newPage({ viewport: { width: 1180, height: 400 } });
await page.setContent(`<!doctype html><html data-theme="light"><head><style>${css}</style></head><body>
<table class="m-table"><thead><tr>
<th scope="col"><button class="m-th-sort">Number<span class="m-th-sort-mark">x</span></button></th>
<th scope="col">Assignee</th>
</tr></thead></table></body></html>`, { waitUntil: "load" });
const out = await page.evaluate(() => {
  const rows = [];
  for (const th of document.querySelectorAll("th")) {
    const cs = getComputedStyle(th);
    const btn = th.querySelector("button");
    const bs = btn ? getComputedStyle(btn) : null;
    rows.push({
      th: th.textContent.trim(),
      thTransform: cs.textTransform,
      btnTransform: bs ? bs.textTransform : null,
      btnFont: bs ? bs.font : null,
    });
  }
  return rows;
});
console.log(JSON.stringify(out, null, 1));
await browser.close();
