/**
 * Wave-gaps capture — Wave 23 lane 1 (p23-gaps).
 *
 * The touched surfaces, fixture-routed (no backend): BoardNav mobile drawer
 * (768/480, open drawer), kanban human category tags, dashboard widget
 * drill-down deep links, /reports?spec= seeded run, ticket workspace stacked
 * at 768 + app-header at 480. Pattern: scripts/backend-gaps-capture.mjs.
 *
 * Usage: start the web app (dev on :3000), then node scripts/wave-gaps-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-wave-gaps');
const BASE = process.argv[2] ?? 'http://localhost:3000';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: '00000000-0000-4000-8000-000000000002',
  email: 'owner@meridian.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_users', 'special:manage_boards', 'admin:org'],
};

const BOARD_ID = '00000000-0000-4000-8000-0000000000b1';
const BOARDS_NAV = [
  { id: '00000000-0000-4000-8000-0000000000b1', name: 'Client requests', description: null, group_id: null, is_default: true, favorite: true, my_role: 'admin', group_name: null },
  { id: '00000000-0000-4000-8000-0000000000b2', name: 'Preventive maintenance', description: null, group_id: null, is_default: false, favorite: false, my_role: 'member', group_name: null },
];
const STATUSES = [
  { id: '00000000-0000-4000-8000-0000000000s1', board_id: BOARD_ID, name: 'New', category: 'open', sort_order: 0, is_entry: true, wip_limit: null },
  { id: '00000000-0000-4000-8000-0000000000s2', board_id: BOARD_ID, name: 'In progress', category: 'in_progress', sort_order: 1, is_entry: false, wip_limit: 5 },
];
const WORKFLOW = { mode: 'floor', edges: [] };
const TICKETS = [
  { id: '00000000-0000-4000-8000-0000000000t1', board_id: BOARD_ID, number: 'MER-101', title: 'Fix the mail relay', status_id: '00000000-0000-4000-8000-0000000000s1', category: 'open', priority: 'P1', created_at: new Date().toISOString(), updated_at: new Date().toISOString(), assignee_id: '00000000-0000-4000-8000-000000000001', client_id: null },
  { id: '00000000-0000-4000-8000-0000000000t2', board_id: BOARD_ID, number: 'MER-102', title: 'Renew TLS cert on HQ gateway', status_id: '00000000-0000-4000-8000-0000000000s1', category: 'open', priority: 'P3', created_at: new Date().toISOString(), updated_at: new Date().toISOString(), assignee_id: null, client_id: null },
];

const REPORT_TIME_PAGE = {
  rows: [
    { group_id: '00000000-0000-4000-8000-000000000001', label: 'Taylor Reed', metric: 210, billable_minutes: 150, entry_count: 9 },
    { group_id: '00000000-0000-4000-8000-0000000000u2', label: 'Ada Ortiz', metric: 96, billable_minutes: 96, entry_count: 4 },
  ],
  summary: { entries_count: 13, total_minutes: 306, billable_minutes: 150 },
  next_cursor: null,
};

const SPEC_RUN_PAGE = {
  kind: 'tickets',
  view: 'ops.v1_tickets',
  columns: ['priority', 'count'],
  rows: [
    { priority: 'P1', count: 12 },
    { priority: 'P2', count: 30 },
  ],
  summary: { count: 42 },
  next_cursor: null,
};

const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/boards/nav': BOARDS_NAV,
  '/api/v1/boards/{id}/statuses': STATUSES,
  '/api/v1/tickets': TICKETS,
  '/api/v1/board-groups': [],
  '/api/v1/notifications/unread-count': { count: 2 },
  '/api/v1/notifications': { items: [], next_cursor: null },
  '/api/v1/timers/working-set': [],
  '/api/v1/reports/time_by_user': REPORT_TIME_PAGE,
  '/api/v1/reports/time_by_client': REPORT_TIME_PAGE,
  '/api/v1/reports/ticket_throughput': {
    rows: Array.from({ length: 10 }, (_, i) => ({
      day: new Date(Date.now() - (9 - i) * 864e5).toISOString().slice(0, 19) + 'Z',
      created_count: 2 + (i % 3),
      resolved_count: 1 + (i % 4),
    })),
    summary: { created_total: 24, resolved_total: 21 },
    next_cursor: null,
  },
  '/api/v1/reports/ticket_aging': {
    rows: [
      { bucket: 'under_1d', bucket_ordinal: 0, open_count: 6, oldest_age_days: 0.6, avg_age_days: 0.3 },
      { bucket: 'd1_3', bucket_ordinal: 1, open_count: 4, oldest_age_days: 2.9, avg_age_days: 1.8 },
      { bucket: 'd7_30', bucket_ordinal: 3, open_count: 2, oldest_age_days: 22, avg_age_days: 14 },
    ],
    summary: { open_total: 14, oldest_age_days: 22, breached_open: 2 },
    next_cursor: null,
  },
  '/api/v1/reports/sla_attainment': {
    rows: [{ group_id: '00000000-0000-4000-8000-0000000000p1', label: 'P1 critical', metric: 92, resolutions_met: 18, attainment_pct: 92 }],
    summary: { open_total: 0, resolutions_total: 20, attainment_pct: 92 },
    next_cursor: null,
  },
  '/api/v1/reports/first_response': {
    rows: [{ ticket_id: '00000000-0000-4000-8000-0000000000t1', ticket_number: 'MER-101', client_id: null, assignee_id: '00000000-0000-4000-8000-000000000001', created_at: new Date().toISOString(), first_staff_comment_at: new Date().toISOString(), first_response_secs: 620, first_response_due: new Date().toISOString() }],
    summary: { tickets_total: 1, answered_count: 1, unanswered_count: 0, response_p50_secs: 180, response_p90_secs: 420 },
    next_cursor: null,
  },
  '/api/v1/report-catalog': {
    views: [
      {
        view: 'tickets',
        columns: [
          { name: 'org_id', value_type: 'uuid' },
          { name: 'number', value_type: 'text' },
          { name: 'priority', value_type: 'text' },
          { name: 'created_at', value_type: 'timestamp' },
        ],
        filterable: [{ name: 'priority', value_type: 'text' }],
      },
    ],
  },
  '/api/v1/report-definitions': { definitions: [], next_cursor: null },
  '/api/v1/report-schedules': { schedules: [], next_cursor: null },
  '/api/v1/settings/user': [],
  '/api/v1/org/users': { users: [] },
  '/api/v1/tickets/my-work': [],
};

function mockApi(route, request) {
  const url = new URL(request.url());
  if (!url.pathname.startsWith('/api/')) return route.abort();
  const key = url.pathname;
  const body =
    key === '/api/v1/tickets' ? TICKETS
    : key === `/api/v1/tickets/${TICKETS[0].id}` ? TICKETS[0]
    : key.startsWith('/api/v1/boards/') && key.endsWith('/statuses') ? STATUSES
    : key.startsWith('/api/v1/boards/') && key.endsWith('/transitions') ? WORKFLOW
    : key.endsWith('/comments') ? []
    : key.endsWith('/checklist') ? null
    : key.endsWith('/sla') ? null
    : key.endsWith('/attachments') ? []
    : key.endsWith('/time-entries') ? []
    : key.endsWith('/emails') ? []
    : key.endsWith('/fields') ? []
    : key.endsWith('/tags') ? []
    : key.endsWith('/scheduling-links') ? []
    : key === '/api/v1/meeting-types' ? []
    : key.endsWith('/contacts') ? []
    : key === '/api/v1/tags' ? []
    : key === '/api/v1/boards' ? []
    : key === '/api/v1/clients' ? []
    : key === '/api/v1/audit-events' ? { items: [] }
    : key.endsWith('/custom-fields') ? []
    : key === '/api/v1/tickets/{id}' ? {}
    : key === '/api/v1/reports/run-spec' ? SPEC_RUN_PAGE
    : key === '/api/v1/report-definitions' ? { definitions: [], next_cursor: null }
    : key === '/api/v1/report-schedules' ? { schedules: [], next_cursor: null }
    : FIXTURES[key];
  if (body === undefined) {
    // Quiet empty-list defaults for the chrome hits we don't fixture.
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ items: [], next_cursor: null, definitions: [], schedules: [], settings: [] }) });
  }
  return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
}

async function capture(browser, { viewport, theme, name, path, before }) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', name, String(e).slice(0, 200), '\nSTACK', (e.stack ?? '').split('\n').slice(1, 5).join(' | ')));
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
  }, theme === 'dark' ? 'dark' : 'light');
  await page.route('**/api/**', (route, request) => mockApi(route, request));
  await page.goto(`${BASE}${path}`, { waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(1800);
  if (before !== undefined) await before(page);
  await page.waitForTimeout(250);
  await page.screenshot({ path: resolve(OUT, `${name}.png`), fullPage: false });
  await context.close();
  console.log('captured', name);
}

const VIEWPORTS = {
  1440: { width: 1440, height: 900 },
  1024: { width: 1024, height: 800 },
  768: { width: 768, height: 900 },
  480: { width: 480, height: 900 },
};

const SPEC_QUERY = Buffer.from(JSON.stringify({
  v: 1,
  spec: { view: 'tickets', filters: [{ column: 'priority', op: 'eq', value: { text: 'p1' } }], group_by: ['priority'], measures: [{ agg: 'count' }], viz: 'bar' },
  from: '',
  to: '',
})).toString('base64').replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');

const browser = await chromium.launch();
const jobs = [
  // Item 1: BoardNav drawer, open drawer state below the 768 breakpoint.
  { viewport: { width: 640, height: 900 }, theme: 'light', name: 'boardnav-drawer-640-light', path: `/boards/${BOARD_ID}`, before: async (p) => { await p.click('[data-testid="board-nav-toggle"]'); await p.waitForTimeout(200); } },
  { viewport: { width: 600, height: 900 }, theme: 'dark', name: 'boardnav-drawer-600-dark', path: `/boards/${BOARD_ID}`, before: async (p) => { await p.click('[data-testid="board-nav-toggle"]'); await p.waitForTimeout(200); } },
  { viewport: VIEWPORTS[480], theme: 'light', name: 'boardnav-drawer-480-light', path: `/boards/${BOARD_ID}`, before: async (p) => { await p.click('[data-testid="board-nav-toggle"]'); await p.waitForTimeout(200); } },
  { viewport: VIEWPORTS[480], theme: 'dark', name: 'boardnav-drawer-480-dark', path: `/boards/${BOARD_ID}`, before: async (p) => { await p.click('[data-testid="board-nav-toggle"]'); await p.waitForTimeout(200); } },
  // Rail regression 1440 light (drawer chip must be gone at desktop width).
  { viewport: VIEWPORTS[1440], theme: 'light', name: 'boardnav-rail-1440-light', path: `/boards/${BOARD_ID}` },
  { viewport: VIEWPORTS[1024], theme: 'dark', name: 'boardnav-rail-1024-dark', path: `/boards/${BOARD_ID}` },
  // Items 2+3: widget Builder links + table row deep links at default widths.
  { viewport: VIEWPORTS[1440], theme: 'light', name: 'dashboard-widgets-1440-light', path: '/dashboards' },
  { viewport: VIEWPORTS[1440], theme: 'dark', name: 'dashboard-widgets-1440-dark', path: '/dashboards' },
  // /reports?spec= seeded, auto-run preview.
  { viewport: VIEWPORTS[1440], theme: 'light', name: 'reports-speclink-1440-light', path: `/reports?spec=${SPEC_QUERY}` },
  { viewport: VIEWPORTS[1440], theme: 'dark', name: 'reports-speclink-1440-dark', path: `/reports?spec=${SPEC_QUERY}` },
  { viewport: VIEWPORTS[1024], theme: 'light', name: 'reports-speclink-1024-light', path: `/reports?spec=${SPEC_QUERY}` },
  { viewport: VIEWPORTS[768], theme: 'dark', name: 'reports-speclink-768-dark', path: `/reports?spec=${SPEC_QUERY}` },
  // Item 11: workspace stacked at 768 (light+dark) + header at 480.
  { viewport: VIEWPORTS[768], theme: 'light', name: 'workspace-768-light', path: `/tickets/00000000-0000-4000-8000-0000000000t1` },
  { viewport: VIEWPORTS[768], theme: 'dark', name: 'workspace-768-dark', path: `/tickets/00000000-0000-4000-8000-0000000000t1` },
  { viewport: VIEWPORTS[480], theme: 'light', name: 'workspace-header-480-light', path: `/tickets/00000000-0000-4000-8000-0000000000t1` },
  { viewport: VIEWPORTS[480], theme: 'dark', name: 'workspace-480-dark', path: `/tickets/00000000-0000-4000-8000-0000000000t1` },
  { viewport: VIEWPORTS[1440], theme: 'light', name: 'workspace-1440-light', path: `/tickets/00000000-0000-4000-8000-0000000000t1` },
];

for (const job of jobs) {
  await capture(browser, job);
}

await browser.close();
console.log('done');
