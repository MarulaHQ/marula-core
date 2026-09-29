/**
 * p22 dedupe-workbench capture — Wave 22 lane 2.
 *
 * Screenshots the lane's surfaces — /inventory/workbench (3 tabs, queue +
 * side-by-side detail, contact merge ledger) and /settings/inventory
 * (threshold, precedence, Jev workflow editor) — at 1440/1024/768/480 in
 * light AND dark, into design/qa-p22-ecosystem-dedupe-workbench/. API
 * responses are fixtures served by Playwright routing — no backend.
 * Pattern: scripts/money-capture.mjs (Wave 17 lane 5) + the Wave-19
 * auth jar (authed pages bootstrap through a mocked /auth/refresh).
 *
 * Usage: start the web app (dev on :3000), then node scripts/dedupe-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-p22-ecosystem-dedupe-workbench');
const BASE = process.argv[2] ?? 'http://localhost:3000';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ORG = '00000000-0000-4000-8000-000000000002';
const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: ORG,
  email: 'owner@marula.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['inventory:view', 'inventory:write', 'integrations:write', 'admin:org', 'tickets:create'],
};
const CLIENT_ID = 'c0000000-0000-4000-8000-000000000001';

const CI_CANDIDATE = {
  id: '11111111-1111-4111-8111-111111111111',
  primary_tag: 'LAP-0142',
  candidate_tag: 'ITG-00231',
  org_id: ORG,
  primary_config_item_id: 'a0000000-0000-4000-8000-000000000001',
  candidate_config_item_id: 'a0000000-0000-4000-8000-000000000002',
  confidence: 0.92,
  reason: 'MAC overlap 2/2 + same serial',
  evidence: { signals: [{ macs_common: ['aa:bb:cc:dd:ee:01', 'aa:bb:cc:dd:ee:02'], serials_common: ['C02X3L9QGH7H'] }] },
  status: 'pending',
  decided_by: null,
  decided_at: null,
  created_at: '2026-09-22T10:00:00Z',
  updated_at: '2026-09-22T10:00:00Z',
};
const CI_CANDIDATE_DEFERRED = {
  ...CI_CANDIDATE,
  id: '22222222-2222-4222-8222-222222222222',
  primary_config_item_id: 'a0000000-0000-4000-8000-000000000003',
  candidate_config_item_id: 'a0000000-0000-4000-8000-000000000004',
  confidence: 0.6,
  reason: 'MAC overlap 1/2',
  evidence: { signals: [{ macs_common: ['aa:bb:cc:dd:ee:09'] }] },
  status: 'deferred',
  decided_by: ME.user_id,
  decided_at: '2026-09-22T11:00:00Z',
  created_at: '2026-09-22T09:00:00Z',
  updated_at: '2026-09-22T11:00:00Z',
};
const CI_CANDIDATES = [CI_CANDIDATE, CI_CANDIDATE_DEFERRED];

const CONTACT_CANDIDATE = {
  id: '33333333-3333-4333-8333-333333333333',
  primary_label: 'Jane Okafor (ninjaone)',
  candidate_label: 'Jane Okafor (it_glue)',
  org_id: ORG,
  entity_kind: 'contact',
  primary_entity_id: 'e0000000-0000-4000-8000-000000000001',
  candidate_entity_id: 'e0000000-0000-4000-8000-000000000002',
  source_system: 'ninjaone',
  confidence: 0.85,
  reason: 'email + phone overlap',
  evidence: { signals: [{ emails_common: ['jane@acme.test'] }, { phones_common: ['+1 555 0100'] }] },
  status: 'pending',
  decided_by: null,
  decided_at: null,
  created_at: '2026-09-22T10:00:00Z',
  updated_at: '2026-09-22T10:00:00Z',
};
const CONTACT_CANDIDATES = [CONTACT_CANDIDATE];

const CI_DETAIL = {
  candidate: CI_CANDIDATE,
  primary: {
    item: {
      id: 'a0000000-0000-4000-8000-000000000001', org_id: ORG, client_id: CLIENT_ID,
      tag: 'LAP-0142', name: 'MacBook Pro 16 M3', category: 'laptop', status: 'active',
      created_at: '2025-11-02T00:00:00Z', updated_at: '2026-09-20T00:00:00Z',
    },
    source_links: [
      { source_system: 'ninjaone', external_id: 'N1-9001', org_id: ORG, config_item_id: 'a0000000-0000-4000-8000-000000000001', first_seen_at: '2025-11-02T00:00:00Z', last_seen_at: '2026-09-22T08:00:00Z' },
      { source_system: 'auvix', external_id: 'AV-7742', org_id: ORG, config_item_id: 'a0000000-0000-4000-8000-000000000001', first_seen_at: '2025-12-01T00:00:00Z', last_seen_at: '2026-09-22T08:00:00Z' },
    ],
    curated_fields: { os_version: 'macOS 15.6.1', assigned_user: 'Dana Whitfield', serial_number: 'C02X3L9QGH7H' },
    observed_fields: { os_version: 'macOS 15.6.1', assigned_user: 'Dana Whitfield', serial_number: 'C02X3L9QGH7H', last_reboot: '2026-09-21T02:11:00Z' },
    field_sources: { os_version: 'ninjaone', assigned_user: 'ninjaone', serial_number: 'auvix' },
  },
  candidate_side: {
    item: {
      id: 'a0000000-0000-4000-8000-000000000002', org_id: ORG, client_id: CLIENT_ID,
      tag: 'ITG-00231', name: 'MacBook Pro 16', category: 'laptop', status: 'active',
      created_at: '2026-01-14T00:00:00Z', updated_at: '2026-09-21T00:00:00Z',
    },
    source_links: [
      { source_system: 'it_glue', external_id: 'ITG-00231', org_id: ORG, config_item_id: 'a0000000-0000-4000-8000-000000000002', first_seen_at: '2026-01-14T00:00:00Z', last_seen_at: '2026-09-21T08:00:00Z' },
    ],
    curated_fields: { os_version: 'macOS 15.6.1', assigned_user: 'Dana Whitfield', serial_number: 'C02X3L9QGH7H' },
    observed_fields: { os_version: 'macOS 15.6.1', assigned_user: 'Dana Whitfield' },
    field_sources: { os_version: 'it_glue', assigned_user: 'it_glue' },
  },
  fields: [
    { key: 'os_version', primary_observed: 'macOS 15.6.1', primary_curated: 'macOS 15.6.1', primary_source: 'ninjaone', candidate_observed: 'macOS 15.6.1', candidate_curated: 'macOS 15.6.1', candidate_source: 'it_glue' },
    { key: 'assigned_user', primary_observed: 'Dana Whitfield', primary_curated: 'Dana Whitfield', primary_source: 'ninjaone', candidate_observed: 'Dana Whitfield', candidate_curated: 'Dana Whitfield', candidate_source: 'it_glue' },
    { key: 'serial_number', primary_observed: 'C02X3L9QGH7H', primary_curated: 'C02X3L9QGH7H', primary_source: 'auvix', candidate_observed: null, candidate_curated: 'C02X3L9QGH7H', candidate_source: 'it_glue' },
    { key: 'last_reboot', primary_observed: '2026-09-21T02:11:00Z', primary_curated: null, primary_source: 'ninjaone', candidate_observed: null, candidate_curated: null, candidate_source: null },
  ],
  related_pending: [],
  auto_merge_threshold: 0.92,
  decision: null,
};

const CONTACT_DETAIL = {
  candidate: CONTACT_CANDIDATE,
  primary: {
    contact: { id: 'e0000000-0000-4000-8000-000000000001', org_id: ORG, client_id: CLIENT_ID, name: 'Jane Okafor', email: 'jane@acme.test', phone: '+1 555 0100', created_at: '2025-06-01T00:00:00Z', updated_at: '2026-09-20T00:00:00Z' },
    source_links: [{ source_system: 'ninjaone', external_id: 'N1-C-551', org_id: ORG, entity_id: 'e0000000-0000-4000-8000-000000000001', first_seen_at: '2025-06-01T00:00:00Z', last_seen_at: '2026-09-22T08:00:00Z' }],
    curated_fields: { email: 'jane@acme.test', phone: '+1 555 0100', title: 'IT Director' },
    observed_fields: { email: 'jane@acme.test', phone: '+1 555 0100' },
    field_sources: { email: 'ninjaone', phone: 'ninjaone', title: 'hudu' },
  },
  candidate_side: {
    contact: { id: 'e0000000-0000-4000-8000-000000000002', org_id: ORG, client_id: CLIENT_ID, name: 'Jane Okafor', email: 'jane@acme.test', phone: null, created_at: '2026-02-10T00:00:00Z', updated_at: '2026-09-19T00:00:00Z' },
    source_links: [{ source_system: 'it_glue', external_id: 'ITG-P-118', org_id: ORG, entity_id: 'e0000000-0000-4000-8000-000000000002', first_seen_at: '2026-02-10T00:00:00Z', last_seen_at: '2026-09-19T08:00:00Z' }],
    curated_fields: { email: 'jane@acme.test', title: 'Director of IT' },
    observed_fields: { email: 'jane@acme.test' },
    field_sources: { email: 'it_glue', title: 'it_glue' },
  },
  fields: [
    { key: 'email', primary_observed: 'jane@acme.test', primary_curated: 'jane@acme.test', primary_source: 'ninjaone', candidate_observed: 'jane@acme.test', candidate_curated: 'jane@acme.test', candidate_source: 'it_glue' },
    { key: 'phone', primary_observed: '+1 555 0100', primary_curated: '+1 555 0100', primary_source: 'ninjaone', candidate_observed: null, candidate_curated: null, candidate_source: null },
    { key: 'title', primary_observed: null, primary_curated: 'IT Director', primary_source: 'hudu', candidate_observed: null, candidate_curated: 'Director of IT', candidate_source: 'it_glue' },
  ],
  auto_merge_threshold: 0.9,
  decision: null,
};

const ORG_MAP = [{
  id: '55555555-5555-4555-8555-555555555555', org_id: ORG, source_system: 'it_glue',
  vendor_org_id: 'org-itg-1201', client_id: CLIENT_ID, vendor_label: 'Acme Corp',
  created_at: '2026-09-20T00:00:00Z', updated_at: '2026-09-20T00:00:00Z',
}];

const CI_SETTINGS = { auto_merge_threshold: 0.92, precedence: ['serial_number', 'macs', 'source_guid'] };
const DEDUPE_WORKFLOW = {
  workflow: {
    org_id: ORG,
    enabled: true,
    confidence_threshold: 0.9,
    mode: 'report_only',
    interval_hours: null,
    notify_user_ids: [ME.user_id],
    entity_kinds: ['contact'],
    next_run_at: null,
    last_run_at: '2026-09-22T09:00:00Z',
    last_result: { scanned: 26, auto_merged: 1 },
  },
  decision_configured: true,
};

// Exact-pathname fixtures (the workbench + settings surfaces only).
const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/tickets': [],
  '/api/v1/notifications/unread-count': { count: 2 },
  '/api/v1/notifications': [],
  '/api/v1/settings/org': { name: 'Marula Demo Org', timezone: 'UTC', currency: 'USD' },
  '/api/v1/settings/user': { display_name: 'Taylor Reed', timezone: 'UTC' },
  '/api/v1/config-items/merge-candidates': CI_CANDIDATES,
  '/api/v1/config-items/merge-candidates/11111111-1111-4111-8111-111111111111/workbench': CI_DETAIL,
  '/api/v1/entity-dedupe/candidates': CONTACT_CANDIDATES,
  '/api/v1/entity-dedupe/candidates/33333333-3333-4333-8333-333333333333/workbench': CONTACT_DETAIL,
  '/api/v1/clients/org-map': ORG_MAP,
  '/api/v1/config-items/settings': CI_SETTINGS,
  '/api/v1/config-items/precedence': [
    { id: '88000000-0000-4000-8000-000000000001', org_id: ORG, layout_id: null, semantic_class: 'device', field_key: 'os_version', source_system: 'ninjaone', priority: 1, updated_at: '2026-09-20T00:00:00Z' },
    { id: '88000000-0000-4000-8000-000000000002', org_id: ORG, layout_id: null, semantic_class: 'device', field_key: 'assigned_user', source_system: 'ninjaone', priority: 2, updated_at: '2026-09-20T00:00:00Z' },
  ],
  '/api/v1/dedupe/workflow': DEDUPE_WORKFLOW,
};

async function shot(browser, viewport, theme, name, path, interactions) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', name, String(e).slice(0, 300)));
  page.on('response', (r) => { if (r.url().includes('/workbench') && r.request().resourceType() === 'fetch') console.log('NET', name, r.status(), new URL(r.url()).pathname); });
  const url = new URL(`${BASE}${path}`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');
  await page.addInitScript((t) => {
    window.localStorage.setItem('marula-theme', t);
    const style = document.createElement('style');
    style.textContent = 'nextjs-portal{display:none!important}';
    document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
  }, theme === 'dark' ? 'dark' : 'light');

  await page.route('**/api/**', async (r) => {
    const key = new URL(r.request().url()).pathname;
    const method = r.request().method();
    if (method === 'OPTIONS') return r.fulfill({ status: 204 });
    if (key === '/api/v1/auth/refresh') {
      return r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ access_token: 'test-access', refresh_token: 'test-refresh', session_id: 'sess', expires_in: 900 }),
      });
    }
    if (key === '/api/v1/dedupe/workflow/run' && method === 'POST') {
      return r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          dry_run: true,
          report: {
            scanned: 26, asked: 4, above_threshold: 2, auto_merged: 0, truncated: false,
            outcomes: [
              { label: 'MacBook Pro 16 M3 ← MacBook Pro 16', disposition: 'auto_merged' },
              { label: 'Jane Okafor (ninjaone) ← Jane Okafor (it_glue)', disposition: 'auto_merged' },
            ],
          },
        }),
      });
    }
    if (key === '/api/v1/entity-dedupe/candidates/33333333-3333-4333-8333-333333333333/accept' && method === 'POST') {
      return r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          id: '77777777-7777-4777-8777-777777777777', org_id: ORG, entity_kind: 'contact',
          primary_entity_id: 'e0000000-0000-4000-8000-000000000001', merged_entity_id: 'e0000000-0000-4000-8000-000000000002',
          source_system: 'ninjaone', status: 'merged', merged_by: ME.user_id, created_at: new Date().toISOString(),
        }),
      });
    }
    if (FIXTURES[key] !== undefined) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES[key]) });
      return;
    }
    await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
  });

  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForSelector('main', { timeout: 20_000 });
  await page.waitForTimeout(1100);
  if (interactions) await interactions(page);
  await page.screenshot({ path: `${OUT}/${name}.png`, fullPage: true });
  await context.close();
  console.log(`[dedupe-capture] ${name}`);
}

const browser = await chromium.launch();
const VIEWPORTS = [[1440, 900], [1024, 800], [768, 900], [480, 900]];

for (const theme of ['light', 'dark']) {
  for (const [w, h] of VIEWPORTS) {
    await shot(browser, { width: w, height: h }, theme, `workbench-${w}-${theme}`, '/inventory/workbench');
    await shot(browser, { width: w, height: h }, theme, `settings-inventory-${w}-${theme}`, '/settings/inventory');
  }
}

// Interaction states at 1440, both themes.
for (const theme of ['light', 'dark']) {
  await shot(browser, { width: 1440, height: 900 }, theme, `workbench-detail-1440-${theme}`, '/inventory/workbench', async (page) => {
    await page.getByTestId('workbench-row-11111111-1111-4111-8111-111111111111').getByRole('button').click();
    await page.waitForTimeout(800);
  });
  await shot(browser, { width: 1440, height: 900 }, theme, `workbench-contacts-1440-${theme}`, '/inventory/workbench', async (page) => {
    await page.getByTestId('workbench-tab-contacts').click();
    await page.waitForTimeout(800);
    await page.getByTestId('workbench-row-33333333-3333-4333-8333-333333333333').getByRole('button').click();
    await page.waitForTimeout(1000);
    // Trigger the accept so the merge ledger card renders (best-effort —
    // the ledger card is the point; the click needs the detail mounted).
    try {
      await page.getByTestId('entity-detail-accept').click({ timeout: 4000 });
      await page.waitForTimeout(900);
    } catch {
      console.log('[dedupe-capture] contacts ledger click skipped');
    }
  });
  await shot(browser, { width: 1440, height: 900 }, theme, `workbench-clients-1440-${theme}`, '/inventory/workbench', async (page) => {
    await page.getByTestId('workbench-tab-clients').click();
    await page.waitForTimeout(900);
  });
  await shot(browser, { width: 1440, height: 900 }, theme, `settings-jev-editor-1440-${theme}`, '/settings/inventory', async (page) => {
    // Open the Jev workflow editor if it is behind a control.
    const edit = page.getByRole('button', { name: /edit|configure|edit workflow/i }).first();
    if (await edit.isVisible().catch(() => false)) {
      await edit.click();
      await page.waitForTimeout(800);
    }
  });
}

await browser.close();
console.log(`[dedupe-capture] done → ${OUT}`);
