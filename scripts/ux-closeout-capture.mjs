/**
 * ux-closeout-capture — Wave 17 lane 10 (p17-ux-sweep-closeout).
 *
 * The wave-closing FULL QA matrix: fixture-routed screenshots (no backend)
 * of every app AREA after lanes 1–13 — staff dashboards, ticket workspace,
 * boards, calendar, projects, clients, documents, assets, kb, alerts,
 * automations, audit, provenance, inventory, search, the Money surfaces,
 * every /settings section (incl. plugins, skills, agent-platform, AI,
 * portal branding, export), the workbar-capable ticket surface, the auth
 * covers, and /portal/login. Light AND dark × 1440/1024/768/480 into
 * design/qa-ux-sweep-closeout/. Pattern: scripts/backend-gaps-capture.mjs
 * (Wave 17 lane 13), fixtures merged from the lane-13/money/settings rigs.
 *
 * Usage: start the web app (dev), then node scripts/ux-closeout-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-ux-sweep-closeout');
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

const ORG = {
  id: '00000000-0000-4000-8000-000000000002',
  name: 'Meridian IT Ltd',
  slug: 'meridian',
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

const BACKEND_FIXTURES = {
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
const MORIG = '00000000-0000-4000-8000-000000000002';
const MME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: MORIG,
  email: 'owner@meridian.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:approve_time', 'financial:read', 'special:manage_users', 'admin:org', 'tickets:create'],
};
const CLIENT_ID = 'c0000000-0000-4000-8000-000000000001';
const CLIENT2_ID = 'c0000000-0000-4000-8000-000000000002';

const CLIENTS = [
  { id: CLIENT_ID, name: 'Acme Corp', display_name: 'Acme Corp', notes: null, created_at: '2026-01-05T00:00:00Z' },
  { id: CLIENT2_ID, name: 'Northwind Traders', display_name: null, notes: null, created_at: '2026-02-14T00:00:00Z' },
];

const now = Date.now();
const plusDate = (days) => new Date(now + days * 86_400_000).toISOString().slice(0, 10);

const INVOICE = (id, number, status, dunning, paid, total, days, extra = {}) => ({
  id,
  org_id: MORIG,
  number,
  client_id: CLIENT_ID,
  contract_id: 'ct1',
  currency: 'USD',
  status,
  issued_at: status === 'draft' ? null : plusDate(-20),
  due_on: plusDate(days),
  total_minor: total,
  total_paid_minor: paid,
  dunning_state: dunning,
  dunning_note: dunning !== 'none' ? 'Automated reminder — terms net 30.' : null,
  dunning_updated_at: dunning !== 'none' ? new Date(now - 3 * 86_400_000).toISOString() : null,
  period_from: '2026-08-01',
  period_to: '2026-08-31',
  run_id: 'r1',
  notes: null,
  created_at: new Date().toISOString(),
  created_by: null,
  updated_at: new Date().toISOString(),
  ...extra,
});

const INVOICES = [
  INVOICE('i1', 'INV-000101', 'issued', 'overdue', 5000, 42000, -1),
  INVOICE('i2', 'INV-000102', 'issued', 'none', 0, 18000, 12),
  INVOICE('i3', 'INV-000103', 'paid', 'none', 28500, 28500, -5),
  INVOICE('i4', 'INV-000104', 'draft', 'none', 0, 96000, 30),
  INVOICE('i5', 'INV-000105', 'void', 'none', 0, 1200, 4),
];

const INVOICE_DETAIL = {
  ...INVOICE('i1', 'INV-000101', 'issued', 'overdue', 5000, 42000, -1),
  lines: [
    {
      id: 'l1', invoice_id: 'i1', org_id: MORIG, description: 'Managed services — August retainer', kind: 'retainer',
      period_start: '2026-08-01', period_end: '2026-08-31', quantity_minutes: 0, amount_minor: 32000,
      created_at: '2026-09-01T00:00:00Z', updated_at: '2026-09-01T00:00:00Z', created_by: null,
    },
    {
      id: 'l2', invoice_id: 'i1', org_id: MORIG, description: 'On-site cabling work (2 × 5 h)', kind: 'time',
      period_start: '2026-08-14', period_end: '2026-08-21', quantity_minutes: 600, amount_minor: 10000,
      created_at: '2026-09-01T00:00:00Z', updated_at: '2026-09-01T00:00:00Z', created_by: null,
    },
  ],
  payments: [
    {
      id: 'p1', org_id: MORIG, invoice_id: 'i1', amount_minor: 5000, currency: 'USD', provider: 'none',
      provider_ref: null, received_at: new Date(now - 6 * 86_400_000).toISOString(), created_by: null,
      notes: 'Partial — ACH so far',
    },
  ],
};

const CONTRACT = (id, number, name, type, status, extra = {}) => ({
  id,
  org_id: MORIG,
  client_id: CLIENT_ID,
  number,
  name,
  contract_type: type,
  status,
  currency: 'USD',
  effective_from: '2026-01-01',
  effective_until: type === 'fixed_fee' ? plusDate(21) : null,
  amount_minor: type === 'recurring' ? 3200000 : type === 'retainer' ? 250000 : null,
  billing_frequency: type === 'recurring' ? 'monthly' : type === 'retainer' ? 'monthly' : null,
  billing_anchor: type === 'recurring' ? '2026-01-01' : null,
  hourly_rate_minor: type === 'time_and_materials' ? 18000 : null,
  included_minutes: type === 'retainer' ? 480 : null,
  overage_rate_minor: type === 'retainer' ? 21000 : null,
  notes: null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  ...extra,
});

const CONTRACTS = [
  CONTRACT('ct1', 'C-2026-001', 'Managed services 2026', 'recurring', 'active'),
  CONTRACT('ct2', 'C-2026-002', 'Server room refresh', 'fixed_fee', 'active'),
  CONTRACT('ct3', 'C-2026-003', 'Helpdesk retainer', 'retainer', 'active'),
  CONTRACT('ct4', 'C-2025-010', 'Legacy onboarding', 'time_and_materials', 'draft'),
  CONTRACT('ct5', 'C-2024-002', 'Legacy firewall TAM', 'time_and_materials', 'ended'),
];

const CONTRACT_DETAIL = {
  ...CONTRACT('ct1', 'C-2026-001', 'Managed services 2026', 'recurring', 'active'),
  rates: [],
  fees: [],
};

const CATALOG = [
  { id: 'cat1', org_id: MORIG, name: 'Block hours (managed)', unit: 'hour', currency: 'USD', unit_price_minor: 15000, cost_minor: 9500, taxable: true, active: true, category: 'Services', group_name: 'Managed IT', sku: 'SRV-HRS', description: 'Prepaid managed services hours', created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z', created_by: null },
  { id: 'cat2', org_id: MORIG, name: 'Firewall (hardware)', unit: 'each', currency: 'USD', unit_price_minor: 96000, cost_minor: 72000, taxable: true, active: true, category: 'Hardware', group_name: 'Network', sku: 'HW-FW-01', description: null, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z', created_by: null },
  { id: 'cat3', org_id: MORIG, name: 'License seat (M365)', unit: 'seat/month', currency: 'USD', unit_price_minor: 2500, cost_minor: null, taxable: false, active: true, category: 'Licensing', group_name: null, sku: 'LIC-SEAT', description: null, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z', created_by: null },
  { id: 'cat4', org_id: MORIG, name: 'Legacy drive imaging', unit: 'each', currency: 'USD', unit_price_minor: 8900, cost_minor: 4000, taxable: true, active: false, category: 'Services', group_name: null, sku: null, description: null, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z', created_by: null },
];

const STAGES = [
  { id: 'st1', org_id: MORIG, name: 'Qualifying', stage_type: 'open', position: 0, active: true, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z' },
  { id: 'st2', org_id: MORIG, name: 'Proposal', stage_type: 'open', position: 1, active: true, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z' },
  { id: 'st3', org_id: MORIG, name: 'Negotiation', stage_type: 'open', position: 2, active: true, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z' },
  { id: 'st4', org_id: MORIG, name: 'Won', stage_type: 'won', position: 10, active: true, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z' },
  { id: 'st5', org_id: MORIG, name: 'Lost', stage_type: 'lost', position: 20, active: true, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z' },
];

const DEAL = (id, name, stage_id, amount, currency = 'USD', extra = {}) => ({
  id,
  org_id: MORIG,
  client_id: CLIENT_ID,
  stage_id,
  name,
  amount_minor: amount,
  currency,
  status: 'open',
  created_at: '2026-08-01T00:00:00Z',
  updated_at: '2026-08-01T00:00:00Z',
  closed_at: null,
  created_by: null,
  owner_user_id: null,
  quote_id: null,
  expected_close_on: null,
  notes: null,
  ...extra,
});

const DEALS = [
  DEAL('d1', 'Office refresh — HQ', 'st1', 4800000),
  DEAL('d2', 'Meridian pilot (EUR)', 'st1', 2100000, 'EUR'),
  DEAL('d3', 'Firewall replacement fleet', 'st2', 9600000),
  DEAL('d4', 'Security retainer upsell', 'st3', 1400000),
  DEAL('d5', 'Managed backup add-on', 'st4', 1482000, 'USD', { status: 'won', closed_at: '2026-08-20T00:00:00Z' }),
  DEAL('d6', 'Disconnect project', 'st5', 500000, 'USD', { status: 'lost', closed_at: '2026-08-11T00:00:00Z' }),
];

const QUOTE = (id, number, status, extra = {}) => ({
  id,
  org_id: MORIG,
  number,
  client_id: CLIENT_ID,
  status,
  currency: 'USD',
  issued_on: plusDate(-5),
  valid_until: plusDate(25),
  subtotal_minor: 46500,
  tax_rate_bps: 800,
  tax_minor: 3720,
  total_minor: 50220,
  created_at: new Date().toISOString(),
  updated_at: new Date().toISOString(),
  sent_at: null,
  decided_at: null,
  converted_at: null,
  converted_invoice_id: null,
  converted_contract_id: null,
  notes: null,
  terms: null,
  created_by: null,
  ...extra,
});

const QUOTES = [
  QUOTE('q1', 'Q-000101', 'draft'),
  QUOTE('q2', 'Q-000102', 'sent'),
  QUOTE('q3', 'Q-000103', 'accepted'),
  QUOTE('q4', 'Q-000104', 'accepted', { converted_invoice_id: 'i2', converted_at: '2026-09-05T00:00:00Z' }),
  QUOTE('q5', 'Q-000105', 'declined', { valid_until: plusDate(-3), decided_at: '2026-09-10T00:00:00Z' }),
];

const CREDIT_NOTE = (id, number, extra = {}) => ({
  id,
  org_id: MORIG,
  number,
  client_id: CLIENT_ID,
  issued_against_invoice_id: 'i1',
  status: 'issued',
  currency: 'USD',
  total_minor: 32000,
  applied_minor: 12000,
  refunded_minor: 0,
  reason: 'Goodwill credit — missed SLA in August',
  created_at: new Date().toISOString(),
  updated_at: new Date().toISOString(),
  created_by: null,
  ...extra,
});

const CREDIT_NOTES = [
  CREDIT_NOTE('cn1', 'CN-000101'),
  CREDIT_NOTE('cn2', 'CN-000102', { issued_against_invoice_id: null, total_minor: 15000, applied_minor: 0, reason: null }),
  CREDIT_NOTE('cn3', 'CN-000103', { client_id: CLIENT2_ID, status: 'void', total_minor: 5000, applied_minor: 0 }),
];

const CREDIT_NOTE_DETAIL = {
  ...CREDIT_NOTE('cn1', 'CN-000102'),
  remaining_minor: 15000,
  refunded_minor: 0,
  applications: [
    { id: 'ap1', credit_note_id: 'cn1', invoice_id: 'i1', amount_minor: 12000, created_at: '2026-09-08T00:00:00Z', created_by: null },
  ],
};

const ENTRY = (id, minutes, status, locked, extra = {}) => ({
  id,
  org_id: MORIG,
  ticket_id: 't-1',
  user_id: '00000000-0000-4000-8000-00000000000a',
  started_at: new Date(now - 5 * 3600_000).toISOString(),
  ended_at: new Date(now - 5 * 3600_000 + minutes * 60_000).toISOString(),
  minutes,
  billable: true,
  rate_minor: null,
  notes: 'Network troubleshooting + firewall rule change',
  approval_status: status,
  approval_reason_code: status === 'rejected' ? 'missing_notes' : null,
  created_at: new Date().toISOString(),
  updated_at: new Date().toISOString(),
  batch_started_at: null,
  batch_ended_at: null,
  ...extra,
});

const QUEUE_ITEM = (entry, extra = {}) => ({
  entry,
  submitted_by: '00000000-0000-4000-8000-00000000000a',
  submitted_at: new Date(now - 2 * 3600_000).toISOString(),
  decided_by: null,
  decided_at: null,
  locked,
  ...extra,
});

const time_ = (extra) => ENTRY(extra.id, extra.minutes, extra.status, extra.locked ?? false);
void time_;

const APPROVAL_QUEUE = [
  { entry: ENTRY('e1', 95, 'submitted', false), submitted_by: 'a', submitted_at: new Date(now - 2 * 3600_000).toISOString(), decided_by: null, decided_at: null, locked: false },
  { entry: ENTRY('e2', 45, 'submitted', false, { ticket_id: 't-2', notes: 'On-site upstairs printer install' }), submitted_by: 'a', submitted_at: new Date(now - 26 * 3600_000).toISOString(), decided_by: null, decided_at: null, locked: false },
  { entry: ENTRY('e3', 130, 'submitted', false, { ticket_id: 't-3', notes: 'Server migration dry-run' }), submitted_by: 'a', submitted_at: new Date(now - 74 * 3600_000).toISOString(), decided_by: null, decided_at: null, locked: false },
];

const MONEY_FIXTURES = {
  '/api/v1/me': MME,
  '/api/v1/org': { id: MORIG, name: 'Meridian Demo Org', base_currency: 'USD', created_at: '2025-01-01T00:00:00Z' },
  '/api/v1/settings/user': [],
  '/api/v1/settings/org': [],
  '/api/v1/clients': CLIENTS,
  '/api/v1/user': {},
  '/api/v1/timers/active': null,
  '/api/v1/timers/working-set': [],
  '/api/v1/notifications/unread-count': { count: 0 },
  '/api/v1/notifications': { items: [], unread_count: 0, total: 0, next_cursor: null },
  // Billing
  '/api/v1/invoices': INVOICES,
  '/api/v1/invoices/i1': INVOICE_DETAIL,
  '/api/v1/billing/invoice-runs': [
    { id: 'r1', org_id: MORIG, number: 'RUN-000004', period_from: '2026-08-01', period_to: '2026-08-31', due_in_days: 30, invoice_count: 2, total_minor: 60000, currency: 'USD', created_at: new Date(now - 15 * 86_400_000).toISOString(), created_by: null },
  ],
  '/api/v1/invoice-candidates': [],
  '/api/v1/billing/preview': {
    from: plusDate(-30),
    to: plusDate(0),
    total_minor: 113000,
    evaluations: [
      { contract_id: 'ct1', contract_number: 'C-2026-001', client_id: CLIENT_ID, contract_type: 'recurring', currency: 'USD', from: '2026-08-01', to: '2026-08-31', lines: [], total_minor: 113000 },
    ],
  },
  // Credit notes
  '/api/v1/credit-notes': CREDIT_NOTES,
  '/api/v1/credit-notes/cn1': CREDIT_NOTE_DETAIL,
  // Contracts
  '/api/v1/contracts': CONTRACTS,
  '/api/v1/contracts/ct1': CONTRACT_DETAIL,
  // Catalog
  '/api/v1/catalog-items': CATALOG,
  // Pipeline
  '/api/v1/pipeline/stages': STAGES,
  '/api/v1/pipeline/deals': DEALS,
  // Quotes
  '/api/v1/quotes': QUOTES,
  // Approvals
  '/api/v1/time-entries/approvals/queue': APPROVAL_QUEUE,
  '/api/v1/time-entries/e1/approvals': [
    { id: 'ar1', entry_id: 'e1', action: 'submitted', reason_code: null, note: null, created_at: new Date(now - 2 * 3600_000).toISOString() },
  ],
};
const REGISTRY = {
  settings: [
    { key: 'general.eod_hour', label: 'End-of-day hour', description: 'Hour of day (0-23) after which unfinished billable work is flagged. Consumed by missing-time detection and EOD nudges.', scope: 'org', section: 'general', visibility: 'ui', kind: { type: 'integer', value: { min: 0, max: 23 } }, options: [], default: 18 },
    { key: 'general.default_timezone', label: 'Default timezone', description: 'Baseline IANA timezone for business-hours calendars and SLA clocks; individual users may override in their own preferences later.', scope: 'org', section: 'general', visibility: 'ui', kind: { type: 'enumeration' }, options: ['UTC', 'Europe/London', 'Europe/Berlin', 'America/New_York'], default: 'UTC' },
    { key: 'general.week_start', label: 'First day of week', description: 'Anchor day for weekly calendars, capacity views, and reports.', scope: 'org', section: 'general', visibility: 'ui', kind: { type: 'enumeration' }, options: ['monday', 'sunday'], default: 'monday' },
    { key: 'notifications.client_events', label: 'Notify on client ticket activity', description: 'Org default for firing notifications on client ticket activity. A client may override through its workflow overrides, including an explicit disable.', scope: 'org', section: 'general', visibility: 'ui', kind: { type: 'boolean', value: null }, options: [], default: true },
    { key: 'automations.client_rules', label: 'Run automations for client tickets', description: 'Org default for running automation rules against client tickets.', scope: 'org', section: 'general', visibility: 'ui', kind: { type: 'boolean', value: null }, options: [], default: true },
    { key: 'ai.completion_provider', label: 'AI completion provider', description: 'Active LLM completion provider for BYOK intelligence. none disables all outgoing model calls.', scope: 'org', section: 'integrations', visibility: 'ui', kind: { type: 'enumeration' }, options: ['none', 'openai', 'anthropic', 'google'], default: 'none' },
    { key: 'ai.completion_model', label: 'AI completion model', description: 'Model identifier to invoke on the active completion provider.', scope: 'org', section: 'integrations', visibility: 'ui', kind: { type: 'text', value: { max_len: 128 } }, options: [], default: 'none' },
    { key: 'connectors.ci_feeders', label: 'CI feeders connector (NinjaOne / IT Glue / Auvix)', description: 'Accept device/configuration feed upserts from the RMM / docs / NMS feeder sources; off rejects their writes with a typed 403.', scope: 'org', section: 'integrations', visibility: 'ui', kind: { type: 'boolean', value: null }, options: [], default: true },
    { key: 'connectors.immybot', label: 'ImmyBot connector', description: 'Operator-side toggle for the ImmyBot patch-state inventory connector.', scope: 'org', section: 'integrations', visibility: 'ui', kind: { type: 'boolean', value: null }, options: [], default: true },
    { key: 'connectors.m365_graph', label: 'M365 Graph connector', description: 'Operator-side toggle for the Microsoft Intune/Entra device inventory connector.', scope: 'org', section: 'integrations', visibility: 'ui', kind: { type: 'boolean', value: null }, options: [], default: true },
    { key: 'connectors.dms_sync', label: 'DMS sync connector (IT Glue / Hudu ↔ Meridian docs)', description: 'Operator-side toggle for the documentation-system sync.', scope: 'org', section: 'integrations', visibility: 'ui', kind: { type: 'boolean', value: null }, options: [], default: true },
  ],
};

const VALUES = [{ key: 'org_default_timezone', value: 'UTC' }];

const USERS = {
  users: [
    {
      id: 'u1',
      org_id: ORG.id,
      email: 'taylor@meridian.test',
      display_name: 'Taylor Reed',
      timezone: 'UTC',
      is_active: true,
      created_at: new Date(Date.now() - 90 * 864e5).toISOString(),
      is_self: true,
    },
    {
      id: 'u2',
      org_id: ORG.id,
      email: 'ada@meridian.test',
      display_name: 'Ada Whitfield',
      timezone: 'Europe/London',
      is_active: true,
      created_at: new Date(Date.now() - 30 * 864e5).toISOString(),
      is_self: false,
    },
  ],
  total: 2,
};
const EXPORT_CREATED = {
  bundle_version: 1,
  export_id: 'e1',
  download_url: '/api/v1/org/exports/o/e1/download?expires=4200000000&signature=sig',
  expires_at: new Date(Date.now() + 15 * 60_000).toISOString(),
  sha256: 'd3f1c2b4a59687766a1f2e3d4c5b6a7980f1e2d3c4b5a6978f0e1d2c3b4a5967',
  size_bytes: 48213,
};

const tagRow = (id, name, count) => ({
  tag: { id, org_id: ORG.id, name, color: null, created_at: '' },
  applied_count: 0,
  mutation: null,
});
const keyRow = (id, name) => ({
  id,
  org_id: ORG.id,
  name,
  prefix: 'mk_a1b2',
  raw_key: null,
  scopes: ['org:write'],
  last_used_at: new Date(Date.now() - 5 * 3600_000).toISOString(),
  created_at: new Date(Date.now() - 20 * 864e5).toISOString(),
  disabled: false,
});
const webhookRow = (id, name) => ({
  id,
  org_id: ORG.id,
  name,
  url: 'https://ops.example.com/hooks/meridian',
  description: null,
  event_types: ['ticket.created', 'ticket.updated'],
  active: true,
  created_at: new Date(Date.now() - 10 * 864e5).toISOString(),
});

// Empty-department fixture pages rely on the catch-all ([]); the ones that
// render meaningful content at review time get real fixtures here.
const SETTINGS_FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/org': ORG,
  '/api/v1/settings/registry': REGISTRY,
  '/api/v1/settings/org': VALUES,
  '/api/v1/org/users': USERS,
  '/api/v1/boards/nav': [
    { id: 'b1', name: 'Service Desk', description: null, group_id: null, group_name: null, favorite: true, is_default: true, my_role: 'admin' },
  ],
  '/api/v1/notifications/unread-count': { count: 0 },
  '/api/v1/notifications': { items: [], unread_count: 0, total: 0, next_cursor: null },
  '/api/v1/boards': [],
  '/api/v1/boards/b1/statuses': [
    { id: 's1', board_id: 'b1', org_id: ORG.id, name: 'New', category: 'open', sort_order: 0, is_entry: true, color: null },
    { id: 's2', board_id: 'b1', org_id: ORG.id, name: 'In Progress', category: 'in_progress', sort_order: 1, is_entry: false, color: null },
    { id: 's3', board_id: 'b1', org_id: ORG.id, name: 'Done', category: 'closed', sort_order: 2, is_entry: false, color: null },
  ],
  '/api/v1/boards/b1/transitions': { mode: 'floor', edges: [] },
  '/api/v1/boards/b1/checklist-template': {
    items: [{ id: 'i1', org_id: ORG.id, template_id: 't1', sort_order: 0, text: 'Backups verified' }],
    require_checklist_close: true,
    template: { id: 't1', board_id: 'b1', org_id: ORG.id, name: 'Standard close', created_at: '', updated_at: '' },
  },
  '/api/v1/tags': [tagRow('tag1', 'billing', 3), tagRow('tag2', 'network', 12)],
  '/api/v1/boards/taxonomy/types': [],
  '/api/v1/boards/taxonomy/categories': [],
  '/api/v1/api-keys': { keys: [keyRow('k1', 'deploy automation'), keyRow('k2', 'read-only billing export')] },
  '/api/v1/webhooks': { webhooks: [webhookRow('wh1', 'Customer ops relay')] },
  '/api/v1/fx/rates': [
    { id: 'fx1', base_currency: 'USD', quote_currency: 'EUR', rate: '0.9100', source: 'manual', as_of_date: new Date().toISOString().slice(0, 10), captured_at: new Date().toISOString(), org_id: ORG.id },
    { id: 'fx2', base_currency: 'USD', quote_currency: 'GBP', rate: '0.7800', source: 'manual', as_of_date: new Date().toISOString().slice(0, 10), captured_at: new Date().toISOString(), org_id: ORG.id },
  ],
  '/api/v1/fx/reconciliation': [],
  '/api/v1/roles': { portal_tiers: [], presets: [], roles: [] },};

function shot(browser, viewport, theme, name, path, interactions) {
  return shotInner(browser, viewport, theme, name, path, interactions);
}

const viewports = [
  { width: 1440, height: 1024 },
  { width: 1024, height: 768 },
  { width: 768, height: 1024 },
  { width: 480, height: 900 },
];

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, args: ['--no-sandbox', '--disable-dev-shm-usage'] });

// One staged-approvals payload so the plugins surface carries its rows.
const PLUGIN_FIXTURES = {
  '/api/v1/plugins/registry': { plugins: [] },
  '/api/v1/plugins/installed': [],
  '/api/v1/agent-skills/registry': { skills: [] },
  '/api/v1/agent-skills/installed': [],
  '/api/v1/plugin-staging': { items: [] },
};

const FIXTURES = {
  ...BACKEND_FIXTURES,
  ...MONEY_FIXTURES,
  ...SETTINGS_FIXTURES,
  ...PLUGIN_FIXTURES,
};
async function shotInner(browser, viewport, theme, name, path, interactions) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', name, String(e.stack ?? e).slice(0, 700)));
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
  await page.locator('main').first().waitFor({ state: 'attached', timeout: 20_000 });
  await page.waitForTimeout(1100);
  if (interactions) await interactions(page);
  await page.waitForTimeout(400);
  await page.screenshot({ path: resolve(OUT, name), fullPage: false });
  console.log('captured', name);
  await context.close();
}

const surfaces = [
  // Staff dashboards + reports (lane 1; Table/Chart toggle + drilldown)
  { tag: 'today', path: '/today' },
  { tag: 'today-drilldown', path: '/today', interactions: async (page) => {
      const t = page.getByTestId('unanswered-drilldown').first();
      if (await t.count()) { await t.click(); await page.waitForTimeout(500); }
    } },

  // Ticket workspace + workbar (lane 3 boards-dispatch pod lives in Details)
  { tag: 'ticket-workspace', path: '/tickets/00000000-0000-4000-8000-0000000000t9' },

  // Boards / calendar / projects / clients / documents / assets
  { tag: 'boards-wip', path: '/boards/b1' },
  { tag: 'calendar-deadlines', path: '/calendar' },
  { tag: 'projects', path: '/projects' },
  { tag: 'clients', path: '/clients' },
  { tag: 'client-detail', path: '/clients/00000000-0000-4000-8000-0000000000c1' },
  { tag: 'document-attachments', path: '/documents/00000000-0000-4000-8000-0000000000d1' },
  { tag: 'assets', path: '/assets' },

  // Knowledge + ops tooling (lane 6 / 13 surfaces)
  { tag: 'kb', path: '/kb' },
  { tag: 'alerts', path: '/alerts' },
  { tag: 'automations', path: '/automations', interactions: async (page) => {
      const t = page.locator('[data-testid^="automation-test-run-"]').first();
      if (await t.count()) { await t.click(); await page.waitForTimeout(700); }
    } },
  { tag: 'audit', path: '/audit' },
  { tag: 'provenance', path: '/provenance' },
  { tag: 'inventory', path: '/inventory' },
  { tag: 'search-pins', path: '/search' },

  // Money (lane 5)
  { tag: 'billing', path: '/billing' },
  { tag: 'billing-detail', path: '/billing/i1' },
  { tag: 'contracts', path: '/contracts' },
  { tag: 'pipeline', path: '/sales/pipeline' },
  { tag: 'quotes', path: '/sales/quotes' },
  { tag: 'approvals', path: '/approvals' },

  // Settings — SaveBar adopter sections + wave surfaces (lanes 7/8/9/13)
  { tag: 'settings-general', path: '/settings/general' },
  { tag: 'settings-boards', path: '/settings/boards' },
  { tag: 'settings-taxonomy', path: '/settings/taxonomy' },
  { tag: 'settings-team', path: '/settings/team' },
  { tag: 'settings-workflow', path: '/settings/workflow' },
  { tag: 'settings-api-keys', path: '/settings/api-keys' },
  { tag: 'settings-pm-schedules', path: '/settings/pm-schedules' },
  { tag: 'settings-plugins', path: '/settings/plugins' },
  { tag: 'settings-agent-skills', path: '/settings/plugins?tab=skills' },
  { tag: 'settings-agent-platform', path: '/settings/agent-platform' },
  { tag: 'settings-ai', path: '/settings/ai' },
  { tag: 'settings-export', path: '/settings/export' },
  { tag: 'settings-portal', path: '/settings/portal' },
  { tag: 'settings-security', path: '/settings/security' },

  // Auth covers (headerless, lane 9 de-serifed) + public/portal entry
  { tag: 'login', path: '/login' },
  { tag: 'mfa', path: '/mfa' },
  { tag: 'sso', path: '/sso' },
  { tag: 'accept-invite', path: '/accept-invite' },
  { tag: 'portal-login', path: '/portal/login' },
];

let count = 0;
let failed = 0;
// ONLY=a,b — re-capture just the named surface tags (re-verification runs).
const ONLY = (process.env.ONLY ?? "").split(",").map((s) => s.trim()).filter(Boolean);
for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    for (const s of surfaces) {
      if (ONLY.length > 0 && !ONLY.includes(s.tag)) continue;
      const name = `${s.tag}-${theme}-${vp.width === 1440 ? '1440' : String(vp.width)}.png`;
      try {
        await shot(browser, vp, theme, name, s.path, s.interactions);
        count += 1;
      } catch (e) {
        failed += 1;
        console.error('FAIL', name, e?.message ?? e);
      }
    }
  }
}

console.log(`DONE ${count} captures, ${failed} failures → design/qa-ux-sweep-closeout/`);
