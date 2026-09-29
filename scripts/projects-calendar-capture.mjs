#!/usr/bin/env node
/**
 * Projects/Calendar capture — Wave 17 lane 4 (p17-projects-calendar).
 *
 * Screenshots the three owned surfaces: /projects (portfolio list, plus
 * the create dialog at 1440), /projects/[id] (delivery surface w/ phase
 * rail + triage, plus the dependencies dialog at 1440), /calendar (week,
 * month, agenda views + the expanded inbound mirror) — at
 * 1440/1024/768/480 in light AND dark, into design/qa-projects-calendar/.
 * API responses are fixtures served by Playwright routing — no backend
 * needed. Pattern: scripts/dashboards-today-capture.mjs.
 *
 * Usage: start the web app (dev on :3000), then node scripts/projects-calendar-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-projects-calendar');
const BASE = process.argv[2] ?? 'http://localhost:3000';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ORG = '00000000-0000-4000-8000-000000000002';
const ME_ID = '00000000-0000-4000-8000-000000000001';
const ME = {
  user_id: ME_ID,
  org_id: ORG,
  email: 'owner@marula.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_users', 'admin:org'],
};
const CLIENT_ID = '00000000-0000-4000-8000-0000000000c1';
const P_OVERDUE = '00000000-0000-4000-8000-0000000000p1';
const P_HEALTHY = '00000000-0000-4000-8000-0000000000p2';
const P_HOLD = '00000000-0000-4000-8000-0000000000p3';
const PH_DONE = '00000000-0000-4000-8000-0000000000h1';
const PH_ACTIVE = '00000000-0000-4000-8000-0000000000h2';
const PH_MILESTONE = '00000000-0000-4000-8000-0000000000h3';
const TASK_BLOCKED = '00000000-0000-4000-8000-0000000000t1';
const TASK_BLOCKING = '00000000-0000-4000-8000-0000000000t2';
const TASK_CALM = '00000000-0000-4000-8000-0000000000t3';
const APPT_ID = '00000000-0000-4000-8000-0000000000a1';
const ONCALL_ID = '00000000-0000-4000-8000-0000000000a2';
const SYNCED_ID = '00000000-0000-4000-8000-0000000000e1';
const TICKET_ID = '00000000-0000-4000-8000-0000000000t9';
const CONN_IN = '00000000-0000-4000-8000-0000000000s1';
const CONN_OUT = '00000000-0000-4000-8000-0000000000s2';

const now = Date.now();
const iso = (offsetMs) => new Date(now + offsetMs).toISOString();
const dateStr = (offsetDays) => new Date(now + offsetDays * 86_400_000).toISOString().slice(0, 10);
const TODAY = new Date();
const at = (day, h, m) =>
  new Date(day.getFullYear(), day.getMonth(), day.getDate(), h, m).toISOString();

const PROJECTS = [
  { id: P_OVERDUE, org_id: ORG, name: 'Exchange migration', status: 'active', billing_method: 'milestone', client_id: CLIENT_ID, budget_minor: 5000000, currency: 'USD', description: 'Move Acme from on-prem Exchange to Exchange Online.', start_date: dateStr(-30), due_date: dateStr(-2), created_at: iso(-40 * 86400000), updated_at: iso(-3600_000) },
  { id: P_HEALTHY, org_id: ORG, name: 'Onboard Initech', status: 'planning', billing_method: 'time_and_materials', client_id: null, budget_minor: null, currency: null, description: null, start_date: dateStr(7), due_date: dateStr(45), created_at: iso(-6 * 86400000), updated_at: iso(-86400_000) },
  { id: P_HOLD, org_id: ORG, name: 'Datacenter decommission', status: 'on_hold', billing_method: 'fixed_fee', client_id: CLIENT_ID, budget_minor: 1200000, currency: 'USD', description: 'Phase 2 awaits the lease signature.', start_date: dateStr(-90), due_date: dateStr(5), created_at: iso(-90 * 86400000), updated_at: iso(-7 * 86400000) },
];

function phase(id, name, sortOrder, status, isMilestone, amount) {
  return {
    id, org_id: ORG, project_id: P_OVERDUE, name, sort_order: sortOrder, status,
    is_milestone: isMilestone, milestone_amount_minor: amount, milestone_currency: amount === null ? null : 'USD',
    description: null, completed_at: null, created_at: iso(-30 * 86400000), updated_at: iso(-86400_000),
  };
}

const PHASES_ALL = [
  phase(PH_DONE, 'Discovery', 0, 'completed', false, null),
  phase(PH_ACTIVE, 'Mailbox migration', 1, 'active', false, null),
  phase(PH_MILESTONE, 'Cutover sign-off', 2, 'pending', true, 1200000),
];

function task(id, title, sortOrder, over = {}) {
  return {
    id, org_id: ORG, project_id: P_OVERDUE, title, status: 'pending', sort_order: sortOrder,
    is_milestone: false, milestone_amount_minor: null, milestone_currency: null, phase_id: PH_ACTIVE,
    assignee_id: ME_ID, description: null, due_at: null, completed_at: null,
    created_at: iso(-20 * 86400000), updated_at: iso(-7200_000), ...over,
  };
}

const TASKS = [
  task(TASK_BLOCKED, 'Cut over mail flow', 0, { due_at: iso(-3 * 3600_000) }),
  task(TASK_BLOCKING, 'Freeze mailbox deltas', 1, { due_at: iso(2 * 86400_000 + 3600_000) }),
  task(TASK_CALM, 'Update DNS TTL documentation', 2, { phase_id: PH_DONE }),
];

const CLIENT = { id: CLIENT_ID, org_id: ORG, name: 'Acme Corp', created_at: '', updated_at: '', visibility: 'private' };

function calendarView() {
  return {
    from: at(TODAY, 0, 0),
    to: at(new Date(TODAY.getFullYear(), TODAY.getMonth(), TODAY.getDate() + 7), 0, 0),
    timezone: 'UTC',
    user_id: ME_ID,
    availability_windows: [],
    items: [
      { id: APPT_ID, item_type: 'appointment', starts_at: at(TODAY, 9, 0), ends_at: at(TODAY, 10, 30), title: 'Site visit — Acme HQ', kind: 'on_site', status: null, ticket_id: TICKET_ID, user_id: ME_ID },
      { id: ONCALL_ID, item_type: 'on_call', starts_at: at(TODAY, 14, 0), ends_at: at(TODAY, 15, 0), title: null, category: 'Primary', kind: null, status: null, ticket_id: null, user_id: ME_ID },
      { id: '00000000-0000-4000-8000-0000000000a3', item_type: 'appointment', starts_at: at(TODAY, 9, 45), ends_at: at(TODAY, 10, 15), title: 'Vendor sync', kind: 'remote', status: null, ticket_id: null, user_id: ME_ID },
    ],
    exclusions: [
      { id: SYNCED_ID, org_id: ORG, user_id: ME_ID, starts_at: at(TODAY, 11, 0), ends_at: at(TODAY, 12, 0), reason: 'Calendar sync: Vendor maintenance window', created_at: '' },
      { id: '00000000-0000-4000-8000-0000000000e2', org_id: ORG, user_id: ME_ID, starts_at: at(new Date(TODAY.getFullYear(), TODAY.getMonth(), TODAY.getDate() + 2), 9, 0), ends_at: at(new Date(TODAY.getFullYear(), TODAY.getMonth(), TODAY.getDate() + 2), 17, 0), reason: 'Time off', created_at: '' },
    ],
  };
}

const SYNC_CONNECTIONS = [
  { id: CONN_IN, org_id: ORG, user_id: ME_ID, name: 'Work calendar (Google)', provider: 'ical', direction: 'inbound', status: 'active', feed_url: 'https://calendar.google.com/calendar/ical/...', feed_path: null, last_synced_at: iso(-5 * 60_000), last_error: null, created_at: iso(-20 * 86400000), updated_at: iso(-300_000) },
  { id: CONN_OUT, org_id: ORG, user_id: ME_ID, name: 'Publish to client portal', provider: 'ical', direction: 'outbound', status: 'error', feed_url: null, feed_path: '/api/v1/calendar-sync/feed/tok123', last_synced_at: iso(-26 * 3600_000), last_error: 'the external calendar is unreachable', created_at: iso(-10 * 86400000), updated_at: iso(-3600_000) },
];

const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/org': { id: ORG, name: 'Marula Demo', ticket_prefix: 'MER' },
  '/api/v1/settings/user': [],
  '/api/v1/settings/org': [],
  '/api/v1/notifications/unread-count': { count: 0 },
  '/api/v1/notifications': { items: [], unread_count: 0, total: 0, next_cursor: null },
  '/api/v1/boards': [],
  '/api/v1/board-groups': [],
  '/api/v1/boards/nav': [],
  '/api/v1/user': {},
  '/api/v1/tickets': [
    { id: TICKET_ID, number: 'MER-42', title: 'Mail server rejecting inbound', due_at: at(TODAY, 12, 30), category: 'open', merged_into_ticket_id: null },
  ],
  '/api/v1/projects': PROJECTS,
  '/api/v1/clients': [CLIENT],
  [`/api/v1/projects/${P_OVERDUE}`]: PROJECTS[0],
  [`/api/v1/projects/${P_HEALTHY}`]: PROJECTS[1],
  [`/api/v1/projects/${P_HOLD}`]: PROJECTS[2],
  [`/api/v1/clients/${CLIENT_ID}`]: CLIENT,
  [`/api/v1/projects/${P_OVERDUE}/phases`]: PHASES_ALL,
  [`/api/v1/projects/${P_HEALTHY}/phases`]: [],
  [`/api/v1/projects/${P_HOLD}/phases`]: [phase(PH_DONE, 'Wave 1', 0, 'completed', false, null)],
  [`/api/v1/projects/${P_OVERDUE}/tasks`]: TASKS,
  [`/api/v1/projects/${P_HEALTHY}/tasks`]: [],
  [`/api/v1/projects/${P_HOLD}/tasks`]: [],
  [`/api/v1/projects/${P_OVERDUE}/tasks/${TASK_BLOCKED}/dependencies`]: { depends_on: [TASKS[1]] },
  [`/api/v1/projects/${P_OVERDUE}/tasks/${TASK_BLOCKING}/dependencies`]: { depends_on: [] },
  [`/api/v1/projects/${P_OVERDUE}/tasks/${TASK_CALM}/dependencies`]: { depends_on: [] },
  '/api/v1/calendar/week': calendarView(),
  '/api/v1/calendar/team': calendarView(),
  '/api/v1/calendar-sync': SYNC_CONNECTIONS,
  [`/api/v1/calendar-sync/${CONN_IN}/events`]: {
    connection_id: CONN_IN,
    events: [
      { id: '00000000-0000-4000-8000-0000000000v1', connection_id: CONN_IN, external_uid: 'uid-1', user_id: ME_ID, org_id: ORG, starts_at: at(TODAY, 11, 0), ends_at: at(TODAY, 12, 0), title: 'Vendor maintenance window', sequence: 3, first_seen_at: iso(-3 * 86400000), last_seen_at: iso(-300_000), exclusion_id: SYNCED_ID },
      { id: '00000000-0000-4000-8000-0000000000v2', connection_id: CONN_IN, external_uid: 'uid-2', user_id: ME_ID, org_id: ORG, starts_at: at(new Date(TODAY.getFullYear(), TODAY.getMonth(), TODAY.getDate() + 1), 13, 0), ends_at: at(new Date(TODAY.getFullYear(), TODAY.getMonth(), TODAY.getDate() + 1), 14, 0), title: '1:1 with ops', sequence: 2, first_seen_at: iso(-2 * 86400000), last_seen_at: iso(-300_000), exclusion_id: null },
    ],
  },
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
    if (FIXTURES[key] !== undefined) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES[key]) });
    } else {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
    }
  });

  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForSelector('main', { timeout: 20_000 });
  await page.waitForTimeout(1100);
  if (interactions) await interactions(page);
  await page.mouse.move(4, Math.floor(viewport.height / 2));
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

const DETAIL = `/projects/${P_OVERDUE}`;

for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    try {
      await shot(browser, vp, theme, `projects-${vp.width}-${theme}.png`, '/projects');
    } catch (e) {
      console.error('FAIL', vp.width, theme, e.message);
    }
  }
}

for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    try {
      await shot(browser, vp, theme, `project-detail-${vp.width}-${theme}.png`, DETAIL);
    } catch (e) {
      console.error('FAIL detail', vp.width, theme, e.message);
    }
  }
}

for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    try {
      await shot(browser, vp, theme, `calendar-week-${vp.width}-${theme}.png`, '/calendar');
    } catch (e) {
      console.error('FAIL calendar', vp.width, theme, e.message);
    }
  }
}

for (const theme of ['light', 'dark']) {
  try {
    await shot(browser, { width: 1440, height: 1024 }, theme, `projects-create-1440-${theme}.png`, '/projects', async (page) => {
      await page.getByRole('button', { name: 'New project' }).click();
      await page.waitForTimeout(400);
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `project-detail-deps-1440-${theme}.png`, DETAIL, async (page) => {
      await page.getByRole('button', { name: /Dependencies \(1\)/ }).first().click();
      await page.waitForTimeout(400);
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `calendar-month-1440-${theme}.png`, '/calendar', async (page) => {
      await page.getByRole('button', { name: 'Month' }).click();
      await page.waitForTimeout(600);
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `calendar-agenda-1440-${theme}.png`, '/calendar', async (page) => {
      await page.getByRole('button', { name: 'Agenda' }).click();
      await page.waitForTimeout(600);
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `calendar-mirror-1440-${theme}.png`, '/calendar', async (page) => {
      await page.getByTestId(`mirror-toggle-00000000-0000-4000-8000-0000000000s1`).click();
      await page.waitForTimeout(700);
    });
    await shot(browser, { width: 480, height: 900 }, theme, `calendar-agenda-480-${theme}.png`, '/calendar', async (page) => {
      await page.getByRole('button', { name: 'Agenda' }).click();
      await page.waitForTimeout(600);
    });
  } catch (e) {
    console.error('FAIL interaction', theme, e.message);
  }
}

await browser.close();
console.log('done');
