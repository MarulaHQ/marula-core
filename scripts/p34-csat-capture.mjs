// p34-csat-surveys — live capture of the three new UI surfaces against the
// booted e2e stack (real login, real data, no mocks).
// Usage: source e2e/.e2e-stack.env first (E2E_API_URL / E2E_WEB_URL /
// E2E_DATABASE_URL), then: node scripts/p34-csat-capture.mjs <out-dir> <pg-container>
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';
import { execSync } from 'child_process';

const OUT = resolve(process.argv[2] ?? 'design/qa-p34-csat-surveys/captures');
const PG = process.argv[3] ?? '';
const API = (process.env.E2E_API_URL ?? 'http://localhost:8080').replace(/\/+$/, '');
const WEB = process.env.E2E_WEB_URL ?? 'http://localhost:3000';
const OWNER = 'owner@demo.local';
const PASS = 'demo-password-123';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

async function api(path, { method = 'GET', token, body } = {}) {
  const res = await fetch(`${API}/api/v1${path}`, {
    method,
    headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let json;
  try { json = JSON.parse(text); } catch { json = text; }
  if (!res.ok) throw new Error(`${method} ${path} -> ${res.status}: ${text.slice(0, 300)}`);
  return json;
}

// --- staff login ------------------------------------------------------------
const login = await api('/auth/login', { method: 'POST', body: { email: OWNER, password: PASS } });
const staff = login.access_token;
console.log('staff login ok');

// --- seed: client, board exposure, portal contact, ticket, survey row -------
const client = await api('/clients', { method: 'POST', token: staff, body: { name: 'Northwind Traders' } });
const boards = await api('/boards', { token: staff });
const board = boards[0];
await api(`/clients/${client.id}/board-exposures`, { method: 'PUT', token: staff, body: { board_id: board.id, exposed: true, visible_status_ids: [] } });
const email = `csat-capture-${Date.now()}@northwind.test`;
await api(`/clients/${client.id}/portal-users`, { method: 'POST', token: staff, body: { email, display_name: 'Northwind Contact', password: 'portal-passphrase-1', role: 'contact' } });
const ticket = await api('/tickets', { method: 'POST', token: staff, body: { board_id: board.id, title: 'Replace failed UPS battery bank', priority: 'p2', client_id: client.id } });
console.log('seeded client/ticket/portal user', ticket.id);

// Resolve org + insert a SENT survey with a known raw token via psql.
const rawToken = `tok${Date.now().toString(16)}`;
const orgId = (await api('/me', { token: staff })).org_id;
const crypto = await import('crypto');
const hash = crypto.createHash('sha256').update(rawToken).digest('hex');
execSync(
  `docker exec ${PG} psql -U postgres -d "${process.env.E2E_DATABASE_URL.split('/').pop()}" -c "INSERT INTO csat_surveys (id, org_id, ticket_id, client_id, contact_email, token_hash, token_expires_at, status, scheduled_for) VALUES (gen_random_uuid(), '${orgId}', '${ticket.id}', '${client.id}', '${email}', '${hash}', now() + interval '14 days', 'sent', now())"`,
  { stdio: 'pipe' },
);
console.log('survey row seeded, raw token ready');

// --- browser captures -------------------------------------------------------
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });

// Staff surfaces: log in through the UI so the web session jar is real.
await page.goto(`${WEB}/login`);
await page.getByLabel(/email/i).fill(OWNER);
await page.locator('input[type="password"]').first().fill(PASS);
await page.getByRole('button', { name: /sign in|log in/i }).click();
await page.waitForURL(/tickets|today|dashboard/, { timeout: 30000 }).catch(() => {});
await page.waitForTimeout(1200);

await page.goto(`${WEB}/settings/surveys`);
await page.waitForTimeout(1500);
await page.screenshot({ path: `${OUT}/settings-surveys-light.png`, fullPage: true });

await page.goto(`${WEB}/scorecards`);
await page.waitForTimeout(2000);
await page.screenshot({ path: `${OUT}/scorecards-csat.png`, fullPage: true });

// Portal surface: real page code, mocked transport (the auth-flows
// capture pattern) — the portal shell's in-memory auth jar cannot survive
// a scripted cross-page login, and the visual target is the CSAT response
// view itself.
const portalView = {
  ticket_id: ticket.id,
  ticket_number: 'T-40231',
  ticket_subject: 'Replace failed UPS battery bank',
  status: 'valid',
  expires_at: new Date(Date.now() + 14 * 86400_000).toISOString(),
};
const portalPage = await browser.newPage({ viewport: { width: 1440, height: 900 } });
await portalPage.route((url) => url.pathname.includes('/api/'), async (r) => {
  const key = new URL(r.request().url()).pathname.replace(/\/+api/, '/api');
  if (key === '/api/v1/portal/me') {
    await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ portal_user: { id: 'u1', email, display_name: 'Northwind Contact', client_id: 'c1', role: 'contact' } }) });
    return;
  }
  if (key.startsWith('/api/v1/portal/csat/') && r.request().method() === 'POST') {
    portalView.status = 'responded';
    await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(portalView) });
    return;
  }
  if (key.startsWith('/api/v1/portal/csat/')) {
    await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(portalView) });
    return;
  }
  await r.fulfill({ status: 401, contentType: 'application/json', body: JSON.stringify({ error: { code: 401, message: 'unauthenticated' } }) });
});
await portalPage.addInitScript(() => {
  const style = document.createElement('style');
  style.textContent = 'nextjs-portal{display:none!important}';
  document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
});
await portalPage.goto(`${WEB}/portal/csat?token=${rawToken}`);
await portalPage.waitForTimeout(2000);
await portalPage.screenshot({ path: `${OUT}/portal-csat-form.png`, fullPage: true });

await portalPage.getByTestId('csat-score-4').click();
await portalPage.locator('[data-testid="csat-comment"]').fill('Fast fix, clear updates throughout.');
await portalPage.getByTestId('csat-submit').click();
await portalPage.waitForTimeout(1500);
await portalPage.screenshot({ path: `${OUT}/portal-csat-thanks.png`, fullPage: true });
await portalPage.close();

await browser.close();
console.log('captures written to', OUT);
