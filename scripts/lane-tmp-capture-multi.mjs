// Lane capture, parametrized: hero workbench + dispatch board at 2K and 4K.
// Usage: node capture-multi.mjs <cssWidth> <cssHeight> <suffix>
import { chromium } from 'playwright';

const BASE = 'http://localhost:3001';
const W = Number(process.argv[2] ?? 2560);
const H = Number(process.argv[3] ?? 1440);
const S = process.argv[4] ?? 'alt';

const browser = await chromium.launch({
  executablePath: '/Users/iordn/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing',
  headless: true,
});

async function dismissTours(page) {
  for (let i = 0; i < 10; i++) {
    const banner = page.locator('text=Help lives in the top bar');
    const done = page.getByRole('button', { name: 'Done', exact: true });
    if ((await banner.count()) && (await done.count())) {
      await done.first().click({ timeout: 3000 }).catch(() => {});
      await page.waitForSelector('text=Help lives in the top bar', { state: 'detached', timeout: 5000 }).catch(() => {});
      break;
    }
    await page.waitForTimeout(700);
  }
  await page.waitForTimeout(600);
}

const ctx = await browser.newContext({ deviceScaleFactor: 2 });
const page = await ctx.newPage();
await page.setViewportSize({ width: W, height: H });
await page.emulateMedia({ reducedMotion: 'reduce' });

// hero workbench as the technician
await page.goto(`${BASE}/login`, { waitUntil: 'domcontentloaded' });
await page.waitForSelector('#login-email', { timeout: 20000 });
await page.fill('#login-email', 'tech@demo.local');
await page.fill('#login-password', 'demo-password-123');
await page.press('#login-password', 'Enter');
await page.waitForURL('**/today', { timeout: 30000 });
await dismissTours(page);
await page.goto(`${BASE}/tickets/6d657269-6469-616e-0000-000000000065`, { waitUntil: 'domcontentloaded' });
await page.waitForSelector('text=Symptoms match the IKE rekey overlap', { timeout: 40000 });
await page.waitForSelector('text=firewall IKE rekey fix', { timeout: 20000 });
try { await page.waitForSelector('text=Searching the knowledge base', { state: 'detached', timeout: 15000 }); } catch {}
await page.waitForTimeout(1500);
await page.screenshot({ path: `assets/shots/ticket_hero_light_${S}.png` });
await page.getByRole('button', { name: 'Switch to dark mode' }).click();
await page.waitForSelector('text=Symptoms match the IKE rekey overlap', { timeout: 40000 });
try { await page.waitForSelector('text=Searching the knowledge base', { state: 'detached', timeout: 15000 }); } catch {}
await page.waitForTimeout(1500);
await page.screenshot({ path: `assets/shots/ticket_hero_dark_${S}.png` });

// dispatch board as the dispatcher (own context so the tech session stays put)
const ctx2 = await browser.newContext({ deviceScaleFactor: 2 });
const page2 = await ctx2.newPage();
await page2.setViewportSize({ width: W, height: H });
await page2.emulateMedia({ reducedMotion: 'reduce' });
await page2.goto(`${BASE}/login`, { waitUntil: 'domcontentloaded' });
await page2.waitForSelector('#login-email', { timeout: 20000 });
await page2.fill('#login-email', 'dispatcher@demo.local');
await page2.fill('#login-password', 'demo-password-123');
await page2.press('#login-password', 'Enter');
await page2.waitForURL('**/today', { timeout: 30000 });
await dismissTours(page2);
await page2.goto(`${BASE}/dispatch`, { waitUntil: 'domcontentloaded' });
await page2.waitForSelector('text=Unassigned', { timeout: 30000 });
await page2.waitForTimeout(2500);
if (await page2.locator('text=Your session expired').count()) { console.error('banner on dispatch'); process.exit(1); }
await page2.screenshot({ path: `assets/shots/dispatch_light_${S}.png` });
await page2.getByRole('button', { name: /Switch to dark mode/i }).click();
await page2.waitForTimeout(2500);
if (await page2.locator('text=Your session expired').count()) { console.error('banner after theme switch'); process.exit(1); }
await page2.screenshot({ path: `assets/shots/dispatch_dark_${S}.png` });

await browser.close();
console.log(`captured ${W}x${H} as *_${S}.png`);
