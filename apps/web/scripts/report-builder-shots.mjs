// Screenshot pass for /reports (R6 builder): light+dark × desktop/narrow.
// Login rides the real API; tokens are seeded into localStorage so the
// AuthProvider restores instantly.
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const BASE = "http://localhost:3000";
const API = "http://localhost:8080";
const OUT = "design/qa-report-builder";

mkdirSync(OUT, { recursive: true });

const login = await fetch(`${API}/api/auth/login`, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ email: "owner@demo.local", password: "demo-password-123" }),
});
if (!login.ok) throw new Error(`login failed: ${login.status}`);
const tokens = await login.json();

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });

async function seedTokens(p) {
  await p.goto(`${BASE}/login`);
  await p.evaluate((tokens) => {
    window.localStorage.setItem("marula-tokens", JSON.stringify(tokens));
    window.localStorage.setItem("marula-tokens-exp", String(Date.now() + 900_000));
  }, tokens);
}

await seedTokens(page);

for (const theme of ["light", "dark"]) {
  await page.goto(`${BASE}/reports`);
  await page.reload();
  // Toggle via the header control so the app's own path applies the theme.
  const want = theme;
  const current = await page.evaluate(() => document.documentElement.dataset.theme ?? "light");
  if (current !== want) {
    await page.getByRole("button", { name: /Switch to/ }).click();
  }
  await page.waitForSelector("text=Reports", { timeout: 20000 });
  await page.waitForTimeout(1200);
  await page.screenshot({ path: `${OUT}/${theme}-builder-desktop.png`, fullPage: true });

  // Run a preview with the default draft (grand total).
  await page.getByRole("button", { name: "Run preview" }).click();
  await page.waitForTimeout(2000);
  await page.screenshot({ path: `${OUT}/${theme}-preview-desktop.png`, fullPage: true });

  // Grouped bar chart: add a grouping, run, capture chart + table.
  await page.getByRole("button", { name: "Add grouping" }).click();
  await page.getByRole("button", { name: "Run preview" }).click();
  await page.waitForTimeout(2000);
  await page.screenshot({ path: `${OUT}/${theme}-chart-desktop.png`, fullPage: true });

  // Narrow viewport (768) — config stacks above preview.
  await page.setViewportSize({ width: 768, height: 1024 });
  await page.waitForTimeout(400);
  await page.screenshot({ path: `${OUT}/${theme}-builder-768.png`, fullPage: true });

  // Saved reports tab.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.getByRole("tab", { name: "Saved reports" }).click();
  await page.waitForTimeout(800);
  await page.screenshot({ path: `${OUT}/${theme}-saved-desktop.png`, fullPage: true });
}

await browser.close();
console.log("screenshots written to", OUT);
