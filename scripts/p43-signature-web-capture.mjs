/**
 * Capture the web signature pad — the parity half of the mobile pack's
 * 08/11 wrap-up states, in both themes.
 *
 * The web surface is a DERIVED state (the pack's desktop wrap-up states 18-22
 * draw no sign-off), so the geometry held against here is the pack's own mobile
 * pad rule applied to the desktop modal: a "Client sign-off" heading, a dashed
 * pad with the placeholder bottom-left and Clear bottom-right, and the stroke
 * in the theme's text colour.
 *
 * Usage (from the repo root):
 *   bash scripts/e2e-stack.sh up
 *   bash scripts/e2e-build-web.sh
 *   PORT=3003 node apps/web/.next/standalone/server.js &
 *   node scripts/p43-signature-web-capture.mjs
 */
import { mkdir, readFile } from "node:fs/promises";
import { resolve } from "node:path";

import { chromium } from "playwright";

const BASE = process.env.WEB_URL ?? "http://localhost:3003";
const API = process.env.API_URL ?? "http://localhost:28481";
const OUT = "design/qa-p43-signature-pad/captures";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
/** The worktree-level secret the native capture script owns and writes. */
const TOTP_FILE = resolve(import.meta.dirname, "..", ".kilo", "capture-totp.json");

async function settle(page, selector, timeout = 15_000) {
  await page.waitForSelector(selector, { timeout, state: "visible" });
  await page.waitForTimeout(350);
}

/**
 * RFC 6238 TOTP (SHA-1, 30s, 6 digits) — the real code the server expects.
 * Same derivation as native/scripts/capture-pack.mjs, which owns the secret
 * file, so the two runners never disagree about a counter window.
 */
async function totpFor(base32Secret) {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  let bits = "";
  for (const char of base32Secret.replace(/=+$/, "").toUpperCase()) {
    const index = alphabet.indexOf(char);
    if (index < 0) continue;
    bits += index.toString(2).padStart(5, "0");
  }
  const bytes = [];
  for (let i = 0; i + 8 <= bits.length; i += 8) bytes.push(parseInt(bits.slice(i, i + 8), 2));
  const counter = Math.floor(Date.now() / 1000 / 30);
  const buffer = new ArrayBuffer(8);
  const view = new DataView(buffer);
  view.setUint32(0, Math.floor(counter / 2 ** 32), false);
  view.setUint32(4, counter % 2 ** 32, false);

  const key = await crypto.subtle.importKey(
    "raw",
    new Uint8Array(bytes),
    { name: "HMAC", hash: "SHA-1" },
    false,
    ["sign"],
  );
  const digest = new Uint8Array(await crypto.subtle.sign("HMAC", key, buffer));
  const offset = digest[digest.length - 1] & 0x0f;
  const code =
    ((digest[offset] & 0x7f) << 24) |
    (digest[offset + 1] << 16) |
    (digest[offset + 2] << 8) |
    digest[offset + 3];
  return String(code % 1_000_000).padStart(6, "0");
}

async function login() {
  const response = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: EMAIL, password: PASSWORD }),
  });
  if (!response.ok) throw new Error(`login failed: ${response.status}`);
  const body = await response.json();
  if (typeof body.access_token === "string") {
    return { accessToken: body.access_token, setCookies: [] };
  }
  if (body.mfa_required !== true) {
    throw new Error("login returned neither a token nor an MFA challenge");
  }
  const secret = JSON.parse(await readFile(TOTP_FILE, "utf8")).secret_base32;
  // `/auth/mfa/verify` takes the bridge token IN THE BODY — posting it as a
  // header 404s.
  const verify = await fetch(`${API}/api/v1/auth/mfa/verify`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ mfa_token: body.mfa_token, code: await totpFor(secret) }),
  });
  if (!verify.ok) throw new Error(`mfa verify failed: ${verify.status}`);
  const verified = await verify.json();
  // The API answers with the session cookie pair, and the web app
  // authenticates browser-side off those cookies rather than a bearer token
  // in storage.
  return {
    accessToken: verified.access_token ?? verified.tokens?.access_token,
    setCookies: verify.headers.getSetCookie?.() ?? [],
  };
}

async function firstTicket(token) {
  const response = await fetch(`${API}/api/v1/tickets?limit=5`, {
    headers: { authorization: `Bearer ${token}` },
  });
  const body = await response.json();
  const rows = Array.isArray(body) ? body : (body.items ?? body.tickets ?? []);
  if (rows.length === 0) throw new Error("no tickets in the demo org");
  return rows[0];
}

