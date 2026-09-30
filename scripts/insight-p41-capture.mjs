#!/usr/bin/env node
/**
 * Insight capture — Wave 41 lane 11 (p41-ui-insight).
 *
 * Photographs the six Insight routes against the Marula surface pack, in the
 * neutral light theme and in dark, at 1600 (the pack's capture width) plus a
 * 1280 pass for the dense ledgers. Lane 15 (p41-ui-audit) diffs `states/`
 * against brand/marula-mockups/marula-surface-pack and reads
 * `contact-sheet-<theme>.png` for every picture on one page.
 *
 * Every state is reached through the real UI — clicks and keys, never a
 * direct state poke — so a capture that succeeds is also proof the
 * interaction works. The clock is pinned with Playwright's clock API so
 * relative timestamps ("22m", "3h") cannot drift between runs.
 *
 * API responses are fixtures served by Playwright routing — no backend needed.
 * Pattern: scripts/dispatch-p41-capture.mjs (Wave 41 lane 6).
 *
 * Usage: start the web app (dev on :3000), then
 *   node scripts/insight-p41-capture.mjs [baseUrl]
 * Env: CHROME_PATH (chromium executable), ONLY (comma-separated state ids).
 */
import { chromium } from 'playwright';
import { mkdir, readdir, readFile } from 'fs/promises';

import { existsSync } from 'fs';
import { join, resolve } from 'path';

const OUT = resolve('design/qa-p41-ui-insight');
const STATES_DIR = resolve(OUT, 'states');
const BASE = process.argv[2] ?? 'http://localhost:3000';
const ONLY = process.env.ONLY ? new Set(process.env.ONLY.split(',')) : null;
const FROZEN = new Date('2026-03-11T15:20:00Z');

if (!existsSync(STATES_DIR)) await mkdir(STATES_DIR, { recursive: true });

const ORG = '00000000-0000-4000-8000-000000000002';
const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: ORG,
  email: 'owner@marula.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: [
    'special:manage_users',
    'special:manage_boards',
    'special:manage_reports',
    'tickets:create',
    'admin:org',
  ],
};

const at = (h, m = 0) => {
  const d = new Date(FROZEN);
  d.setUTCHours(h, m, 0, 0);
  return d.toISOString();
};

/* ------------------------------------------------------------------ *
 * Fixtures
 * ------------------------------------------------------------------ */

/** Audit events: a mix of actor kinds so the Kind column has all its tones. */
const AUDIT_EVENTS = [
  {
    id: '90000000-0000-4000-8000-000000000001',
    action: 'auth.login.succeeded',
    actor: { kind: 'human', id: ME.user_id },
    entity_type: 'user',
    entity_id: '00000000-0000-4000-8000-000000000001',
    ip: '203.0.113.7',
    detail: { method: 'password', mfa: false },
    occurred_at: at(15, 18),
  },
  {
    id: '90000000-0000-4000-8000-000000000002',
    action: 'ticket.status.changed',
    actor: { kind: 'agent', id: '82000000-0000-4000-8000-000000000001' },
    entity_type: 'ticket',
    entity_id: '81000000-0000-4000-8000-000000000009',
    ip: null,
    detail: { from: 'open', to: 'waiting', skill: 'triage' },
    occurred_at: at(15, 12),
  },
  {
    id: '90000000-0000-4000-8000-000000000003',
    action: 'settings.domain.verified',
    actor: { kind: 'human', id: '00000000-0000-4000-8000-000000000004' },
    entity_type: 'domain',
    entity_id: '83000000-0000-4000-8000-000000000001',
    ip: '198.51.100.22',
    detail: { domain: 'acme.example', status: 'verified' },
    occurred_at: at(14, 55),
  },
  {
    id: '90000000-0000-4000-8000-000000000004',
    action: 'api_key.created',
    actor: { kind: 'api_key', id: '84000000-0000-4000-8000-000000000001' },
    entity_type: 'api_key',
    entity_id: '84000000-0000-4000-8000-000000000001',
    ip: '203.0.113.9',
    detail: { label: 'Zapier', scopes: ['tickets:read'] },
    occurred_at: at(14, 41),
  },
  {
    id: '90000000-0000-4000-8000-000000000005',
    action: 'report.scheduled',
    actor: { kind: 'system', id: null },
    entity_type: 'report_schedule',
    entity_id: '85000000-0000-4000-8000-000000000001',
    ip: null,
    detail: { cadence: 'daily', recipients: 4 },
    occurred_at: at(14, 30),
  },
  {
    id: '90000000-0000-4000-8000-000000000006',
    action: 'client.merged',
    actor: { kind: 'human', id: '00000000-0000-4000-8000-000000000004' },
    entity_type: 'client',
    entity_id: '86000000-0000-4000-8000-000000000002',
    ip: '198.51.100.31',
    detail: { into: '86000000-0000-4000-8000-000000000001' },
    occurred_at: at(13, 58),
  },
];

