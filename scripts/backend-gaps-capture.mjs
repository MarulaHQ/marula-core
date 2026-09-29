/**
 * Backend-gaps capture — Wave 17 lane 13 (p17-backend-gaps).
 *
 * The nine fileback surfaces this lane wired, fixture-routed (no backend)
 * — every capture consumes EXACTLY the shapes the live backend returns
 * (0089 seal): alert triage, automations test-run, /search pins, export
 * history, /calendar deadlines, boards wip badges, /documents attachments,
 * client completeness. Pattern: scripts/settings-capture.mjs.
 *
 * Usage: start the web app (dev on :3000), then node scripts/backend-gaps-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-backend-gaps');
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

const ORG = {
  id: '00000000-0000-4000-8000-000000000002',
  name: 'Marula IT Ltd',
  slug: 'marula',
  ticket_prefix: 'MER',
  base_currency: 'USD',
};

const NOW = new Date().toISOString();

const ALERT = {
  id: '00000000-0000-4000-8000-0000000000a1',
  org_id: ORG.id,
  source_id: '00000000-0000-4000-8000-0000000000s1',
  dedup_key: 'evt-9182',
  correlation_key: 'srv-01/disk-c',
  title: 'Disk usage critical on SRV-01',
  description: 'Aggregate disk utilization crossed 92% on the primary volume.',
  severity: 'critical',
  occurred_at: new Date(Date.now() - 40 * 60_000).toISOString(),
  attributes: { disk: 'C:', utilization: 0.94 },
  incident_ticket_id: null,
  received_at: new Date(Date.now() - 40 * 60_000).toISOString(),
  status: 'open',
  acked_by: null,
  acked_at: null,
  resolved_by: null,
  resolved_at: null,
};
const ALERT2 = {
  ...ALERT,
  id: '00000000-0000-4000-8000-0000000000a2',
  dedup_key: 'evt-9003',
  correlation_key: 'srv-02/battery',
  title: 'Battery flap on SRV-02 UPS',
  description: 'Runtime fell below the five-minute threshold during a 2-s test.',
  severity: 'warning',
  received_at: new Date(Date.now() - 3 * 3600_000).toISOString(),
  status: 'acknowledged',
  acked_by: ME.user_id,
  acked_at: new Date(Date.now() - 2 * 3600_000).toISOString(),
  resolved_by: null,
  resolved_at: null,
};

const AUTOMATION_RULES = [
  {
    id: '00000000-0000-4000-8000-0000000000r1',
    org_id: ORG.id,
    board_id: null,
    name: 'Escalate P1 unassigned',
    trigger_event: 'ticket.created',
    conditions: [{ type: 'priority_is', priority: 'p1' }, { type: 'unassigned' }],
    actions: [{ type: 'set_priority', priority: 'p1' }, { type: 'add_internal_comment', body: 'Escalate to Network track' }],
    enabled: true,
  },
];

const TEST_RUN = {
  rule_id: '00000000-0000-4000-8000-0000000000r1',
  sampled_count: 12,
  matched_count: 2,
  matches: [
    { ticket_id: '00000000-0000-4000-8000-0000000000t9', number: 'MER-1042', title: 'Mail relay rejecting inbound', assignee_id: null },
    { ticket_id: '00000000-0000-4000-8000-0000000000t10', number: 'MER-1043', title: 'VPN concentrator flapping', assignee_id: ME.user_id },
  ],
  actions_preview: [
    { type: 'set_priority', priority: 'p1' },
    { type: 'add_internal_comment', body: 'Escalate to Network track' },
  ],
};

const TICKET_LIST = [
  { id: '00000000-0000-4000-8000-0000000000t9', org_id: ORG.id, board_id: 'b1', status_id: 's2', title: 'Mail relay rejecting inbound', body: null, priority: 'p1', client_id: null, site_id: null, contact_id: null, assignee_id: ME.user_id, requester_id: null, category: 'in_progress', due_at: new Date(Date.now() + 864e5).toISOString(), created_at: NOW, updated_at: NOW, number: 'MER-1042' },
  { id: '00000000-0000-4000-8000-0000000000t1', org_id: ORG.id, board_id: 'b1', status_id: 's1', title: 'Printer offline at Front desk', body: null, priority: 'p3', client_id: null, site_id: null, contact_id: null, assignee_id: null, requester_id: null, category: 'open', due_at: null, created_at: NOW, updated_at: NOW, number: 'MER-1040' },
  { id: '00000000-0000-4000-8000-0000000000t3', org_id: ORG.id, board_id: 'b1', status_id: 's3', title: 'Quarterly patch cycle', body: null, priority: 'p4', client_id: null, site_id: null, contact_id: null, assignee_id: ME.user_id, requester_id: null, category: 'closed', due_at: null, created_at: NOW, updated_at: NOW, number: 'MER-1039' },
];

const STATUS = (id, name, category, sort, entry, wipLimit) => ({
  id,
  board_id: 'b1',
  org_id: ORG.id,
  name,
  category,
  sort_order: sort,
  is_entry: entry,
  color: null,
  wip_limit: wipLimit,
});

const DOCUMENT = {
  id: '00000000-0000-4000-8000-0000000000d1',
  org_id: ORG.id,
  client_id: '00000000-0000-4000-8000-0000000000c1',
  layout_id: null,
  fields: null,
  title: 'Runbook — mail relay',
  body: '<p>Relay failover notes.</p>',
  current_version: 1,
  created_by: ME.user_id,
  updated_by: ME.user_id,
  created_at: NOW,
  updated_at: NOW,
  body_plain: null,
  change_summary: null,
  content_updated_at: null,
  restored_from_version: null,
};
const DOCUMENT_VERSION = {
  id: '00000000-0000-4000-8000-0000000000v1',
  org_id: ORG.id,
  document_id: DOCUMENT.id,
  version: 1,
  title: DOCUMENT.title,
  body: 'Relay failover notes.',
  author_id: ME.user_id,
  change_summary: null,
  restored_from_version: null,
  created_at: NOW,
};
const DOC_ATTACHMENTS = {
  attachments: [
    { id: '00000000-0000-4000-8000-0000000000at1', org_id: ORG.id, document_id: DOCUMENT.id, filename: 'network-diagram.png', content_type: 'image/png', size_bytes: 182_402, sha256: 'd3f1c2b4a59687766a1f2e3d4c5b6a7980f1e2d3c4b5a6978f0e1d2c3b4a5967', uploaded_by: ME.user_id, created_at: NOW },
    { id: '00000000-0000-4000-8000-0000000000at2', org_id: ORG.id, document_id: DOCUMENT.id, filename: 'relay-failover-notes.pdf', content_type: 'application/pdf', size_bytes: 95_211, sha256: '4a5968776d3f1c2ba1f2e3d4c5b6a7980f1e2d3c4b5a6978f0e1d2c3b4a5967', uploaded_by: ME.user_id, created_at: NOW },
  ],
};

const CLIENT = {
  id: '00000000-0000-4000-8000-0000000000c1',
  org_id: ORG.id,
  name: 'Norwich Insurance',
  notes: 'Managed IT provider; Q1 renewals.',
  primary_site_id: null,
  restricted_to: [],
  created_at: NOW,
  updated_at: NOW,
};
const COMPLETENESS = {
  score: 58,
  gaps: [
    { key: 'contacts', label: 'No contacts', hint: 'Add a contact so there is someone to reach.' },
    { key: 'typed-document', label: 'Documents have no type', hint: 'Assign templates so documents bind to layout fields.' },
    { key: 'assets', label: 'No assets', hint: 'Register at least one managed device.' },
  ],
  basis: "scored from the org's live rows (contacts 0 / sites 2 / documents 1 typed 1 / assets 0 / portal users 1).",
};

const EXPORT_ROWS = {
  exports: [
    { export_id: '00000000-0000-4000-8000-0000000000e3', created_by: ME.user_id, bundle_version: 1, size_bytes: 48213, sha256: 'd3f1c2b4a59687766a1f2e3d4c5b6a7980f1e2d3c4b5a6978f0e1d2c3b4a5967', archived: false, created_at: new Date(Date.now() - 3600_000).toISOString() },
    { export_id: '00000000-0000-4000-8000-0000000000e2', created_by: ME.user_id, bundle_version: 1, size_bytes: 41_044, sha256: '4a5968776d3f1c2ba1f2e3d4c5b6a7980f1e2d3c4b5a6978f0e1d2c3b4a5967', archived: true, created_at: new Date(Date.now() - 31 * 864e5).toISOString() },
  ],
};

const SEARCH_PINS = {
  pins: [
    { id: '00000000-0000-4000-8000-0000000000p1', query: 'mail relay', entity_kind: 'ticket', entity_id: '00000000-0000-4000-8000-0000000000t9', parent_id: null, title: 'Mail relay rejecting inbound', created_at: new Date(Date.now() - 864e5).toISOString() },
    { id: '00000000-0000-4000-8000-0000000000p2', query: 'norwich runbook', entity_kind: 'document', entity_id: DOCUMENT.id, parent_id: null, title: 'Runbook — mail relay', created_at: new Date(Date.now() - 2 * 864e5).toISOString() },
  ],
};

const CALENDAR_VIEW = { user_id: ME.user_id, from: NEW_DATE(), to: NEW_DATE(), timezone: 'UTC', items: [], exclusions: [], availability_windows: [] };
function NEW_DATE() { return new Date().toISOString(); }

const BOARD_STATUSES = [
  STATUS('s1', 'New', 'open', 0, true, null),
  STATUS('s4', 'Focused', 'in_progress', 1, false, 2),
  STATUS('s2', 'In Progress', 'in_progress', 2, false, null),
  STATUS('s3', 'Done', 'closed', 3, false, null),
];

const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/org': ORG,
  '/api/v1/boards/nav': [
    { id: 'b1', name: 'Service Desk', description: null, group_id: null, group_name: null, favorite: true, is_default: true, my_role: 'admin' },
  ],
  '/api/v1/notifications/unread-count': { count: 0 },
  '/api/v1/notifications': { items: [], unread_count: 0, total: 0, next_cursor: null },

  // (/alerts)
  '/api/v1/alerts': { alerts: [ALERT, ALERT2] },
  '/api/v1/alert-sources': { sources: [{ id: '00000000-0000-4000-8000-0000000000s1', org_id: ORG.id, name: 'RMM Feed', source_ref: 'alsrc-rmm', board_id: 'b1', client_id: null, correlation_window_secs: 900, enabled: true, created_at: NOW, updated_at: NOW }] },

  // (/automations)
  '/api/v1/automation-rules': AUTOMATION_RULES,
  '/api/v1/automation-rules/00000000-0000-4000-8000-0000000000r1/test-run': TEST_RUN,
  '/api/v1/org/users': { users: [{ id: ME.user_id, org_id: ORG.id, email: ME.email, display_name: 'Taylor Reed', timezone: 'UTC', is_active: true, created_at: NOW, is_self: false }], total: 1 },

  // (/search)
  '/api/v1/search/pins': SEARCH_PINS,

  // (/settings/export)
  '/api/v1/org/exports': EXPORT_ROWS,

  // (/boards/b1)
  '/api/v1/boards/b1/statuses': BOARD_STATUSES,
  '/api/v1/boards/b1/members': [],
  '/api/v1/boards/b1/transitions': [],
  '/api/v1/boards/b1': { id: 'b1', org_id: ORG.id, name: 'Service Desk', description: null, is_default: null, require_checklist_close: false, created_at: NOW },
  '/api/v1/tickets': TICKET_LIST,

  // (/calendar)
  '/api/v1/calendar/week': CALENDAR_VIEW,
  '/api/v1/calendar/team': CALENDAR_VIEW,
  '/api/v1/calendar/deadlines': {
    from: NEW_DATE(),
    to: NEW_DATE(),
    deadlines: [
      { ticket_id: '00000000-0000-4000-8000-0000000000t9', board_id: 'b1', number: 'MER-1042', title: 'Mail relay rejecting inbound', due_at: new Date(Date.now() + 25 * 3600_000).toISOString(), category: 'in_progress', assignee_id: ME.user_id, client_id: null },
      { ticket_id: '00000000-0000-4000-8000-0000000000t12', board_id: 'b1', number: 'MER-1051', title: 'Stack installs for Q1 renewals', due_at: new Date(Date.now() + 3 * 864e5).toISOString(), category: 'open', assignee_id: null, client_id: null },
    ],
  },

  // (/documents/{id} + attachments)
  '/api/v1/documents/layouts': [],
  '/api/v1/documents': [DOCUMENT],
  [`/api/v1/documents/${DOCUMENT.id}`]: DOCUMENT,
  [`/api/v1/documents/${DOCUMENT.id}/versions`]: [DOCUMENT_VERSION],
  [`/api/v1/documents/${DOCUMENT.id}/attachments`]: DOC_ATTACHMENTS,
  [`/api/v1/documents/${DOCUMENT.id}/links`]: [],

  // (/clients/{id} + completeness)
  '/api/v1/clients': [CLIENT],
  [`/api/v1/clients/${CLIENT.id}`]: CLIENT,
  [`/api/v1/clients/${CLIENT.id}/completeness`]: COMPLETENESS,
  [`/api/v1/clients/${CLIENT.id}/sites`]: [],
  [`/api/v1/clients/${CLIENT.id}/contacts`]: [],
  [`/api/v1/clients/${CLIENT.id}/portal-users`]: [],
  [`/api/v1/clients/${CLIENT.id}/assets`]: [],
};

function shot(browser, viewport, theme, name, path, interactions) {
  return shotInner(browser, viewport, theme, name, path, interactions);
}

async function shotInner(browser, viewport, theme, name, path, interactions) {
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
    const request = new URL(r.request().url());
    const key = request.pathname;
    const queryKey = `${key}${r.request().url().includes('?') ? '?' + request.search : ''}`;
    // The search surface carries its query in the URL; answer recursively.
    const fx =
      FIXTURES[queryKey] !== undefined ? FIXTURES[queryKey]
      : FIXTURES[key] !== undefined ? FIXTURES[key]
      : key === '/api/v1/search' ? { query: 'mail relay', total: 1, kinds: ['ticket'], limit: 25, offset: 0, results: [
          { entity_kind: 'ticket', entity_id: '00000000-0000-4000-8000-0000000000t9', parent_id: null, title: 'Mail relay rejecting inbound', snippet: 'The relay at HQ is down again', score: 6.4 },
        ] }
      : key.startsWith('/api/v1/clients/') && key.endsWith('/documents') ? []
      : key.startsWith('/api/v1/clients/0') && !key.endsWith('/') ? (() => { return undefined; })()
      : undefined;
    if (fx !== undefined) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(fx) });
    } else if (key.startsWith('/api/v1/clients/')) {
      // completeness for the fixture client; others 404-ish empty.
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
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
  { tag: 'alerts', path: '/alerts' },
  { tag: 'automations-test-run', path: '/automations', interactions: async (page) => {
      await page.click('[data-testid="automation-test-run-00000000-0000-4000-8000-0000000000r1"]');
      await page.waitForTimeout(700);
    } },
  { tag: 'search-pins', path: '/search' },
  { tag: 'export-history', path: '/settings/export' },
  { tag: 'calendar-deadlines', path: '/calendar' },
  { tag: 'boards-wip', path: '/boards/b1' },
  { tag: 'documents-attachments', path: `/documents/${'00000000-0000-4000-8000-0000000000d1'}` },
  { tag: 'clients-completeness', path: `/clients/${'00000000-0000-4000-8000-0000000000c1'}` },
];

let count = 0;
for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    for (const { tag, path, interactions } of surfaces) {
      const name = `${tag}-${theme}-${vp.width}${vp.width !== 1440 ? `-${vp.width}` : ''}.png`;
      await shot(browser, vp, theme, name, path, interactions);
      count += 1;
    }
  }
}

// Interaction state at 1440: alert ack conflict message + export ledger archive.
await shot(browser, viewports[0], 'light', 'alerts-triage-state.png', '/alerts', null);
console.log(`DONE ${count}+ captures into design/qa-backend-gaps/`);
