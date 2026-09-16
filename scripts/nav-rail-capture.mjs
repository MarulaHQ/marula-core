#!/usr/bin/env node
/**
 * Nav-rail capture — Wave 15 lane 6 (p15-nav-rail).
 *
 * Screenshots the app shell (dark left icon rail + slim top bar + dark
 * bottom work bar) on the /tickets standalone list at 1440/1024/768/480
 * in light AND dark, into design/qa-nav-rail/. API responses are
 * fixtures served by Playwright routing — no backend needed.
 *
 * Usage: start `pnpm --filter web dev` on :3000, then
 *        node scripts/nav-rail-capture.mjs
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const CHROME = '/Users/iordn/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const OUT = resolve('design/qa-nav-rail');

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: '00000000-0000-4000-8000-000000000002',
  email: 'owner@meridian.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_boards', 'tickets:edit', 'tickets:read', 'financial:read', 'admin:org'],
};

const TICKET = (n, title, priority, category, minutesAgo) => ({
  id: `t-${n}`,
  number: `MER-${n}`,
  title,
  priority,
  category,
  status_id: 'open',
  board_id: 'board-1',
  client_id: 'client-1',
  contact_id: null,
  assignee_id: ME.user_id,
  requester_id: null,
  site_id: null,
  body: null,
  due_at: null,
  resolved_at: null,
  created_at: new Date(Date.now() - minutesAgo * 60000).toISOString(),
  updated_at: new Date(Date.now() - minutesAgo * 60000).toISOString(),
  comments: [],
  custom_fields: [],
});

const TICKETS = [
  TICKET(18432, 'VPN disconnects every 20–30 minutes', 'p1', 'in_progress', 4),
  TICKET(18410, 'MFA prompt loops on new laptop', 'p2', 'open', 38),
  TICKET(18395, 'Quarterly patch cycle — server fleet', 'p3', 'waiting', 240),
  TICKET(18380, 'Onboarding: mail routing cutover', 'p3', 'open', 600),
  TICKET(18312, 'Printer driver crashes on Windows 11 24H2', 'p4', 'open', 1500),
];

const ASSETS = [
  { id: 'a-1', tag: 'LAP-0142', name: 'JOHN-LATITUDE-7440', kind: 'workstation', status: 'deployed', client_id: 'client-1', org_id: 'o', created_at: '2026-01-10T00:00:00Z', updated_at: '2026-09-01T00:00:00Z', identity_keys: {}, fields: {}, curated_fields: {}, observed_fields: {} },
  { id: 'a-2', tag: 'FW-0007', name: 'ACME-EDGE-FIREWALL', kind: 'network', status: 'deployed', client_id: 'client-1', org_id: 'o', created_at: '2026-01-12T00:00:00Z', updated_at: '2026-08-20T00:00:00Z', identity_keys: {}, fields: {}, curated_fields: {}, observed_fields: {} },
];

const WORKING_SET = [
  {
    id: 'sess-1',
    org_id: 'o',
    user_id: ME.user_id,
    ticket_id: 't-18432',
    started_at: new Date(Date.now() - 25 * 60000).toISOString(),
    stopped_at: null,
    segments: [{ started_at: new Date(Date.now() - 25 * 60000).toISOString(), ended_at: null }],
  },
];

const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/tickets': TICKETS,
  '/api/v1/boards': [],
  '/api/v1/assets': ASSETS,
  '/api/v1/timers/working-set': WORKING_SET,
  '/api/v1/tickets/t-18432': TICKETS[0],
  '/api/v1/notifications/unread-count': { count: 3 },
  '/api/v1/notifications': {
    items: [
      { id: 'n1', kind: 'assignment', title: 'MER-18432 assigned to you', body: 'P1 · Acme Corporation', created_at: new Date(Date.now() - 5 * 60000).toISOString(), read_at: null, ticket_id: 't-18432' },
      { id: 'n2', kind: 'sla_risk', title: 'SLA risk on MER-18410', body: 'First response due in 22m', created_at: new Date(Date.now() - 30 * 60000).toISOString(), read_at: null, ticket_id: 't-18410' },
      { id: 'n3', kind: 'missing_time', title: 'Missing time yesterday', body: 'Log 1h to keep the week clean', created_at: new Date(Date.now() - 26 * 3600000).toISOString(), read_at: null, ticket_id: null },
    ],
    next_cursor: null,
  },
  '/api/dev/status': { enabled: false },
};

async function capture(viewport, theme, route) {
  const browser = await chromium.launch({ executablePath: CHROME, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();

  const url = new URL(`http://localhost:3100${route}`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');

  await page.addInitScript(() => {
    window.localStorage.setItem('meridian-theme', window.location.search.includes('theme=dark') ? 'dark' : 'light');
    window.localStorage.setItem('meridian-tokens', JSON.stringify({
      access_token: 'test-access',
      refresh_token: 'test-refresh',
      session_id: 'sess',
      expires_in: 900,
    }));
    window.localStorage.setItem('meridian-tokens-exp', String(Date.now() + 60_000));
    // Hide the Next.js dev-tools indicator so it never occludes the work bar.
    const style = document.createElement('style');
    style.textContent = 'nextjs-portal{display:none!important}';
    document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
  });

  await page.route('**/api/**', async (r) => {
    const key = new URL(r.request().url()).pathname;
    if (FIXTURES[key]) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES[key]) });
    } else {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ data: [] }) });
    }
  });

  await page.goto(url.toString(), { waitUntil: 'networkidle' });
  await page.waitForTimeout(2500);

  const name = `shell-${route.replace(/\//g, '') || 'home'}-${viewport.width}-${theme}.png`;
  await page.screenshot({ path: resolve(OUT, name), fullPage: false });
  console.log('captured', name);
  await browser.close();
}

const viewports = [
  { width: 1440, height: 1024 },
  { width: 1024, height: 768 },
  { width: 768, height: 1024 },
  { width: 480, height: 900 },
];

for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    try {
      await capture(vp, theme, '/tickets');
    } catch (e) {
      console.error('FAIL', vp.width, theme, e.message);
    }
  }
}
console.log('done');