/** Provenance events: field-level before/after, both human and agent writes. */
const PROVENANCE_EVENTS = [
  {
    write_id: '80000000-0000-4000-8000-000000000001',
    entity: 'ticket',
    entity_id: '81000000-0000-4000-8000-000000000001',
    seq: 12,
    op: 'update',
    actor_kind: 'human',
    actor_id: ME.user_id,
    field: 'status',
    before: 'open',
    after: 'waiting',
    undo: false,
    occurred_at: at(15, 12),
    idempotency_key: null,
    ruleset_hash: null,
    session_id: null,
    skill: null,
  },
  {
    write_id: '80000000-0000-4000-8000-000000000002',
    entity: 'ticket',
    entity_id: '81000000-0000-4000-8000-000000000001',
    seq: 13,
    op: 'update',
    actor_kind: 'agent',
    actor_id: '82000000-0000-4000-8000-000000000001',
    field: 'priority',
    before: 'p3',
    after: 'p2',
    undo: false,
    occurred_at: at(15, 12),
    idempotency_key: 'idem-0001',
    ruleset_hash: null,
    session_id: '82000000-0000-4000-8000-000000000002',
    skill: 'triage',
  },
  {
    write_id: '80000000-0000-4000-8000-000000000003',
    entity: 'ticket',
    entity_id: '81000000-0000-4000-8000-000000000009',
    seq: 4,
    op: 'update',
    actor_kind: 'human',
    actor_id: '00000000-0000-4000-8000-000000000004',
    field: 'assignee_id',
    before: '00000000-0000-4000-8000-000000000004',
    after: '00000000-0000-4000-8000-000000000001',
    undo: false,
    occurred_at: at(14, 41),
    idempotency_key: null,
    ruleset_hash: null,
    session_id: null,
    skill: null,
  },
  {
    write_id: '80000000-0000-4000-8000-000000000004',
    entity: 'client',
    entity_id: '86000000-0000-4000-8000-000000000001',
    seq: 2,
    op: 'update',
    actor_kind: 'import',
    actor_id: null,
    field: 'phone',
    before: '+44 20 7946 0000',
    after: '+44 20 7946 0958',
    undo: false,
    occurred_at: at(13, 58),
    idempotency_key: 'import-77',
    ruleset_hash: 'rs-9f2',
    session_id: null,
    skill: null,
  },
];

/** Saved report definitions, one org-wide and one private. */
const REPORT_DEFS = [
  {
    id: '87000000-0000-4000-8000-000000000001',
    name: 'Weekly ticket volume',
    description: 'Opened, closed and reopened per week',
    spec: {
      view: 'tickets',
      group_by: null,
      metrics: [{ metric: 'count', alias: null }],
      filters: [],
      time_bucket: 'week',
      sort: null,
      limit: 50,
    },
    scope: 'org',
    created_by: ME.user_id,
    updated_at: at(9, 12),
  },
  {
    id: '87000000-0000-4000-8000-000000000002',
    name: 'Response time by client',
    description: null,
    spec: {
      view: 'tickets',
      group_by: 'client',
      metrics: [{ metric: 'median', field: 'first_response_seconds', alias: null }],
      filters: [],
      time_bucket: null,
      sort: null,
      limit: 50,
    },
    scope: 'self',
    created_by: ME.user_id,
    updated_at: at(11, 2),
  },
  {
    id: '87000000-0000-4000-8000-000000000003',
    name: 'Approved time by work type',
    description: 'Billable split for the month',
    spec: {
      view: 'time_entries',
      group_by: 'work_type',
      metrics: [{ metric: 'sum', field: 'approved_minutes', alias: null }],
      filters: [],
      time_bucket: 'month',
      sort: null,
      limit: 50,
    },
    scope: 'org',
    created_by: ME.user_id,
    updated_at: at(8, 44),
  },
];

