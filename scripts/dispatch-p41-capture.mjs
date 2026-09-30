#!/usr/bin/env node
/**
 * Dispatch capture — Wave 41 lane 6 (p41-ui-dispatch).
 *
 * Photographs the four dispatch routes against the Marula surface pack's
 * twenty-nine states, in both light themes and dark, at 1600 (the pack's
 * capture width) plus narrower widths for the board. Lane 15
 * (p41-ui-audit) diffs `states/` against
 * brand/marula-mockups/marula-surface-pack/web/dispatch (neutral and dark) states, and reads
 * `contact-sheet.png` for the same 29 pictures on one page, the way the
 * pack's own overview sheet works.
 *
 * Every state is reached through the real UI — clicks, keys and pointer
 * drags — so a capture that succeeds is also proof the interaction works. The
 * clock is pinned with Playwright's clock API so the now-line, the live block
 * and the date label cannot drift between runs.
 *
 * API responses are fixtures served by Playwright routing — no backend needed.
 * Pattern: scripts/boards-dispatch-capture.mjs (Wave 17 lane 3).
 *
 * Usage: start the web app (dev on :3000), then
 *   node scripts/dispatch-p41-capture.mjs [baseUrl]
 * Env: CHROME_PATH (chromium executable), ONLY (comma-separated state ids).
 */
import { chromium } from 'playwright';
import { mkdir, readdir, readFile } from 'fs/promises';

import { existsSync } from 'fs';
import { join, resolve } from 'path';

const OUT = resolve('design/qa-p41-ui-dispatch');
const STATES_DIR = resolve(OUT, 'states');
const BASE = process.argv[2] ?? 'http://localhost:3000';
const ONLY = process.env.ONLY ? new Set(process.env.ONLY.split(',')) : null;
const FROZEN = new Date('2026-09-29T11:41:00Z');

if (!existsSync(STATES_DIR)) await mkdir(STATES_DIR, { recursive: true });

const ORG = '00000000-0000-4000-8000-000000000002';
const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: ORG,
  email: 'owner@marula.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_users', 'special:manage_boards', 'tickets:create', 'admin:org'],
};

const iso = (h, m = 0) => {
  const d = new Date(FROZEN);
  d.setUTCHours(h, m, 0, 0);
  return d.toISOString();
};
const DAY = '2026-09-29';

/** The pack's capacity week, with Wed 30 over capacity and Sun 4 uncovered —
 *  the two callouts the planning state asserts. */
const CAPACITY = {
  from: DAY,
  days: 7,
  unassigned_count: 6,
  technicians: [
    {
      user_id: ME.user_id,
      display_name: 'Tumi L.',
      days: [
        { date: DAY, available_minutes: 480, booked_minutes: 400, free_minutes: 80, utilization_percent: 83 },
        { date: '2026-09-30', available_minutes: 480, booked_minutes: 512, free_minutes: -32, utilization_percent: 107 },
        { date: '2026-10-01', available_minutes: 480, booked_minutes: 420, free_minutes: 60, utilization_percent: 88 },
        { date: '2026-10-02', available_minutes: 480, booked_minutes: 300, free_minutes: 180, utilization_percent: 63 },
        { date: '2026-10-03', available_minutes: 240, booked_minutes: 96, free_minutes: 144, utilization_percent: 40 },
        { date: '2026-10-04', available_minutes: 0, booked_minutes: 0, free_minutes: 0, utilization_percent: 0 },
        { date: '2026-10-05', available_minutes: 480, booked_minutes: 360, free_minutes: 120, utilization_percent: 75 },
      ],
    },
    {
      user_id: '00000000-0000-4000-8000-0000000000a2',
      display_name: 'Alex M.',
      days: [
        { date: DAY, available_minutes: 480, booked_minutes: 494, free_minutes: -14, utilization_percent: 103 },
        { date: '2026-09-30', available_minutes: 480, booked_minutes: 456, free_minutes: 24, utilization_percent: 95 },
        { date: '2026-10-01', available_minutes: 480, booked_minutes: 408, free_minutes: 72, utilization_percent: 85 },
        { date: '2026-10-02', available_minutes: 480, booked_minutes: 264, free_minutes: 216, utilization_percent: 55 },
        { date: '2026-10-03', available_minutes: 240, booked_minutes: 48, free_minutes: 192, utilization_percent: 20 },
        { date: '2026-10-04', available_minutes: 0, booked_minutes: 0, free_minutes: 0, utilization_percent: 0 },
        { date: '2026-10-05', available_minutes: 480, booked_minutes: 432, free_minutes: 48, utilization_percent: 90 },
      ],
    },
  ],
};

