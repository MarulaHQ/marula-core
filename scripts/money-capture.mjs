/**
 * Money-area capture — Wave 17 lane 5 (p17-money).
 *
 * Screenshots the lane's surfaces — /billing, /billing/{id},
 * /billing/credit-notes, /contracts, /contracts/{id}, /sales/catalog,
 * /sales/pipeline (kanban), /sales/quotes, /approvals — at
 * 1440/1024/768/480 in light AND dark, into design/qa-money/. API
 * responses are fixtures served by Playwright routing — no backend.
 * Interaction states (invoice status bar + payment dialog, quote builder
 * dialog, credit-note detail drawer, pipeline board, approvals trail)
 * at 1440, both themes.
 * Pattern: scripts/boards-dispatch-capture.mjs (Wave 17 lane 3).
 *
 * Usage: start the web app (dev on :3000), then node scripts/money-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-money');
const BASE = process.argv[2] ?? 'http://localhost:3000';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ORG = '00000000-0000-4000-8000-000000000002';
const ME = {
  user_id: '00000000-0000-4000-8000-000000000001',
  org_id: ORG,
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
  org_id: ORG,
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
      id: 'l1', invoice_id: 'i1', org_id: ORG, description: 'Managed services — August retainer', kind: 'retainer',
      period_start: '2026-08-01', period_end: '2026-08-31', quantity_minutes: 0, amount_minor: 32000,
      created_at: '2026-09-01T00:00:00Z', updated_at: '2026-09-01T00:00:00Z', created_by: null,
    },
    {
      id: 'l2', invoice_id: 'i1', org_id: ORG, description: 'On-site cabling work (2 × 5 h)', kind: 'time',
      period_start: '2026-08-14', period_end: '2026-08-21', quantity_minutes: 600, amount_minor: 10000,
      created_at: '2026-09-01T00:00:00Z', updated_at: '2026-09-01T00:00:00Z', created_by: null,
    },
  ],
  payments: [
    {
      id: 'p1', org_id: ORG, invoice_id: 'i1', amount_minor: 5000, currency: 'USD', provider: 'none',
      provider_ref: null, received_at: new Date(now - 6 * 86_400_000).toISOString(), created_by: null,
      notes: 'Partial — ACH so far',
    },
  ],
};

const CONTRACT = (id, number, name, type, status, extra = {}) => ({
  id,
  org_id: ORG,
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
  { id: 'cat1', org_id: ORG, name: 'Block hours (managed)', unit: 'hour', currency: 'USD', unit_price_minor: 15000, cost_minor: 9500, taxable: true, active: true, category: 'Services', group_name: 'Managed IT', sku: 'SRV-HRS', description: 'Prepaid managed services hours', created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z', created_by: null },
  { id: 'cat2', org_id: ORG, name: 'Firewall (hardware)', unit: 'each', currency: 'USD', unit_price_minor: 96000, cost_minor: 72000, taxable: true, active: true, category: 'Hardware', group_name: 'Network', sku: 'HW-FW-01', description: null, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z', created_by: null },
  { id: 'cat3', org_id: ORG, name: 'License seat (M365)', unit: 'seat/month', currency: 'USD', unit_price_minor: 2500, cost_minor: null, taxable: false, active: true, category: 'Licensing', group_name: null, sku: 'LIC-SEAT', description: null, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z', created_by: null },
  { id: 'cat4', org_id: ORG, name: 'Legacy drive imaging', unit: 'each', currency: 'USD', unit_price_minor: 8900, cost_minor: 4000, taxable: true, active: false, category: 'Services', group_name: null, sku: null, description: null, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z', created_by: null },
];

const STAGES = [
  { id: 'st1', org_id: ORG, name: 'Qualifying', stage_type: 'open', position: 0, active: true, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z' },
  { id: 'st2', org_id: ORG, name: 'Proposal', stage_type: 'open', position: 1, active: true, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z' },
  { id: 'st3', org_id: ORG, name: 'Negotiation', stage_type: 'open', position: 2, active: true, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z' },
  { id: 'st4', org_id: ORG, name: 'Won', stage_type: 'won', position: 10, active: true, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z' },
  { id: 'st5', org_id: ORG, name: 'Lost', stage_type: 'lost', position: 20, active: true, created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z' },
];

const DEAL = (id, name, stage_id, amount, currency = 'USD', extra = {}) => ({
  id,
  org_id: ORG,
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
  org_id: ORG,
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
  org_id: ORG,
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
  org_id: ORG,
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

const FIXTURES = {
  '/api/v1/me': ME,
  '/api/v1/org': { id: ORG, name: 'Meridian Demo Org', base_currency: 'USD', created_at: '2025-01-01T00:00:00Z' },
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
    { id: 'r1', org_id: ORG, number: 'RUN-000004', period_from: '2026-08-01', period_to: '2026-08-31', due_in_days: 30, invoice_count: 2, total_minor: 60000, currency: 'USD', created_at: new Date(now - 15 * 86_400_000).toISOString(), created_by: null },
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

const invoiceById = (id) => {
  const inv = INVOICES.find((i) => i.id === id);
  if (inv === undefined) return [];
  return [];
};

const invoiceDetail = (id) => {
  const base = INVOICES.find((i) => i.id === id);
  return { ...INVOICE_DETAIL, ...base };
};

const contractDetail = (id) => {
  const base = CONTRACTS.find((c) => c.id === id);
  return { ...CONTRACT_DETAIL, ...base };
};

async function shot(browser, viewport, theme, name, path, interactions) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', name, String(e).slice(0, 300)));
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
    const key = new URL(r.request().url()).pathname;
    void invoiceById;
    // Invoice detail pages resolve their aggregate by id.
    const invoiceMatch = key.match(/^\/api\/v1\/invoices\/([a-z0-9-]+)$/);
    if (invoiceMatch !== null) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(invoiceDetail(invoiceMatch[1])) });
      return;
    }
    const contractMatch = key.match(/^\/api\/v1\/contracts\/([a-z0-9-]+)$/);
    if (contractMatch !== null) {
      const base = CONTRACTS.find((c) => c.id === contractMatch[1]);
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ...CONTRACT_DETAIL, ...base }) });
      return;
    }
    const creditDetailMatch = key.match(/^\/api\/v1\/credit-notes\/([a-z0-9-]+)$/);
    if (creditDetailMatch !== null) {
      const note = CREDIT_NOTES.find((c) => c.id === creditDetailMatch[1]);
      await r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ ...CREDIT_NOTE_DETAIL, ...note, refunded_minor: note?.total_minor !== undefined ? 0 : 0, remaining_minor: (note?.total_minor ?? 0) - (note?.applied_minor ?? 0) }),
      });
      return;
    }
    const approvalsTrailMatch = key.match(/^\/api\/v1\/time-entries\/([a-z0-9-]+)\/approvals$/);
    if (approvalsTrailMatch !== null) {
      await r.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          { id: 'ar1', entry_id: 'e1', action: 'submitted', reason_code: null, note: null, created_at: new Date(now - 2 * 3600_000).toISOString() },
          { id: 'ar2', entry_id: 'e1', action: 'rejected', reason_code: 'missing_notes', note: 'Please describe what was done', created_at: new Date(now - 20 * 3600_000).toISOString() },
        ]),
      });
      return;
    }
    if (FIXTURES[key] !== undefined) {
      const fx = FIXTURES[key];
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(fx) });
      return;
    }
    await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
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
  { tag: 'billing', path: '/billing' },
  { tag: 'invoice', path: '/billing/i1' },
  { tag: 'credit-notes', path: '/billing/credit-notes' },
  { tag: 'contracts', path: '/contracts' },
  { tag: 'contract', path: '/contracts/ct1' },
  { tag: 'catalog', path: '/sales/catalog' },
  { tag: 'pipeline', path: '/sales/pipeline' },
  { tag: 'quotes', path: '/sales/quotes' },
  { tag: 'approvals', path: '/approvals' },
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
    await shot(browser, { width: 1440, height: 1024 }, theme, `invoice-pay-dialog-1440-${theme}.png`, '/billing/i1', async (page) => {
      await page.getByRole('button', { name: 'Record payment' }).click();
      await page.waitForTimeout(300);
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `billing-run-dialog-1440-${theme}.png`, '/billing', async (page) => {
      await page.getByRole('button', { name: 'New invoice run' }).click();
      await page.getByRole('button', { name: /Preview/ }).click();
      await page.waitForTimeout(500);
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `credit-note-detail-1440-${theme}.png`, '/billing/credit-notes', async (page) => {
      await page.getByRole('button', { name: 'CN-000101' }).click();
      await page.waitForTimeout(400);
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `quote-builder-1440-${theme}.png`, '/sales/quotes', async (page) => {
      await page.getByTestId('quotes-new').click();
      await page.waitForTimeout(400);
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `approvals-trail-1440-${theme}.png`, '/approvals', async (page) => {
      await page.getByTestId('history-e1').click();
      await page.getByTestId('approval-row-e2').getByRole('checkbox').click();
      await page.waitForTimeout(300);
    });
    await shot(browser, { width: 1440, height: 1024 }, theme, `pipeline-drag-hover-1440-${theme}.png`, '/sales/pipeline', async (page) => {
      const card = page.getByTestId('pipeline-deal-d1');
      await card.hover();
      await page.waitForTimeout(300);
    });
  } catch (e) {
    console.error('FAIL interactions', theme, e.message);
  }
}

await browser.close();
console.log('done');