const REPORT_SCHEDULES = [
  {
    id: '88000000-0000-4000-8000-000000000001',
    name: 'Monday digest',
    definition_id: REPORT_DEFS[0].id,
    definition_name: REPORT_DEFS[0].name,
    cadence: 'weekly',
    time_of_day: '07:30',
    timezone: 'UTC',
    recipients: [ME.user_id, '00000000-0000-4000-8000-000000000004'],
    next_run_at: at(16, 0),
    last_run_at: at(9, 12),
    last_error: null,
    enabled: true,
    created_at: at(1, 0),
  },
  {
    id: '88000000-0000-4000-8000-000000000002',
    name: 'SLA exceptions',
    definition_id: REPORT_DEFS[1].id,
    definition_name: REPORT_DEFS[1].name,
    cadence: 'daily',
    time_of_day: '08:00',
    timezone: 'UTC',
    recipients: [ME.user_id],
    next_run_at: null,
    last_run_at: at(8, 0),
    last_error: 'mailbox unavailable',
    enabled: true,
    created_at: at(2, 0),
  },
];

const CATALOG = {
  views: [
    {
      view: 'tickets',
      columns: [
        { name: 'org_id', value_type: 'uuid' },
        { name: 'number', value_type: 'text' },
        { name: 'client_name', value_type: 'text' },
        { name: 'priority', value_type: 'text' },
        { name: 'status', value_type: 'text' },
        { name: 'created_at', value_type: 'timestamp' },
        { name: 'first_response_seconds', value_type: 'int' },
        { name: 'breached', value_type: 'bool' },
      ],
      filterable: [
        { name: 'priority', value_type: 'text' },
        { name: 'status', value_type: 'text' },
        { name: 'client_id', value_type: 'uuid' },
        { name: 'breached', value_type: 'bool' },
      ],
    },
    {
      view: 'time_entries',
      columns: [
        { name: 'org_id', value_type: 'uuid' },
        { name: 'user_display_name', value_type: 'text' },
        { name: 'minutes', value_type: 'int' },
        { name: 'billable', value_type: 'bool' },
      ],
      filterable: [
        { name: 'billable', value_type: 'bool' },
        { name: 'user_id', value_type: 'uuid' },
      ],
    },
  ],
};
/** A report run page, for the builder preview and the dashboard widgets. */
const RUN_PAGE = {
  kind: 'tickets',
  view: 'ops.v1_tickets',
  columns: ['priority', 'count'],
  rows: [
    { priority: 'P1', count: 12 },
    { priority: 'P2', count: 30 },
    { priority: 'P3', count: 41 },
    { priority: 'P4', count: 9 },
  ],
  truncated: false,
};
const SCORECARD = {
  user_id: ME.user_id,
  display_name: 'Taylor Reed',
  window: { from: at(15, 0), to: at(16, 0) },
  response: { median_seconds: 812, p95_seconds: 2400 },
    resolution: { median_seconds: 2600, p95_seconds: 7200 },
  sla: {
    response_attainment: 0.94,
    response_met: 16,
    response_total: 17,
    resolution_attainment: 0.88,
    resolution_met: 15,
    resolution_total: 17,
    breached: 2,
    total: 17,
  },
  workload: {
    approved_minutes: 1680,
    by_work_type: [
      { work_type: 'Remote support', minutes: 900 },
      { work_type: 'On site', minutes: 480 },
      { work_type: 'Project', minutes: 300 },
    ],
  },
  csat: { available: true, responses: 8, avg_score: 4.6 },
  gamification_opt_in: false,
};