const SKILLS = {
  skills: [
    { skill: 'Network', holders: 4, active_holders: 3 },
    { skill: 'Security', holders: 2, active_holders: 2 },
    { skill: 'M365', holders: 3, active_holders: 2 },
    { skill: 'Hardware', holders: 4, active_holders: 2 },
    { skill: 'Telecoms', holders: 1, active_holders: 1 },
    { skill: 'Servers', holders: 3, active_holders: 2 },
  ],
};

/** Travel rows include a deliberate near-duplicate (same ticket, same window,
 *  same distance) so the pack's "Possible duplicate / Merge" state is real
 *  output rather than a hard-coded flag. */
const TRAVEL_DRAFTS = [
  { id: 'td1', user_id: 'u1', ticket_id: '18437', started_at: iso(14, 0), ended_at: iso(14, 32), distance: 12.4, distance_unit: 'km', vehicle: 'Van 2', billable: true, notes: null, status: 'draft', submitted_time_entry_id: null, expense_id: null, created_at: iso(9), updated_at: iso(9) },
  { id: 'td2', user_id: 'u2', ticket_id: '18433', started_at: iso(15, 0), ended_at: iso(15, 42), distance: 18.1, distance_unit: 'km', vehicle: 'Car 1', billable: true, notes: null, status: 'draft', submitted_time_entry_id: null, expense_id: null, created_at: iso(9, 30), updated_at: iso(9, 30) },
  { id: 'td3', user_id: 'u3', ticket_id: '18433', started_at: iso(15, 5), ended_at: iso(15, 44), distance: 18.1, distance_unit: 'km', vehicle: 'Car 1', billable: true, notes: null, status: 'draft', submitted_time_entry_id: null, expense_id: null, created_at: iso(9, 35), updated_at: iso(9, 35) },
  { id: 'td4', user_id: 'u4', ticket_id: '18357', started_at: iso(10, 0), ended_at: iso(10, 32), distance: 9.8, distance_unit: 'km', vehicle: 'Van 1', billable: true, notes: null, status: 'submitted', submitted_time_entry_id: 'te1', expense_id: 'ex1', created_at: iso(8), updated_at: iso(11) },
];

const USERS = [
  { id: 'u1', org_id: ORG, email: 'sam@marula.test', display_name: 'Sam K.', role: 'technician', is_active: true, created_at: iso(8) },
  { id: 'u2', org_id: ORG, email: 'alex@marula.test', display_name: 'Alex M.', role: 'technician', is_active: true, created_at: iso(8) },
  { id: 'u3', org_id: ORG, email: 'kagiso@marula.test', display_name: 'Kagiso B.', role: 'technician', is_active: true, created_at: iso(8) },
  { id: 'u4', org_id: ORG, email: 'tumi@marula.test', display_name: 'Tumi L.', role: 'admin', is_active: true, created_at: iso(8) },
];

const RECONCILIATION = {
  currency: 'GBP',
  mileage_rate_minor: 4500,
  mileage_rate_unit: 'km',
  reimburse_techs: true,
  rows: [
    { user_id: 'u1', display_name: 'Sam K.', draft_count: 1, submitted_count: 0, expensed_count: 0, distance: 12.4, distance_unit: 'km', minutes: 32, expense_minor: 0, reimbursement_minor: 5580, billable_minor: 0 },
    { user_id: 'u2', display_name: 'Alex M.', draft_count: 2, submitted_count: 0, expensed_count: 0, distance: 36.2, distance_unit: 'km', minutes: 81, expense_minor: 0, reimbursement_minor: 16290, billable_minor: 0 },
    { user_id: 'u4', display_name: 'Tumi L.', draft_count: 0, submitted_count: 1, expensed_count: 1, distance: 9.8, distance_unit: 'km', minutes: 32, expense_minor: 4410, reimbursement_minor: 4410, billable_minor: 4410 },
  ],
  totals: { user_id: '00000000-0000-0000-0000-000000000000', display_name: 'Total', draft_count: 3, submitted_count: 1, expensed_count: 1, distance: 58.4, distance_unit: 'km', minutes: 145, expense_minor: 4410, reimbursement_minor: 26280, billable_minor: 4410 },
};

