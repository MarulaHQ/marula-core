#!/usr/bin/env node
/**
 * Settings-shell narrow-viewport probe + capture — Wave 16 lane 10
 * (p16-followup-sweep, item (c)).
 *
 * Purpose: prove the shared settings-shell layout no longer clips content
 * at 768/480 (the pre-existing lane-8 finding: pane scrollWidth ==
 * viewport with unreachable content), across settings index / general /
 * AI / integrations / plugins, light AND dark. Evidence:
 * design/qa-followup-sweep/ (PNGs + probe result JSON).
 *
 * Reachability rule: the document may not scroll horizontally (a sticky
 * chrome artifact would read as reachable), so the probe asserts that no
 * visible element's bounding box extends past the viewport's right edge
 * beyond a small tolerance — i.e. content is either visible or inside an
 * intentional per-pane `overflow-x-auto` scroller.
 *
 * Usage: start the web app (dev on :3000), then
 *   node scripts/followup-sweep-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir, writeFile } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const CHROME = process.env.CHROME_PATH
  ?? '/Users/iordn/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const OUT = resolve('design/qa-followup-sweep');
const BASE = process.argv[2] ?? 'http://localhost:3000';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ORG = '00000000-0000-4000-8000-000000000002';
const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: ORG,
  email: 'owner@meridian.test',
  display_name: 'Taylor Reed',
  timezone: 'UTC',
  permissions: ['special:manage_plugins', 'admin:org'],
};
const INSTALL = '00000000-0000-4000-8000-000000000001';

const INSTALLS = [
  {
    id: INSTALL,
    slug: 'invoice-export',
    name: 'Invoice Export',
    version: '1.0.0',
    status: 'enabled',
    granted_capabilities: ['clients.read', 'time.read'],
    artifact_kind: 'wasm',
    effective_permissions: [],
    installed_at: '2026-09-16T00:00:00Z',
  },
  {
    id: 'b0000000-0000-4000-8000-000000000001',
    slug: 'collector-http',
    name: 'Collector HTTP',
    version: '0.9.0',
    status: 'disabled',
    granted_capabilities: ['tickets.read'],
    artifact_kind: 'http_connector',
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
      default_bucket: { type: 'string', title: 'Default work bucket', enum: ['none', 'retainer', 'project'] },
      auto_include_billable: { type: 'boolean', title: 'Include billable entries automatically' },
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
  current_granted_capabilities: ['clients.read', 'time.read'],
  consent_diff: { added: [], removed: [], unchanged: ['clients.read', 'time.read'] },
};

const STAGED_ROWS = [
  {
    id: 'a6000000-0000-4000-8000-000000000001',
    artifact_kind: 'plugin',
    source_kind: 'local_zip',
    source_url: 'zip://invoice-export-1.2.0.zip',
    fetched_ref: null,
    artifact_digest: 'a'.repeat(64),
    status: 'pending',
    declared_capabilities: ['clients.read', 'financial.write'],
    manifest: { name: 'invoice-export', version: '1.2.0' },
    uploaded_by: 'c0000000-0000-4000-8000-000000000001',
    review_note: null,
    adopted_install_id: null,
    adopted_skill_id: null,
  },
];

const SKILLS = [
  {
    id: 'a7000000-0000-4000-8000-000000000001',
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
];

const CATALOG = [
  {
    id: 'a8000000-0000-4000-8000-000000000001',
    slug: 'collector-http',
    name: 'Collector HTTP',
    publisher: 'Meridian Labs',
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
    publisher: 'Meridian Labs',
    description: 'WASM watchdog that flags at-risk tickets.',
    latest_version: '2.1.0',
    revoked: false,
    created_at: '2026-09-01T00:00:00Z',
    interface_version: 1,
  },
];

const GENERIC_LIST = { data: [], grants: [], entries: [] };

const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/plugins/installed': INSTALLS,
  [`/api/v1/plugins/installed/${INSTALL}/settings`]: SETTINGS,
  [`/api/v1/plugins/installed/${INSTALL}/provenance`]: PROVENANCE,
  [`/api/v1/plugins/installed/${INSTALL}/update`]: UPDATE,
  '/api/v1/plugins/distribution/staged': STAGED_ROWS,
  '/api/v1/plugins/distribution/agent-skills': SKILLS,
  '/api/v1/plugins/catalog': CATALOG,
  // Shell fixtures keep the nav rail / timer dock / notification surfaces
  // from crashing on unmatched routes.
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

const PAGES = [
  { path: '/settings', label: 'index' },
  { path: '/settings/general', label: 'general' },
  { path: '/settings/ai', label: 'ai' },
  { path: '/settings/integrations', label: 'integrations' },
  { path: '/settings/plugins', label: 'plugins' },
];

// Content wider than the viewport must sit inside an intentional
// overflow-x-auto scroller; anything else is the clip.
const REACHABILITY_JS = () => {
  const vw = document.documentElement.clientWidth;
  const offenders = [];
  const inScroller = (el) => {
    for (let n = el.parentElement; n; n = n.parentElement) {
      const s = getComputedStyle(n);
      if (/(auto|scroll)/.test(`${s.overflowX}${s.overflow}`) && n.scrollWidth > n.clientWidth) return true;
    }
    return false;
  };
  for (const el of document.querySelectorAll('body *')) {
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) continue;
    const s = getComputedStyle(el);
    if (s.display === 'none' || s.visibility === 'hidden') continue;
    if (el.closest('nav[aria-label="Settings sections"]')) continue; // the nav strip intentionally scrolls horizontally
    if (r.right > vw + 1 && !inScroller(el)) {
      offenders.push({ tag: el.tagName, cls: String(el.className).slice(0, 60), right: Math.round(r.right) });
    }
  }
  return { vw, offenders: offenders.slice(0, 8), offenderCount: offenders.length };
};

async function probe(browser, viewport, theme, page_, name) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', String(e).slice(0, 200)));
  const url = new URL(`${BASE}${page_.path}`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');
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
  }, theme === 'dark' ? 'dark' : 'light');

  await page.route('**/api/**', async (r) => {
    const key = new URL(r.request().url()).pathname;
    const fixture = FIXTURES[key];
    await r.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(fixture ?? GENERIC_LIST),
    });
  });

  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForTimeout(1500);
  const probeResult = await page.evaluate(REACHABILITY_JS);
  await page.screenshot({ path: resolve(OUT, name), fullPage: true });
  await context.close();
  return probeResult;
}

const browser = await chromium.launch({ executablePath: CHROME, args: ['--no-sandbox', '--disable-dev-shm-usage'] });

const results = [];
let failures = 0;
for (const vp of [{ width: 1440, height: 1024 }, { width: 768, height: 1024 }, { width: 480, height: 900 }]) {
  for (const theme of ['light', 'dark']) {
    for (const p of PAGES) {
      const name = `${p.label}-${vp.width}-${theme}.png`;
      try {
        const r = await probe(browser, vp, theme, p, name);
        const ok = r.offenderCount === 0;
        if (!ok) failures += 1;
        results.push({ page: p.label, width: vp.width, theme, ok, offenders: r.offenders });
        console.log(`${ok ? 'PASS' : 'FAIL'} ${p.label} ${vp.width} ${theme}${ok ? '' : ` — ${JSON.stringify(r.offenders)}`}`);
      } catch (e) {
        failures += 1;
        results.push({ page: p.label, width: vp.width, theme, ok: false, error: String(e).slice(0, 300) });
        console.error('FAIL', p.label, vp.width, theme, e.message);
      }
    }
  }
}

await writeFile(resolve(OUT, 'viewport-probe.json'), JSON.stringify({ failures, results }, null, 2));
console.log(failures === 0 ? 'ALL REACHABLE — done' : `${failures} probe failures — see design/qa-followup-sweep/viewport-probe.json`);
await browser.close();
process.exitCode = failures === 0 ? 0 : 1;
