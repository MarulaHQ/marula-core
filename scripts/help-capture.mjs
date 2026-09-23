#!/usr/bin/env node
/**
 * Help + tours capture — Wave 26 lane 2 (p26-contextual-help).
 *
 * Screenshots the contextual help affordance and the spotlight tour in
 * light AND dark into design/qa-p26-contextual-help/captures/. API
 * responses are fixtures served by Playwright routing — no backend
 * needed (auth bootstraps through a stubbed refresh rotation, mirroring
 * the memory-only token flow).
 *
 * Usage: start the web app (dev on :3000), then node scripts/help-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir, rm } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-p26-contextual-help/captures');
const BASE = process.argv[2] ?? 'http://localhost:3000';

if (existsSync(OUT)) await rm(OUT, { recursive: true });
await mkdir(OUT, { recursive: true });

const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: '00000000-0000-4000-8000-000000000002',
  email: 'owner@meridian.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_users', 'special:manage_boards', 'tickets:create', 'admin:org'],
};

const TOKENS = {
  access_token: 'test-access',
  refresh_token: 'test-refresh',
  session_id: 'sess',
  expires_in: 900,
};

const TICKETS = [
  {
    id: '30000000-0000-4000-8000-000000000001',
    org_id: ME.org_id,
    number: 'MER-1042',
    title: 'Laptop will not join the VPN after update',
    status: 'open',
    priority: 'high',
    client_name: 'Northwind Trading',
    created_at: new Date(Date.now() - 3600_000).toISOString(),
    updated_at: new Date(Date.now() - 600_000).toISOString(),
  },
  {
    id: '30000000-0000-4000-8000-000000000002',
    org_id: ME.org_id,
    number: 'MER-1041',
    title: 'Onboard two new hires (accounts + hardware)',
    status: 'in_progress',
    priority: 'normal',
    client_name: 'Northwind Trading',
    created_at: new Date(Date.now() - 26 * 3600_000).toISOString(),
    updated_at: new Date(Date.now() - 2 * 3600_000).toISOString(),
  },
];

async function shot(browser, viewport, theme, name, path, interactions, seedTours = true) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) =>
    console.error('PAGEERROR', name, String(e).slice(0, 300)),
  );
  const url = new URL(`${BASE}${path}`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');
  const themeToken = theme === 'dark' ? 'dark' : 'light';
  await page.addInitScript((opts) => {
    window.localStorage.setItem('meridian-theme', opts.theme);
    window.sessionStorage.clear();
    // Suppress auto tours on every shot except the dedicated overlay shot
    // (which clears this key right after) — the tour must not pollute the
    // closed/panel/search captures.
    if (opts.seedTours) {
      window.localStorage.setItem(
        'meridian-tours-completed',
        JSON.stringify(['workbar', 'inventory-workbench', 'help-entry']),
      );
    }
    const style = document.createElement('style');
    style.textContent = 'nextjs-portal{display:none!important}';
    document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
  }, { theme: themeToken, seedTours });

  await page.route('**/api/**', async (r) => {
    const url = new URL(r.request().url());
    const key = url.pathname;
    const method = r.request().method();
    // Auth bootstrap: one cookie-driven rotation returns the token pair.
    if (key === '/api/v1/auth/refresh') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(TOKENS) });
      return;
    }
    if (key === '/api/v1/me') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(ME) });
      return;
    }
    if (key === '/api/v1/settings/user' && method === 'GET') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
      return;
    }
    if (key === '/api/v1/settings/org' && method === 'GET') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
      return;
    }
    if (key === '/api/v1/settings/user' && method === 'PUT') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(r.request().postDataJSON()) });
      return;
    }
    if (key === '/api/v1/tickets' && method === 'GET') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(TICKETS) });
      return;
    }
    if (key === '/api/v1/tickets/categories' || key === '/api/v1/timers/active') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
      return;
    }
    if (key === '/api/v1/notifications' || key === '/api/v1/outbox') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ items: [] }) });
      return;
    }
    await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
  });

  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForSelector('main', { timeout: 30_000 });
  await page.waitForTimeout(1200);
  if (interactions) await interactions(page);
  await page.waitForTimeout(500);
  await page.screenshot({ path: resolve(OUT, name), fullPage: false });
  console.log('captured', name);
  await context.close();
}

const browser = await chromium.launch({
  executablePath: process.env.CHROME_PATH,
  args: ['--no-sandbox', '--disable-dev-shm-usage'],
});

const VP = { width: 1440, height: 900 };

for (const theme of ['light', 'dark']) {
  const t = `${theme}`;

  // 1. Surface with the ? affordance visible (closed state).
  await shot(browser, VP, theme, `help-closed-${t}.png`, '/tickets');

  // 2. Help panel open on /tickets — route-keyed entries.
  await shot(browser, VP, theme, `help-panel-tickets-${t}.png`, '/tickets', async (page) => {
    await page.getByTestId('help-button').click();
    await page.waitForSelector('[data-testid="help-doc-entry"]', { timeout: 15_000 });
  });

  // 3. Help panel on /settings/fx — different route key.
  await shot(browser, VP, theme, `help-panel-fx-${t}.png`, '/settings/fx', async (page) => {
    await page.getByTestId('help-button').click();
    await page.waitForTimeout(600);
  });

  // 4. Help panel search across all guides.
  await shot(browser, VP, theme, `help-search-${t}.png`, '/tickets', async (page) => {
    await page.getByTestId('help-button').click();
    await page.getByTestId('help-search').fill('invoice');
    await page.waitForTimeout(700);
  });

  // 5. Spotlight tour overlay on /boards (help-entry tour: fresh session,
  // targets the ? affordance). /today is avoided because the onboarding
  // resume banner (lane 1) crashes under minimal fixtures, not under the
  // real backend (e2e flow 13 covers the real-stack path).
  await shot(browser, VP, theme, `tour-overlay-${t}.png`, '/boards', async (page) => {
    try {
      await page.waitForSelector('[data-testid="tour-overlay"]', { state: 'attached', timeout: 25_000 });
      await page.waitForTimeout(400);
    } catch {
      console.error('tour did not auto-start on', theme);
    }
  }, false);

  // 6. Preferences with the tours toggle.
  await shot(browser, VP, theme, `preferences-tours-${t}.png`, '/settings/preferences', async (page) => {
    await page.waitForSelector('[data-testid="pref-tours-card"]', { timeout: 15_000 });
  });
}

await browser.close();
console.log('help capture complete →', OUT);