const TEAM_SCORECARDS = {
  scorecards: [
    {
      user_id: ME.user_id,
      display_name: 'Taylor Reed',
      window: { from: at(15, 0), to: at(16, 0) },
      response: { median_seconds: 812, p95_seconds: 2400 },
      resolution: { median_seconds: 2600, p95_seconds: 7200 },
      sla: {
        response_attainment: 0.94,
        response_met: 16,
        response_total: 17,
        resolution_attainment: 0.88,
        resolution_met: 15,
        resolution_total: 17,
        breached: 2,
        total: 17,
      },
      workload: {
        approved_minutes: 1680,
        by_work_type: [
          { work_type: 'Remote support', minutes: 1200 },
          { work_type: 'On site', minutes: 480 },
        ],
      },
      csat: { available: true, responses: 8, avg_score: 4.6 },
    },
    {
      user_id: '00000000-0000-4000-8000-000000000004',
      display_name: 'Jordan T.',
      window: { from: at(15, 0), to: at(16, 0) },
      response: { median_seconds: 1440, p95_seconds: 3900 },
    resolution: { median_seconds: 2600, p95_seconds: 7200 },
      sla: {
        response_attainment: 0.81,
        response_met: 17,
        response_total: 21,
        resolution_attainment: 0.79,
        resolution_met: 16,
        resolution_total: 21,
        breached: 5,
        total: 21,
      },
      workload: {
        approved_minutes: 1500,
        by_work_type: [
          { work_type: 'On site', minutes: 900 },
          { work_type: 'Remote support', minutes: 600 },
        ],
      },
      csat: { available: true, responses: 5, avg_score: 4.2 },
    },
    {
      user_id: '00000000-0000-4000-8000-000000000005',
      display_name: 'Amara O.',
      window: { from: at(15, 0), to: at(16, 0) },
      response: { median_seconds: 640, p95_seconds: 1800 },
    resolution: { median_seconds: 2600, p95_seconds: 7200 },
      sla: {
        response_attainment: 0.97,
        response_met: 18,
        response_total: 19,
        resolution_attainment: 0.95,
        resolution_met: 18,
        resolution_total: 19,
        breached: 0,
        total: 19,
      },
      workload: {
        approved_minutes: 1740,
        by_work_type: [
          { work_type: 'Remote support', minutes: 1400 },
          { work_type: 'Project', minutes: 340 },
        ],
      },
      csat: { available: true, responses: 11, avg_score: 4.8 },
    },
  ],
};

const SEARCH_RESULTS = {
  query: 'vpn',
  results: [
    {
      entity_kind: 'ticket',
      entity_id: '81000000-0000-4000-8000-000000000001',
      title: 'VPN disconnects every 20 to 30 minutes',
      snippet: 'Acme Corporation reported drops every half hour across two sites',
      parent_id: null,
      score: 0.94,
    },
    {
      entity_kind: 'ticket',
      entity_id: '81000000-0000-4000-8000-000000000009',
      title: 'Site-to-site tunnel drops during backup window',
      snippet: 'Kestrel Logistics — backup job overlaps the tunnel rekey',
      parent_id: null,
      score: 0.81,
    },
    {
      entity_kind: 'comment',
      entity_id: '89000000-0000-4000-8000-000000000001',
      title: 'Re: VPN disconnects every 20 to 30 minutes',
      snippet: 'Client confirms the drops line up with the lunch-hour backup',
      parent_id: '81000000-0000-4000-8000-000000000001',
      score: 0.77,
    },
    {
      entity_kind: 'client',
      entity_id: '86000000-0000-4000-8000-000000000001',
      title: 'Acme Corporation',
      snippet: '4 sites, 3 contacts, 2 open tickets',
      parent_id: null,
      score: 0.72,
    },
    {
      entity_kind: 'kb_article',
      entity_id: '8a000000-0000-4000-8000-000000000001',
      title: 'Diagnosing a flapping site-to-site tunnel',
      snippet: 'Check the rekey window against the backup schedule first',
      parent_id: null,
      score: 0.68,
    },
  ],
  total: 5,
  next_cursor: null,
};

/**
 * A rollup page for a widget card. The dashboards grid reads
 * /api/v1/reports/{kind} and renders label + metric, so this is what makes the
 * Planning cards carry data rather than an ornament each.
 */
