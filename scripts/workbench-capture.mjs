#!/usr/bin/env node
/**
 * Workbench-v2 capture — Wave 15 lane 7 (p15-workbench-v2).
 *
 * Screenshots the new ticket workbench (context panel / center stream /
 * agent panel / composer / wrap-up sheet) on /tickets/{id} at
 * 1440/1024/768/480 in light AND dark, into design/qa-workbench-v2/.
 * API responses are fixtures served by Playwright routing — no backend
 * needed. Pattern: scripts/nav-rail-capture.mjs (lane 6 harness).
 *
 * Usage: start the web app (dev on :3100 or the standalone build on :3000),
 *        then node scripts/workbench-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const CHROME = '/Users/iordn/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const OUT = resolve('design/qa-workbench-v2');
const BASE = process.argv[2] ?? 'http://localhost:3000';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ORG = '00000000-0000-4000-8000-000000000002';
const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: ORG,
  email: 'owner@marula.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_boards', 'tickets:edit', 'tickets:read', 'financial:read', 'admin:org'],
};
const TICKET_ID = '00000000-0000-4000-8000-00000000a001';
const CLIENT_ID = '00000000-0000-4000-8000-00000000c001';
const BOARD_ID = '00000000-0000-4000-8000-00000000b001';
const min = (n) => new Date(Date.now() - n * 60000).toISOString();

const TICKET = {
  id: TICKET_ID,
  org_id: ORG,
  board_id: BOARD_ID,
  number: 'MER-18432',
  title: 'VPN disconnects every 20–30 minutes',
  body: 'Since this morning my VPN drops every 20 to 30 minutes. Error 800 in the client. Other apps stay online.',
  status_id: 'st-working',
  category: 'in_progress',
  priority: 'p1',
  client_id: CLIENT_ID,
  contact_id: '00000000-0000-4000-8000-00000000k001',
  assignee_id: ME.user_id,
  requester_id: null,
  site_id: null,
  due_at: null,
  resolved_at: null,
  created_at: min(210),
  updated_at: min(4),
  custom_fields: [],
};
const STATUSES = [
  { id: 'st-open', name: 'New', category: 'open', sort_order: 0, is_entry: true, color: null },
  { id: 'st-working', name: 'In Progress', category: 'in_progress', sort_order: 1, is_entry: false, color: null },
  { id: 'st-waiting', name: 'Waiting on customer', category: 'waiting', sort_order: 2, is_entry: false, color: null },
  { id: 'st-closed', name: 'Resolved', category: 'closed', sort_order: 3, is_entry: false, color: null },
];
const COMMENTS = [
  { id: 'cm-1', org_id: ORG, ticket_id: TICKET_ID, author_id: 'u-john', kind: 'client_visible', body: 'Hi, I keep getting disconnected from the VPN every 20–30 minutes. This started this morning. Any ideas what could be causing this?', created_at: min(180), updated_at: null, attachment_ids: [] },
  { id: 'cm-2', org_id: ORG, ticket_id: TICKET_ID, author_id: ME.user_id, kind: 'internal', body: '@Alex can you check if this is happening on other devices? Looks like a possible Always-On VPN profile issue based on the error.', created_at: min(160), updated_at: null, attachment_ids: [] },
  { id: 'cm-3', org_id: ORG, ticket_id: TICKET_ID, author_id: ME.user_id, kind: 'client_visible', body: "We've identified the issue and are working on a fix. This appears to be related to the Always-On VPN profile configuration. I'll keep you updated shortly.", created_at: min(96), updated_at: null, attachment_ids: [] },
];
const AUDIT = {
  events: [
    { id: 'ev-1', entity_type: 'ticket', entity_id: TICKET_ID, action: 'ticket.created', occurred_at: min(210), ip: null, detail: null },
    { id: 'ev-2', entity_type: 'ticket', entity_id: TICKET_ID, action: 'ticket.status_change', occurred_at: min(150), ip: null, detail: { from_name: 'New', to_name: 'In Progress' } },
    { id: 'ev-3', entity_type: 'ticket', entity_id: TICKET_ID, action: 'ticket.checklist_updated', occurred_at: min(40), ip: null, detail: null },
  ],
  next_cursor: null,
};
const TIME_ENTRIES = [
  { id: 'te-1', org_id: ORG, ticket_id: TICKET_ID, user_id: ME.user_id, started_at: min(120), ended_at: min(75), minutes: 45, billable: true, work_type: 'Remote Support', notes: 'Troubleshooting / investigation', created_at: min(74), approval_status: 'draft', approval_reason_code: null },
];
const EMAILS = {
  messages: [
    { id: 'em-1', org_id: ORG, ticket_id: TICKET_ID, direction: 'inbound', from_address: 'john.smith@acme-corp.com', to_addresses: ['support@marula.test'], subject: 'VPN drops every 20-30 min', body_text: 'Since this morning my VPN drops…', reply_to_message_id: null, delivery: null, scan_status: 'clean', created_at: min(185) },
  ],
};
const CLIENT = { id: CLIENT_ID, org_id: ORG, name: 'Acme Corporation', notes: null, restricted_to: [], created_at: min(90000), updated_at: min(500) };
const CONTACTS = [
  { id: '00000000-0000-4000-8000-00000000k001', org_id: ORG, client_id: CLIENT_ID, site_id: null, name: 'John Smith', email: 'john.smith@acme-corp.com', phone: '+27 11 555 0198', title: 'IT Manager', is_primary: true, created_at: min(90000) },
];
const ASSETS = [
  { id: 'as-1', org_id: ORG, client_id: CLIENT_ID, name: 'JOHN-LATITUDE-7440', kind: 'workstation', status: 'deployed', tag: 'LAP-0142', created_at: min(90000), updated_at: min(2), identity_keys: {}, fields: {}, curated_fields: {}, observed_fields: {} },
  { id: 'as-2', org_id: ORG, client_id: CLIENT_ID, name: 'ACME-EDGE-FIREWALL', kind: 'network', status: 'deployed', tag: 'FW-0007', created_at: min(90000), updated_at: min(400), identity_keys: {}, fields: {}, curated_fields: {}, observed_fields: {} },
  { id: 'as-3', org_id: ORG, client_id: CLIENT_ID, name: 'LAPTOP-ACME-017', kind: 'workstation', status: 'retired', tag: 'LAP-0097', created_at: min(90000), updated_at: min(4000), identity_keys: {}, fields: {}, curated_fields: {}, observed_fields: {} },
];
const CONTRACTS = [
  { id: 'ct-1', org_id: ORG, client_id: CLIENT_ID, contract_type: 'managed_services', status: 'active', amount_minor: 250000, currency: 'USD', billing_frequency: 'monthly', billing_anchor: '2026-01-01', effective_from: '2026-01-01', effective_to: null, created_at: min(90000), updated_at: min(500) },
];
const SLA = { applied: true, breached: false, minutes_remaining: 72, paused: false, state: { breached: false, org_id: ORG, policy_id: 'pol-1', ticket_id: TICKET_ID, first_responded_at: min(170), first_response_due: min(160), resolution_due: min(-72), paused_business_secs: 0, paused_since: null } };
const KB = [
  { id: 'kb-1123', org_id: ORG, category_id: null, title: 'VPN Troubleshooting Guide', slug: 'vpn-troubleshooting', body: '…', status: 'published', client_visible: true, created_at: min(90000), updated_at: min(12) },
  { id: 'kb-1047', org_id: ORG, category_id: null, title: 'Always-On VPN Runbook', slug: 'always-on-vpn-runbook', body: '…', status: 'published', client_visible: false, created_at: min(90000), updated_at: min(4000) },
  { id: 'kb-1198', org_id: ORG, category_id: null, title: 'VPN Profile Configuration (Windows 11)', slug: 'vpn-profile-config', body: '…', status: 'published', client_visible: true, created_at: min(90000), updated_at: min(300) },
];
const SEARCH = {
  kinds: ['ticket', 'comment', 'client', 'site', 'contact'],
  limit: 10, offset: 0, total: 1, query: 'vpn disconnects',
  results: [
    { entity_id: 't-17601', entity_kind: 'ticket', parent_id: null, score: 0.87, title: 'MER-17601 · VPN issue on sales laptops', snippet: 'Same error 800, resolved with a RasClient profile reset.' },
  ],
};
const PROPOSALS = {
  proposals: [
    { id: 'ap-1', org_id: ORG, agent_kind: 'triage', agent_ref: 'triage-v2', base_version: 4, before_value: null, proposed_value: {}, capability: 'triage', classification: 'suggest', created_at: min(10), decided_at: null, decided_by: null, decision_note: null, dial_mode: 'suggest', entity: 'ticket', entity_id: TICKET_ID, for_user: null, id: 'ap-1', op: 'update', rationale: 'Inspect RasClient events and verify the Always-On VPN profile settings — matches the resolution path of the similar ticket in search.', rule_set_hash: null, session_id: null, skill_slug: 'vpn-triage', skill_version: 2, status: 'proposed', updated_at: min(10) },
    { id: 'ap-0', org_id: ORG, agent_kind: 'tagger', agent_ref: 'tagger-v1', base_version: 2, before_value: null, proposed_value: {}, capability: null, classification: 'suggest', created_at: min(150), decided_at: min(140), decided_by: ME.user_id, decision_note: null, dial_mode: 'suggest', entity: 'ticket', entity_id: TICKET_ID, for_user: null, id: 'ap-0', op: 'update', rationale: 'Tagged vpn / windows-11 from the error signature.', rule_set_hash: null, session_id: null, skill_slug: 'auto-tagger', skill_version: 1, status: 'applied', updated_at: min(140) },
  ],
  next_cursor: null,
};
const ORG_USERS = {
  users: [
    { id: ME.user_id, display_name: 'Taylor Reed', org_id: ORG, email: ME.email, timezone: 'UTC', is_active: true },
    { id: 'u-john', display_name: 'John Smith', org_id: ORG, email: 'john.smith@acme-corp.com', timezone: 'UTC', is_active: true },
  ],
};
const WORKING_SET = [
  { id: 'sess-1', org_id: ORG, user_id: ME.user_id, ticket_id: TICKET_ID, started_at: min(24), stopped_at: null, segments: [{ started_at: min(24), ended_at: null }] },
];
const CHECKLIST = { blocking: [], enforcement_active: false, items: [
  { id: 'chk-1', org_id: ORG, ticket_id: TICKET_ID, template_item_id: null, text: 'Collect RasClient event log', checked: true, checked_at: min(60), checked_by: ME.user_id, sort_order: 0 },
  { id: 'chk-2', org_id: ORG, ticket_id: TICKET_ID, template_item_id: null, text: 'Verify Always-On VPN profile settings', checked: false, checked_at: null, checked_by: null, sort_order: 1 },
] };

const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/tickets/my-work': [],
  [`/api/v1/tickets/${TICKET_ID}`]: TICKET,
  [`/api/v1/tickets/${TICKET_ID}/comments`]: COMMENTS,
  [`/api/v1/tickets/${TICKET_ID}/checklist`]: CHECKLIST,
  [`/api/v1/tickets/${TICKET_ID}/sla`]: SLA,
  [`/api/v1/tickets/${TICKET_ID}/attachments`]: [],
  [`/api/v1/tickets/${TICKET_ID}/time-entries`]: TIME_ENTRIES,
  [`/api/v1/tickets/${TICKET_ID}/emails`]: EMAILS,
  [`/api/v1/boards/${BOARD_ID}/statuses`]: STATUSES,
  [`/api/v1/boards/${BOARD_ID}/transitions`]: { mode: 'floor', edges: [] },
  '/api/v1/boards': [{ id: BOARD_ID, org_id: ORG, name: 'Service Desk', is_default: true, require_checklist_close: false }],
  [`/api/v1/clients/${CLIENT_ID}`]: CLIENT,
  [`/api/v1/clients/${CLIENT_ID}/contacts`]: CONTACTS,
  [`/api/v1/clients/${CLIENT_ID}/assets`]: ASSETS,
  '/api/v1/contracts': CONTRACTS,
  '/api/v1/assets': ASSETS,
  '/api/v1/kb/articles': KB,
  '/api/v1/search': SEARCH,
  '/api/v1/agent-proposals': PROPOSALS,
  '/api/v1/org/users': ORG_USERS,
  '/api/v1/timers/working-set': WORKING_SET,
  '/api/v1/timers/active': WORKING_SET[0],
  '/api/v1/audit-events': AUDIT,
  '/api/v1/notifications/unread-count': { count: 2 },
  '/api/v1/notifications': { items: [], next_cursor: null },
  '/api/dev/status': { enabled: false },
};

async function capture(viewport, theme, route, name) {
  const browser = await chromium.launch({ executablePath: CHROME, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();

  const url = new URL(`${BASE}${route}`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');

  await page.addInitScript(() => {
    window.localStorage.setItem('marula-theme', window.location.search.includes('theme=dark') ? 'dark' : 'light');
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
  });

  await page.route('**/api/**', async (r) => {
    const key = new URL(r.request().url()).pathname;
    if (FIXTURES[key]) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES[key]) });
    } else {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ data: [] }) });
    }
  });

  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForTimeout(3500);

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
      await capture(vp, theme, `/tickets/${TICKET_ID}`, `workbench-${vp.width}-${theme}.png`);
    } catch (e) {
      console.error('FAIL', vp.width, theme, e.message);
    }
  }
}

// Wrap-up sheet open (light + dark, wide only) — the workbar-driven sheet.
for (const theme of ['light', 'dark']) {
  try {
    const browser = await chromium.launch({ executablePath: CHROME, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
    const context = await browser.newContext({ viewport: { width: 1440, height: 1024 } });
    const page = await context.newPage();
    const url = new URL(`${BASE}/tickets/${TICKET_ID}`);
    if (theme === 'dark') url.searchParams.set('theme', 'dark');
    await page.addInitScript(() => {
      window.localStorage.setItem('marula-theme', window.location.search.includes('theme=dark') ? 'dark' : 'light');
      window.localStorage.setItem('marula-tokens', JSON.stringify({ access_token: 'test-access', refresh_token: 'test-refresh', session_id: 'sess', expires_in: 900 }));
      window.localStorage.setItem('marula-tokens-exp', String(Date.now() + 60_000));
      const style = document.createElement('style');
      style.textContent = 'nextjs-portal{display:none!important}';
      document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
    });
    await page.route('**/api/**', async (r) => {
      const key = new URL(r.request().url()).pathname;
      if (FIXTURES[key]) await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES[key]) });
      else await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ data: [] }) });
    });
    await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
    await page.waitForTimeout(3500);
    await page.getByTestId('wb-close-ticket').click().catch(() => {});
    await page.waitForTimeout(600);
    const name = `workbench-wrapup-1440-${theme}.png`;
    await page.screenshot({ path: resolve(OUT, name), fullPage: false });
    console.log('captured', name);
    await browser.close();
  } catch (e) {
    console.error('FAIL wrapup', theme, e.message);
  }
}
console.log('done');