/** Draws a stroke across a pad's own box, the way the pack's state 11 does. */
async function drawStroke(page, selector) {
  const canvas = page.locator(selector).first();
  const box = await canvas.boundingBox();
  if (box === null) return;
  const cx = box.x + box.width * 0.5;
  const cy = box.y + box.height * 0.45;
  const half = box.width * 0.3;
  // Several small mouse steps, so the SVG path renders as a continuous curve
  // rather than one jump that rounds off into a dot.
  await page.mouse.move(cx - half, cy);
  await page.mouse.down();
  for (let i = 1; i <= 10; i += 1) {
    const t = i / 10;
    await page.mouse.move(
      cx - half + 2 * half * t,
      cy + Math.sin(t * Math.PI * 2.2) * box.height * 0.16,
    );
    await page.waitForTimeout(12);
  }
  await page.mouse.up();
  await page.waitForTimeout(500);
}

/**
 * ONE session, ONE browser context, both themes. Two constraints force this,
 * and neither is a preference:
 *   1. the httpOnly session cookie ROTATES on the first refresh a context
 *      performs, so cookies held across contexts are stale by the second one
 *      and the API refuses the restore as a replay;
 *   2. an MFA code is single-use inside its 30s window, so a second sign-in in
 *      the same window is refused too.
 * Switching theme through the app's own preference (localStorage + navigation)
 * is the only way to get both modes from one session.
 */
async function main() {
  await mkdir(OUT, { recursive: true });
  const session = await login();
  const ticket = await firstTicket(session.accessToken);
  const browser = await chromium.launch();

  const context = await browser.newContext({
    viewport: { width: 1440, height: 960 },
    deviceScaleFactor: 2,
    colorScheme: "light",
  });
  for (const raw of session.setCookies) {
    const [pair] = raw.split(";");
    const separator = pair.indexOf("=");
    await context.addCookies([
      {
        name: pair.slice(0, separator).trim(),
        value: pair.slice(separator + 1).trim(),
        domain: "localhost",
        // The API scopes the pair to /api, which is where the app calls it.
        path: "/api",
        httpOnly: true,
        secure: false,
        sameSite: "Lax",
      },
    ]);
  }
  await context.addInitScript(([mode]) => {
    window.localStorage.setItem("marula-theme", mode);
  }, ["light"]);
  const page = await context.newPage();

  // The preference is written only against a real document: the first page in
  // a fresh context is about:blank, where touching storage is a SecurityError.
  await page.goto(`${BASE}/login`, { waitUntil: "domcontentloaded", timeout: 90_000 });

  for (const theme of ["light", "dark"]) {
    await page.evaluate((mode) => {
      window.localStorage.setItem("marula-theme", mode);
    }, theme);
    await page.goto(`${BASE}/tickets/${ticket.id}`, { waitUntil: "domcontentloaded", timeout: 90_000 });
    const open = page.locator('[data-testid="wb-close-ticket"], [data-testid="workbar-wrap-up"]').first();
    await open.waitFor({ state: "visible", timeout: 60_000 });
    await open.click({ force: true, timeout: 20_000 });
    await settle(page, '[data-testid="signature-pad"]', 30_000);

    // The onboarding tour opens over the sheet's top-right and would appear in
    // every capture. Clicking by role is unreliable here (the overlay's
    // Done is inside a dialog whose accessible name is the step copy), so the
    // button is found by its visible text and clicked by force.
    const tourDone = page.getByText("Done", { exact: true }).first();
    if (await tourDone.isVisible().catch(() => false)) {
      await tourDone.click({ force: true, timeout: 5_000 }).catch(() => {});
      await tourDone.waitFor({ state: "hidden", timeout: 3_000 }).catch(() => {});
      await page.waitForTimeout(200);
    }

    // Scroll the sheet so the whole sign-off section is inside the frame:
    // the consent line and the confirm sit below the fold at 960px, and a
    // capture that cuts them off verifies nothing about either.
    const consent = page.locator('[data-testid="signature-consent"]');
    if ((await consent.count()) > 0) {
      await consent.scrollIntoViewIfNeeded({ timeout: 10_000 }).catch(() => {});
    }
    await page.screenshot({ path: resolve(OUT, `${theme}-web-wrapup-empty.png`), timeout: 60_000 });
    console.log(`captured ${theme}/web-wrapup-empty`);

    await drawStroke(page, '[data-testid="signature-canvas"]');
    await page.screenshot({ path: resolve(OUT, `${theme}-web-wrapup-signed.png`), timeout: 60_000 });
    console.log(`captured ${theme}/web-wrapup-signed`);
  }

  await browser.close();
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