function rollupPage(kind) {
  const rows = {
    time_by_user: [
      { group_id: 'u1', label: 'Taylor Reed', metric: 1680, billable_minutes: 1500, entry_count: 22 },
      { group_id: 'u2', label: 'Jordan T.', metric: 1500, billable_minutes: 1200, entry_count: 19 },
      { group_id: 'u3', label: 'Amara O.', metric: 1740, billable_minutes: 1740, entry_count: 26 },
    ],
    time_by_client: [
      { group_id: 'c1', label: 'Acme Corporation', metric: 900, billable_minutes: 900, entry_count: 12 },
      { group_id: 'c2', label: 'Kestrel Logistics', metric: 620, billable_minutes: 480, entry_count: 9 },
      { group_id: 'c3', label: 'Harbour Dental', metric: 310, billable_minutes: 310, entry_count: 6 },
    ],
    ticket_throughput: [
      { group_id: 'd1', label: '11 Mar', metric: 42, billable_minutes: 0, entry_count: 42 },
      { group_id: 'd2', label: '10 Mar', metric: 37, billable_minutes: 0, entry_count: 37 },
      { group_id: 'd3', label: '09 Mar', metric: 51, billable_minutes: 0, entry_count: 51 },
      { group_id: 'd4', label: '08 Mar', metric: 29, billable_minutes: 0, entry_count: 29 },
    ],
    ticket_aging: [
      { group_id: 'a1', label: 'Under 24h', metric: 18, billable_minutes: 0, entry_count: 18 },
      { group_id: 'a2', label: '1 to 3 days', metric: 9, billable_minutes: 0, entry_count: 9 },
      { group_id: 'a3', label: 'Over 3 days', metric: 4, billable_minutes: 0, entry_count: 4 },
    ],
    sla_attainment: [
      { group_id: 's1', label: 'Responded in time', metric: 47, billable_minutes: 0, entry_count: 47 },
      { group_id: 's2', label: 'Missed', metric: 5, billable_minutes: 0, entry_count: 5 },
    ],
    sla_breaches: [
      {
        group_id: 'b1',
        ticket_id: '81000000-0000-4000-8000-000000000001',
        ticket_number: 18402,
        priority: 'P1',
        policy_name: 'P1 resolution',
        resolution_due: '2026-03-11T14:20:00Z',
        label: 'Backup job failed on FILESRV01',
        metric: 65,
        billable_minutes: 0,
        entry_count: 1,
      },
      {
        group_id: 'b2',
        ticket_id: '81000000-0000-4000-8000-000000000017',
        ticket_number: 18417,
        priority: 'P2',
        policy_name: 'P2 resolution',
        resolution_due: '2026-03-11T09:02:00Z',
        label: 'MFA prompts not arriving',
        metric: 130,
        billable_minutes: 0,
        entry_count: 1,
      },
      {
        group_id: 'b3',
        ticket_id: '81000000-0000-4000-8000-000000000030',
        ticket_number: 18430,
        priority: 'P3',
        policy_name: 'Standard resolution',
        resolution_due: '2026-03-09T08:00:00Z',
        label: 'Domain renewal overdue',
        metric: 2880,
        billable_minutes: 0,
        entry_count: 1,
      },
    ],
    first_response: [
      { group_id: 'f1', label: 'Median first response', metric: 812, billable_minutes: 0, entry_count: 0 },
    ],
  }[kind];
  return {
    kind: 'tickets',
    view: `ops.v1_${kind}`,
    columns: ['label', 'metric'],
    rows: rows ?? [],
    summary: {
      total_minutes: 4920,
      billable_minutes: 4440,
      entry_count: 67,
      answered_count: 52,
      unanswered_count: 0,
      tickets_total: 52,
      response_p50_secs: 812,
      response_p90_secs: 2400,
    },
    truncated: false,
  };
}

/** Unrouted fallbacks, so an endpoint this lane forgot cannot 500 the page. */
function baseFixtures() {
  return {
    '/api/v1/me': ME,
    '/api/v1/settings/user': [
      {
        key: 'dashboards.widgets',
        value: JSON.stringify({
          version: 1,
          widgets: ['time_by_user:org', 'sla_breaches:org', 'ticket_throughput:org'],
        }),
      },
      {
        key: 'appearance.data_surfaces',
        value: {},
      },
      { key: 'appearance.density', value: 'comfortable' },
    ],
    '/api/v1/audit-events': { events: AUDIT_EVENTS, next_cursor: null },
    '/api/v1/provenance': { events: PROVENANCE_EVENTS, next_cursor: null },
    '/api/v1/report-definitions': { definitions: REPORT_DEFS },
    '/api/v1/report-schedules': { schedules: REPORT_SCHEDULES },
    '/api/v1/report-catalog': CATALOG,
    '/api/v1/scorecards/me': SCORECARD,
    '/api/v1/scorecards/me/settings': { gamification_opt_in: false },
    '/api/v1/scorecards': TEAM_SCORECARDS,
    '/api/v1/scorecards/clients': {
      available: true,
      rollups: [
        { client_id: '86000000-0000-4000-8000-000000000001', client_name: 'Acme Corporation', responses: 6, avg_score: 4.7, low_scores: 0 },
        { client_id: '86000000-0000-4000-8000-000000000002', client_name: 'Kestrel Logistics', responses: 4, avg_score: 4.2, low_scores: 1 },
        { client_id: '86000000-0000-4000-8000-000000000003', client_name: 'Summit Physio', responses: 3, avg_score: 3.9, low_scores: 1 },
      ],
    },
    '/api/v1/search': SEARCH_RESULTS,
    '/api/v1/search/pins': { pins: [] },
    '/api/v1/org/users': {
      users: [
        { id: ME.user_id, display_name: 'Taylor Reed', email: 'owner@marula.test', is_active: true },
        { id: '00000000-0000-4000-8000-000000000004', display_name: 'Jordan T.', email: 'jordan@marula.test', is_active: true },
      ],
    },
  };
}