const MEETING_TYPES = [
  { id: 'mt1', org_id: ORG, name: 'On-site visit', description: 'A booked visit at the client site.', duration_minutes: 60, buffer_minutes: 15, booking_window_days: 30, assignee_scope: 'group', assignee_user_id: null, assignee_group_id: 'sg1', auto_email_enabled: false, auto_email_subject: null, auto_email_body: null, is_active: true, created_at: iso(8), updated_at: iso(8) },
  { id: 'mt2', org_id: ORG, name: 'Remote support call', description: 'A screen-share support call.', duration_minutes: 30, buffer_minutes: 0, booking_window_days: 14, assignee_scope: 'user', assignee_user_id: ME.user_id, assignee_group_id: null, auto_email_enabled: true, auto_email_subject: 'Your support call is booked', auto_email_body: 'Your support call with Marula is booked.', is_active: true, created_at: iso(8), updated_at: iso(8) },
  { id: 'mt3', org_id: ORG, name: 'Discovery workshop', description: null, duration_minutes: 90, buffer_minutes: 15, booking_window_days: 60, assignee_scope: 'group', assignee_user_id: null, assignee_group_id: 'sg1', auto_email_enabled: false, auto_email_subject: null, auto_email_body: null, is_active: false, created_at: iso(8), updated_at: iso(8) },
];

const SCHEDULING_GROUPS = [
  { id: 'sg1', org_id: ORG, name: 'Service Desk', description: 'The availability pool front-line tickets draw from.', member_user_ids: [ME.user_id, 'u2'], created_at: iso(8), updated_at: iso(8) },
  { id: 'sg2', org_id: ORG, name: 'Field team', description: 'Technicians who travel.', member_user_ids: ['u1', 'u3'], created_at: iso(8), updated_at: iso(8) },
];

const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/settings/user': [{ key: 'appearance.density', value: 'comfortable', scope: 'user' }],
  '/api/v1/dispatch/capacity': CAPACITY,
  '/api/v1/dispatch/skills': SKILLS,
  '/api/v1/dispatch/travel-drafts': TRAVEL_DRAFTS,
  '/api/v1/dispatch/reconciliation': RECONCILIATION,
  '/api/v1/dispatch-connector/technicians': {
    technicians: [
      { user_id: ME.user_id, display_name: 'Tumi L.', skills: ['Network', 'Service desk'] },
      { user_id: 'u2', display_name: 'Alex M.', skills: ['Network', 'Cabling'] },
    ],
  },
  '/api/v1/dispatch-connector/schedule': { from: DAY, days: 7, appointments: [] },
  '/api/v1/meeting-types': MEETING_TYPES,
  '/api/v1/scheduling-groups': SCHEDULING_GROUPS,
  // The staff directory is a page, not a bare array.
  '/api/v1/org/users': { total: USERS.length, users: USERS },
  // A clean routing day: the strip is visibly absent by design (ADR 0010), so
  // the capture shows the board without it, exactly as the pack draws it.
  '/api/v1/dispatch/routing': {
    date: DAY,
    status: { provider_id: 'mapbox', quality: 'good', measured: true },
    technicians: [],
    suggestions: { available: false, reason: 'no_improvement', items: [] },
  },
};

/** Select the queue tickets the plan states work from: a remote P2, a remote
 *  P3, an on-site P3 and a remote P4, which is the mix that produces a plan
 *  with travel, an SLA note and no conflicts. */
const PLAN_TICKETS = [18441, 18437, 18433, 18435];

/** Open the Unassigned view with the plan tickets ticked. */
async function openQueueSelection(page) {
  await page.getByRole('button', { name: /^Unassigned/ }).click();
  for (const tk of PLAN_TICKETS) await page.getByLabel(`Select #${tk}`).check();
}

/** Propose a plan and wait for the panel. */
async function proposePlan(page) {
  await openQueueSelection(page);
  await page.getByRole('button', { name: 'Plan with AI' }).click();
  await page.getByRole('button', { name: 'Propose a plan' }).click();
  await page.getByTestId('plan-panel').waitFor();
}

/** Drag a queue card onto a technician lane at a wall-clock time, using real
 *  pointer events so the ghost, the chip and the drop evaluator all run. */
async function dragQueueCardOnto(page, tk, techId, hhmm) {
  const card = page.locator(`[data-q="${tk}"]`);
  await card.scrollIntoViewIfNeeded();
  const box = await card.boundingBox();
  if (!box) throw new Error(`no queue card for #${tk}`);
  const lane = page.locator(`[data-tech="${techId}"] .dp-lt`);
  const laneBox = await lane.boundingBox();
  if (!laneBox) throw new Error(`no lane ${techId}`);
  const [h, m] = hhmm.split(':').map(Number);
  const minutes = h * 60 + m;
  const x = laneBox.x + ((minutes - 420) / 660) * laneBox.width;
  const y = laneBox.y + laneBox.height / 2;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(x, y, { steps: 24 });
  await page.waitForTimeout(240);
}

