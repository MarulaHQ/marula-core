import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const CHROME = '/Users/iordn/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const OUT = resolve('/Users/iordn/Documents/GitHub/marula-open-psa-core/.kilo/worktrees/p15-theme-inkling/design/qa-tangerine');

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const FIXTURES = {
  '/api/v1/me': {
    user_id: '00000000-0000-4000-8000-000000000001',
    org_id: '00000000-0000-4000-8000-000000000002',
    email: 'owner@marula.test',
    display_name: 'Owner',
    timezone: 'UTC',
    permissions: ['special:manage_boards', 'tickets:edit', 'tickets:read', 'financial:read', 'admin:org'],
  },
  '/api/v1/tickets/18432': {
    id: '18432',
    number: 'T-18432',
    title: 'Review Tangerine theme on ticket workbench',
    status_id: 'open',
    category: 'feature',
    priority: 'high',
    board_id: 'board-1',
    client_id: 'client-1',
    contact_id: 'contact-1',
    assignee_id: '00000000-0000-4000-8000-000000000001',
    created_at: '2026-09-15T10:00:00Z',
    resolved_at: null,
    custom_fields: [],
    merged_into_ticket_id: null,
  },
  '/api/v1/tickets/18432/sla': { sla: null },
  '/api/v1/tickets/18432/comments': { data: [] },
  '/api/v1/tickets/18432/checklist': { data: null },
  '/api/v1/tickets/18432/attachments': { data: [] },
  '/api/v1/boards/board-1/statuses': { data: [{ id: 'open', name: 'Open' }, { id: 'in-progress', name: 'In Progress' }] },
  '/api/v1/boards/board-1/transitions': { mode: 'floor', edges: [] },
  '/api/v1/org/users': { data: { users: [{ id: '00000000-0000-4000-8000-000000000001', display_name: 'Owner' }] } },
  '/api/v1/org/pod-visibility': { data: { config: {} } },
  '/api/v1/tags': { data: [] },
  '/api/dev/status': { enabled: false },
};

async function capture(viewport, theme) {
  const browser = await chromium.launch({ executablePath: CHROME, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();

  const url = new URL('http://localhost:3000/tickets/18432');
  if (theme === 'dark') {
    url.searchParams.set('theme', 'dark');
  }

  await page.addInitScript(() => {
    window.localStorage.setItem('marula-theme', window.location.search.includes('theme=dark') ? 'dark' : 'light');
    window.localStorage.setItem('marula-tokens', JSON.stringify({
      access_token: 'test-access',
      refresh_token: 'test-refresh',
      session_id: 'sess',
      expires_in: 900,
    }));
    window.localStorage.setItem('marula-tokens-exp', String(Date.now() + 60_000));
  });

  await page.route('**/api/**', async (route) => {
    const url = new URL(route.request().url());
    const key = url.pathname;
    if (FIXTURES[key]) {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES[key]) });
    } else {
      await route.fulfill({ status: 404, contentType: 'application/json', body: JSON.stringify({ error: { message: 'not found' } }) });
    }
  });

  await page.goto(url.toString(), { waitUntil: 'networkidle' });
  await page.waitForTimeout(3000);

  // Hide the Next.js dev tools overlay so it doesn't block screenshots
  await page.evaluate(() => {
    const dialog = document.querySelector('[data-nextjs-dev-error-dialog]') ||
      document.querySelector('.nextjs-error-overlay') ||
      document.querySelector('nextjs-error-overlay') ||
      [...document.querySelectorAll('*')].find(el => el.tagName === 'NEXTJS-ERROR-OVERLAY');
    if (dialog) dialog.style.display = 'none';
    // Also hide any portal overlay
    document.querySelectorAll('div').forEach(el => {
      if (el.textContent?.includes('Hydration failed')) el.style.display = 'none';
    });
  });
  await page.waitForTimeout(500);

  const name = `ticket-workbench-${viewport.width}-${theme}.png`;
  const path = resolve(OUT, name);
  await page.screenshot({ path, fullPage: true });
  console.log('captured', name);

  // Check for runtime errors
  const logs = [];
  page.on('console', msg => { if (msg.type() === 'error') logs.push(msg.text()); });
  page.on('pageerror', err => logs.push(err.message));

  await browser.close();
  return logs;
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
      await capture(vp, theme);
    } catch (e) {
      console.error('FAIL', vp.width, theme, e.message);
    }
  }
}
console.log('done');