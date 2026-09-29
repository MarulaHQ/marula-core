#!/usr/bin/env node
/**
 * Stragglers capture — Wave 17 lane 12 (p17-stragglers).
 *
 * Screenshot evidence for the stragglers fixes into design/qa-stragglers/:
 * - /dashboards widget grid + chart↔data-table toggle + unanswered drilldown
 *   (lanes 1 charts + First response deep-link)
 * - /boards/b1 kanban with the per-assignee directory picker + active filter
 *   state (lane 3)
 * - workbench-v2 /tickets/{id} Details tab with the mounted scheduling pod,
 *   and the classic workspace (lane 3)
 * - /calendar month view (lane 4; keyboard month cell navigation captured
 *   as a states via arrow-key interaction)
 * - /mfa, /sso, /accept-invite (lane 9 de-serif titles) and
 *   /settings/security TOTP setup with the copy-once affordance
 * Light AND dark × 1440/1024/768/480 for the base surfaces; interaction
 * states at 1440 both themes. API responses are Playwright fixtures —
 * no backend needed. Pattern: scripts/settings-capture.mjs.
 *
 * Usage: node scripts/stragglers-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-stragglers');
const BASE = process.argv[2] ?? 'http://localhost:3001';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ORG = '00000000-0000-4000-8000-000000000002';
const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: ORG,
  email: 'owner@marula.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_users', 'special:approve_time', 'special:manage_boards', 'tickets:create', 'admin:org'],
};
const now = Date.now();
const iso = (offsetMs) => new Date(now + offsetMs).toISOString();

const BOARDS_NAV = [
  { id: 'b1', name: 'Service Desk', description: null, group_id: 'g1', group_name: 'Managed', favorite: true, is_default: true, my_role: 'admin' },
  { id: 'b2', name: 'Projects', description: null, group_id: null, group_name: null, favorite: false, is_default: false, my_role: 'member' },
];

const USERS = {
  users: [
    { id: 'u1', org_id: ORG, email: 'taylor@marula.test', display_name: 'Taylor Reed', timezone: 'UTC', is_active: true, created_at: iso(-90 * 864e5), is_self: true },
    { id: 'u2', org_id: ORG, email: 'ada@marula.test', display_name: 'Ada Whitfield', timezone: 'Europe/London', is_active: true, created_at: iso(-30 * 864e5), is_self: false },
  ],
  total: 2,
};

const STATUSES = [
  { id: 's1', board_id: 'b1', org_id: ORG, name: 'New', category: 'open', sort_order: 0, is_entry: true, color: null },
  { id: 's2', board_id: 'b1', org_id: ORG, name: 'In Progress', category: 'in_progress', sort_order: 1, is_entry: false, color: null },
  { id: 's3', board_id: 'b1', org_id: ORG, name: 'Done', category: 'closed', sort_order: 2, is_entry: false, color: null },
];

const BOARD_TICKETS = [
  { id: 't0000000-0000-4000-8000-0000000000a1', board_id: 'b1', number: 'MER-104', title: 'Mail server rejecting inbound', priority: 'p1', status_id: 's2', category: 'in_progress', assignee_id: 'u2', client_id: null, created_at: iso(-6 * 864e5), updated_at: iso(-600_000), due_at: iso(-3600_000), sla_resolution_due: null, merged_into_ticket_id: null, contact_id: null },
  { id: 't0000000-0000-4000-8000-0000000000a2', board_id: 'b1', number: 'MER-107', title: 'VPN drops every hour', priority: 'p2', status_id: 's1', category: 'open', assignee_id: 'u1', client_id: null, created_at: iso(-4 * 864e5), updated_at: iso(-1_000_000), due_at: iso(2 * 864e5), sla_resolution_due: null, merged_into_ticket_id: null, contact_id: null },
  { id: 't0000000-0000-4000-8000-0000000000a3', board_id: 'b1', number: 'MER-109', title: 'Printer queue stuck', priority: 'p3', status_id: 's2', category: 'in_progress', assignee_id: 'u2', client_id: null, created_at: iso(-9 * 864e5), updated_at: iso(-2_000_000), due_at: null, sla_resolution_due: null, merged_into_ticket_id: null, contact_id: null },
];

const REPORT = (kind, rows, summary) => ({ columns: [], kind, view: 'ops.v1_test', rows, summary, next_cursor: null });

const REPORTS = {
  time_by_user: REPORT('time_by_user', [
    { group_id: 'u1', label: 'Taylor Reed', metric: 391, billable_minutes: 300, entry_count: 9 },
    { group_id: 'u2', label: 'Ada Whitfield', metric: 347, billable_minutes: 280, entry_count: 8 },
    { group_id: 'u3', label: 'Marco Diaz', metric: 268, billable_minutes: 190, entry_count: 7 },
  ], { entries_count: 24, total_minutes: 1211, billable_minutes: 930 }),
  ticket_throughput: REPORT('ticket_throughput', [
    { day: '2026-09-14', created_count: 6, resolved_count: 4 },
    { day: '2026-09-15', created_count: 5, resolved_count: 6 },
    { day: '2026-09-16', created_count: 7, resolved_count: 5 },
    { day: '2026-09-17', created_count: 4, resolved_count: 4 },
  ], { created_total: 21, resolved_total: 19 }),
  ticket_aging: REPORT('ticket_aging', [
    { bucket: 'under_1d', bucket_ordinal: 0, open_count: 12, oldest_age_days: 0, avg_age_days: 0.4 },
    { bucket: 'd1_3', bucket_ordinal: 1, open_count: 8, oldest_age_days: 3, avg_age_days: 1.9 },
    { bucket: 'd3_7', bucket_ordinal: 2, open_count: 5, oldest_age_days: 6, avg_age_days: 4.2 },
  ], { open_total: 25, oldest_age_days: 6, breached_open: 2 }),
  sla_attainment: REPORT('sla_attainment', [
    { group_id: 'p1', label: 'Gold SLA', metric: 10, resolutions_met: 9, attainment_pct: 90 },
    { group_id: 'u4', label: 'Silver SLA', metric: 14, resolutions_met: 11, attainment_pct: 79 },
  ], { resolutions_total: 24, resolutions_met: 20, attainment_pct: 79 }),
  sla_breaches: REPORT('sla_breaches', [
    { ticket_id: 't0000000-0000-4000-8000-0000000000b1', ticket_number: 'MER-42', priority: 'p1', policy_name: 'Gold SLA', resolution_due: iso(-4 * 864e5), resolution_met: false },
  ], { breaches_total: 1, paused_business_secs_total: 3600 }),
  first_response: REPORT('first_response', [
    { ticket_id: 't0000000-0000-4000-8000-0000000000a1', ticket_number: 'MER-104', created_at: iso(-6 * 864e5), client_id: null, assignee_id: 'u1', first_staff_comment_at: iso(-5 * 864e5), first_response_secs: 6400, first_response_due: iso(-5 * 864e5) },
    { ticket_id: 't0000000-0000-4000-8000-0000000000b2', ticket_number: 'MER-88', created_at: iso(-2 * 864e5), client_id: null, assignee_id: 'u2', first_staff_comment_at: null, first_response_secs: null, first_response_due: iso(864e5) },
    { ticket_id: 't0000000-0000-4000-8000-0000000000b3', ticket_number: 'MER-91', created_at: iso(-864e5), client_id: null, assignee_id: 'u2', first_staff_comment_at: null, first_response_secs: null, first_response_due: iso(2 * 864e5) },
    { ticket_id: 't0000000-0000-4000-8000-0000000000b4', ticket_number: 'MER-95', created_at: iso(-3600_000), client_id: null, assignee_id: 'u1', first_staff_comment_at: null, first_response_secs: null, first_response_due: iso(12 * 3600_000) },
  ], { tickets_total: 22, answered_count: 19, unanswered_count: 3, response_p50_secs: 260, response_p90_secs: 900 }),
};

const TICKET = {
  id: 't0000000-0000-4000-8000-0000000000a1',
  org_id: ORG,
  board_id: 'b1',
  number: 'MER-104',
  title: 'Mail server rejecting inbound',
  priority: 'p1',
  status_id: 's2',
  status_name: 'In Progress',
  category: 'in_progress',
  assignee_id: 'u1',
  client_id: 'c1',
  contact_id: 'ct1',
  created_at: iso(-6 * 864e5),
  updated_at: iso(-600_000),
  resolved_at: null,
  merged_into_ticket_id: null,
  custom_fields: [],
  due_at: null,
  sla_resolution_due: null,
  site_id: null,
  tags: [],
};

const MEETING_TYPES = [
  { id: 'mt1', org_id: ORG, name: 'Site visit', description: null, duration_mins: 60, buffer_before_mins: 0, buffer_after_mins: 0, booking_window_days: null, assignee_scope: 'assignee', assignee_user_id: null, assignee_group_id: null, auto_email_enabled: false, auto_email_subject: null, auto_email_body: null, parameters: null, is_active: true, created_at: iso(-86e5), updated_at: iso(-86e5) },
];

const SCHEDULING_LINKS = [
  { id: 'e0000000-0000-4000-8000-0000000000c1', org_id: ORG, ticket_id: 't0000000-0000-4000-8000-0000000000a1', meeting_type_id: 'mt1', label: 'Client — options call', created_at: iso(-3600_000), created_by: null, expires_at: iso(2 * 864e5), scheduled_appointment_id: null, last_viewed_at: iso(-1800_000), first_viewed_at: iso(-1800_000), scheduled_at: null, revoked_at: null },
];

const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/org': { id: ORG, name: 'Marula Demo', slug: 'marula', ticket_prefix: 'MER', base_currency: 'USD' },
  '/api/v1/settings/user': [],
  '/api/v1/settings/org': [],
  '/api/v1/boards/nav': BOARDS_NAV,
  '/api/v1/org/users': USERS,
  '/api/v1/boards': [],
  '/api/v1/boards/b1/statuses': STATUSES,
  '/api/v1/boards/b1/transitions': { mode: 'floor', edges: [] },
  '/api/v1/boards/b1/members': [],
  '/api/v1/board-groups': [{ id: 'g1', name: 'Managed', position: 0 }],
  '/api/v1/tickets': BOARD_TICKETS,
  '/api/v1/tickets/t0000000-0000-4000-8000-0000000000a1': TICKET,
  '/api/v1/tickets/t0000000-0000-4000-8000-0000000000a1/sla': { applied: false, breached: false, paused: false, minutes_remaining: null },
  '/api/v1/tickets/t0000000-0000-4000-8000-0000000000a1/comments': [],
  '/api/v1/tickets/t0000000-0000-4000-8000-0000000000a1/attachments': [],
  '/api/v1/tickets/t0000000-0000-4000-8000-0000000000a1/checklist': { items: [{ id: 'ck1', ticket_id: 't0000000-0000-4000-8000-0000000000a1', template_item_id: 'i1', text: 'Backups verified', state: 'pending' }], enforcement_active: false, blocking: [] },
  '/api/v1/tickets/t0000000-0000-4000-8000-0000000000a1/time-entries': [],
  '/api/v1/clients': [],
  '/api/v1/clients/c1': { id: 'c1', org_id: ORG, name: 'Acme Corp', created_at: iso(-86e5) },
  '/api/v1/clients/c1/contacts': [{ id: 'ct1', client_id: 'c1', org_id: ORG, name: 'Dana Reyes', email: 'dana@acme.test' }],
  '/api/v1/meeting-types': MEETING_TYPES,
  '/api/v1/scheduling-links': SCHEDULING_LINKS,
  '/api/v1/scheduling-links/t0000000-0000-4000-8000-0000000000a1/bookings': [],
  '/api/v1/timers/working-set': [],
  '/api/v1/timers/active': null,
  '/api/v1/notifications/unread-count': { count: 0 },
  '/api/v1/notifications': { items: [], unread_count: 0, total: 0, next_cursor: null },
  '/api/v1/calendar-events': [],
  '/api/v1/time-entries': [],
  '/api/v1/on-call': [],
  '/api/v1/boards/b1/checklist-template': {
    items: [{ id: 'i1', org_id: ORG, template_id: 't1', sort_order: 0, text: 'Backups verified' }],
    require_checklist_close: true,
    template: { id: 't1', board_id: 'b1', org_id: ORG, name: 'Standard close', created_at: '', updated_at: '' },
  },
  '/api/v1/calendar/week': { items: [], exclusions: [] },
  '/api/v1/calendar/team': { items: [], exclusions: [] },
  '/api/v1/roles': { portal_tiers: [], presets: [], roles: [] },
  '/api/v1/calendar-days/me': [],
  '/api/v1/calendar-days/team': [],
  '/api/v1/calendar-weeks/me': [],
  '/api/v1/calendar-weeks/team': [],
  '/api/v1/exclusions': [],
  '/api/v1/user': {},
  '/api/v1/workflows': { mode: 'floor', edges: [] },
  '/api/v1/auth/mfa/status': { totp_enabled: false, passkeys: [], recovery_remaining: 0 },
  '/api/v1/time-gaps/me': { user_id: ME.user_id, timezone: 'UTC', expected_minutes: 480, days: [] },
  '/api/v1/auth/mfa/totp/enroll-begin': { secret_base32: 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP', otpauth_url: 'otpauth://totp/Marula:owner%40marula.test?issuer=Marula&secret=JBSWY3DPEHPK3PXP' },
};

async function shot(browser, viewport, theme, name, path, interactions, unauthed = false) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', name, String(e), '\nSTACK', String(e.stack ?? '').split('\n').slice(1, 12).join('\n')));
  const url = new URL(`${BASE}${path}`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');
  const themeToken = theme === 'dark' ? 'dark' : 'light';
  await page.addInitScript(({ t, unauthed }) => {
    window.localStorage.setItem('marula-theme', t);
    if (!unauthed) {
      window.localStorage.setItem('marula-tokens', JSON.stringify({
        access_token: 'test-access',
        refresh_token: 'test-refresh',
        session_id: 'sess',
        expires_in: 900,
      }));
      window.localStorage.setItem('marula-tokens-exp', String(Date.now() + 60_000));
    }
    if (unauthed === 'mfa') {
      window.sessionStorage.setItem('marula-mfa-pending', JSON.stringify({
        mfa_token: 'bridge-token',
        methods: ['totp', 'recovery', 'passkey'],
      }));
    }
    const style = document.createElement('style');
    style.textContent = 'nextjs-portal{display:none!important}';
    document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
  }, { t: themeToken, unauthed });

  await page.route('**/api/**', async (r) => {
    const key = new URL(r.request().url()).pathname;
    const method = r.request().method();
    if (method === 'POST' && key === '/api/v1/auth/mfa/totp/enroll-begin') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES['/api/v1/auth/mfa/totp/enroll-begin']) });
      return;
    }
    if (key.startsWith('/api/v1/reports/')) {
      const kind = key.replace('/api/v1/reports/', '');
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(REPORTS[kind] ?? REPORT(kind, [], {})) });
      return;
    }
    if (FIXTURES[key] !== undefined) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES[key]) });
      return;
    }
    await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
  });

  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForSelector('main', { timeout: 20_000 });
  await page.waitForTimeout(1100);
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
const TRY = (name) => (e) => console.error('FAIL', name, e?.message ?? e);

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, args: ['--no-sandbox', '--disable-dev-shm-usage'] });

// dashboards: widget grid (charts + stat widgets)
for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    await shot(browser, vp, theme, `dashboards-${vp.width}-${theme}.png`, '/dashboards').catch(TRY('dashboards'));
  }
}
// chart → data-table alternative (per-widget toggle), SLA health preset
for (const theme of ['light', 'dark']) {
  await shot(browser, { width: 1440, height: 1024 }, theme, `dashboards-table-1440-${theme}.png`, '/dashboards', async (page) => {
    const toggle = page.getByTestId('widget-table-toggle').first();
    await toggle.scrollIntoViewIfNeeded();
    await toggle.click();
    await page.waitForTimeout(300);
  }).catch(TRY('dashboards-table'));
}
// unanswered drilldown (First response widget)
for (const theme of ['light', 'dark']) {
  await shot(browser, { width: 1440, height: 1024 }, theme, `dashboards-unanswered-1440-${theme}.png`, '/dashboards', async (page) => {
    const drill = page.getByTestId('unanswered-drilldown').first();
    await drill.scrollIntoViewIfNeeded();
    await drill.click();
    await page.waitForTimeout(300);
  }).catch(TRY('unanswered'));
}

// kanban with the directory-driven assignee filter; active state too
for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    await shot(browser, vp, theme, `boards-${vp.width}-${theme}.png`, '/boards/b1').catch(TRY('boards'));
  }
}
for (const theme of ['light', 'dark']) {
  await shot(browser, { width: 1440, height: 1024 }, theme, `boards-assignee-1440-${theme}.png`, '/boards/b1', async (page) => {
    const select = page.locator('[data-testid="board-filter-assignee"]');
    await select.selectOption({ label: 'Ada Whitfield' });
    await page.waitForTimeout(400);
  }).catch(TRY('boards-assignee'));
}

// calendar month view; keyboard-nav state at 1440
for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    await shot(browser, vp, theme, `calendar-${vp.width}-${theme}.png`, '/calendar', async (page) => {
      await page.getByRole('button', { name: 'Month' }).click();
      await page.getByTestId('month-grid').waitFor({ timeout: 10_000 });
      await page.waitForTimeout(300);
    }).catch(TRY('calendar'));
  }
}
for (const theme of ['light', 'dark']) {
  await shot(browser, { width: 1440, height: 1024 }, theme, `calendar-keys-1440-${theme}.png`, '/calendar', async (page) => {
    await page.getByRole('button', { name: 'Month' }).click();
    await page.getByTestId('month-grid').waitFor({ timeout: 10_000 });
    const cell = page.getByTestId('month-day').nth(10);
    await cell.scrollIntoViewIfNeeded();
    await cell.focus();
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('ArrowRight');
    await page.waitForTimeout(250);
  }).catch(TRY('calendar-keys'));
}

// auth de-serif cards
for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    await shot(browser, vp, theme, `mfa-${vp.width}-${theme}.png`, '/mfa', null, 'mfa').catch(TRY('mfa'));
    await shot(browser, vp, theme, `sso-${vp.width}-${theme}.png`, '/sso', null, true).catch(TRY('sso'));
    await shot(browser, vp, theme, `accept-invite-${vp.width}-${theme}.png`, '/accept-invite?token=bridge-invite', null, true).catch(TRY('accept-invite'));
  }
}

// settings/security TOTP setup with copy affordance
for (const vp of [1440, 480]) {
  for (const theme of ['light', 'dark']) {
    await shot(browser, { width: vp, height: vp === 480 ? 900 : 1024 }, theme, `security-totp-setup-${vp}-${theme}.png`, '/settings/security', async (page) => {
      await page.getByRole('button', { name: 'Start setup' }).click();
      await page.waitForTimeout(600);
    }).catch(TRY('security-totp'));
  }
}

// ticket surfaces: classic + workbench-v2 Details tab (scheduling pod)
for (const theme of ['light', 'dark']) {
  await shot(browser, { width: 1440, height: 1024 }, theme, `classic-workspace-1440-${theme}.png`, `/tickets/classic/t0000000-0000-4000-8000-0000000000a1`).catch(TRY('classic'));
  await shot(browser, { width: 1440, height: 1024 }, theme, `workbench-details-1440-${theme}.png`, `/tickets/t0000000-0000-4000-8000-0000000000a1`, async (page) => {
    await page.getByTestId('detail-tab-details').click();
    await page.getByTestId('pod-scheduling').waitFor({ timeout: 10_000 });
    await page.waitForTimeout(400);
  }).catch(TRY('workbench-details'));
}
await shot(browser, { width: 1024, height: 768 }, 'light', `workbench-details-1024-light.png`, `/tickets/t0000000-0000-4000-8000-0000000000a1`, async (page) => {
  await page.getByTestId('detail-tab-details').click();
  await page.getByTestId('pod-scheduling').waitFor({ timeout: 10_000 });
  await page.waitForTimeout(400);
}).catch(TRY('workbench-details-1024'));

await browser.close();
console.log('done →', OUT);
