#!/usr/bin/env node
/**
 * Self-host services pane capture — Wave 36 lane 3 (p36-selfhost-visibility).
 *
 * Screenshots /settings/self-host with Playwright route fixtures (no backend):
 * the mixed stack (one down), the all-green / nothing-configured state, and
 * the scanner-off state — at 1440/1024/768/480 in light AND dark for the
 * mixed stack, 1440 both themes for the other two states.
 *
 * Pattern: scripts/dedupe-capture.mjs (auth via a mocked /auth/refresh,
 * memory-only token jar) and scripts/settings-capture.mjs.
 *
 * Usage: start the web app (dev on :3000), then
 *   node scripts/selfhost-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-p36-selfhost-visibility/captures');
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

const SERVICE = (id, label, state, detail) => ({ id, label, state, detail, loops: null });

const MIXED = {
  scan: {
    provider: 'clamd',
    clamd_addr: 'clamav:3310',
    last_scan_at: '2026-09-26T08:00:00Z',
    last_detection_at: '2026-09-26T08:05:00Z',
    detection_count: 3,
  },
  detections: [
    {
      message_id: '00000000-0000-4000-8000-0000000000d1',
      subject: 'Invoice attached',
      from_address: 'accounts@supplier.example',
      verdict: 'infected',
      ingest_source: 'webhook/clamd',
      occurred_at: '2026-09-26T08:05:00Z',
      attachments: [
        { filename: 'invoice-4471.pdf.exe', content_type: 'application/octet-stream', size_bytes: 284672, sha256: 'a1b2c3' },
      ],
    },
    {
      message_id: '00000000-0000-4000-8000-0000000000d2',
      subject: null,
      from_address: 'billing@stray-mailer.example',
      verdict: 'infected',
      ingest_source: 'webhook/clamd',
      occurred_at: '2026-09-25T17:42:00Z',
      attachments: [
        { filename: 'statement.docm', content_type: 'application/vnd.ms-word.document.macroEnabled.12', size_bytes: 1183744, sha256: 'd4e5f6' },
        { filename: 'statement.xlsm', content_type: 'application/vnd.ms-excel.sheet.macroEnabled.12', size_bytes: 482304, sha256: 'a7b8c9' },
      ],
    },
  ],
  services: [
    SERVICE('core_api', 'core-api', 'up', 'served this request'),
    SERVICE('database', 'PostgreSQL', 'up', 'SELECT 1 answered'),
    SERVICE('valkey', 'Valkey (Redis)', 'up', 'valkey:6379 answered PING'),
    SERVICE('realtime', 'realtime hub', 'up', 'http://realtime:8081/healthz: 200 OK'),
    SERVICE('worker', 'worker', 'down', 'http://worker:8082/healthz: timed out'),
  ],
  glitchtip_dsns: [
    { service: 'core-api', state: 'configured', detail: "read from this process's environment" },
    { service: 'realtime', state: 'unset', detail: "reported by the service's /healthz" },
    { service: 'worker', state: 'unknown', detail: "unknown — the service's /healthz did not answer" },
  ],
  links: {
    glitchtip: null,
    grafana: 'http://localhost:3001',
    tempo: null,
    loki: null,
    otel_metrics: 'http://localhost:18899',
  },
  garage: {
    volumes: ['garagedata', 'garagemeta'],
    container_paths: ['/var/lib/garage/data', '/var/lib/garage/meta'],
    bootstrap_commands: [
      'docker compose --profile stage2 up -d garage',
      'docker compose --profile stage2 exec garage /garage status',
      'docker compose --profile stage2 exec garage /garage layout assign -z dc1 -c 1G <NODE_ID>',
      'docker compose --profile stage2 exec garage /garage layout apply --version 1',
      'docker compose --profile stage2 exec garage /garage bucket create marula',
      'docker compose --profile stage2 exec garage /garage key create marula',
      'docker compose --profile stage2 exec garage /garage bucket allow --read --write marula --key marula',
    ],
    admin_api_docs_url: 'https://garagehq.deuxfleurs.fr/documentation/reference-manual/admin-api/',
    s3_endpoint: 'http://garage:3900',
    s3_bucket: 'marula',
  },
};

const UNCONFIGURED = {
  scan: { provider: 'clamd', clamd_addr: 'clamav:3310', last_scan_at: null, last_detection_at: null, detection_count: 0 },
  detections: [],
  services: [
    SERVICE('core_api', 'core-api', 'up', 'served this request'),
    SERVICE('database', 'PostgreSQL', 'up', 'SELECT 1 answered'),
    SERVICE('valkey', 'Valkey (Redis)', 'up', 'valkey:6379 answered PING'),
    SERVICE('realtime', 'realtime hub', 'not_configured', 'health URL is not set for this process'),
    SERVICE('worker', 'worker', 'not_configured', 'health URL is not set for this process'),
  ],
  glitchtip_dsns: [
    { service: 'core-api', state: 'unset', detail: "read from this process's environment" },
    { service: 'realtime', state: 'unknown', detail: "unknown — the service's /healthz did not answer" },
    { service: 'worker', state: 'unknown', detail: "unknown — the service's /healthz did not answer" },
  ],
  links: { glitchtip: null, grafana: null, tempo: null, loki: null, otel_metrics: null },
  garage: {
    volumes: ['garagedata', 'garagemeta'],
    container_paths: ['/var/lib/garage/data', '/var/lib/garage/meta'],
    bootstrap_commands: MIXED.garage.bootstrap_commands,
    admin_api_docs_url: MIXED.garage.admin_api_docs_url,
    s3_endpoint: null,
    s3_bucket: null,
  },
};

const NONE_MODE = {
  scan: { provider: 'none', clamd_addr: null, last_scan_at: '2026-09-26T08:00:00Z', last_detection_at: null, detection_count: 0 },
  detections: [],
  services: MIXED.services.map((s) => (s.id === 'worker' ? SERVICE('worker', 'worker', 'up', 'http://worker:8082/healthz: 200 OK') : s)),
  glitchtip_dsns: [
    { service: 'core-api', state: 'configured', detail: "read from this process's environment" },
    { service: 'realtime', state: 'configured', detail: "reported by the service's /healthz" },
    { service: 'worker', state: 'configured', detail: "reported by the service's /healthz" },
  ],
  links: {
    glitchtip: 'http://localhost:8010',
    grafana: 'http://localhost:3001',
    tempo: 'http://localhost:3200',
    loki: 'http://localhost:3100',
    otel_metrics: 'http://localhost:18899',
  },
  garage: UNCONFIGURED.garage,
};

const ROUTES = {
  '/api/v1/me': ME,
  '/api/v1/notifications/unread-count': { count: 0 },
  '/api/v1/notifications': { items: [], unread_count: 0, total: 0, next_cursor: null },
  '/api/v1/boards/nav': [
    { id: 'b1', name: 'Service Desk', description: null, group_id: null, group_name: null, favorite: true, is_default: true, my_role: 'admin' },
  ],
};

async function shot(browser, viewport, theme, name, selfHost) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', name, String(e).slice(0, 300)));
  const url = new URL(`${BASE}/settings/self-host`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');
  await page.addInitScript((t) => {
    window.localStorage.setItem('marula-theme', t);
    const style = document.createElement('style');
    style.textContent = 'nextjs-portal{display:none!important}';
    document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
  }, theme === 'dark' ? 'dark' : 'light');

  await page.route('**/api/**', async (r) => {
    const key = new URL(r.request().url()).pathname;
    if (r.request().method() === 'OPTIONS') return r.fulfill({ status: 204 });
    if (key === '/api/v1/auth/refresh') {
      return r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ access_token: 'test-access', refresh_token: 'test-refresh', session_id: 'sess', expires_in: 900 }),
      });
    }
    if (key === '/api/v1/settings/self-host') {
      return r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(selfHost) });
    }
    if (ROUTES[key] !== undefined) {
      return r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(ROUTES[key]) });
    }
    return r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
  });

  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForSelector('[data-testid^="self-host-service-"]', { timeout: 20_000 });
  await page.waitForTimeout(600);
  await page.screenshot({ path: resolve(OUT, name), fullPage: true });
  console.log('captured', name);
  await context.close();
}

const viewports = [
  { width: 1440, height: 1024 },
  { width: 1024, height: 768 },
  { width: 768, height: 1024 },
  { width: 480, height: 900 },
];

const browser = await chromium.launch({
  executablePath: process.env.CHROME_PATH,
  args: ['--no-sandbox', '--disable-dev-shm-usage'],
});

// Mixed stack: the full viewport/theme matrix (the primary review set).
for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    try {
      await shot(browser, vp, theme, `mixed-${vp.width}-${theme}.png`, MIXED);
    } catch (e) {
      console.error('FAIL mixed', vp.width, theme, e.message);
    }
  }
}

// The two honest alternative states at 1440, both themes.
for (const theme of ['light', 'dark']) {
  try {
    await shot(browser, { width: 1440, height: 1024 }, theme, `unconfigured-1440-${theme}.png`, UNCONFIGURED);
  } catch (e) {
    console.error('FAIL unconfigured', theme, e.message);
  }
  try {
    await shot(browser, { width: 1440, height: 1024 }, theme, `none-mode-1440-${theme}.png`, NONE_MODE);
  } catch (e) {
    console.error('FAIL none-mode', theme, e.message);
  }
}

await browser.close();
console.log('done →', OUT);