const STATES = [
  { id: '01-board-rest', path: '/dispatch', ai: false },
  { id: '02-board-ai-granted', path: '/dispatch' },

  // A free drop: #18441 lands on Riya N. at 11:45, clear of everything.
  { id: '03-drag-in-flight', path: '/dispatch', run: (page) => dragQueueCardOnto(page, 18441, 'RN', '11:45') },

  // Blocked: Jordan T. is off from 14:00.
  { id: '04-drag-blocked', path: '/dispatch', run: (page) => dragQueueCardOnto(page, 18437, 'JT', '14:30') },
  { id: '05-drop-rejected', path: '/dispatch', run: (page) => dragQueueCardOnto(page, 18437, 'JT', '14:30') },

  {
    id: '06-keyboard-cursor',
    path: '/dispatch',
    async run(page) {
      await page.locator('[data-dscroll]').click({ position: { x: 6, y: 6 } });
      await page.keyboard.press('ArrowRight');
      await page.keyboard.press('ArrowRight');
      await page.keyboard.press('ArrowDown');
      await page.keyboard.press('Enter');
      await page.waitForTimeout(220);
    },
  },

  // The banner only means something on a day that really has a double booking
  // nobody chose; `?demo=collision` loads the pack's state-07 day.
  { id: '07-collisions-banner', path: '/dispatch?demo=collision' },

  { id: '08-density-compact', path: '/dispatch', run: (page) => page.getByRole('button', { name: 'compact' }).click() },
  { id: '09-density-dense', path: '/dispatch', run: (page) => page.getByRole('button', { name: 'dense' }).click() },
  { id: '10-week-view', path: '/dispatch', run: (page) => page.getByRole('button', { name: 'Week', exact: true }).click() },
  { id: '11-unassigned-queue', path: '/dispatch', run: openQueueSelection },

  {
    id: '12-plan-scope',
    path: '/dispatch',
    async run(page) {
      await openQueueSelection(page);
      await page.getByRole('button', { name: 'Plan with AI' }).click();
      await page.getByRole('dialog', { name: /choose the scope/ }).waitFor();
    },
  },

  { id: '13-plan-review', path: '/dispatch', run: proposePlan },

  // Nudge a proposed row 15 minutes later, so the −/time/+ controls are doing
  // something the eye can see.
  {
    id: '14-plan-resize',
    path: '/dispatch',
    async run(page) {
      await proposePlan(page);
      await page.getByRole('button', { name: '15 minutes later' }).first().click();
      await page.waitForTimeout(220);
    },
  },

  // Split the first proposed visit into two parts.
  {
    id: '15-plan-split',
    path: '/dispatch',
    async run(page) {
      await proposePlan(page);
      await page.locator('.dp-prow').first().click({ position: { x: 200, y: 10 } });
      await page.keyboard.press('s');
      await page.waitForTimeout(250);
    },
  },

  // Put a proposed row on top of real work, then keep the overlap on purpose.
  {
    id: '16-plan-overlap',
    path: '/dispatch',
    async run(page) {
      await proposePlan(page);
      await page.getByLabel('Technician for #18441').selectOption('TL');
      await page.waitForTimeout(250);
    },
  },

  // A plan that cannot be confirmed: the row is parked on an absence.
  {
    id: '17-plan-approval-failed',
    path: '/dispatch',
    async run(page) {
      await proposePlan(page);
      await page.getByLabel('Technician for #18441').selectOption('JT');
      await page.waitForTimeout(250);
    },
  },

  { id: '18-planning-loaded', path: '/dispatch/planning' },
  { id: '19-planning-loading', path: '/dispatch/planning', hold: 'loading' },

  { id: '20-travel-drafts', path: '/dispatch/travel' },
  { id: '21-travel-form', path: '/dispatch/travel', run: (page) => page.getByRole('button', { name: 'New travel draft' }).click() },
  { id: '22-travel-reconciliation', path: '/dispatch/travel', run: (page) => page.getByRole('button', { name: 'Reconciliation' }).click() },

  // Two remote jobs at once: an overlap the dispatcher can reason about.
  {
    id: '23-drag-overlap',
    path: '/dispatch',
    run: (page) => dragQueueCardOnto(page, 18437, 'TL', '11:00'),
  },
  {
    id: '24-overlap-reason',
    path: '/dispatch',
    async run(page) {
      await dragQueueCardOnto(page, 18437, 'TL', '11:00');
      await page.mouse.up();
      await page.getByTestId('overlap-card').waitFor();
      await page.waitForTimeout(220);
    },
  },
  {
    // Answer "Remote in parallel", so the lane stacks and the chip says why.
    id: '25-stacked-lanes',
    path: '/dispatch',
    async run(page) {
      await dragQueueCardOnto(page, 18437, 'TL', '11:00');
      await page.mouse.up();
      await page.getByTestId('overlap-card').waitFor();
      await page.keyboard.press('1');
      await page.waitForTimeout(260);
    },
  },
  {
    // The same approved overlap on a day that also has an unchosen one.
    id: '26-banner-and-stacks',
    path: '/dispatch?demo=collision',
    async run(page) {
      await dragQueueCardOnto(page, 18437, 'TL', '11:00');
      await page.mouse.up();
      await page.getByTestId('overlap-card').waitFor();
      await page.keyboard.press('1');
      await page.waitForTimeout(260);
    },
  },
  {
    id: '27-plan-overlap-approved',
    path: '/dispatch',
    async run(page) {
      await proposePlan(page);
      await page.getByLabel('Technician for #18441').selectOption('TL');
      await page.waitForTimeout(200);
      const approve = page.getByRole('button', { name: 'Remote in parallel' }).first();
      if (await approve.isVisible().catch(() => false)) await approve.click();
      await page.waitForTimeout(240);
    },
  },
  {
    id: '28-settings-overlap-policy',
    path: '/dispatch',
    async run(page) {
      await page.getByLabel('Dispatch settings').click();
      await page.getByTestId('dispatch-settings').waitFor();
      await page.waitForTimeout(220);
    },
  },
  {
    // #18433 is on-site at Fernhill; dropping it across #18351 (Lumen) is two
    // sites at once, which is the sharper overlap the pack draws.
    id: '29-overlap-different-sites',
    path: '/dispatch',
    run: (page) => dragQueueCardOnto(page, 18433, 'AM', '09:00'),
  },

  { id: '30-scheduling-templates', path: '/dispatch/scheduling' },
];

