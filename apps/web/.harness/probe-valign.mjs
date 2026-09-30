import { chromium } from "playwright";
import { readFileSync } from "node:fs";
const css = readFileSync(new URL("./app.css", import.meta.url), "utf8");
const browser = await chromium.launch({ args: ["--no-proxy-server"] });
const page = await browser.newPage({ viewport: { width: 1180, height: 400 } });
await page.setContent(`<!doctype html><html data-theme="light"><head><style>${css}</style></head><body>
<table class="m-table"><thead><tr>
<th scope="col"><button class="m-th-sort">Number</button></th>
<th scope="col">Assignee</th>
</tr></thead></table></body></html>`, { waitUntil: "load" });
console.log(JSON.stringify(await page.evaluate(() => {
  const [a, b] = [...document.querySelectorAll("th")];
  const ra = a.getBoundingClientRect(), rb = b.getBoundingClientRect();
  const ba = a.querySelector("button").getBoundingClientRect();
  return {
    thHeight: [ra.height, rb.height],
    thTop: [ra.top, rb.top],
    buttonBox: { top: ba.top, h: ba.height },
    thAlign: getComputedStyle(a).verticalAlign,
    btnLine: getComputedStyle(a.querySelector("button")).lineHeight,
    thLine: getComputedStyle(b).lineHeight,
  };
}), null, 1));
await browser.close();
