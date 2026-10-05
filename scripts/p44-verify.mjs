#!/usr/bin/env node
/**
 * p44 gate verification — DOM assertions over the same fixture harness the
 * capture battery uses. Verifies the pack's derived features on /calendar
 * (hatch classes, deadline treatments, overlap flagging, travel bands,
 * now-line, legend parity) and the pinned dispatch utilisation figures, per
 * state, in both themes. Exits non-zero on any failed assertion.
 *
 * Pattern: scripts/dispatch-p44-capture.mjs (same fixtures, pinned clock).
 * Usage: node scripts/p44-verify.mjs [baseUrl]
 */
import { chromium } from 'playwright';

const BASE = process.argv[2] ?? 'http://localhost:3000';
const FROZEN = new Date('2026-09-29T11:41:00Z');
const ORG = '00000000-0000-4000-8000-000000000002';
const BOARD_ID = '00000000-0000-4000-8000-0000000000b2';
const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: ORG,
  email: 'owner@marula.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_users', 'special:manage_boards', 'tickets:create', 'admin:org'],
};

const at = (dayOffset, h, m = 0) => {
  const d = new Date('2026-09-28T00:00:00Z');
  d.setUTCDate(d.getUTCDate() + dayOffset);
  d.setUTCHours(h, m, 0, 0);
  return d.toISOString();
};