async function shot(browser, { viewport, theme, preset, id, path, run, hold, ai }) {
  // BOTH dimensions belong in the filename. The narrow-width passes reuse the
  // same state ids, and the two light themes share the theme name, so a name
  // carrying only (id, theme) means marula-warm silently replaces the default
  // marula light and the 1024 shot silently replaces the 1600 one. Lane 15
  // diffs these against the pack, and a missing file is a missing state.
  const tag = `${id}-${viewport.width}-${preset ?? 'marula'}-${theme}`;
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', id, theme, String(e).slice(0, 300)));
  const url = new URL(`${BASE}${path}`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');
  if (preset) url.searchParams.set('preset', preset);
  await page.clock.install({ time: FROZEN });
  await page.addInitScript(
    ({ t, p }) => {
      window.localStorage.setItem('marula-theme', t);
      // A pre-paint hint only. AppearanceSync re-resolves the preset from the
      // org setting after auth, so the settings/org fixture is what decides
      // which preset the shot really shows.
      window.localStorage.setItem('marula-theme-preset', p);
      window.localStorage.setItem('marula-density', 'comfortable');
      window.localStorage.setItem(
        'marula-tokens',
        JSON.stringify({ access_token: 'test-access', refresh_token: 'test-refresh', session_id: 'sess', expires_in: 900 }),
      );
      window.localStorage.setItem('marula-tokens-exp', String(Date.now() + 3_600_000));
      const style = document.createElement('style');
      style.textContent = 'nextjs-portal{display:none!important}';
      document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
    },
    { t: theme === 'dark' ? 'dark' : 'light', p: preset ?? 'marula' },
  );

  await page.route('**/api/**', async (r) => {
    const key = new URL(r.request().url()).pathname;
    if (key === '/api/v1/settings/org') {
      // The pack's state 01 is the board with NO grant: the toolbar's right
      // side holds only Filter. Every other state has it on.
      //
      // `appearance.theme` is what actually selects the preset. AppearanceSync
      // runs after auth, resolves user override then org theme, and calls
      // setThemePreset(null) when neither is set — which wipes whatever the
      // inline bootstrap put on data-preset. A capture that sets the preset in
      // localStorage alone photographs the default theme twice under two names.
      await r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          { key: 'ai.features_enabled', value: ai !== false, scope: 'org' },
          { key: 'appearance.theme', value: preset ?? 'marula', scope: 'org' },
        ]),
      });
      return;
    }
    if (hold === 'loading' && key === '/api/v1/dispatch/capacity') {
      await new Promise((res) => setTimeout(res, 20_000));
      return;
    }
    if (FIXTURES[key] !== undefined) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES[key]) });
    } else {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
    }
  });

  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForSelector('main', { timeout: 30_000 });
  await page.waitForTimeout(1200);
  if (run) await run(page);
  await page.waitForTimeout(450);
  await page.screenshot({ path: resolve(STATES_DIR, `${tag}.png`), fullPage: false });
  console.log('captured', tag);
  await context.close();
}