/* ------------------------------------------------------------------ *
 * Capture
 * ------------------------------------------------------------------ */

async function shot(
  browser,
  { viewport, theme, id, path, fixtures = {}, errors = [], hold, prepare, expect: expectFn },
) {
  // The width belongs in the filename: a narrow pass reuses the state id, and
  // a shared name would silently replace the 1600 shot lane 15 diffs.
  const tag = `${id}-${viewport.width}-${theme}`;
  const all = { ...baseFixtures(), ...fixtures };
  const errorPaths = new Set(errors);
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', id, theme, String(e).slice(0, 300)));
  const url = new URL(`${BASE}${path}`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');
  await page.clock.install({ time: FROZEN });
  await page.addInitScript(
    (t) => {
      window.localStorage.setItem('marula-theme', t);
      window.localStorage.setItem('marula-preset', 'marula');
      window.localStorage.setItem('marula-density', 'comfortable');
      window.localStorage.setItem(
        'marula-tokens',
        JSON.stringify({
          access_token: 'test-access',
          refresh_token: 'test-refresh',
          session_id: 'sess',
          expires_in: 900,
        }),
      );
      window.localStorage.setItem('marula-tokens-exp', String(Date.now() + 3_600_000));
      const style = document.createElement('style');
      style.textContent = 'nextjs-portal{display:none!important}';
      document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
    },
    theme === 'dark' ? 'dark' : 'light',
  );

  await page.route('**/api/**', async (r) => {
    const key = new URL(r.request().url()).pathname;
    // A state that declares a failing endpoint fails on the FIRST load, so
    // this has to be checked before the fixture table below.
    if (errorPaths.has(key)) {
      await r.fulfill({
        status: 503,
        contentType: 'application/json',
        body: JSON.stringify({ error: { message: 'the service did not answer' } }),
      });
      return;
    }
    if (hold !== undefined && hold.includes(key)) {
      await new Promise((res) => setTimeout(res, 20_000));
      return;
    }
    if (all[key] !== undefined) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(all[key]) });
      return;
    }
    // A widget card's rollup page.
    if (/^\/api\/v1\/reports\/[^/]+$/.test(key)) {
      await r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(rollupPage(key.split('/').pop())),
      });
      return;
    }
    // A report run answers whatever definition id the page asks for.
    if (/^\/api\/v1\/report-definitions\/[^/]+\/run$/.test(key)) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(RUN_PAGE) });
      return;
    }
    await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
  });

  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForSelector('main', { timeout: 30_000 });
  await page.waitForTimeout(900);
  if (prepare !== undefined) await prepare(page);
  await page.waitForTimeout(450);
  if (expectFn !== undefined) {
    const problem = await expectFn(page);
    if (problem !== null) console.error('CHECK FAILED', tag, problem);
  }
  await page.screenshot({ path: resolve(STATES_DIR, `${tag}.png`), fullPage: false });
  console.log('captured', tag);
  await context.close();
}

/** The pack's overview sheet: every state on one page, labelled. */
async function contactSheet(browser, theme) {
  const files = (await readdir(STATES_DIR))
    .filter((f) => f.endsWith(`-1600-${theme}.png`) && /^\d\d-/.test(f))
    .sort();
  if (files.length === 0) {
    console.error('no captures for the', theme, 'sheet');
    return;
  }
  // Inlined as data URIs: `setContent` leaves relative src resolving against
  // about:blank, so a routed `**/states/**` fetch never fires and the sheet
  // comes out as captions with empty frames.
  const cells = (
    await Promise.all(
      files.map(async (f) => {
        const body = await readFile(join(STATES_DIR, f));
        const uri = `data:image/png;base64,${body.toString('base64')}`;
        const caption = f.replace(`-1600-${theme}.png`, '');
        return `<figure><img src="${uri}" alt="${caption}"><figcaption>${caption}</figcaption></figure>`;
      }),
    )
  ).join('\n');
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
  await page.setContent(html, { waitUntil: 'load' });
  await page.waitForTimeout(600);
  await page.screenshot({ path: resolve(OUT, `contact-sheet-${theme}.png`), fullPage: true });
  console.log('contact sheet', theme, files.length, 'states');
  await context.close();
}

/* ------------------------------------------------------------------ *
 * The states
 * ------------------------------------------------------------------ */

