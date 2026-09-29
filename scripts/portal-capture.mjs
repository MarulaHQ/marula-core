#!/usr/bin/env node
/**
 * Client portal capture — Wave 17 lane 8 (p17-portal).
 *
 * Screenshots the client-facing portal — /portal (login),
 * /portal/tickets (list), /portal/tickets/{id} (detail), /portal/invoices,
 * /portal/invoices/{id}, /portal/kb, /portal/kb/articles/{id},
 * /portal/schedule, plus the public /s/{token} booking page and
 * /accept-invite — at 1440/1024/768/480 in light AND dark, into
 * design/qa-portal/. API responses are fixtures served by Playwright
 * routing — no backend needed.
 * Pattern: scripts/boards-dispatch-capture.mjs (Wave 17 lane 3).
 *
 * Usage: start the web app (dev on :3000), then node scripts/portal-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-portal');
const BASE = process.argv[2] ?? 'http://localhost:3000';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const now = Date.now();
const iso = (offsetMs) => new Date(now + offsetMs).toISOString();
const today = new Date(now);
const dayIso = (offsetMs) => new Date(now + offsetMs).toISOString();
const dateOnly = (offsetMs) => iso(offsetMs).slice(0, 10);

const CLIENT_ME = {
  id: 'p1',
  org_id: 'o1',
  client_id: 'c1',
  email: 'ops@acme.com',
  display_name: 'Casey Nolan',
  role: 'contact',
};

const portalTicket = (n, title, category, priority, extra = {}) => ({
  id: `t-${n}`,
  number: `MER-${100 + n}`,
  title,
  status_id: 'st1',
  category,
  priority,
  board_id: 'b1',
  due_at: extra.due ?? null,
  created_at: iso(-3 * 86_400_000),
  updated_at: iso((-(extra.updatedH ?? 2)) * 3600_000),
  resolved_at: category === 'closed' ? iso(-86_400_000) : null,
  requester_id: null,
  client_id: 'c1',
  contact_id: null,
});

const TICKETS = [
  portalTicket(1, 'Mail server rejecting inbound', 'in_progress', 'p1'),
  portalTicket(2, 'VPN drops every hour on site', 'waiting', 'p2', { updatedH: 0.5, due: dayIso(26 * 3600_000) }),
  portalTicket(3, 'Printer fleet driver rollout', 'open', 'p3', { updatedH: 0.1 }),
  portalTicket(4, 'Backup verification January', 'closed', 'p4', { updatedH: 40 * 24 }),
];

const COMMENTS = [
  { id: 'c-1', kind: 'client_visible', body: 'Mail is down for the whole office since this morning; we have not received anything since 8:30.', created_at: iso(-20 * 3600_000) },
  { id: 'c-2', kind: 'in_house_update', body: 'Vendor escalated with the provider — engineers engaged.', created_at: iso(-4 * 3600_000) },
];

const ATTACHMENTS = [
  { id: 'att-1', filename: 'smtp-log.txt', content_type: 'text/plain', size_bytes: 45_211, sha256: 'aa', created_at: iso(-3 * 3600_000) },
  { id: 'att-2', filename: 'trace-route.png', content_type: 'image/png', size_bytes: 284_000, sha256: 'bb', created_at: iso(-3 * 3600_000) },
];

const INVOICES = [
  {
    id: 'inv-1',
    number: 'INV-2026-100',
    status: 'issued',
    currency: 'USD',
    issued_at: dateOnly(-20 * 86_400_000),
    due_on: dateOnly(-1 * 86_400_000),
    total_minor: 640_000,
    total_paid_minor: 0,
    outstanding_minor: 640_000,
    created_at: iso(-20 * 86_400_000),
  },
  {
    id: 'inv-2',
    number: 'INV-2026-101',
    status: 'issued',
    currency: 'USD',
    issued_at: dateOnly(-4 * 86_400_000),
    due_on: dateOnly(6 * 86_400_000),
    total_minor: 320_000,
    total_paid_minor: 0,
    outstanding_minor: 320_000,
    created_at: iso(-4 * 86_400_000),
  },
  {
    id: 'inv-3',
    number: 'INV-2026-097',
    status: 'paid',
    currency: 'USD',
    issued_at: dateOnly(-50 * 86_400_000),
    due_on: dateOnly(-35 * 86_400_000),
    total_minor: 480_000,
    total_paid_minor: 640_000,
    outstanding_minor: 0,
    created_at: iso(-50 * 86_400_000),
  },
];

const INVOICE_DETAIL = {
  invoice: INVOICES[0],
  lines: [
    { id: 'l1', kind: 'labor', description: 'Migratory network support — January', quantity_minutes: 540, unit_amount_minor: 180_00, amount_minor: 480_000 },
    { id: 'l2', kind: 'labor', description: 'Backup verification', quantity_minutes: 80, unit_amount_minor: 200_00, amount_minor: 160_000 },
  ],
  payments: [
    { id: 'pay-1', amount_minor: 250_000, currency: 'USD', provider: 'none', received_at: iso(-15 * 86_400_000) },
  ],
};

const KB_CATEGORIES = [
  { id: 'cat-1', name: 'Network', description: 'Connectivity, VPN, and troubleshooting.' },
  { id: 'cat-2', name: 'Email & Security', name_ok: true, description: 'Mail hygiene, phishing, MFA.' },
  { id: 'cat-3', name: 'Devices', name_ok: true, description: 'Imaging, drives, and fleet.' },
];

const kbArticle = (n, title, body) => ({
  id: `art-${n}`,
  category_id: 'cat-1',
  title,
  body,
  updated_at: iso(-5 * 86_400_000),
});

const KB_ARTICLES = [
  kbArticle(1, 'Restart a VPN connection on Windows', 'Open the Marula VPN client, click Disconnect, wait ten seconds, then Connect again. If the tunnel still drops, report it from your portal Requests page so we can correlate with our edge logs.'),
  kbArticle(2, 'Run the mail queue check quickly', 'Sign in to the mail portal, go to Queues, and look for brown entries. A queue that grows for over an hour is usually a failed connector; restart the connector from Actions.'),
  kbArticle(3, 'Request a laptop image refresh', 'Use the portal if your device is on contract and you have not re-imaged in twelve months.'),
];

const SLOT = (offsetH) => {
  const s = dayIso(offsetH * 3600_000);
  const e = dayIso((offsetH + 1) * 3600_000);
  return { starts_at: s, ends_at: e };
};
const SLOTS = [
  {
    user_id: 'u-ana',
    display_name: 'Ana Sousa',
    slots: [SLOT(20), SLOT(26), SLOT(40)],
  },
  {
    user_id: 'u-marco',
    display_name: 'Marco Diaz',
    slots: [SLOT(27)],
  },
];

const APPOINTMENTS = [
  {
    id: 'apr-1',
    user_id: 'u-ana',
    title: 'Quarterly review',
    starts_at: dayIso(48 * 3600_000),
    ends_at: dayIso(49 * 3600_000),
    kind: 'remote',
    notes: null,
    status: 'pending',
    decision_note: null,
    created_at: iso(-2 * 86_400_000),
  },
];

const PUBLIC_LINK = {
  token: 'stok',
  label: 'Mail server follow-up — Reception',
  meeting_type_name: 'On-site visit',
  meeting_type_description: 'A 90-minute field visit at your site.',
  duration_minutes: 90,
  slots: [
    { starts_at: dayIso(25 * 3600_000), ends_at: dayIso(27 * 3600_000) },
    { starts_at: dayIso(49 * 3600_000), ends_at: dayIso(50.5 * 3600_000) },
  ],
  is_used: false,
  is_revoked: false,
  is_expired: false,
};

const FIXTURES = {
  '/api/v1/portal/me': CLIENT_ME,
  '/api/v1/me': {
    ...CLIENT_ME,
    user_id: 'staff-me',
    org_id: 'o1',
    email: 'owner@marula.test',
    display_name: 'Taylor Reed',
    timezone: 'UTC',
    permissions: ['admin:org'],
  },
  '/api/v1/org': { id: 'o1', name: 'Acme Corp' },
  '/api/v1/settings/user': [],
  '/api/v1/settings/org': [],
  '/api/v1/notifications/unread-count': { count: 0 },
  // Tickets
  '/api/v1/portal/tickets': TICKETS,
  [`/api/v1/portal/tickets/t-1`]: { ticket: TICKETS[0], comments: COMMENTS, can_view_in_house_updates: false },
  [`/api/v1/portal/tickets/t-1/attachments`]: ATTACHMENTS,
  // Invoices
  '/api/v1/portal/invoices': INVOICES,
  [`/api/v1/portal/invoices/inv-1`]: INVOICE_DETAIL,
  // KB
  '/api/v1/portal/kb/categories': KB_CATEGORIES,
  '/api/v1/portal/kb/articles': KB_ARTICLES,
  [`/api/v1/portal/kb/articles/art-1`]: KB_ARTICLES[0],
  '/api/v1/portal/kb/search': [
    { article: KB_ARTICLES[0], score: 12 },
    { article: KB_ARTICLES[1], score: 8 },
  ],
  // Schedule
  '/api/v1/portal/schedule/slots': SLOTS,
  '/api/v1/portal/appointment-requests': APPOINTMENTS,
  // Public scheduling link
  '/api/v1/public/scheduling-links/stok': PUBLIC_LINK,
};

async function shot(browser, viewport, theme, name, path, interactions, anon = false) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', name, String(e).slice(0, 300)));
  const url = new URL(`${BASE}${path}`);
  const themeToken = theme === 'dark' ? 'dark' : 'light';
  await page.addInitScript(({ t, anonymous }) => {
    window.localStorage.setItem('marula-theme', t);
    if (!anonymous) {
      window.localStorage.setItem('marula-portal-tokens', JSON.stringify({
        access_token: 'test-access',
        refresh_token: 'test-refresh',
        session_id: 'sess',
        expires_in: 900,
      }));
    }
    const style = document.createElement('style');
    style.textContent = 'nextjs-portal{display:none!important}';
    document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
  }, { t: themeToken, anonymous: anon });

  await page.route('**/api/**', async (r) => {
    const key = new URL(r.request().url()).pathname;
    if (key === '/api/v1/portal/me' && anon) {
      await r.fulfill({ status: 401, contentType: 'application/json', body: JSON.stringify({ error: { code: 401, message: 'unauthorized' } }) });
      return;
    }
    if (FIXTURES[key] !== undefined) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES[key]) });
    } else {
      await r.fulfill({ status: 404, contentType: 'application/json', body: JSON.stringify([]) });
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

const browser2 = await chromium.launch({ executablePath: process.env.CHROME_PATH, args: ['--no-sandbox', '--disable-dev-shm-usage'] });

const surfaces = [
  { tag: 'tickets', path: '/portal/tickets' },
  { tag: 'ticket', path: '/portal/tickets/t-1' },
  { tag: 'invoices', path: '/portal/invoices' },
  { tag: 'invoice', path: '/portal/invoices/inv-1' },
  { tag: 'kb', path: '/portal/kb' },
  { tag: 'kb-article', path: '/portal/kb/articles/art-1' },
  { tag: 'schedule', path: '/portal/schedule' },
  { tag: 'login', path: '/portal/login', anon: true },
];

for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    for (const s of surfaces) {
      try {
        await shot(browser2, vp, theme, `${s.tag}-${vp.width}-${theme}.png`, s.path, undefined, s.anon === true);
      } catch (e) {
        console.error('FAIL', s.tag, vp.width, theme, e.message);
      }
    }
  }
}

// Public flows + interaction states at narrow (mobile) and desktop, both themes.
const extraStates = [
  {
    tag: 's-link',
    path: '/s/stok',
    anon: true,
    interactions: async (page) => {
      const first = page.locator('[data-testid="public-slot-grid"] button').first();
      await first.click();
      await page.waitForTimeout(200);
    },
  },
  { tag: 'accept-invite', path: '/accept-invite?token=stok', anon: true },
  {
    tag: 'reply-composing',
    path: '/portal/tickets/t-1',
    interactions: async (page) => {
      const box = page.getByLabel('Reply');
      await box.fill('Rebooted the exchange connector at 09:42 — still bouncing.');
      await page.waitForTimeout(200);
    },
  },
  {
    tag: 'kb-search',
    path: '/portal/kb',
    interactions: async (page) => {
      await page.getByLabel('Search the knowledge base').fill('vpn');
      await page.getByRole('button', { name: 'Search' }).click();
      await page.waitForTimeout(500);
    },
  },
  {
    tag: 'schedule-picked',
    path: '/portal/schedule',
    interactions: async (page) => {
      await page.waitForTimeout(600);
      const pick = page.getByRole('button', { name: /with Ana Sousa/ }).first();
      await pick.click();
      await page.waitForTimeout(300);
    },
  },
  {
    tag: 'invoice-pay',
    path: '/portal/invoices/inv-1',
    interactions: async (page) => {
      await page.waitForTimeout(200);
      await page.getByRole('button', { name: /Full amount/ }).click();
      await page.waitForTimeout(200);
    },
  },
];

for (const state of extraStates) {
  for (const theme of ['light', 'dark']) {
    const width = 1440;
    try {
      await shot(browser2, { width, height: 1024 }, theme, `${state.tag}-${width}-${theme}.png`, state.path, state.interactions, state.anon === true);
    } catch (e) {
      console.error('FAIL states', state.tag, theme, e.message);
    }
    try {
      await shot(browser2, { width: 480, height: 900 }, theme, `${state.tag}-480-${theme}.png`, state.path, state.interactions, state.anon === true);
    } catch (e) {
      console.error('FAIL states', state.tag, theme, '480', e.message);
    }
  }
}

await browser2.close();
console.log('done');
