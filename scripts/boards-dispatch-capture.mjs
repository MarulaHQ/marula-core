#!/usr/bin/env node
/**
 * Boards/Dispatch/Scheduling capture — Wave 17 lane 3 (p17-boards-dispatch).
 *
 * Screenshots the lane's surfaces — /boards (list), /boards/{id} (kanban,
 * plus one collapsed-column state at 1440), /boards/{id}/fields (editor,
 * plus one add-dialog state at 1440), /dispatch (day board),
 * /dispatch/planning (capacity horizon), /dispatch/travel (mileage
 * ledger), /dispatch/scheduling (template admin, plus one new-template
 * dialog state at 1440) — at 1440/1024/768/480 in light AND dark, into
 * design/qa-boards-dispatch/. API responses are fixtures served by
 * Playwright routing — no backend needed.
 * Pattern: scripts/dashboards-today-capture.mjs (Wave 17 lane 1).
 *
 * Usage: start the web app (dev on :3000), then node scripts/boards-dispatch-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-boards-dispatch');
const BASE = process.argv[2] ?? 'http://localhost:3000';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ORG = '00000000-0000-4000-8000-000000000002';
const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: ORG,
  email: 'owner@marula.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_users', 'special:manage_boards', 'tickets:create', 'admin:org'],
};
const now = Date.now();
const iso = (offsetMs) => new Date(now + offsetMs).toISOString();
const dayIso = (h, m = 0) => {
  const d = new Date(now);
  d.setUTCHours(h, m, 0, 0);
  return d.toISOString();
};
const BOARD_ID = 'b1';
const U_ANA = '00000000-0000-4000-8000-00000000s0b1';
const U_MARCO = '00000000-0000-4000-8000-00000000s0b2';

const BOARDS_NAV = [
  { id: BOARD_ID, name: 'Service Desk', description: null, group_id: 'g1', group_name: 'Managed', favorite: true, is_default: true, my_role: 'admin' },
  { id: 'b2', name: 'Projects', description: null, group_id: null, group_name: null, favorite: false, is_default: false, my_role: 'member' },
];

const STATUSES = [
  { id: 's1', board_id: BOARD_ID, org_id: ORG, name: 'New', category: 'open', sort_order: 0, is_entry: true, color: null },
  { id: 's2', board_id: BOARD_ID, org_id: ORG, name: 'In Progress', category: 'in_progress', sort_order: 1, is_entry: false, color: null },
  { id: 's3', board_id: BOARD_ID, org_id: ORG, name: 'Waiting on Client', category: 'waiting', sort_order: 2, is_entry: false, color: null },
  { id: 's4', board_id: BOARD_ID, org_id: ORG, name: 'Done', category: 'closed', sort_order: 3, is_entry: false, color: null },
];

const ticket = (n, title, status_id, category, priority, extra = {}) => ({
  id: `t-${n}`,
  board_id: BOARD_ID,
  org_id: ORG,
  number: `MER-${100 + n}`,
  title,
  status_id,
  category,
  priority,
  assignee_id: extra.assignee ?? null,
  due_at: extra.due ?? null,
  created_at: iso(-3 * 86_400_000),
  updated_at: iso(-2 * 3600_000),
  requester_id: null,
  client_id: extra.client ?? null,
});

const TICKETS = [
  ticket(1, 'Mail server rejecting inbound', 's2', 'in_progress', 'p1', { assignee: U_ANA }),
  ticket(2, 'VPN drops every hour on site', 's2', 'in_progress', 'p1', { assignee: U_MARCO }),
  ticket(3, 'Laptop imaging round two', 's1', 'open', 'p2', {}),
  ticket(4, 'Printer fleet driver rollout', 's1', 'open', 'p3', { assignee: U_ANA }),
  ticket(5, 'Awaiting license keys from vendor', 's3', 'waiting', 'p2', { due: iso(26 * 3600_000) }),
  ticket(6, 'Quarterly patch cycle sign-off', 's3', 'waiting', 'p3', {}),
  ticket(7, 'Backup verification January', 's4', 'closed', 'p4', {}),
  ticket(8, 'Onboard two new hires', 's4', 'closed', 'p3', { due: iso(-30 * 3600_000) }),
];

const FIELDS = [
  { id: 'f1', board_id: BOARD_ID, org_id: ORG, entity: 'ticket', name: 'Severity', kind: 'select', options: ['low', 'medium', 'high'], required: true, sort_order: 0 },
  { id: 'f2', board_id: BOARD_ID, org_id: ORG, entity: 'ticket', name: 'RMA number', kind: 'text', options: [], required: false, sort_order: 1 },
  { id: 'f3', board_id: BOARD_ID, org_id: ORG, entity: 'ticket', name: 'Assets touched', kind: 'number', options: [], required: false, sort_order: 2 },
  { id: 'f4', board_id: BOARD_ID, org_id: ORG, entity: 'ticket', name: 'Site visit booked', kind: 'bool', options: [], required: false, sort_order: 3 },
];

const DISPATCH_APPT = (n, user, title, startH, endH, extra = {}) => ({
  id: `a-${n}`,
  org_id: ORG,
  user_id: user,
  ticket_id: `t-${n}`,
  ticket_number: `MER-${100 + n}`,
  title,
  client_name: extra.client ?? 'Acme Corp',
  client_id: null,
  site_id: null,
  kind: 'onsite',
  status: 'confirmed',
  dispatch_status: extra.dispatch ?? null,
  starts_at: dayIso(startH),
  ends_at: dayIso(endH),
  travel_minutes: extra.travel ?? 20,
  notes: null,
  scheduling_link_id: null,
  created_at: iso(-86_400_000),
  updated_at: iso(-3600_000),
});

const DAY_VIEW = {
  date: new Date(now).toISOString().slice(0, 10),
  from: dayIso(0),
  to: dayIso(23, 59),
  technicians: [
    {
      user_id: U_ANA,
      display_name: 'Ana Sousa',
      timezone: 'UTC',
      available_minutes: 480,
      booked_minutes: 300,
      utilization_percent: 63,
      appointments: [
        DISPATCH_APPT(1, U_ANA, 'Mail server repair', 9, 12, { travel: 30 }),
        DISPATCH_APPT(4, U_ANA, 'Printer driver rollout', 13, 15, { client: 'Northwind', travel: 15 }),
      ],
      exclusions: [],
      on_call: [{ id: 'oc1', user_id: U_ANA, category: 'primary', starts_at: dayIso(17), ends_at: dayIso(22), created_at: iso(-3 * 86_400_000) }],
      availability_windows: [],
    },
    {
      user_id: U_MARCO,
      display_name: 'Marco Diaz',
      timezone: 'UTC',
      available_minutes: 480,
      booked_minutes: 450,
      utilization_percent: 112,
      appointments: [
        DISPATCH_APPT(2, U_MARCO, 'VPN investigation', 8, 12, { client: 'Initech', travel: 25 }),
        DISPATCH_APPT(3, U_MARCO, 'Firewall swap', 12, 17, { dispatch: 'en_route' }),
      ],
      exclusions: [],
      on_call: [],
      availability_windows: [],
    },
  ],
  unassigned: [
    { id: 't-9', board_id: BOARD_ID, board_name: 'Service Desk', number: 'MER-109', title: 'Switch uplink flapping', priority: 'p1', category: 'open', status_name: 'New', client_name: 'Acme Corp', due_at: iso(-2 * 3600_000), sla_resolution_due: iso(-1 * 3600_000), updated_at: iso(-3600_000) },
    { id: 't-10', board_id: BOARD_ID, board_name: 'Service Desk', number: 'MER-110', title: 'WiFi dead zone survey', priority: 'p2', category: 'open', status_name: 'New', client_name: null, due_at: null, sla_resolution_due: null, updated_at: iso(-7200_000) },
    { id: 't-11', board_id: BOARD_ID, board_name: 'Service Desk', number: 'MER-111', title: 'Door badge reader offline', priority: 'p3', category: 'open', status_name: 'New', client_name: 'Northwind', due_at: null, sla_resolution_due: null, updated_at: iso(-86_400_000) },
  ],
};

const capacityDay = (i, util) => ({
  date: new Date(now + i * 86_400_000).toISOString().slice(0, 10),
  available_minutes: 480,
  booked_minutes: Math.round(480 * util / 100),
  free_minutes: Math.max(0, 480 - Math.round(480 * util / 100)),
  utilization_percent: util,
});

const CAPACITY = {
  from: new Date(now).toISOString().slice(0, 10),
  days: 7,
  unassigned_count: 3,
  technicians: [
    { user_id: U_ANA, display_name: 'Ana Sousa', days: [0, 1, 2, 3, 4, 5, 6].map((i) => capacityDay(i, [55, 62, 48, 90, 70, 40, 35][i])) },
    { user_id: U_MARCO, display_name: 'Marco Diaz', days: [0, 1, 2, 3, 4, 5, 6].map((i) => capacityDay(i, [88, 104, 96, 70, 55, 30, 25][i])) },
    { user_id: '00000000-0000-4000-8000-00000000s0b3', display_name: 'Priya Nair', days: [0, 1, 2, 3, 4, 5, 6].map((i) => capacityDay(i, [30, 42, 66, 52, 88, 45, 20][i])) },
  ],
};

const SKILLS = {
  skills: [
    { skill: 'networking', holders: 3, active_holders: 2 },
    { skill: 'windows', holders: 4, active_holders: 3 },
    { skill: 'macos', holders: 2, active_holders: 1 },
  ],
};

const CONNECTOR_TECHS = {
  technicians: [
    { user_id: U_ANA, display_name: 'Ana Sousa', skills: ['networking'], active: true },
    { user_id: U_MARCO, display_name: 'Marco Diaz', skills: ['windows'], active: true },
  ],
};

const CONNECTOR_SCHEDULE = {
  days: 7,
  appointments: [DISPATCH_APPT(1, U_ANA, 'Mail server repair', 9, 12), DISPATCH_APPT(2, U_MARCO, 'VPN investigation', 8, 12)],
};

const DRAFTS = [
  { id: 'd1', org_id: ORG, user_id: ME.user_id, ticket_id: 't-1', started_at: dayIso(8, 30), ended_at: dayIso(9, 0), distance: 32.5, distance_unit: 'km', vehicle: 'Van 1', billable: true, notes: 'Site run', status: 'submitted', submitted_time_entry_id: 'te-1', created_at: iso(-7200_000), updated_at: iso(-7200_000) },
  { id: 'd2', org_id: ORG, user_id: ME.user_id, ticket_id: 't-2', started_at: dayIso(12, 45), ended_at: dayIso(13, 5), distance: 18, distance_unit: 'km', vehicle: 'Van 1', billable: true, notes: null, status: 'draft', submitted_time_entry_id: null, created_at: iso(-3600_000), updated_at: iso(-3600_000) },
  { id: 'd3', org_id: ORG, user_id: ME.user_id, ticket_id: 't-5', started_at: dayIso(15, 10), ended_at: dayIso(15, 40), distance: 41.2, distance_unit: 'km', vehicle: 'Car 2', billable: false, notes: 'Parts pickup', status: 'draft', submitted_time_entry_id: null, created_at: iso(-1800_000), updated_at: iso(-1800_000) },
];

const USERS = {
  users: [
    { id: U_ANA, org_id: ORG, email: 'ana@marula.test', display_name: 'Ana Sousa', is_active: true, org_role: 'member', created_at: iso(-90 * 86_400_000) },
    { id: U_MARCO, org_id: ORG, email: 'marco@marula.test', display_name: 'Marco Diaz', is_active: true, org_role: 'member', created_at: iso(-80 * 86_400_000) },
    { id: '00000000-0000-4000-8000-00000000s0b3', org_id: ORG, email: 'priya@marula.test', display_name: 'Priya Nair', is_active: true, org_role: 'admin', created_at: iso(-70 * 86_400_000) },
  ],
  total: 3,
};

const GROUPS = [
  { id: 'g-sched', org_id: ORG, name: 'Service Desk', description: 'Front-line availability pool', member_user_ids: [U_ANA, U_MARCO], created_at: iso(-30 * 86_400_000), created_by: null },
];

const MEETING_TYPES = [
  { id: 'mt-1', org_id: ORG, name: 'On-site visit', description: '90-minute field visit', duration_minutes: 90, buffer_minutes: 15, booking_window_days: 21, assignee_scope: 'group', assignee_user_id: null, assignee_group_id: 'g-sched', auto_email_enabled: true, auto_email_subject: 'Schedule your service visit', auto_email_body: 'Pick a slot that works for you:', is_active: true, parameters: {}, created_at: iso(-20 * 86_400_000), created_by: null },
  { id: 'mt-2', org_id: ORG, name: 'Phone intake', description: null, duration_minutes: 30, buffer_minutes: 0, booking_window_days: 14, assignee_scope: 'user', assignee_user_id: U_ANA, assignee_group_id: null, auto_email_enabled: false, auto_email_subject: null, auto_email_body: null, is_active: true, parameters: {}, created_at: iso(-10 * 86_400_000), created_by: null },
];

const ORG_FIXTURE = { id: ORG, name: 'Marula Demo', ticket_prefix: 'MER' };
const TIMER = {
  id: 's0000000-0000-4000-8000-000000000001',
  org_id: ORG,
  user_id: ME.user_id,
  ticket_id: 't-1',
  started_at: iso(-37 * 60_000),
  stopped_at: null,
  segments: [{ started_at: iso(-37 * 60_000), ended_at: null }],
};

const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/org': ORG_FIXTURE,
  '/api/v1/settings/user': [],
  '/api/v1/settings/org': [],
  '/api/v1/boards/nav': BOARDS_NAV,
  '/api/v1/board-groups': [{ id: 'g1', name: 'Managed', position: 0 }],
  '/api/v1/boards': [],
  '/api/v1/timers/working-set': [TIMER],
  '/api/v1/timers/active': TIMER,
  '/api/v1/notifications/unread-count': { count: 0 },
  '/api/v1/notifications': { items: [], unread_count: 0, total: 0, next_cursor: null },
  '/api/v1/clients': [],
  '/api/v1/user': {},
  // Kanban board
  [`/api/v1/boards/${BOARD_ID}/statuses`]: STATUSES,
  [`/api/v1/boards/${BOARD_ID}/transitions`]: { mode: 'floor', edges: [] },
  [`/api/v1/boards/${BOARD_ID}/members`]: { status: 403, json: { error: { code: 403, reason: 'forbidden', message: 'no board role' } } },
  [`/api/v1/boards/${BOARD_ID}/fields`]: FIELDS,
  '/api/v1/tickets': TICKETS,
  // Dispatch day board
  '/api/v1/dispatch/day': DAY_VIEW,
  // Planning
  '/api/v1/dispatch/capacity': CAPACITY,
  '/api/v1/dispatch/skills': SKILLS,
  '/api/v1/dispatch-connector/technicians': CONNECTOR_TECHS,
  '/api/v1/dispatch-connector/schedule': CONNECTOR_SCHEDULE,
  // Travel
  '/api/v1/dispatch/travel-drafts': DRAFTS,
  // Scheduling admin
  '/api/v1/meeting-types': MEETING_TYPES,
  '/api/v1/scheduling-groups': GROUPS,
  '/api/v1/org/users': USERS,
  '/api/v1/scheduling-links': [],
};

async function shot(browser, viewport, theme, name, path, interactions) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', name, String(e).slice(0, 300)));
  const url = new URL(`${BASE}${path}`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');
  const themeToken = theme === 'dark' ? 'dark' : 'light';
  await page.addInitScript((t) => {
    window.localStorage.setItem('marula-theme', t);
    window.localStorage.setItem('marula-tokens', JSON.stringify({
      access_token: 'test-access',
      refresh_token: 'test-refresh',
      session_id: 'sess',
      expires_in: 900,
    }));
    window.localStorage.setItem('marula-tokens-exp', String(Date.now() + 60_000));
    const style = document.createElement('style');
    style.textContent = 'nextjs-portal{display:none!important}';
    document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
  }, themeToken);

  await page.route('**/api/**', async (r) => {
    const key = new URL(r.request().url()).pathname;
    if (key === `/api/v1/boards/${BOARD_ID}/members`) {
      await r.fulfill({ status: 403, contentType: 'application/json', body: JSON.stringify({ error: { code: 403, reason: 'forbidden', message: 'no board role' } }) });
      return;
    }
    if (FIXTURES[key] !== undefined) {
      const fx = FIXTURES[key];
      if (typeof fx === 'object' && fx !== null && 'status' in fx) {
        await r.fulfill({ status: fx.status, contentType: 'application/json', body: fx.body });
      } else {
        await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(fx) });
      }
    } else {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
    }
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

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, args: ['--no-sandbox', '--disable-dev-shm-usage'] });

const surfaces = [
  { tag: 'kanban', path: `/boards/${BOARD_ID}` },
  { tag: 'fields', path: `/boards/${BOARD_ID}/fields` },
  { tag: 'dispatch', path: '/dispatch' },
  { tag: 'planning', path: '/dispatch/planning' },
  { tag: 'travel', path: '/dispatch/travel' },
  { tag: 'scheduling', path: '/dispatch/scheduling' },
];

for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    for (const s of surfaces) {
      try {
        await shot(browser, vp, theme, `${s.tag}-${vp.width}-${theme}.png`, s.path);
      } catch (e) {
        console.error('FAIL', s.tag, vp.width, theme, e.message);
      }
    }
  }
}

// Interaction states at 1440, both themes.
for (const theme of ['light', 'dark']) {
  try {
    await shot(browser, { width: 1440, height: 1024 }, theme, `kanban-collapsed-1440-${theme}.png`, `/boards/${BOARD_ID}`, async (page) => {
      await page.getByTestId('kanban-collapse-Done').click();
      await page.getByTestId('kanban-collapse-Waiting on Client').click();
      await page.getByTestId('board-filter-p1').click();
      await page.waitForTimeout(300);
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `fields-dialog-1440-${theme}.png`, `/boards/${BOARD_ID}/fields`, async (page) => {
      await page.getByTestId('add-field').click();
      await page.getByTestId('field-kind-select').click();
      await page.waitForTimeout(300);
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `scheduling-dialog-1440-${theme}.png`, '/dispatch/scheduling', async (page) => {
      await page.getByTestId('new-template').click();
      await page.waitForTimeout(400);
    });
  } catch (e) {
    console.error('FAIL interactions', theme, e.message);
  }
}

await browser.close();
console.log('done');