/** Click the tab with this exact name in the surface's view control. */
async function useTab(page, name) {
  await page.getByRole('tab', { name, exact: false }).first().click();
  await page.waitForTimeout(500);
}

/** Poll until `probe` returns null (the check passed) or the budget runs out. */
async function until(probe, budgetMs = 8000) {
  const deadline = Date.now() + budgetMs;
  for (;;) {
    const problem = await probe();
    if (problem === null) return null;
    if (Date.now() > deadline) return problem;
    await new Promise((res) => setTimeout(res, 250));
  }
}

const CHECKS = {
  /** The three rules this lane is judged on, asserted on the rendered page. */
  noBulkBar: async (page) => {
    // Wait for the table to arrive first: a not-yet-rendered surface has no
    // checkboxes either, and that would pass for the wrong reason.
    await page.waitForSelector('table, [role="alert"]', { timeout: 8000 }).catch(() => undefined);
    const boxes = await page.locator('input[type="checkbox"]').count();
    const bulk = await page.getByText(/^\d+ selected$/).count();
    if (boxes > 0) return `found ${boxes} checkbox(es) — audit and provenance take no bulk bar`;
    if (bulk > 0) return 'found a bulk selection bar';
    return null;
  },
  noRedOnTrouble: async (page) => {
    const html = await page.content();
    // The danger token must not appear anywhere in a trouble state.
    if (/--m-status-danger|--m-destructive|#b42318/i.test(html)) {
      return 'a danger/red token is present in the state';
    }
    return null;
  },
  noAiOnSearch: async (page) => {
    await page.waitForTimeout(400);
    const text = (await page.locator('body').innerText()).toLowerCase();
    const bad = ['ask ai', 'assistant', 'sparkle', 'try ai', 'let ai', 'generate with'].find((w) =>
      text.includes(w),
    );
    if (bad !== undefined) return `AI affordance on search: "${bad}"`;
    if ((await page.locator('.m-assistant-stub').count()) > 0) return 'assistant stub on search';
    return null;
  },
  /** The small-caps header treatment has to survive the render. */
  smallCapsHeaders: async (page) => {
    await page.waitForSelector('table thead th', { timeout: 8000 }).catch(() => undefined);
    const th = page.locator('table thead th').first();
    if ((await th.count()) === 0) return 'no table header to check';
    const transform = await th.evaluate((el) => getComputedStyle(el).textTransform);
    if (!transform.toLowerCase().includes('uppercase')) {
      return `column header is not small caps (text-transform: ${transform})`;
    }
    return null;
  },
};