/** The pack's overview sheet: every state on one page, labelled, so a reviewer
 *  reads the whole surface in one image. Built as a page and screenshotted,
 *  which keeps the capture dependency-free. */
async function contactSheet(browser, theme) {
  // Only the reference width belongs on the sheet; the narrow passes are for
  // the responsive claim, not for the surface overview.
  const files = (await readdir(STATES_DIR))
    .filter((f) => f.endsWith(`-1600-marula-${theme}.png`) && /^\d\d-/.test(f))
    .sort();
  if (files.length === 0) {
    console.error('no captures for the', theme, 'sheet');
    return;
  }
  const cells = files
    .map((f) => `<figure><img src="${f}"><figcaption>${f.replace(`-1600-marula-${theme}.png`, '')}</figcaption></figure>`)
    .join('\n');
  const html = `<!doctype html><meta charset="utf-8">
<style>
  body { margin: 0; padding: 20px; background: #fff; font: 12px/1.4 ui-sans-serif, system-ui, sans-serif; color: #111; }
  .grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 22px 16px; }
  figure { margin: 0; }
  img { width: 100%; display: block; border: 1px solid #d8d4c8; border-radius: 6px; }
  figcaption { margin-top: 5px; font-weight: 600; letter-spacing: .01em; }
</style>
<div class="grid">${cells}</div>`;
  const context = await browser.newContext({ viewport: { width: 1560, height: 1200 } });
  const page = await context.newPage();
  await page.route('**/states/**', async (r) => {
    const name = decodeURIComponent(new URL(r.request().url()).pathname.split('/').pop() ?? '');
    const body = await readFile(join(STATES_DIR, name));
    await r.fulfill({ status: 200, contentType: 'image/png', body });
  });
  await page.setContent(html, { waitUntil: 'load' });
  await page.waitForTimeout(600);
  await page.screenshot({ path: resolve(OUT, `contact-sheet-${theme}.png`), fullPage: true });
  console.log('contact sheet', theme, files.length, 'states');
  await context.close();
}

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, args: ['--no-sandbox', '--disable-dev-shm-usage'] });

// The pack draws the neutral light theme and dark; the platform decision (D2)
// makes marula-warm the default on mobile, so it is captured too.
const THEMES = [
  { theme: 'light', preset: 'marula' },
  { theme: 'light', preset: 'marula-warm' },
  { theme: 'dark', preset: 'marula' },
];

for (const s of STATES) {
  if (ONLY && !ONLY.has(s.id)) continue;
  for (const t of THEMES) {
    try {
      await shot(browser, { viewport: { width: 1600, height: 994 }, ...t, id: s.id, path: s.path, run: s.run, hold: s.hold, ai: s.ai });
    } catch (e) {
      console.error('FAIL', s.id, t.theme, t.preset, e.message);
    }
  }
}

// The narrow widths are the responsive claim for the board's lane stack.
for (const vp of [{ width: 1280, height: 900 }, { width: 1024, height: 800 }]) {
  for (const [id, path] of [['01-board-rest', '/dispatch'], ['18-planning-loaded', '/dispatch/planning']]) {
    try {
      await shot(browser, { viewport: vp, theme: 'light', preset: 'marula', id, path });
    } catch (e) {
      console.error('FAIL narrow', vp.width, id, e.message);
    }
  }
}

// One sheet per theme, taken from the DEFAULT preset's files, the way the
// pack's own overview works. The warm capture stays on disk for comparison.
for (const t of THEMES) {
  await contactSheet(browser, t.theme).catch((e) => console.error('FAIL sheet', t.theme, e.message));
}

await browser.close();
console.log('done →', OUT);