#!/usr/bin/env node
/**
 * Auth flows capture — Wave 17 lane 9 (p17-auth-flows).
 *
 * Screenshots the lane's auth surfaces — /login (sign-in + typed-error
 * interaction, org-registration segment, slug-suggestion), /mfa
 * (challenge with method pills, typed error), /sso (working / linked /
 * error states), /accept-invite (setup form, missing-token + mismatch
 * error), /welcome, and the MFA enrollment surface /settings/security —
 * at 1440/1024/768/480 in light AND dark, into design/qa-auth-flows/.
 * API responses are fixtures served by Playwright routing — no backend
 * needed. Pattern: scripts/boards-dispatch-capture.mjs (Wave 17 lane 3).
 *
 * Usage: start the web app (dev on :3099), then node scripts/auth-flows-capture.mjs [baseUrl]
 */
import { chromium } from 'playwright';
import { mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';

const OUT = resolve('design/qa-auth-flows');
const BASE = process.argv[2] ?? 'http://localhost:3099';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const ORG = '00000000-0000-4000-8000-000000000002';
const PROVIDERS = [
  { name: 'oidc', display_name: 'Okta' },
  { name: 'entra-id', display_name: 'Microsoft Entra ID' },
];

const FIXTURES = {
  '/api/v1/auth/oidc/providers/public': PROVIDERS,
  '/api/v1/me': {
    user_id: '00000000-0000-4000-8000-000000000001',
    org_id: ORG,
    email: 'owner@meridian.test',
    display_name: 'Taylor Reed',
    timezone: 'UTC',
    permissions: ['special:manage_users', 'admin:org'],
  },
  '/api/v1/org': {
    id: ORG,
    name: 'Northwind Managed IT',
    ticket_prefix: 'NW',
    slug: 'northwind-it',
  },
  '/api/v1/auth/mfa/webauthn/credentials': [
    { id: 'cred-1', label: 'MacBook Touch ID', created_at: new Date(Date.now() - 30 * 86_400_000).toISOString(), last_used_at: null },
  ],
  '/api/v1/auth/sessions': [
    {
      id: 'sess-current',
      user_agent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)',
      created_at: new Date(Date.now() - 2 * 3600_000).toISOString(),
      expires_at: new Date(Date.now() + 86_400_000).toISOString(),
      current: true,
    },
    {
      id: 'sess-other',
      user_agent: 'Windows Chrome · Sinclair St',
      created_at: new Date(Date.now() - 5 * 86_400_000).toISOString(),
      expires_at: new Date(Date.now() + 3 * 86_400_000).toISOString(),
      current: false,
    },
  ],
  '/api/v1/auth/oidc/providers': [
    {
      id: 'p1',
      name: 'oidc',
      display_name: 'Okta',
      issuer: 'https://northwind.okta.com/oauth2/default',
      enabled: true,
    },
    {
      id: 'p2',
      name: 'entra-id',
      display_name: 'Microsoft Entra ID',
      issuer: 'https://login.microsoftonline.com/meridian',
      enabled: false,
    },
  ],
};

/**
 * opts:
 *  authed         → seed local tokens (shell routes)
 *  badPassword    → fake 401 on /login (typed error evidence)
 *  slugTaken      → fake 409 + suggestion on register
 *  badMfaCode     → submit a wrong code on /mfa
 *  inviteMismatch → submit mismatched passwords on accept-invite
 *  ssoFreeze      → 'working' (defer navigation) | 'linked'
 */