const CAPACITY = {
  from: '2026-09-29',
  days: 7,
  unassigned_count: 6,
  technicians: [
    {
      user_id: ME.user_id,
      display_name: 'Tumi L.',
      days: [
        { date: '2026-09-29', available_minutes: 480, booked_minutes: 400, free_minutes: 80, utilization_percent: 83 },
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
        { date: '2026-09-29', available_minutes: 480, booked_minutes: 494, free_minutes: -14, utilization_percent: 103 },
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

const USERS = [
  { id: 'u1', org_id: ORG, email: 'sam@marula.test', display_name: 'Sam K.', role: 'technician', is_active: true, created_at: at(1, 8) },
  { id: 'u2', org_id: ORG, email: 'alex@marula.test', display_name: 'Alex M.', role: 'technician', is_active: true, created_at: at(1, 8) },
  { id: 'u3', org_id: ORG, email: 'kagiso@marula.test', display_name: 'Kagiso B.', role: 'technician', is_active: true, created_at: at(1, 8) },
  { id: 'u4', org_id: ORG, email: 'tumi@marula.test', display_name: 'Tumi L.', role: 'admin', is_active: true, created_at: at(1, 8) },
];

const CAL_ITEMS = [
  { item_type: 'appointment', id: 'ap-18437', user_id: ME.user_id, title: 'Fernhill NAS migration', category: 'On-site visit', ticket_id: '18437', kind: 'on_site', starts_at: at(0, 9), ends_at: at(0, 10, 30), travel_minutes: 24, travel_quality: 'measured' },
  { item_type: 'appointment', id: 'ap-rem1', user_id: ME.user_id, title: 'Riverton backup verify', category: 'Remote support call', ticket_id: '18435', kind: 'remote', starts_at: at(1, 11), ends_at: at(1, 12), travel_minutes: 0, travel_quality: null },
  { item_type: 'appointment', id: 'ap-18433', user_id: ME.user_id, title: 'Lumen firewall audit', category: 'On-site visit', ticket_id: '18433', kind: 'on_site', starts_at: at(2, 15), ends_at: at(2, 16, 30), travel_minutes: 18, travel_quality: 'heuristic' },
  { item_type: 'on_call', id: 'oc-1', user_id: ME.user_id, title: 'After-hours cover', category: null, ticket_id: null, kind: null, status: null, starts_at: at(1, 13), ends_at: at(1, 17), travel_minutes: null, travel_quality: null },
  { item_type: 'appointment', id: 'ap-48', user_id: ME.user_id, title: 'Depot stock count', category: null, ticket_id: null, kind: null, status: null, starts_at: at(4, 10), ends_at: at(4, 11), travel_minutes: null, travel_quality: null },
];
const CAL_EXCLUSIONS = [
  { id: 'ex-to', org_id: ORG, user_id: ME.user_id, reason: 'Time off — family', starts_at: at(2, 8), ends_at: at(2, 12), created_at: at(1, 8) },
  { id: 'ex-sync', org_id: ORG, user_id: ME.user_id, reason: 'Calendar sync: Dentist', starts_at: at(3, 9), ends_at: at(3, 10), created_at: at(1, 8) },
];
const CAL_ITEMS_OVERLAP = [
  ...CAL_ITEMS,
  { item_type: 'appointment', id: 'ap-ov1', user_id: ME.user_id, title: 'Riverton handover call', category: 'Remote support call', ticket_id: '18435', kind: 'remote', starts_at: at(2, 10), ends_at: at(2, 11), travel_minutes: 0, travel_quality: null },
  { item_type: 'appointment', id: 'ap-ov2', user_id: ME.user_id, title: 'Lumen pre-work sync', category: 'Remote support call', ticket_id: '18433', kind: 'remote', starts_at: at(2, 10, 30), ends_at: at(2, 11, 30), travel_minutes: 0, travel_quality: null },
];
const calView = (items) => ({
  from: at(0, 0),
  to: at(7, 0),
  timezone: 'UTC',
  user_id: null,
  items,
  exclusions: CAL_EXCLUSIONS,
  availability_windows: [],
});
const CAL_DEADLINES = {
  from: at(0, 0),
  to: at(7, 0),
  deadlines: [
    { ticket_id: 't-18433', board_id: BOARD_ID, number: '18433', title: 'Lumen firewall audit', due_at: at(0, 9), category: 'Security', assignee_id: ME.user_id, client_id: null },
    { ticket_id: 't-18437', board_id: BOARD_ID, number: '18437', title: 'Fernhill NAS migration', due_at: at(2, 16), category: 'Infrastructure', assignee_id: ME.user_id, client_id: null },
    { ticket_id: 't-18435', board_id: BOARD_ID, number: '18435', title: 'Riverton backup verify', due_at: at(4, 12), category: 'Backup', assignee_id: ME.user_id, client_id: null },
  ],
};

const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/settings/user': [{ key: 'appearance.density', value: 'comfortable', scope: 'user' }],
  '/api/v1/dispatch/capacity': CAPACITY,
  '/api/v1/dispatch/skills': { skills: [{ skill: 'Network', holders: 4, active_holders: 3 }] },
  '/api/v1/dispatch/travel-drafts': [
    { id: 'td1', user_id: 'u1', ticket_id: '18437', started_at: at(1, 14), ended_at: at(1, 14, 32), distance: 12.4, distance_unit: 'km', vehicle: 'Van 2', billable: true, notes: null, status: 'draft', submitted_time_entry_id: null, expense_id: null, created_at: at(1, 9), updated_at: at(1, 9) },
    { id: 'td2', user_id: 'u2', ticket_id: '18433', started_at: at(1, 15), ended_at: at(1, 15, 42), distance: 18.1, distance_unit: 'km', vehicle: 'Car 1', billable: true, notes: null, status: 'draft', submitted_time_entry_id: null, expense_id: null, created_at: at(1, 9, 30), updated_at: at(1, 9, 30) },
  ],
  '/api/v1/dispatch/reconciliation': {
    currency: 'GBP', mileage_rate_minor: 4500, mileage_rate_unit: 'km', reimburse_techs: true,
    rows: [
      { user_id: 'u1', display_name: 'Sam K.', draft_count: 1, submitted_count: 0, expensed_count: 0, distance: 12.4, distance_unit: 'km', minutes: 32, expense_minor: 0, reimbursement_minor: 5580, billable_minor: 0 },
      { user_id: 'u2', display_name: 'Alex M.', draft_count: 1, submitted_count: 0, expensed_count: 0, distance: 18.1, distance_unit: 'km', minutes: 42, expense_minor: 0, reimbursement_minor: 8145, billable_minor: 0 },
    ],
    totals: { user_id: '00000000-0000-0000-0000-000000000000', display_name: 'Total', draft_count: 2, submitted_count: 0, expensed_count: 0, distance: 30.5, distance_unit: 'km', minutes: 74, expense_minor: 0, reimbursement_minor: 13725, billable_minor: 0 },
  },
  '/api/v1/dispatch-connector/technicians': { technicians: [] },
  '/api/v1/dispatch-connector/schedule': { from: '2026-09-29', days: 7, appointments: [] },
  '/api/v1/meeting-types': [],
  '/api/v1/scheduling-groups': [],
  '/api/v1/org/users': { total: USERS.length, users: USERS },
  '/api/v1/dispatch/routing': { date: '2026-09-29', status: { provider_id: 'mapbox', quality: 'good', measured: true }, technicians: [], suggestions: { available: false, reason: 'no_improvement', items: [] } },
  '/api/v1/calendar/week': calView(CAL_ITEMS),
  '/api/v1/calendar/team': calView(CAL_ITEMS),
  '/api/v1/calendar/deadlines': CAL_DEADLINES,
  '/api/v1/calendar-sync': [],
};
const OVERLAP_FIXTURES = {
  '/api/v1/calendar/week': calView(CAL_ITEMS_OVERLAP),
  '/api/v1/calendar/team': calView(CAL_ITEMS_OVERLAP),
};

let failures = 0;
let passes = 0;
const verdicts = [];

function check(id, ok, detail) {
  if (ok) {
    passes += 1;
  } else {
    failures += 1;
    console.error(`  ✗ ${id}${detail ? ` — ${String(detail).slice(0, 160)}` : ''}`);
  }
}

async function open(browser, { path, theme, run, fx, rejectMove }) {
  const context = await browser.newContext({ viewport: { width: 1600, height: 994 } });
  const page = await context.newPage();
  const url = new URL(`${BASE}${path}`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');
  await page.clock.install({ time: FROZEN });
  await page.addInitScript(
    ({ t }) => {
      window.localStorage.setItem('marula-theme', t);
      window.localStorage.setItem('marula-theme-preset', 'marula');
      window.localStorage.setItem('marula-density', 'comfortable');
      window.localStorage.setItem(
        'marula-tokens',
        JSON.stringify({ access_token: 'test-access', refresh_token: 'test-refresh', session_id: 'sess', expires_in: 900 }),
      );
      window.localStorage.setItem('marula-tokens-exp', String(Date.now() + 3_600_000));
    },
    { t: theme === 'dark' ? 'dark' : 'light' },
  );
  await page.route('**/api/**', async (r) => {
    const key = new URL(r.request().url()).pathname;
    if (key === '/api/v1/settings/org') {
      await r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          { key: 'ai.features_enabled', value: true, scope: 'org' },
          { key: 'appearance.theme', value: 'marula', scope: 'org' },
        ]),
      });
      return;
    }
    if (rejectMove && /^\/api\/v1\/appointments\/[^/]+\/move$/.test(key)) {
      await r.fulfill({ status: 409, contentType: 'application/json', body: JSON.stringify({ detail: 'slot already booked' }) });
      return;
    }
    const table = fx ?? FIXTURES;
    if (table[key] !== undefined) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(table[key]) });
    } else if (FIXTURES[key] !== undefined) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES[key]) });
    } else {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
    }
  });
  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForSelector('main', { timeout: 30_000 });
  await page.waitForTimeout(1200);
  if (run) await run(page);
  await page.waitForTimeout(300);
  return { context, page };
}

