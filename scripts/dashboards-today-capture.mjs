#!/usr/bin/env node
/**
 * Dashboards/Today/Welcome capture — Wave 17 lane 1 (p17-dashboards-today).
 *
 * Screenshots the three daily-landing surfaces: /dashboards (widget grid,
 * plus one picker-open state at 1440), /today (grouped work queue with
 * attention strip + focus timer), /welcome (three-tier onboarding) — at
 * 1440/1024/768/480 in light AND dark, into design/qa-dashboards-today/.
 * API responses are fixtures served by Playwright routing — no backend
 * needed. Pattern: scripts/plugin-mgmt-capture.mjs.
 *
 * Usage: start the web app (dev on :3000), then node scripts/dashboards-today-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-dashboards-today');
const BASE = process.argv[2] ?? 'http://localhost:3000';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ORG = '00000000-0000-4000-8000-000000000002';
const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: ORG,
  email: 'owner@meridian.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_users', 'special:approve_time', 'special:manage_plugins', 'admin:org'],
};
const now = Date.now();
const iso = (offsetMs) => new Date(now + offsetMs).toISOString();
const yesterday = new Date(now - 86_400_000).toISOString().slice(0, 10);

const MY_WORK = [
  { id: '90000000-0000-4000-8000-0000000000a1', board_id: 'b1', board_name: 'Service Desk', number: 'MER-104', title: 'Mail server rejecting inbound', priority: 'p1', category: 'in_progress', status_name: 'In progress', client_name: 'Acme Corp', due_at: null, sla_resolution_due: iso(-2 * 3600_000), updated_at: iso(-1800_000) },
  { id: '90000000-0000-4000-8000-0000000000a2', board_id: 'b1', board_name: 'Service Desk', number: 'MER-107', title: 'VPN drops every hour', priority: 'p1', category: 'open', status_name: 'New', client_name: 'Northwind', due_at: iso(-40 * 60_000), sla_resolution_due: null, updated_at: iso(-7200_000) },
  { id: '90000000-0000-4000-8000-0000000000a3', board_id: 'b2', board_name: 'Projects', number: 'MER-109', title: 'Laptop imaging round 2', priority: 'p2', category: 'open', status_name: 'New', client_name: null, due_at: iso(2 * 3600_000), sla_resolution_due: null, updated_at: iso(-3600_000) },
  { id: '90000000-0000-4000-8000-0000000000a4', board_id: 'b2', board_name: 'Projects', number: 'MER-112', title: 'Quarterly patch cycle', priority: 'p3', category: 'open', status_name: 'New', client_name: null, due_at: iso(3 * 86_400_000), sla_resolution_due: null, updated_at: iso(-3600_000) },
  { id: '90000000-0000-4000-8000-0000000000a5', board_id: 'b3', board_name: 'Internal', number: 'MER-115', title: 'Docs cleanup sprint', priority: 'p3', category: 'open', status_name: 'New', client_name: null, due_at: null, sla_resolution_due: null, updated_at: iso(-86_400_000) },
];

const GAP = {
  user_id: ME.user_id,
  timezone: 'UTC',
  expected_minutes: 480,
  days: [{ day: yesterday, activity: true, expected_minutes: 480, logged_minutes: 253, has_gap: true, unaccounted_minutes: 227, nudge_sent_at: iso(-7200_000) }],
};

const APPROVALS = [0, 1].map((i) => ({
  decided_at: null,
  decided_by: null,
  entry: { id: `f0000000-0000-4000-8000-00000000000${i + 1}`, org_id: ORG, ticket_id: '90000000-0000-4000-8000-0000000000a1', user_id: ME.user_id, started_at: iso(-5 * 3600_000), ended_at: iso(-4 * 3600_000), minutes: 60, billable: true, notes: null, submitted_at: iso(-3 * 3600_000), submitted_by: ME.user_id },
  locked: false,
  submitted_at: iso(-3 * 3600_000),
  submitted_by: ME.user_id,
}));

const TIMER = {
  id: 's0000000-0000-4000-8000-000000000001',
  org_id: ORG,
  user_id: ME.user_id,
  ticket_id: '90000000-0000-4000-8000-0000000000a1',
  started_at: iso(-37 * 60_000),
  stopped_at: null,
  segments: [{ started_at: iso(-37 * 60_000), ended_at: null }],
};

const FOCUS_TICKET = {
  id: '90000000-0000-4000-8000-0000000000a1',
  number: 'MER-104',
  title: 'Mail server rejecting inbound',
};

const BOARDS_NAV = [
  { id: 'b1', name: 'Service Desk', description: null, group_id: 'g1', group_name: 'Managed', favorite: true, is_default: true, my_role: 'admin' },
  { id: 'b2', name: 'Projects', description: null, group_id: null, group_name: null, favorite: false, is_default: false, my_role: 'member' },
];

const REPORT = (kind, rows, summary) => ({ columns: [], kind, view: 'ops.v1_test', rows, summary, next_cursor: null });

const DAY = 86_400_000;
const THROUGHPUT_DAYS = Array.from({ length: 14 }, (_, i) => ({
  day: new Date(now - (13 - i) * DAY).toISOString().slice(0, 10),
  created_count: 2 + ((i * 7) % 5),
  resolved_count: 1 + ((i * 11) % 5),
}));

const REPORTS = {
  time_by_user: REPORT('time_by_user', [
    { group_id: ME.user_id, label: 'Taylor Reed', metric: 391, billable_minutes: 300, entry_count: 9 },
    { group_id: '00000000-0000-4000-8000-00000000s0b1', label: 'Ana Sousa', metric: 347, billable_minutes: 280, entry_count: 8 },
    { group_id: '00000000-0000-4000-8000-00000000s0b2', label: 'Marco Diaz', metric: 268, billable_minutes: 190, entry_count: 7 },
    { group_id: '00000000-0000-4000-8000-00000000s0b3', label: 'Priya Nair', metric: 205, billable_minutes: 160, entry_count: 6 },
  ], { entries_count: 30, total_minutes: 1211, billable_minutes: 930 }),
  time_by_client: REPORT('time_by_client', [
    { group_id: '00000000-0000-4000-8000-00000000c001', label: 'Acme Corp', metric: 480, billable_minutes: 480, entry_count: 11 },
    { group_id: '00000000-0000-4000-8000-00000000c002', label: 'Northwind', metric: 312, billable_minutes: 260, entry_count: 8 },
    { group_id: '00000000-0000-4000-8000-00000000c003', label: 'Initech', metric: 190, billable_minutes: 150, entry_count: 6 },
  ], { entries_count: 25, total_minutes: 982, billable_minutes: 890 }),
  ticket_throughput: REPORT('ticket_throughput', THROUGHPUT_DAYS, { created_total: 44, resolved_total: 39 }),
  ticket_aging: REPORT('ticket_aging', [
    { bucket: 'under_1d', bucket_ordinal: 0, open_count: 12, oldest_age_days: 0, avg_age_days: 0.4 },
    { bucket: 'd1_3', bucket_ordinal: 1, open_count: 8, oldest_age_days: 3, avg_age_days: 1.9 },
    { bucket: 'd3_7', bucket_ordinal: 2, open_count: 5, oldest_age_days: 6, avg_age_days: 4.2 },
    { bucket: 'd7_30', bucket_ordinal: 3, open_count: 5, oldest_age_days: 28, avg_age_days: 14 },
    { bucket: 'over_30d', bucket_ordinal: 4, open_count: 2, oldest_age_days: 61, avg_age_days: 47 },
  ], { open_total: 32, oldest_age_days: 61, breached_open: 3 }),
  first_response: REPORT('first_response', [
    { ticket_id: '90000000-0000-4000-8000-0000000000aa', ticket_number: 'MER-99' },
  ], { tickets_total: 22, answered_count: 19, unanswered_count: 3, response_p50_secs: 260, response_p90_secs: 900 }),
  sla_attainment: REPORT('sla_attainment', [
    { group_id: '00000000-0000-4000-8000-00000000p001', label: 'Gold SLA', metric: 10, resolutions_met: 9, attainment_pct: 90 },
    { group_id: '00000000-0000-4000-8000-00000000p002', label: 'Silver SLA', metric: 14, resolutions_met: 11, attainment_pct: 79 },
    { group_id: '00000000-0000-4000-8000-00000000p003', label: 'Bronze SLA', metric: 6, resolutions_met: 4, attainment_pct: 67 },
  ], { resolutions_total: 30, resolutions_met: 24, attainment_pct: 80 }),
  sla_breaches: REPORT('sla_breaches', [
    { ticket_id: '90000000-0000-4000-8000-0000000000b1', ticket_number: 'MER-42', priority: 'P1', policy_name: 'Gold SLA', resolution_due: '2026-09-15T17:00:00Z', resolution_met: false },
    { ticket_id: '90000000-0000-4000-8000-0000000000b2', ticket_number: 'MER-77', priority: 'P2', policy_name: 'Silver SLA', resolution_due: '2026-09-16T12:00:00Z', resolution_met: false },
  ], { breaches_total: 2, paused_business_secs_total: 3600 }),
};

const ORG_FIXTURE = {
  id: ORG,
  name: 'Meridian Demo',
  ticket_prefix: 'MER',
};

const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/org': ORG_FIXTURE,
  '/api/v1/settings/user': [],
  '/api/v1/settings/org': [],
  '/api/v1/tickets/my-work': MY_WORK,
  '/api/v1/time-gaps/me': GAP,
  '/api/v1/time-entries/approvals/queue': APPROVALS,
  '/api/v1/tickets/90000000-0000-4000-8000-0000000000a1': FOCUS_TICKET,
  '/api/v1/boards/nav': BOARDS_NAV,
  '/api/v1/board-groups': [{ id: 'g1', name: 'Managed', position: 0 }],
  '/api/v1/boards': [],
  '/api/v1/timers/working-set': [TIMER],
  '/api/v1/timers/active': TIMER,
  '/api/v1/notifications/unread-count': { count: 0 },
  '/api/v1/notifications': { items: [], unread_count: 0, total: 0, next_cursor: null },
  '/api/v1/tickets': [],
  '/api/v1/clients': [],
  '/api/v1/user': {},
  // Reports (also catches the /api/v1/reports/{kind} shape).
  ...Object.fromEntries(Object.entries(REPORTS).map(([k, v]) => [`/api/v1/reports/${k}`, v])),
};

async function shot(browser, viewport, theme, name, path, interactions) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', name, String(e).slice(0, 300)));
  const url = new URL(`${BASE}${path}`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');
  const themeToken = theme === 'dark' ? 'dark' : 'light';
  await page.addInitScript((t) => {
    window.localStorage.setItem('meridian-theme', t);
    window.localStorage.setItem('meridian-tokens', JSON.stringify({
      access_token: 'test-access',
      refresh_token: 'test-refresh',
      session_id: 'sess',
      expires_in: 900,
    }));
    window.localStorage.setItem('meridian-tokens-exp', String(Date.now() + 60_000));
    const style = document.createElement('style');
    style.textContent = 'nextjs-portal{display:none!important}';
    document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
  }, themeToken);

  await page.route('**/api/**', async (r) => {
    const key = new URL(r.request().url()).pathname;
    if (FIXTURES[key]) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES[key]) });
    } else if (key.startsWith('/api/v1/reports/')) {
      const kind = key.replace('/api/v1/reports/', '');
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(REPORTS[kind] ?? REPORT(kind, [], {})) });
    } else {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
    }
  });

  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForSelector('main', { timeout: 20_000 });
  await page.waitForTimeout(900);
  if (interactions) await interactions(page);
  await page.waitForTimeout(400);
  await page.screenshot({ path: resolve(OUT, name), fullPage: false });
  console.log('captured', name);
  await context.close();
}

const viewports = [
  { width: 1440, height: 1024 },
  { width: 1024, height: 768 },
  { width: 768, height: 1024 },
  { width: 480, height: 900 },
];

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, args: ['--no-sandbox', '--disable-dev-shm-usage'] });

for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    try {
      await shot(browser, vp, theme, `dashboards-${vp.width}-${theme}.png`, '/dashboards');
    } catch (e) {
      console.error('FAIL', vp.width, theme, e.message);
    }
  }
}

for (const theme of ['light', 'dark']) {
  try {
    await shot(browser, { width: 1440, height: 1024 }, theme, `dashboards-picker-1440-${theme}.png`, '/dashboards', async (page) => {
      await page.getByRole('button', { name: 'Add widget' }).click();
      await page.waitForTimeout(500);
    });
  } catch (e) {
    console.error('FAIL picker', theme, e.message);
  }
}

for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    try {
      await shot(browser, vp, theme, `today-${vp.width}-${theme}.png`, '/today');
    } catch (e) {
      console.error('FAIL today', vp.width, theme, e.message);
    }
  }
}

for (const vp of [1440, 480]) {
  for (const theme of ['light', 'dark']) {
    try {
      await shot(browser, { width: vp, height: vp === 480 ? 900 : 1024 }, theme, `welcome-${vp}-${theme}.png`, '/welcome');
    } catch (e) {
      console.error('FAIL welcome', vp, theme, e.message);
    }
  }
}

for (const theme of ['light', 'dark']) {
  try {
    await shot(browser, { width: 1440, height: 1024 }, theme, `welcome-step2-1440-${theme}.png`, '/welcome', async (page) => {
      await page.getByRole('button', { name: 'Save and continue' }).click();
      await page.getByText('Activate modules').waitFor({ timeout: 10_000 });
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `welcome-step3-1440-${theme}.png`, '/welcome', async (page) => {
      await page.getByRole('button', { name: 'Save and continue' }).click();
      await page.getByRole('button', { name: 'Continue' }).click();
      await page.getByText('Playbook runtime').waitFor({ timeout: 10_000 });
    });
  } catch (e) {
    console.error('FAIL welcome steps', theme, e.message);
  }
}

await browser.close();
console.log('done');