async function shot(browser, viewport, theme, name, path, opts = {}) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on('pageerror', (e) => console.error('PAGEERROR', name, String(e).slice(0, 300)));
  const url = new URL(`${BASE}${path}`);
  if (theme === 'dark') url.searchParams.set('theme', 'dark');
  const themeToken = theme === 'dark' ? 'dark' : 'light';

  await page.addInitScript(
    ({ t, authed, mfaPending, ssoFreeze }) => {
      window.localStorage.setItem('meridian-theme', t);
      if (mfaPending) {
        window.sessionStorage.setItem('meridian-mfa-pending', JSON.stringify({
          mfa_token: 'bridge-token',
          methods: ['totp', 'recovery', 'webauthn'],
        }));
      }
      if (authed) {
        window.localStorage.setItem('meridian-tokens', JSON.stringify({
          access_token: 'test-access',
          refresh_token: 'test-refresh',
          session_id: 'sess',
          expires_in: 900,
        }));
        window.localStorage.setItem('meridian-tokens-exp', String(Date.now() + 60_000));
      }
      const style = document.createElement('style');
      style.textContent = 'nextjs-portal{display:none!important}';
      document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
    },
    { t: themeToken, authed: opts.authed, mfaPending: opts.mfaPending },
  );

  await page.route('**/api/**', async (r) => {
    const key = new URL(r.request().url()).pathname;
    // The auth jar is cookie/memory-driven — requests carry no bearer, so
    // gate /me on the surface being an authed one (public pages get 401 so
    // they stay on their own URL instead of bootstrapping into the shell).
    if (key === '/api/v1/me' && !opts.authed) {
      await r.fulfill({ status: 401, contentType: 'application/json', body: JSON.stringify({ error: { code: 401, message: 'unauthenticated' } }) });
      return;
    }
    if (key === '/api/v1/auth/refresh' && !opts.authed) {
      await r.fulfill({ status: 401, contentType: 'application/json', body: JSON.stringify({ error: { code: 401, message: 'no session' } }) });
      return;
    }
    if (opts.authed && key === '/api/v1/auth/refresh') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ access_token: 'test-access', refresh_token: 'test-refresh', session_id: 'sess', expires_in: 900 }) });
      return;
    }
    if (key === '/api/v1/sso/exchange') {
      // RULE: the plain /sso?code=BAD-CODE run mimics a rejected (used or
      // expired) handoff code — the typed error landing. 'working' delays
      // the fulfill so the screenshot lands mid-spin; 'linked' confirms an
      // identity-link instead of issuing tokens.
      if (opts.ssoFreeze === 'working') {
        await new Promise((ok) => setTimeout(ok, 3000));
      }
      if (opts.ssoFreeze === 'linked') {
        await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ linked: true }) });
      } else {
        await r.fulfill({ status: 422, contentType: 'application/json', body: JSON.stringify({ error: { code: 422, message: 'handoff expired' } }) });
      }
      return;
    }
    if (key === '/api/v1/auth/mfa/verify') {
      // Typed typed-error evidence for the /mfa challenge: a rejected code
      // (envelope shell mirrors the real 401 shape).
      await r.fulfill({ status: 401, contentType: 'application/json', body: JSON.stringify({ error: { code: 401, message: 'The code was not accepted. Check the six digits and try again.' } }) });
      return;
    }
    if (key === '/api/v1/auth/login' || key === '/api/v1/auth/register') {
      if (opts.slugTaken) {
        await r.fulfill({
          status: 409,
          contentType: 'application/json',
          body: JSON.stringify({ error: { code: 409, message: 'org slug already taken', suggestion: 'northwind-it-2' } }),
        });
      } else {
        await r.fulfill({
          status: 401,
          contentType: 'application/json',
          body: JSON.stringify({ error: { code: 401, message: 'Sign-in rejected: the email or password is not correct.' } }),
        });
      }
      return;
    }
    if (FIXTURES[key] !== undefined) {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(FIXTURES[key]) });
    } else if (key === '/api/v1/notifications') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ items: [] }) });
    } else if (key === '/api/v1/notifications/unread-count') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ count: 0 }) });
    } else if (key === '/api/v1/board-groups') {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
    } else {
      await r.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
    }
  });

  await page.goto(url.toString(), { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForSelector('main', { timeout: 20_000 });
  await page.waitForTimeout(1100);

  if (opts.registerMode) {
    await page.getByRole('tab', { name: 'Create org' }).click();
  } else if (opts.badPassword || opts.slugTaken) {
    if (opts.slugTaken) {
      await page.getByRole('tab', { name: 'Create org' }).click();
      await page.fill('input[placeholder="Acme IT"]', 'Northwind IT');
      await page.fill('input[placeholder="acme-it"]', 'northwind-it');
      await page.fill('input[placeholder="Dana Owner"]', 'Dana Owner');
    }
    await page.fill('input[type="email"]', 'dana@northwind.test');
    await page.fill('input[type="password"]', 'a-long-enough-password');
  }
  if (opts.inviteMismatch) {
    await page.fill('#invite-password', 'a-long-enough-password');
    await page.fill('#invite-password-confirm', 'a-different-password');
  }

  await page.waitForTimeout(400);

  if (opts.badMfaCode) {
    await page.fill('#code', '999999');
  }
  if (opts.badPassword || opts.slugTaken || opts.inviteMismatch || opts.badMfaCode) {
    await page.getByRole('button', { name: /Sign in|Create organization|Verify|Set password and sign in/ }).last().click();
    if (opts.slugTaken) {
      // The 409 → suggestion replay must LAND before the screenshot.
      await page.getByRole('status').waitFor({ state: 'visible', timeout: 10_000 });
    }
    await page.waitForTimeout(400);
  }

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

// Light+dark × all four widths: every public auth surface, /welcome, and
// the MFA enrollment page.
const surfaces = [
  { tag: 'login', path: '/login' },
  { tag: 'mfa', path: '/mfa', opts: { mfaPending: true } },
  { tag: 'sso-error', path: '/sso?code=BAD-CODE' },
  { tag: 'accept-invite', path: '/accept-invite?token=nw-000000000000' },
  { tag: 'welcome', path: '/welcome', opts: { authed: true } },
  { tag: 'security', path: '/settings/security', opts: { authed: true } },
];

for (const vp of viewports) {
  for (const theme of ['light', 'dark']) {
    for (const s of surfaces) {
      try {
        await shot(browser, vp, theme, `${s.tag}-${vp.width}-${theme}.png`, s.path, s.opts);
      } catch (e) {
        console.error('FAILED', s.tag, vp.width, theme, String(e).slice(0, 200));
      }
    }
  }
}

// Interaction + state variants (1440, light + selected dark).
const cases = [
  { name: 'login-register-1440-light', path: '/login', theme: 'light', opts: { registerMode: true } },
  { name: 'login-register-1440-dark', path: '/login', theme: 'dark', opts: { registerMode: true } },
  { name: 'login-error-1440-light', path: '/login', theme: 'light', opts: { badPassword: true } },
  { name: 'login-error-1440-dark', path: '/login', theme: 'dark', opts: { badPassword: true } },
  { name: 'login-slug-suggestion-1440-light', path: '/login', theme: 'light', opts: { slugTaken: true } },
  { name: 'mfa-error-1440-light', path: '/mfa', theme: 'light', opts: { mfaPending: true, badMfaCode: true } },
  { name: 'sso-working-1440-light', path: '/sso?code=FRESH-CODE', theme: 'light', opts: { ssoFreeze: 'working' } },
  { name: 'sso-linked-1440-light', path: '/sso?code=ANY-CODE', theme: 'light', opts: { ssoFreeze: 'linked' } },
  { name: 'accept-invite-missing-1440-light', path: '/accept-invite', theme: 'light' },
  { name: 'accept-invite-mismatch-1440-light', path: '/accept-invite?token=x', theme: 'light', opts: { inviteMismatch: true } },
];
for (const c of cases) {
  try {
    await shot(browser, { width: 1440, height: 1024 }, c.theme, `${c.name}.png`, c.path, c.opts);
  } catch (e) {
    console.error('FAILED', c.name, String(e).slice(0, 200));
  }
}

await browser.close();