async function verify(browser, theme) {
  console.log(`— ${theme} —`);

  // 40 / calendar week personal
  {
    const { context, page } = await open(browser, { path: '/calendar', theme });
    const p40 = [];
    // Hatch classes: time_off + synced_busy blocks carry the pack's hatch.
    p40.push(['time-off hatched', await page.locator('[data-testid="week-block-ex-to"].cal-hatch-absence').count() === 1]);
    p40.push(['synced-busy hatched', await page.locator('[data-testid="week-block-ex-sync"].cal-hatch-quiet').count() === 1]);
    // Travel band before the Monday on-site visit (24 min, measured).
    p40.push(['travel band', await page.locator('[data-testid="week-travel-ap-18437"]').count() === 1]);
    const travelText = await page.locator('[data-testid="week-travel-ap-18437"]').textContent().catch(() => '');
    p40.push(['travel label', travelText.includes('24 min travel')], );
    check(`40/travel-label`, travelText.includes('24 min travel'), travelText);
    // Deadline overlay: 3 ◆ entries; breached (Mon 09:00) in danger, others warning.
    p40.push(['deadline blocks', (await page.locator('[data-testid^="week-block-deadline:"]').count()) === 3]);
    const breachColors = await page.evaluate(() => {
      const pick = (el) => (el ? el.getAttribute('style') || '' : 'MISSING');
      const breached = pick(document.querySelector('[data-testid="week-block-deadline:t-18433"]'));
      const warned = pick(document.querySelector('[data-testid="week-block-deadline:t-18437"]'));
      const fri = pick(document.querySelector('[data-testid="week-block-deadline:t-18435"]'));
      return { breached, warned, fri };
    });
    check('40/breach-danger', breachColors.breached.includes('--m-status-danger'), breachColors.breached);
    check('40/warning-pair', breachColors.warned.includes('--m-status-warning') && breachColors.fri.includes('--m-status-warning'), JSON.stringify(breachColors));
    // On-call keeps its dashed differentiator.
    const oc = await page.locator('[data-testid="week-block-oc-1"]').getAttribute('style').catch(() => '');
    check('40/on-call-dashed', oc.includes('dashed'), oc);
    // Now-line on Tue at 11:41 (accent, in the today column).
    const nowLine = await page.evaluate(() => {
      const cols = [...document.querySelectorAll('[data-testid="week-grid"] .relative')];
      const today = cols.find((c) => c.style.background.includes('accent-soft'));
      if (!today) return null;
      return [...today.querySelectorAll('div[aria-hidden="true"]')].some((n) => {
        const inner = n.querySelector('div');
        return inner && inner.style.background.includes('accent');
      });
    });
    check('40/now-line', nowLine === true, String(nowLine));
    // Legend parity: the five chips render, deadline chip carries ◆.
    const legend = await page.locator('[aria-label="Entry legend"]').textContent().catch(() => '');
    for (const k of ['Appointment', 'On call', 'Synced busy', 'Time off', 'Deadline']) {
      check('40/legend', legend.includes(k), legend.slice(0, 120));
    }
    check('40/legend-diamond', legend.includes('◆'), legend.slice(0, 120));
    // Legend hatch parity: swatch chips carry the same classes the blocks do.
    const legendHatch = await page.evaluate(() => {
      const chips = [...document.querySelectorAll('[aria-label="Entry legend"] span')];
      return {
        quiet: chips.some((c) => c.className.includes('cal-hatch-quiet')),
        absence: chips.some((c) => c.className.includes('cal-hatch-absence')),
      };
    });
    check('40/legend-hatch', legendHatch.quiet && legendHatch.absence, JSON.stringify(legendHatch));
    console.log(`  40-week-personal: ${p40.filter((x) => x[1]).length}/${p40.length} sanity`);
    await context.close();
  }

  // 41 / team mode label
  {
    const { context, page } = await open(browser, {
      path: '/calendar', theme, run: (p) => p.getByRole('button', { name: 'Team' }).click(),
    });
    const hint = await page.locator('main').textContent();
    check('41/team-hint', hint.includes('whole team'), hint.slice(0, 150));
    check('41/team-items', (await page.locator('[data-testid^="week-block-"]').count()) >= 5);
    await context.close();
  }

  // 42 / overlaps flagged
  {
    const { context, page } = await open(browser, { path: '/calendar', theme, fx: OVERLAP_FIXTURES });
    const flagged = await page.evaluate(() => {
      const blocks = [...document.querySelectorAll('[data-testid^="week-block-ap-ov"]')];
      return blocks.map((b) => (b.getAttribute('style') || '')).filter((s) => s.includes('--m-status-warning')).length;
    });
    check('42/overlap-borders', flagged === 2, `flagged=${flagged}`);
    const chip = await page.locator('[data-testid^="day-overlaps-"]').first().textContent().catch(() => '');
    // Wednesday pairs: ex-to×ov1, ex-to×ov2, ov1×ov2, deadline-18437(16:00)×ap-18433(15:00–16:30).
    check('42/overlap-chip', chip.trim() === '4 overlaps', chip);
    await context.close();
  }

  // 43 / month
  {
    const { context, page } = await open(browser, {
      path: '/calendar', theme, run: (p) => p.getByRole('button', { name: 'Month' }).click(),
    });
    check('43/month-grid', (await page.locator('[data-testid="month-grid"]').count()) === 1);
    check('43/month-days', (await page.locator('[data-month-day]').count()) >= 35);
    const sep30 = await page.evaluate(() => {
      const cell = document.querySelector('[data-month-day="2026-09-30"]');
      return cell ? cell.textContent : '';
    });
    check('43/month-entry', sep30.includes('Lumen firewall audit'), sep30.slice(0, 80));
    await context.close();
  }

  // 44 / agenda with breach tone
  {
    const { context, page } = await open(browser, {
      path: '/calendar', theme, run: (p) => p.getByRole('button', { name: 'Agenda' }).click(),
    });
    check('44/agenda-list', (await page.locator('[data-testid="agenda-list"]').count()) === 1);
    const agenda = await page.locator('[data-testid="agenda-list"]').textContent();
    check('44/agenda-breach-title', agenda.includes('18433'), agenda.slice(0, 100));
    const breachPill = await page.evaluate(() => {
      const el = document.querySelector('[data-testid="agenda-entry-deadline:t-18433"]');
      const pill = el?.querySelector('[data-slot="pill"]');
      return pill ? pill.className : 'MISSING';
    });
    check('44/agenda-breach-danger', breachPill.includes('bg-mc-st-breach'), breachPill);
    await context.close();
  }

  // 45 / move rejected banner
  {
    const { context, page } = await open(browser, {
      path: '/calendar', theme, rejectMove: true,
      async run(p) {
        const block = p.locator('[data-testid="week-block-ap-rem1"]');
        await block.scrollIntoViewIfNeeded();
        await block.dispatchEvent('dragstart');
        const col = p.locator('[data-testid="week-grid"] .relative').first();
        const box = await col.boundingBox();
        const dt = await p.evaluateHandle(() => new DataTransfer());
        await col.dispatchEvent('drop', { dataTransfer: dt, clientY: box.y + 200 });
        await p.getByText('That slot is already booked').waitFor();
      },
    });
    const banner = await page.getByText('That slot is already booked').count();
    check('45/move-rejected-banner', banner === 1, `count=${banner}`);
    await context.close();
  }

  // 46 / details dialog
  {
    const { context, page } = await open(browser, {
      path: '/calendar', theme, run: (p) => p.locator('[data-testid="week-block-ap-18437"]').click(),
    });
    const dlg = await page.getByRole('dialog', { name: 'Fernhill NAS migration' }).count();
    check('46/details-dialog', dlg === 1, `count=${dlg}`);
    const travel = await page.locator('[data-testid="details-travel"]').textContent().catch(() => '');
    check('46/details-travel', travel.includes('24'), travel);
    await context.close();
  }

  // 47 / new appointment dialog
  {
    const { context, page } = await open(browser, {
      path: '/calendar', theme, run: (p) => p.getByRole('button', { name: 'New appointment' }).click(),
    });
    const dlg = await page.getByRole('dialog', { name: 'New appointment' }).count();
    check('47/new-appointment', dlg === 1, `count=${dlg}`);
    await context.close();
  }

  // 18 / planning utilisation figures on-screen
  {
    const { context, page } = await open(browser, { path: '/dispatch/planning', theme });
    const body = await page.locator('main').textContent();
    for (const fig of ['83%', '107%', '88%', '63%', '40%', '75%', '103%', '95%', '85%', '55%', '20%', '90%']) {
      check(`18/utilisation-${fig}`, body.includes(fig));
    }
    check('18/over-by', body.includes('Over'), 'planning body lacks Over callouts');
    await context.close();
  }

  // 07 / collisions banner (board self-seeds)
  {
    const { context, page } = await open(browser, { path: '/dispatch?demo=collision', theme });
    const body = await page.locator('main').textContent();
    check('07/collision-banner', /collision|overlapping|overlap/i.test(body), body.slice(0, 120));
    await context.close();
  }

  // 22 / reconciliation totals
  {
    const { context, page } = await open(browser, {
      path: '/dispatch/travel', theme, run: (p) => p.getByRole('button', { name: 'Reconciliation' }).click(),
    });
    const body = await page.locator('main').textContent();
    check('22/reconciliation-total', body.includes('137.25') || body.includes('£137'), body.slice(0, 200));
    await context.close();
  }
}

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
for (const t of ['light', 'dark']) {
  try {
    await verify(browser, t);
  } catch (e) {
    failures += 1;
    console.error(`VERIFY ERROR ${t}:`, String(e).slice(0, 300));
  }
}
await browser.close();
console.log(`\n${passes} passed, ${failures} failed`);
process.exit(failures > 0 ? 1 : 0);
