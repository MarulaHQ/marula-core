// p44 focused re-capture: the /documents/[id] detail state (both themes) —
// the title cell is a LINK, so the battery's button click never navigated.
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/+$/, "");
const OUT = "design/qa-p44-ui-knowledge-automation/captures";
mkdirSync(OUT, { recursive: true });
const results = [];
const note = (name, ok, detail = "") => {
  results.push({ name, ok, detail });
  console.log(`  [assert] ${ok ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
};

const browser = await chromium.launch();
for (const theme of ["light", "dark"]) {
  const ctx = await browser.newContext({
    colorScheme: theme === "dark" ? "dark" : "light",
    viewport: { width: 1440, height: 1000 },
    reducedMotion: "reduce",
  });
  const page = await ctx.newPage();
  for (let i = 0; i < 4; i += 1) {
    await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
    const submit = page.locator('button[type="submit"]');
    await submit.waitFor({ state: "visible" });
    await page.waitForTimeout(1500);
    await page.fill('input[type="email"]', "owner@demo.local");
    await page.fill('input[type="password"]', "demo-password-123");
    await submit.click();
    try {
      await page.waitForURL(/tickets|today|dashboard/, { timeout: 45000 });
      break;
    } catch {
      await page.waitForTimeout(5000);
    }
  }
  await page.goto(`${WEB}/documents`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(2500);
  await page.getByRole("link", { name: /Runbook — Client Onboarding/ }).first().click();
  await page.getByTestId("document-versions").first().waitFor({ state: "visible", timeout: 25000 }).catch(() => {});
  await page.waitForTimeout(1000);
  await page.screenshot({ path: `${OUT}/${theme}-11-document-detail.png` });
  console.log(`  [shot] ${theme}-11-document-detail`);
  const state = await page.evaluate(() => ({
    url: location.pathname,
    versions: document.querySelectorAll('[data-testid="document-versions"] tbody tr').length,
    rail: !!document.querySelector('[data-testid="document-rail"]'),
    title: document.querySelector("h1")?.textContent?.trim().slice(0, 60),
    h1: `${getComputedStyle(document.querySelector("h1")).fontSize}/${getComputedStyle(document.querySelector("h1")).fontWeight}`,
  }));
  note(`${theme}-11: the detail page opens with its version history`, state.versions > 0 && state.rail, JSON.stringify(state));
  await ctx.close();
}
await browser.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`[capture] ${results.length - failed}/${results.length} assertions pass`);
process.exit(failed > 0 ? 1 : 0);
