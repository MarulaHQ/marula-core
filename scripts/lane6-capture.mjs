// Wave 17 lane 6 vision-review capture: light+dark × 1440/1024/768/480 for
// /kb, /automations, /alerts, /audit, /provenance, /imports, /inventory,
// /search, driven against the double-clickable local test app (ADR 0005):
// API 15433, web 15435. Plus 1440 interaction states: the automation rule
// editor dialog, an expanded audit row, and the kb article reader.
import { chromium } from "playwright";
import { readFileSync } from "node:fs";

const WEB = "http://127.0.0.1:15435";
const OUT = "design/qa-knowledge-automation";
const tokens = JSON.parse(readFileSync("/tmp/kilo/owner-login.json", "utf8"));
const WIDTHS = [1440, 1024, 768, 480];
const THEMES = ["light", "dark"];

const SURFACES = [
  { name: "kb", path: "/kb", ready: "text=Knowledge base" },
  { name: "automations", path: "/automations", ready: "text=Automation rules" },
  { name: "alerts", path: "/alerts", ready: "text=Alerts" },
  { name: "audit", path: "/audit", ready: "text=Audit trail" },
  { name: "provenance", path: "/provenance", ready: /Provenance/.source },
  { name: "imports", path: "/imports", ready: "text=Imports" },
  { name: "inventory", path: "/inventory", ready: "text=Inventory" },
  { name: "search", path: "/search", ready: "input[aria-label=Search]" },
];

const browser = await chromium.launch();
for (const theme of THEMES) {
  // Width-matrix captures share one context per theme.
  for (const surface of SURFACES) {
    for (const width of WIDTHS) {
      const context = await browser.newContext();
      await context.addInitScript(
        ({ tokens, theme }) => {
          window.__MERIDIAN_API_URL__ = "http://127.0.0.1:15433";
          window.localStorage.setItem("meridian-tokens", JSON.stringify(tokens));
          window.localStorage.setItem("meridian-tokens-exp", String(Date.now() + 3600_000));
          window.localStorage.setItem("meridian-theme", theme);
          try {
            document.documentElement.dataset.theme = theme;
          } catch {}
        },
        { tokens, theme },
      );
      const page = await context.newPage();
      await page.setViewportSize({ width, height: width === 480 ? 1400 : 1000 });
      await page.goto(`${WEB}${surface.path}`, { waitUntil: "domcontentloaded" });
      await page.waitForTimeout(1200);
      // Wait for either data or the typed empty state (honest-states rule).
      await page.waitForLoadState("networkidle", { timeout: 12000 }).catch(() => {});
      await page.screenshot({ path: `${OUT}/${surface.name}-${theme}-${width}.png`, fullPage: true });
      await context.close();
    }
  }

  // Interaction states at 1440 only (rule editor, audit expand, kb reader, alerts facet).
  const context = await browser.newContext();
  await context.addInitScript(
    ({ tokens, theme }) => {
      window.__MERIDIAN_API_URL__ = "http://127.0.0.1:15433";
      window.localStorage.setItem("meridian-tokens", JSON.stringify(tokens));
      window.localStorage.setItem("meridian-tokens-exp", String(Date.now() + 3600_000));
      window.localStorage.setItem("meridian-theme", theme);
      try {
        document.documentElement.dataset.theme = theme;
      } catch {}
    },
    { tokens, theme },
  );
  const page = await context.newPage();
  await page.setViewportSize({ width: 1440, height: 1000 });

  // Rule editor dialog
  await page.goto(`${WEB}/automations`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1500);
  const newRule = page.getByRole("button", { name: "New rule", exact: true }).first();
  if (await newRule.isVisible().catch(() => false)) {
    await newRule.click();
    await page.waitForTimeout(600);
    await page.screenshot({ path: `${OUT}/rule-editor-${theme}-1440.png` });
    await page.keyboard.press("Escape");
    await page.waitForTimeout(300);
  }

  // Alerts facet row focus
  await page.goto(`${WEB}/alerts`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1500);
  await page.screenshot({ path: `${OUT}/alerts-facets-${theme}-1440.png` });

  // Audit expand
  await page.goto(`${WEB}/audit`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1500);
  const inspect = page.locator("tbody tr").first();
  if ((await inspect.count()) > 0) {
    await inspect.click();
    await page.waitForTimeout(500);
    await page.screenshot({ path: `${OUT}/audit-expanded-${theme}-1440.png` });
  }

  // KB article reader
  await page.goto(`${WEB}/kb`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1500);
  const viewBtn = page.getByRole("button", { name: "View", exact: true }).first();
  if (await viewBtn.isVisible().catch(() => false)) {
    await viewBtn.click();
    await page.waitForTimeout(1200);
    await page.screenshot({ path: `${OUT}/kb-view-${theme}-1440.png` });
    await page.keyboard.press("Escape");
    await page.waitForTimeout(300);
  }

  // Search with keyboard cursor
  await page.goto(`${WEB}/search`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(800);
  const searchBox = page.locator("input[aria-label=Search]").first();
  if (await searchBox.isVisible().catch(() => false)) {
    await searchBox.fill("invoice");
    await page.waitForTimeout(2000);
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");
    await page.waitForTimeout(400);
  }
  await page.screenshot({ path: `${OUT}/search-cursor-${theme}-1440.png` });

  await context.close();
}
await browser.close();
console.log("captures complete");
