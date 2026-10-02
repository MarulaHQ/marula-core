// Two interaction captures that close specific verdict claims, now that
// we know both dropdowns are Radix Selects (role=combobox, portal options):
//   1. /settings/email — open "Outbound transport", record the option
//      labels (none / postmark / ses / smtp — claim 10), select Amazon SES
//      to photograph its credential form. Local state only; never saved.
//   2. /calendar — open the connect form's "Calendar source" select and
//      record the provider kinds (ical / google / microsoft — claim 5).
import { chromium } from "playwright";
import { mkdir, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { resolve } from "node:path";

const OUT = resolve("design/qa-p34-spec-verdict-sweep/captures");
const WEB = process.env.E2E_WEB_URL ?? "http://localhost:3004";
const EMAIL = process.env.E2E_DEMO_EMAIL ?? "owner@demo.local";
const PASSWORD = process.env.E2E_DEMO_PASSWORD ?? "demo-password-123";
if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const browser = await chromium.launch();
const context = await browser.newContext({
  viewport: { width: 1440, height: 1000 },
  colorScheme: "light",
  deviceScaleFactor: 2,
});
const page = await context.newPage();

await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1500);
await page.fill('input[type="email"]', EMAIL);
await page.fill('input[type="password"]', PASSWORD);
await page.click('button[type="submit"]');
await page.waitForFunction(
  () => !document.querySelector('[data-testid="login-card"]'),
  { timeout: 30_000 },
);
await page.waitForTimeout(600);

// 1. Outbound transport options.
await page.goto(`${WEB}/settings/email`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(2500);
const trigger = page.getByRole("combobox", { name: /outbound transport/i }).first();
await trigger.scrollIntoViewIfNeeded();
await trigger.click();
await page.waitForTimeout(500);
const options = await page.getByRole("option").allTextContents();
console.log("transport options:", JSON.stringify(options));
await writeFile(
  resolve(OUT, "settings-email-transport-options.json"),
  JSON.stringify(options, null, 2),
);
await page.screenshot({ path: resolve(OUT, "settings-email-transport-open.png") });
console.log("captured settings-email-transport-open");

// Select SES to show its credential form (local state; not saved).
const sesOption = page.getByRole("option", { name: /ses/i }).first();
if (await sesOption.count()) {
  await sesOption.click();
  await page.waitForTimeout(700);
  await page.screenshot({ path: resolve(OUT, "settings-email-ses-form.png") });
  console.log("captured settings-email-ses-form");
} else {
  console.log("no SES option found — options list is the evidence");
}

// 2. Calendar source kinds.
await page.goto(`${WEB}/calendar`, { waitUntil: "domcontentloaded" });
await page.waitForTimeout(2000);
const connect = page.getByRole("button", { name: /connect calendar/i }).first();
await connect.click();
await page.waitForTimeout(600);
const source = page.getByRole("combobox").nth(1); // direction is nth(0)
await source.scrollIntoViewIfNeeded();
await source.click();
await page.waitForTimeout(500);
const kinds = await page.getByRole("option").allTextContents();
console.log("calendar source kinds:", JSON.stringify(kinds));
await writeFile(
  resolve(OUT, "calendar-source-kinds.json"),
  JSON.stringify(kinds, null, 2),
);
await page.screenshot({ path: resolve(OUT, "calendar-source-kinds.png") });
console.log("captured calendar-source-kinds");
await page.keyboard.press("Escape");

await context.close();
await browser.close();
