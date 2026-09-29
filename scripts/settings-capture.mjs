#!/usr/bin/env node
/**
 * Settings capture — Wave 17 lane 7 (p17-settings).
 *
 * Screenshots the settings area surfaces — every /settings/* page
 * (general, team, boards, taxonomy, sla, workflow, tags, fx, mail,
 * notifications, security, api-keys, integrations, ai, agent-platform,
 * portal, pm-schedules, preferences, export) — at 1440/1024/768/480 in
 * light AND dark, into design/qa-settings/. Plus interaction states at
 * 1440 both themes: general with a dirty SaveBar, team deactivate dialog,
 * pm-schedules delete dialog, export-ready result card. API responses are
 * fixtures served by Playwright routing — no backend needed.
 * Pattern: scripts/boards-dispatch-capture.mjs (Wave 17 lane 3).
 *
 * Usage: start the web app (dev on :3000), then node scripts/settings-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-settings');
const BASE = process.argv[2] ?? 'http://localhost:3000';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: '00000000-0000-4000-8000-000000000002',
  email: 'owner@marula.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_users', 'special:manage_boards', 'tickets:create', 'admin:org'],
};

const ORG = {
  id: '00000000-0000-4000-8000-000000000002',
  name: 'Marula IT Ltd',
  slug: 'marula',
  ticket_prefix: 'MER',
  base_currency: 'USD',
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
    { key: 'connectors.dms_sync', label: 'DMS sync connector (IT Glue / Hudu ↔ Marula docs)', description: 'Operator-side toggle for the documentation-system sync.', scope: 'org', section: 'integrations', visibility: 'ui', kind: { type: 'boolean', value: null }, options: [], default: true },
  ],
};

const VALUES = [{ key: 'org_default_timezone', value: 'UTC' }];

const USERS = {
  users: [
    {
      id: 'u1',
      org_id: ORG.id,
      email: 'taylor@marula.test',
      display_name: 'Taylor Reed',
      timezone: 'UTC',
      is_active: true,
      created_at: new Date(Date.now() - 90 * 864e5).toISOString(),
      is_self: true,
    },
    {
      id: 'u2',
      org_id: ORG.id,
      email: 'ada@marula.test',
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
  url: 'https://ops.example.com/hooks/marula',
  description: null,
  event_types: ['ticket.created', 'ticket.updated'],
  active: true,
  created_at: new Date(Date.now() - 10 * 864e5).toISOString(),
});

// Empty-department fixture pages rely on the catch-all ([]); the ones that
// render meaningful content at review time get real fixtures here.
const FIXTURES = {
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
  '/api/v1/roles': { portal_tiers: [], presets: [], roles: [] },
};

async function shot(browser, viewport, theme, name, path, interactions) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', name, String(e).slice(0, 300), '\nSTACK', String(e.stack ?? '').split('\n').slice(1, 3).join('\n')));
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
    if (key === '/api/v1/org/exports' && r.request().method() === 'POST') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(EXPORT_CREATED) });
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
  { tag: 'general', path: '/settings/general' },
  { tag: 'team', path: '/settings/team' },
  { tag: 'boards', path: '/settings/boards' },
  { tag: 'taxonomy', path: '/settings/taxonomy' },
  { tag: 'sla', path: '/settings/sla' },
  { tag: 'workflow', path: '/settings/workflow' },
  { tag: 'tags', path: '/settings/tags' },
  { tag: 'fx', path: '/settings/fx' },
  { tag: 'mail', path: '/settings/mail' },
  { tag: 'notifications', path: '/settings/notifications' },
  { tag: 'security', path: '/settings/security' },
  { tag: 'api-keys', path: '/settings/api-keys' },
  { tag: 'integrations', path: '/settings/integrations' },
  { tag: 'ai', path: '/settings/ai' },
  { tag: 'agent-platform', path: '/settings/agent-platform' },
  { tag: 'portal', path: '/settings/portal' },
  { tag: 'pm-schedules', path: '/settings/pm-schedules' },
  { tag: 'preferences', path: '/settings/preferences' },
  { tag: 'export', path: '/settings/export' },
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
    await shot(browser, { width: 1440, height: 1024 }, theme, `general-dirty-1440-${theme}.png`, '/settings/general', async (page) => {
      const input = page.locator('input').first();
      const current = await input.inputValue();
      await input.fill(`${current} Ltd`);
      await page.waitForTimeout(300);
    });
  } catch (e) {
    console.error('FAIL general-dirty', theme, e.message);
  }
  try {
    await shot(browser, { width: 1440, height: 1024 }, theme, `team-deactivate-1440-${theme}.png`, '/settings/team', async (page) => {
      const button = page.getByRole('button', { name: 'Deactivate' }).nth(1);
      await button.click();
      await page.waitForTimeout(300);
    });
  } catch (e) {
    console.error('FAIL team-deactivate', theme, e.message);
  }
  try {
    await shot(browser, { width: 1440, height: 1024 }, theme, `export-ready-1440-${theme}.png`, '/settings/export', async (page) => {
      const create = page.getByTestId('export-create');
      await create.click();
      await page.waitForTimeout(600);
    });
  } catch (e) {
    console.error('FAIL export-ready', theme, e.message);
  }
}

await browser.close();
console.log('done →', OUT);
