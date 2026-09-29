#!/usr/bin/env node
/**
 * Plugin-management capture — Wave 16 lane 8 (p16-plugin-skills-closeout).
 *
 * Screenshots /settings/plugins: Installed tab (detail expanded with the
 * manifest-rendered settings form + provenance chain), the Approvals
 * queue with a consent review open, and the Registry browse — at
 * 1440/1024/768/480 in light AND dark, into design/qa-plugin-mgmt/.
 * API responses are fixtures served by Playwright routing — no backend
 * needed. Pattern: scripts/workbench-capture.mjs.
 *
 * Usage: start the web app (dev on :3000), then node scripts/plugin-mgmt-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const CHROME = '/Users/iordn/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const OUT = resolve('design/qa-plugin-mgmt');
const BASE = process.argv[2] ?? 'http://localhost:3000';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ORG = '00000000-0000-4000-8000-000000000002';
const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: ORG,
  email: 'owner@marula.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_plugins', 'admin:org'],
};
const INSTALL = '00000000-0000-4000-8000-000000000001';
const STAGED = 'a6000000-0000-4000-8000-000000000001';
const SKILL_ID = 'a7000000-0000-4000-8000-000000000001';

const INSTALLS = [
  {
    id: INSTALL,
    slug: 'invoice-export',
    name: 'Invoice Export',
    version: '1.0.0',
    status: 'enabled',
    granted_capabilities: ['clients:read', 'time:read'],
    effective_permissions: [],
    installed_at: '2026-09-16T00:00:00Z',
  },
  {
    id: 'b0000000-0000-4000-8000-000000000001',
    slug: 'collector-http',
    name: 'Collector HTTP',
    version: '0.9.0',
    status: 'disabled',
    granted_capabilities: ['tickets:read'],
    effective_permissions: [],
    installed_at: '2026-09-15T00:00:00Z',
  },
];

const SETTINGS = {
  slug: 'invoice-export',
  version: '1.0.0',
  config: { default_bucket: 'retainer', auto_include_billable: true, invoice_prefix: 'AIS-' },
  settings_schema: {
    type: 'object',
    properties: {
      default_bucket: {
        type: 'string',
        title: 'Default work bucket',
        description: 'Where unbilled work lands',
        enum: ['none', 'retainer', 'project'],
      },
      auto_include_billable: {
        type: 'boolean',
        title: 'Include billable entries automatically',
        description: 'Adds billable time entries to new invoices',
      },
    },
    required: [],
  },
};

const PROVENANCE = [
  {
    id: 'pv-1',
      install_id: INSTALL,
      action: 'installed',
      source_kind: 'local_zip',
      source_url: 'zip://invoice-export-1.0.0.zip',
      fetched_ref: null,
      artifact_digest: 'c'.repeat(64),
      uploader: ME.user_id,
      recorded_at: '2026-09-16T00:00:00Z',
    },
];

const UPDATE = {
  install_id: INSTALL,
  slug: 'invoice-export',
  current_version: '1.0.0',
  pending_version: null,
  pending_digest: null,
  current_granted_capabilities: ['clients:read', 'time:read'],
  consent_diff: { added: [], removed: [], unchanged: ['clients:read', 'time:read'] },
};

const STAGED_ROWS = [
  {
    id: STAGED,
    artifact_kind: 'plugin',
    source_kind: 'local_zip',
    source_url: 'zip://invoice-export-1.2.0.zip',
    fetched_ref: null,
    artifact_digest: 'a'.repeat(64),
    status: 'pending',
    declared_capabilities: ['clients:read', 'financial:write'],
    manifest: { name: 'invoice-export', version: '1.2.0' },
    uploaded_by: 'c0000000-0000-4000-8000-000000000001',
    review_note: null,
    adopted_install_id: null,
    adopted_skill_id: null,
  },
  {
    id: 'a6000000-0000-4000-8000-000000000002',
    artifact_kind: 'skill',
    source_kind: 'skill_community',
    source_url: 'https://skills.example.dev/vpn-triage/manifest.json',
    fetched_ref: 'v2.0.1',
    artifact_digest: 'b'.repeat(64),
    status: 'refused',
    declared_capabilities: ['tickets:read'],
    manifest: { name: 'vpn-triage', version: '2.0.1' },
    uploaded_by: ME.user_id,
    review_note: 'digest did not match the upstream tag',
    adopted_install_id: null,
    adopted_skill_id: null,
  },
];

const CONSENT = {
  artifact_kind: 'plugin',
  source_kind: 'local_zip',
  source_url: 'zip://invoice-export-1.2.0.zip',
  fetched_ref: null,
  artifact_digest: 'a'.repeat(64),
  declared_capabilities: ['clients:read', 'financial:write'],
  settings_schema: {
    type: 'object',
    properties: {
      default_bucket: { type: 'string', enum: ['none', 'retainer'] },
    },
    required: [],
  },
};

const SKILLS = [
  {
    id: SKILL_ID,
    slug: 'vpn-triage',
    version: 2,
    enabled: false,
    manifest: { name: 'vpn-triage', version: '1.0.2' },
    source_kind: 'skill_community',
    source_url: 'https://skills.example.dev/vpn-triage/manifest.json',
    fetched_ref: 'v1.0.2',
    artifact_digest: 'b'.repeat(64),
    created_at: '2026-09-15T00:00:00Z',
  },
  {
    id: 'a7000000-0000-4000-8000-000000000002',
    slug: 'vpn-triage',
    version: 1,
    enabled: false,
    manifest: { name: 'vpn-triage', version: '1.0.0' },
    source_kind: 'skill_community',
    source_url: 'https://skills.example.dev/vpn-triage/manifest.json',
    fetched_ref: 'v1.0.0',
    artifact_digest: 'd'.repeat(64),
    created_at: '2026-09-10T00:00:00Z',
  },
];

const CATALOG = [
  {
    id: 'a8000000-0000-4000-8000-000000000001',
    slug: 'collector-http',
    name: 'Collector HTTP',
    publisher: 'Marula Labs',
    description: 'Webhook collector for inbound client events.',
    latest_version: '0.9.0',
    revoked: false,
    created_at: '2026-09-01T00:00:00Z',
    interface_version: 1,
  },
  {
    id: 'a8000000-0000-4000-8000-000000000002',
    slug: 'sla-watchdog',
    name: 'SLA Watchdog',
    publisher: 'Marula Labs',
    description: 'WASM watchdog that flags at-risk tickets.',
    latest_version: '2.1.0',
    revoked: false,
    created_at: '2026-09-01T00:00:00Z',
    interface_version: 1,
  },
  {
    id: 'a8000000-0000-4000-8000-000000000003',
    slug: 'legacy-importer',
    name: 'Legacy Importer',
    publisher: 'Acme',
    description: 'Deprecated importer — revoked.',
    latest_version: '0.4.2',
    revoked: true,
    created_at: '2026-09-01T00:00:00Z',
    interface_version: 1,
  },
];

const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/plugins/installed': INSTALLS,
  [`/api/v1/plugins/installed/${INSTALL}/settings`]: SETTINGS,
  [`/api/v1/plugins/installed/${INSTALL}/provenance`]: PROVENANCE,
  [`/api/v1/plugins/installed/${INSTALL}/update`]: UPDATE,
  '/api/v1/plugins/distribution/staged': STAGED_ROWS,
  [`/api/v1/plugins/distribution/staged/${STAGED}/consent`]: CONSENT,  '/api/v1/plugins/distribution/agent-skills': SKILLS,
  '/api/v1/plugins/catalog': CATALOG,
  // Shell fixtures (same keys the workbench capture serves) keep the
  // nav rail / timer dock / notification surfaces from crashing on
  // unmatched routes.
  '/api/v1/tickets/my-work': [],
  '/api/v1/timers/working-set': [{ id: 'sess-1', org_id: ORG, user_id: ME.user_id, ticket_id: 't1', started_at: '2026-09-17T00:00:00Z', stopped_at: null, segments: [{ started_at: '2026-09-17T00:00:00Z', ended_at: null }] }],
  '/api/v1/timers/active': { id: 'sess-1', org_id: ORG, user_id: ME.user_id, ticket_id: 't1', started_at: '2026-09-17T00:00:00Z', stopped_at: null, segments: [{ started_at: '2026-09-17T00:00:00Z', ended_at: null }] },
  '/api/v1/notifications/unread-count': { count: 0 },
  '/api/v1/notifications': { items: [], unread_count: 0, total: 0, next_cursor: null },
  '/api/v1/tickets': [],
  '/api/v1/boards': [],
  '/api/v1/clients': [],
  '/api/v1/user': {},
};

async function shot(browser, viewport, theme, name, interactions) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', String(e).slice(0, 300)));
  const url = new URL(`${BASE}/settings/plugins`);
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
    if (FIXTURES[key]) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES[key]) });
    } else {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ data: [], grants: [], entries: [] }) });
    }
  });

  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForSelector('[data-testid="tab-installed"]', { timeout: 20_000 });
  await page.waitForTimeout(600);
  if (interactions) await interactions(page);
  await page.waitForTimeout(500);
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

const expandFirst = async (page) => {
  await page.getByTestId('expand-invoice-export').click();
  await page.waitForSelector('[data-testid="settings-schema-block"]', { timeout: 10_000 }).catch(() => {});
};

for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    try {
      await shot(browser, vp, theme, `plugins-installed-${vp.width}-${theme}.png`, expandFirst);
    } catch (e) {
      console.error('FAIL', vp.width, theme, e.message);
    }
  }
}

for (const theme of ['light', 'dark']) {
  try {
    await shot(browser, { width: 1440, height: 1024 }, theme, `plugins-approvals-consent-1440-${theme}.png`, async (page) => {
      await page.getByTestId('tab-approvals').click();
      await page.getByTestId(`staged-row-${STAGED}`).getByRole('button', { name: 'Review consent' }).click();
      await page.waitForTimeout(700);
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `plugins-registry-1440-${theme}.png`, async (page) => {
      await page.getByTestId('tab-registry').click();
      await page.waitForTimeout(600);
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `plugins-agent-skills-1440-${theme}.png`, async (page) => {
      await page.getByTestId('tab-agent-skills').click();
      await page.waitForTimeout(600);
    });
  } catch (e) {
    console.error('FAIL extra', theme, e.message);
  }
}

await browser.close();
console.log('done');
