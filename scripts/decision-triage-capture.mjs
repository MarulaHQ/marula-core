/**
 * Decision-triage capture — Wave 23 lane 3 (p23-decision-triage).
 *
 * Captures the ticket detail workbench with the inline AI suggestion
 * card (decision_workbench): live answers with act/pending/ignore
 * states, shadow badge, and the degraded empty posture. Fixture-routed
 * (no backend). Pattern: scripts/backend-gaps-capture.mjs.
 *
 * Usage: start the web app (dev on :3000), then node scripts/decision-triage-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-decision-triage');
const BASE = process.argv[2] ?? 'http://localhost:3000';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: '00000000-0000-4000-8000-000000000002',
  email: 'owner@marula.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_users', 'special:manage_boards', 'admin:org'],
};

const TICKET_ID = '00000000-0000-4000-8000-0000000000t9';
const BOARD_ID = '00000000-0000-4000-8000-0000000000b1';

const TICKET = {
  id: TICKET_ID,
  org_id: ME.org_id,
  board_id: BOARD_ID,
  status_id: '00000000-0000-4000-8000-0000000000s1',
  number: 'MER-1042',
  title: 'Mail relay rejecting inbound',
  body: 'Since 09:40 UTC the HQ relay rejects inbound on port 25 with 554.',
  priority: 'p1',
  client_id: '00000000-0000-4000-8000-0000000000c1',
  site_id: null,
  contact_id: null,
  assignee_id: ME.user_id,
  requester_id: null,
  category: 'open',
  due_at: null,
  resolved_at: null,
  created_at: new Date(Date.now() - 3 * 3600_000).toISOString(),
  updated_at: new Date(Date.now() - 600_000).toISOString(),
  merged_into_ticket_id: null,
  type_id: null,
  category_id: null,
};

const STATUSES = [{
  id: '00000000-0000-4000-8000-0000000000s1',
  board_id: BOARD_ID,
  org_id: ME.org_id,
  name: 'New',
  category: 'open',
  sort_order: 0,
  is_entry: true,
  color: null,
  wip_limit: null,
}];

const SUGGESTIONS = {
  ticket_id: TICKET_ID,
  suggestions: [
    {
      surface: 'decision_ticket_triage',
      mode: 'live',
      provider: 'typesafe',
      model: 'jev-1.13.0',
      recorded_at: new Date(Date.now() - 120_000).toISOString(),
      answers: [
        {
          question_id: 'actionable',
          answer: { kind: 'noul', probability: 0.94 },
          act: 0.8,
          review: 0.5,
          state: 'act',
        },
        {
          question_id: 'urgency',
          answer: { kind: 'score', level: 3, distribution: [0, 0, 0.12, 0.85, 0.03] },
          act: 4.0,
          review: 3.0,
          state: 'pending',
        },
        {
          question_id: 'frustrated',
          answer: { kind: 'noul', probability: 0.32 },
          act: 0.75,
          review: 0.5,
          state: 'ignore',
        },
        {
          question_id: 'board',
          answer: { kind: 'choice', picked: 0, picked_label: 'Service Desk', distribution: [0.88, 0.09, 0.03], confidence: 0.88 },
          act: 0.7,
          review: 0.45,
          state: 'act',
        },
      ],
    },
    {
      surface: 'decision_dispatch',
      mode: 'live',
      provider: 'typesafe',
      model: 'jev-1.13.0',
      recorded_at: new Date(Date.now() - 120_000).toISOString(),
      answers: [
        {
          question_id: 'escalation_urgency',
          answer: { kind: 'score', level: 4, distribution: [0, 0, 0, 0.06, 0.94] },
          act: 4.0,
          review: 3.0,
          state: 'act',
        },
      ],
    },
    {
      surface: 'decision_contract_scope',
      mode: 'live',
      provider: 'typesafe',
      model: 'jev-1.13.0',
      recorded_at: new Date(Date.now() - 120_000).toISOString(),
      answers: [
        {
          question_id: 'coverage',
          answer: { kind: 'choice', picked: 0, picked_label: 'Included-Managed-IT', distribution: [0.91, 0.06, 0.03], confidence: 0.91 },
          act: 0.8,
          review: 0.6,
          state: 'act',
        },
        {
          question_id: 'after_hours_surcharge',
          answer: { kind: 'noul', probability: 0.12 },
          act: 0.8,
          review: 0.6,
          state: 'ignore',
        },
      ],
    },
    {
      surface: 'decision_workbench_shadow_demo',
      mode: 'shadow',
      provider: 'typesafe',
      model: 'jev-1.13.0',
      recorded_at: new Date(Date.now() - 120_000).toISOString(),
      answers: [
        {
          question_id: 'severity',
          answer: { kind: 'score', level: 4, distribution: [0, 0, 0, 0.06, 0.94] },
          act: 4.0,
          review: 3.0,
          state: 'act',
        },
      ],
    },
  ],
};

const FIXTURES = {
  '/api/v1/me': { user_id: ME.user_id, org_id: ME.org_id },
  '/api/v1/tickets/00000000-0000-4000-8000-0000000000t9': TICKET,
  '/api/v1/tickets/00000000-0000-4000-8000-0000000000t9/sla': null,
  '/api/v1/tickets/00000000-0000-4000-8000-0000000000t9/comments': [],
  '/api/v1/tickets/00000000-0000-4000-8000-0000000000t9/checklist': null,
  '/api/v1/tickets/00000000-0000-4000-8000-0000000000t9/attachments': [],
  '/api/v1/tickets/00000000-0000-4000-8000-0000000000t9/checklist-items': [],
  '/api/v1/tickets/00000000-0000-4000-8000-0000000000t9/decision-suggestions': SUGGESTIONS,
  '/api/v1/boards/00000000-0000-4000-8000-0000000000b1/statuses': STATUSES,
  '/api/v1/boards/00000000-0000-4000-8000-0000000000b1/transitions': { mode: 'floor', edges: [] },
  '/api/v1/org/users': { users: [ME], total: 1 },
  '/api/v1/boards/nav': [{ id: BOARD_ID, name: 'Service Desk', description: null, is_default: false, group_id: null, group_name: null, color: null, sort_order: 0, ticket_count: 3 }],
};

async function shot(browser, viewport, theme, name, interactions) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', name, String(e).slice(0, 300)));
  const url = new URL(`${BASE}/tickets/${TICKET_ID}`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');
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
  }, theme === 'dark' ? 'dark' : 'light');

  await page.route('**/api/**', async (r) => {
    const request = new URL(r.request().url());
    const key = request.pathname;
    const fx = FIXTURES[key];
    await r.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(fx !== undefined ? fx : (key.endsWith('/presence') ? [] : [])),
    });
  });

  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForSelector('[data-testid="wb-decision-card"], main', { timeout: 20_000 });
  await page.waitForTimeout(1100);
  if (interactions) await interactions(page);
  await page.waitForTimeout(400);
  await page.screenshot({ path: resolve(OUT, name), fullPage: false });
  console.log('captured', name);
  await context.close();
}

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, args: ['--no-sandbox', '--disable-dev-shm-usage'] });

const viewports = [
  { tag: '1440', width: 1440, height: 1024 },
  { tag: '1024', width: 1024, height: 768 },
  { tag: '480', width: 480, height: 900 },
];

for (const theme of ['light', 'dark']) {
  for (const vp of viewports) {
    await shot(browser, vp, theme, `ticket-decision-${theme}-${vp.tag}.png`);
  }
}

// Degraded posture: no recorded answers ⇒ the card is silently absent.
const EMPTY = { ...SUGGESTIONS, suggestions: [] };
const oldFixtures = { ...FIXTURES };
Object.assign(FIXTURES, { '/api/v1/tickets/00000000-0000-4000-8000-0000000000t9/decision-suggestions': EMPTY });
await shot(browser, viewports[0], 'light', 'ticket-decision-off-1440.png');
Object.assign(FIXTURES, oldFixtures);

await browser.close();
console.log('done');