const STATES = [
  // --- Dashboards: the Planning-card grid -------------------------------
  { id: '01-dashboards-default', path: '/dashboards' },
  {
    id: '02-dashboards-catalogue',
    path: '/dashboards',
    prepare: async (page) => {
      await page.getByRole('button', { name: /add widget/i }).click();
      await page.waitForTimeout(350);
    },
  },
  {
    id: '03-dashboards-save-failed',
    path: '/dashboards',
    fixtures: { '/api/v1/settings/user': [] },
    prepare: async (page) => {
      // The registry refuses the write, so the arrangement must stay on
      // screen with a notice rather than dropping the user's change.
      await page.route('**/api/v1/settings/user', (r) =>
        r.request().method() === 'PUT'
          ? r.fulfill({
              status: 409,
              contentType: 'application/json',
              body: JSON.stringify({ error: { message: 'the settings registry rejected the write' } }),
            })
          : r.fallback(),
      );
      await page.getByRole('button', { name: /add widget/i }).click();
      await page.waitForTimeout(250);
      await page.getByRole('button', { name: /time by client/i }).first().click();
      await page.waitForTimeout(900);
    },
    expect: async (page) => {
      const notice = await page.getByTestId('dashboard-save-error').count();
      return notice === 0 ? 'the save failure did not surface' : null;
    },
  },

  // --- Reports: the data-surface frame ----------------------------------
  {
    id: '04-reports-builder',
    path: '/reports',
  },
  {
    id: '05-reports-saved',
    path: '/reports',
    prepare: (page) => useTab(page, 'Saved reports'),
    expect: CHECKS.smallCapsHeaders,
  },
  {
    id: '06-reports-saved-filtered',
    path: '/reports',
    prepare: async (page) => {
      await useTab(page, 'Saved reports');
      await page.getByRole('button', { name: /add filter/i }).click();
      await page.waitForTimeout(250);
      await page.getByLabel('Field').selectOption({ label: 'Data' });
      await page.getByLabel('Value').fill('Time');
      await page.getByRole('button', { name: 'Add', exact: true }).click();
      await page.waitForTimeout(500);
    },
  },
  {
    id: '07-reports-schedules',
    path: '/reports',
    prepare: (page) => useTab(page, 'Schedules'),
  },

  // --- Audit: the dense ledger, no bulk --------------------------------
  { id: '08-audit-default', path: '/audit', expect: async (page) => (await CHECKS.noBulkBar(page)) ?? (await CHECKS.smallCapsHeaders(page)) },
  {
    id: '09-audit-expanded',
    path: '/audit',
    prepare: async (page) => {
      await page.locator('[data-testid^="audit-row-"]').first().click();
      await page.waitForTimeout(400);
    },
  },
  {
    id: '10-audit-agent-view',
    path: '/audit',
    prepare: (page) => useTab(page, 'Agent events'),
  },
  {
    id: '11-audit-empty',
    path: '/audit',
    fixtures: { '/api/v1/audit-events': { events: [], next_cursor: null } },
  },
  {
    id: '12-audit-load-error',
    path: '/audit',
    errors: ['/api/v1/audit-events'],
    expect: (page) =>
      until(async () => {
        const problem = await CHECKS.noRedOnTrouble(page);
        if (problem !== null) return problem;
        // The pack keeps the chrome and swaps the body, so the controls and the
        // density step must still be on screen while the ledger is failing.
        if ((await page.getByRole('button', { name: 'Retry' }).count()) === 0) {
          return 'the error state offers no Retry';
        }
        if ((await page.getByRole('button', { name: 'Columns' }).count()) === 0) {
          return 'the toolbar disappeared behind the error';
        }
        return null;
      }),
  },

  // --- Provenance: the write trail -------------------------------------
  { id: '13-provenance-default', path: '/provenance', expect: async (page) => (await CHECKS.noBulkBar(page)) ?? (await CHECKS.smallCapsHeaders(page)) },
  {
    id: '14-provenance-agent-view',
    path: '/provenance',
    prepare: (page) => useTab(page, 'Agent'),
  },
  {
    id: '15-provenance-empty',
    path: '/provenance',
    fixtures: { '/api/v1/provenance': { events: [], next_cursor: null } },
  },
  {
    id: '16-provenance-load-error',
    path: '/provenance',
    errors: ['/api/v1/provenance'],
    expect: (page) =>
      until(async () => {
        const problem = await CHECKS.noRedOnTrouble(page);
        if (problem !== null) return problem;
        if ((await page.getByRole('button', { name: 'Retry' }).count()) === 0) {
          return 'the error state offers no Retry';
        }
        return null;
      }),
  },

  // --- Search: three drawn states, and no AI ---------------------------
  {
    id: '17-search-results',
    path: '/search',
    prepare: async (page) => {
      await page.getByRole('searchbox', { name: 'Search' }).fill('vpn');
      await page.waitForTimeout(900);
    },
    expect: CHECKS.noAiOnSearch,
  },
  {
    id: '18-search-no-results',
    path: '/search',
    fixtures: { '/api/v1/search': { query: 'zzzz', results: [], total: 0, next_cursor: null } },
    prepare: async (page) => {
      await page.getByRole('searchbox', { name: 'Search' }).fill('zzzz');
      await page.waitForTimeout(900);
    },
    expect: CHECKS.noAiOnSearch,
  },
  {
    id: '19-search-resting',
    path: '/search',
    expect: CHECKS.noAiOnSearch,
  },

  // --- Scorecards: the Planning metric tiles ---------------------------
  { id: '20-scorecards-own', path: '/scorecards' },
  { id: '21-scorecards-team', path: '/scorecards' },
];

async function main() {
  const browser = await chromium.launch({
    executablePath: process.env.CHROME_PATH || undefined,
  });
  const themes = ['neutral', 'dark'];
  const widths = [
    { width: 1600, height: 1000 },
    { width: 1280, height: 900 },
  ];

  for (const theme of themes) {
    for (const viewport of widths) {
      for (const state of STATES) {
        if (ONLY !== null && !ONLY.has(state.id)) continue;
        // The narrow pass is for the dense ledgers' responsive claim; the
        // rest of the surface is read at the pack's own capture width.
        if (viewport.width === 1280 && !/^(05|08|13)-/.test(state.id)) continue;
        await shot(browser, { viewport, theme, ...state });
      }
    }
  }

  for (const theme of themes) await contactSheet(browser, theme);
  await browser.close();
  console.log('done');
}

await main();
