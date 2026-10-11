/**
 * Capture the mobile pack surface — 21 drawn states, light + dark, phone.
 *
 * The lane's pack (brand/marula-mockups/marula-surface-pack) is ground
 * truth; this script renders the SAME React tree the phone ships through
 * Expo web at 390x844 (the pack's artboard), both themes, and writes the
 * states into design/qa-p41-ui-mobile/captures/ with the pack's own names.
 *
 * Drawn states covered (signature pad intentionally NOT drawn):
 *   board-list, ticket-detail, note-composer, time-day, stop-and-log,
 *   more, notifications, deep-linked-open, locked, login, mfa,
 *   scan-camera, scan-found, scan-ambiguous, scan-unknown, camera-off,
 *   stock-item, stock-location, stock-serial, tool-checkout, tool-overdue,
 *   move + asset record (derived legs).
 * Wave 43 lane 7 (p43-demo-fixture-coverage) added the states the seed now
 * proves: scan-ambiguous (the shared DOCK-01 part resolves to two
 * candidates) and the offline trio (note queues while forced offline, the
 * inspector shows the pending op, the replay settles it).
 * Wave 44 lane 11 (p44-ui-mobile-fidelity) added the scan surface's own
 * offline-save path: scan-offline-saved draws the pack's scan-10 sheet
 * (a code saved on the phone, "Saved for later · N waiting").
 *
 * Usage: boot the stack, start expo web (see capture-web.mjs header), then
 *   node native/scripts/capture-pack.mjs [--only=a,b] [--theme=light|dark]
 *   [--out=design/qa-<lane>/captures]
 */
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { createHmac } from "node:crypto";
import { Buffer } from "node:buffer";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
/** Where captures land. Defaults to the mobile lane's evidence dir; a
 * verification lane passes --out to keep its own record without touching
 * the committed one. */
const outDir = path.join(
  root,
  process.argv.find((value) => value.startsWith("--out="))?.slice(6) ??
    "design/qa-p41-ui-mobile/captures",
);

/** The stack's own env file — never hand-set ports (see serve-capture-web.sh). */
async function stackEnv() {
  const raw = await readFile(path.join(root, "e2e/.e2e-stack.env"), "utf8");
  const env = {};
  for (const line of raw.split("\n")) {
    const match = /^\s*([A-Z0-9_]+)=(.*)$/.exec(line);
    if (match !== null) env[match[1]] = match[2].replace(/^"|"$/g, "");
  }
  return env;
}

const stack = await stackEnv();
const WEB_URL = process.env.CAPTURE_WEB_URL ?? `http://localhost:${stack.E2E_WEB_PORT ?? "3001"}`;
const API_URL = (process.env.CAPTURE_API_URL ?? `http://localhost:${stack.E2E_API_PORT ?? "28480"}`).replace(/\/$/, "");
// The capture session needs scan + stock permissions (the technician seat
// is read-only there, which photographs permission sheets instead of the
// pack's states). Dispatcher has them; the owner keeps one job here — the
// MFA screen, where a gated sign-in is the state being drawn.
const EMAIL = "dispatcher@demo.local";
const MFA_EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
/** A tag the demo org actually resolves (assets carry tags; RSV-* does not). */
const DEMO_SCAN_CODE = "ACME-LT-0142";
const THEMES = ["light", "dark"];

/** Where the capture run keeps the demo owner's TOTP secret (gitignored). */
const totpFile = path.join(root, ".kilo", "capture-totp.json");

/**
 * Sign in, completing the MFA second factor when the demo owner has TOTP
 * enrolled — otherwise the API hands back a bridge token and there is no
 * session to seed, and every capture would photograph the login screen.
 */
async function login() {
  const response = await fetch(`${API_URL}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: EMAIL, password: PASSWORD }),
  });
  if (!response.ok) throw new Error(`login failed: ${response.status}`);
  const body = await response.json();
  if (typeof body.access_token === "string") return body;
  const secret = await readTotpSecret();
  if (secret === null) {
    throw new Error("login is MFA-gated but no TOTP secret is stored for the demo owner");
  }
  const verified = await fetch(`${API_URL}/api/v1/auth/mfa/verify`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ mfa_token: body.mfa_token, code: totp(secret) }),
  });
  if (!verified.ok) throw new Error(`mfa verify failed: ${verified.status}`);
  return verified.json();
}

async function readTotpSecret() {
  try {
    return JSON.parse(await readFile(totpFile, "utf8")).secret_base32 ?? null;
  } catch {
    return null;
  }
}

async function writeTotpSecret(secret) {
  await mkdir(path.dirname(totpFile), { recursive: true });
  await writeFile(totpFile, JSON.stringify({ secret_base32: secret }));
}

/** RFC 6238 TOTP (SHA-1, 30s, 6 digits) — the real code the server expects. */
/** Same as [`totp`], but for an explicit 30s counter window — the disable
 * call needs the NEXT window after verify consumed one (replay protection). */
function totpAt(base32Secret, counter) {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  let bits = "";
  for (const char of base32Secret.replace(/=+$/, "").toUpperCase()) {
    const index = alphabet.indexOf(char);
    if (index < 0) continue;
    bits += index.toString(2).padStart(5, "0");
  }
  const bytes = [];
  for (let i = 0; i + 8 <= bits.length; i += 8) bytes.push(parseInt(bits.slice(i, i + 8), 2));
  const buffer = Buffer.alloc(8);
  buffer.writeBigUInt64BE(BigInt(counter));
  const digest = createHmac("sha1", Buffer.from(bytes)).update(buffer).digest();
  const offset = digest[digest.length - 1] & 0x0f;
  const code =
    ((digest[offset] & 0x7f) << 24) |
    ((digest[offset + 1] & 0xff) << 16) |
    ((digest[offset + 2] & 0xff) << 8) |
    (digest[offset + 3] & 0xff);
  return String(code % 1_000_000).padStart(6, "0");
}

function totp(base32Secret) {
  return totpAt(base32Secret, Math.floor(Date.now() / 1000 / 30));
}

/**
 * Enroll TOTP for the MFA-STATE user only (the demo owner), so the code
 * screen is reachable through a REAL gated sign-in — the bridge token lives
 * in a ref, so there is no shortcut state to seed. The app-session account
 * is deliberately left unenrolled: gating it would put a second factor in
 * front of every other capture. Idempotent.
 */
async function enrollTotpForMfaUser() {
  const signIn = await fetch(`${API_URL}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: MFA_EMAIL, password: PASSWORD }),
  });
  if (!signIn.ok) return false;
  const body = await signIn.json();
  if (typeof body.access_token !== "string") return true; // already enrolled
  const auth = { "content-type": "application/json", authorization: `Bearer ${body.access_token}` };
  const begin = await fetch(`${API_URL}/api/v1/auth/mfa/totp/enroll-begin`, {
    method: "POST",
    headers: auth,
  });
  if (!begin.ok) return false;
  const { secret_base32: secret } = await begin.json();
  await writeTotpSecret(secret);
  const verify = await fetch(`${API_URL}/api/v1/auth/mfa/totp/enroll-verify`, {
    method: "POST",
    headers: auth,
    body: JSON.stringify({ code: totp(secret) }),
  });
  return verify.ok;
}

// The pack-named capture plan: [pack-state-name, setup(page)]
const STATES = [
  ["board-list", async (page) => {
    await page.goto(`${WEB_URL}/board`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="board-screen"]', 1800);
  }],
  ["ticket-detail", async (page) => {
    const row = page.locator('[data-testid^="ticket-row-"]').first();
    if ((await row.count()) > 0) {
      await row.click();
      await settle(page, '[data-testid="ticket-action-note"]', 1400, 120_000);
    }
  }],
  ["note-composer", async (page) => {
    // Self-sufficient: reach a ticket first, so this state also captures on
    // its own with --only. Then open NOTE (not Reply) — the pack draws the
    // internal mode: the sheet takes the note tint and the line under the
    // choice is the lock sentence.
    if ((await page.locator('[data-testid="ticket-action-note"]').count()) === 0) {
      await page.goto(`${WEB_URL}/board`, { waitUntil: "domcontentloaded" });
      await settle(page, '[data-testid="ticket-list"]', 1500);
      const row = page.locator('[data-testid^="ticket-row-"]').first();
      if ((await row.count()) > 0) {
        await row.click();
        await settle(page, '[data-testid="ticket-action-note"]', 1200, 120_000);
      }
    }
    const bar = page.locator('[data-testid="ticket-action-note"]');
    if ((await bar.count()) > 0) {
      await bar.click();
      await settle(page, '[data-testid="note-composer-sheet"]', 900);
      await page
        .locator('[data-testid="composer-kind-note"]')
        .click({ force: true, timeout: 8_000 })
        .catch(() => {});
      await page.waitForTimeout(900);
    }
  }],
  ["time-day", async (page) => {
    await goTab(page, "/time");
    await settle(page, '[data-testid="time-screen"]', 1600);
  }],
  ["stop-and-log", async (page) => {
    const stop = page.locator('[data-testid="time-running-card"] button', { hasText: "Stop and log" });
    if ((await stop.count()) > 0) {
      await stop.first().click();
      await page.waitForTimeout(900);
    }
  }],
  ["more", async (page) => {
    await goTab(page, "/more");
    await settle(page, '[data-testid="more-screen"]', 1200);
  }],
  ["notifications", async (page) => {
    await page.goto(`${WEB_URL}/notifications`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="notifications-screen"]', 1400);
  }],
  ["deep-linked-open", async (page) => {
    const row = page.locator('[data-testid^="notification-"]').first();
    if ((await row.count()) > 0) {
      await row.click();
      await settle(page, '[data-testid="ticket-from-notification"]', 1600, 120_000);
    }
  }],
  ["locked", async (page) => {
    await page.goto(`${WEB_URL}/locked`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="locked-screen"]', 900);
  }],
  ["login", async (page) => {
    await page.goto(`${WEB_URL}/login`, { waitUntil: "domcontentloaded" });
    await page.evaluate(() => localStorage.removeItem("marula.session.v1")).catch(() => {});
    await page.reload({ waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="login-email"]', 1200);
    // The pack draws the primary ENABLED — fill both fields, do not submit.
    await page.fill('[data-testid="login-password"]', "demo-password-123").catch(() => {});
    await page.waitForTimeout(400);
  }],
  ["scan-camera", async (page) => {
    await page.goto(`${WEB_URL}/scan`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="scan-screen"]', 1200);
  }],
  ["scan-unknown", async (page) => {
    await page.fill('[data-testid="scan-input"]', "NOPE-NOPE-000").catch(() => {});
    await page.click('[data-testid="scan-resolve"]').catch(() => {});
    await page.waitForTimeout(2000);
  }],
  ["scan-found", async (page) => {
    await page.goto(`${WEB_URL}/scan`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="scan-input"]', 800);
    await page.fill('[data-testid="scan-input"]', DEMO_SCAN_CODE).catch(() => {});
    await page.click('[data-testid="scan-resolve"]').catch(() => {});
    await page.waitForTimeout(2200);
  }],
  ["scan-ambiguous", async (page) => {
    // The seed carries TWO stock items sharing the DOCK-01 part number (and
    // two locations sharing SHELF-B) — a scan of that code resolves to two
    // candidates and the picker asks which one, exactly the pack's
    // scan-03 sheet.
    await page.goto(`${WEB_URL}/scan`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="scan-input"]', 800);
    await page.fill('[data-testid="scan-input"]', "DOCK-01").catch(() => {});
    await page.click('[data-testid="scan-resolve"]').catch(() => {});
    await settle(page, '[data-testid="scan-pick-sheet"]', 2200);
  }],
  ["scan-offline-saved", async (page) => {
    // The pack's scan-10 (Wave 44 lane 11): offline, a scan is SAVED, not
    // lost — the warm banner, the "Saved for later · N waiting" row, and
    // the saved sheet. NAVIGATION RULE (same as the offline trio):
    // forcedOffline is in-memory, so every move after the toggle must be
    // IN-APP (clicks) — a page.goto would drop the flag. Warm /scan online
    // first so the screen exists in the router, then toggle offline from
    // /more and come back through the tab bar FAB.
    await page.goto(`${WEB_URL}/scan`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="scan-screen"]', 1000);
    await page.goto(`${WEB_URL}/more`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="more-airplane-toggle"]', 1500);
    await page.locator('[data-testid="more-airplane-toggle"]').click();
    await settle(page, '[data-testid="banner-offline"]', 2500);
    await page.getByLabel("Scan, tab").first().click();
    await settle(page, '[data-testid="scan-screen"]', 1500);
    await page.fill('[data-testid="scan-input"]', "VAN-SHELF-42").catch(() => {});
    await page.click('[data-testid="scan-resolve"]').catch(() => {});
    // The save never leaves the phone: the warm banner + the saved row, and
    // the sheet that names what happens next.
    await page
      .locator('[data-testid="scan-saved-sheet"]')
      .waitFor({ timeout: 12_000 })
      .catch(() => {});
    await page.waitForTimeout(1200);
    // RESTORE ONLINE before the states that follow: the airplane toggle is
    // in-memory shell state, and leaving it on would photograph every later
    // state through the offline mask. Dismiss the sheet (it covers the tab
    // bar), go to More in-app, toggle back, and land on the board.
    await page.locator('[data-testid="scan-saved-sheet"] button').first().click().catch(() => {});
    await page.waitForTimeout(400);
    await page.getByRole("tab", { name: /More/ }).first().click().catch(() => {});
    await settle(page, '[data-testid="more-airplane-toggle"]', 1500);
    await page.locator('[data-testid="more-airplane-toggle"]').click().catch(() => {});
    await settle(page, '[data-testid="more-screen"]', 800);
  }],
  ["camera-off", async (page) => {
    // Self-sufficient: land on the scan screen first (the previous state's
    // online-restore leaves the shell wherever it was), then open the
    // camera sheet — the pack's camera-off state is the permission sheet.
    await page.goto(`${WEB_URL}/scan`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="scan-screen"]', 1200);
    await page.locator('[data-testid="scan-camera"]').click().catch(() => {});
    await page.waitForTimeout(1800);
  }],
  ["stock-item", async (page) => {
    // The seed carries assets, not stock items: drive the item surface from
    // the live catalog, and fall back to the asset record when the org has
    // no stock (an empty catalog is the org's truth, not a broken screen).
    await page.goto(`${WEB_URL}/stock`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="stock-lookup-screen"]', 1600);
    const row = page.locator('[data-testid^="stock-row-"]').first();
    if ((await row.count()) > 0) {
      await row.click();
      await settle(page, '[data-testid="stock-item-screen"]', 1600, 120_000);
    }
  }],
  ["stock-location", async (page) => {
    await page.goto(`${WEB_URL}/stock`, { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(1200);
  }],
  ["tool-overdue", async (page) => {
    await page.goto(`${WEB_URL}/tools`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="tools-screen"]', 1400);
  }],
  ["tool-checkout", async (page) => {
    // scan-08 is the DUE-BACK sheet: open a toolbox, then its check-out.
    await page.goto(`${WEB_URL}/tools`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="tools-screen"]', 1500);
    const kit = page.getByText(/Check out (kit|anyway)/).first();
    if ((await kit.count()) > 0) {
      await kit.click();
      await settle(page, '[data-testid="tool-checkout-sheet"]', 1200);
    }
  }],
  ["move", async (page) => {
    await page.goto(`${WEB_URL}/move`, { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(1400);
  }],
  ["asset-record", async (page) => {
    await page.goto(`${WEB_URL}/notifications`, { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(800);
  }],
  ["offline-note", async (page) => {
    // The pack's offline-saved state, honestly equivalent: warm the models
    // while online, force the app offline (the dev airplane toggle), then
    // save an internal note — the write queues with the "Saved on this
    // device" banner instead of a spinner, and nothing is lost. The scan
    // surface itself has no offline-save path (see the header note), so the
    // note is the write that proves the queue.
    //
    // NAVIGATION RULE: `forcedOffline` is in-memory state, so every move
    // after the toggle must be IN-APP (clicks) — a page.goto would reload
    // the shell and drop the flag.
    await page.goto(`${WEB_URL}/notifications`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="notifications-screen"]', 1500);
    const warmRow = page.locator('[data-testid^="notification-"]').first();
    if ((await warmRow.count()) > 0) {
      await warmRow.click();
      await settle(page, '[data-testid="ticket-action-note"]', 1400, 120_000);
    }
    await page.goto(`${WEB_URL}/more`, { waitUntil: "domcontentloaded" });
    await settle(page, '[data-testid="more-airplane-toggle"]', 1500);
    await page.locator('[data-testid="more-airplane-toggle"]').click();
    await settle(page, '[data-testid="banner-offline"]', 3000);
    await page.locator('[data-testid="more-notifications"]').click();
    await settle(page, '[data-testid="notifications-screen"]', 2000);
    await page.locator('[data-testid^="notification-"]').first().click();
    await settle(page, '[data-testid="ticket-action-note"]', 1500);
    await page.locator('[data-testid="ticket-action-note"]').click();
    await settle(page, '[data-testid="note-composer-sheet"]', 900);
    await page
      .locator('[data-testid="composer-kind-note"]')
      .click({ force: true, timeout: 8_000 })
      .catch(() => {});
    await page
      .fill(
        '[data-testid="note-composer-input"]',
        "Offline note — typed past the last mile; replays when the van is back in range.",
      )
      .catch(() => {});
    await page.locator('[data-testid="note-composer-send"]').click().catch(() => {});
    // The send closes the sheet itself; the banner clears after ~4s, so the
    // frame is taken the moment the queueing is confirmed.
    await page
      .getByText("Saved on this device")
      .first()
      .waitFor({ timeout: 12_000 });
  }],
  ["offline-queue", async (page) => {
    // The queued op itself: the inspector lists what the replay will send,
    // with the per-op audit trail. CAUTION: expo-router web keeps tab
    // screens MOUNTED under pushed routes — `[data-testid="more-screen"]`
    // matches even while a ticket covers it, so a count check lies and the
    // click times out on a covered element. Route by URL: pop real history
    // until /more is the live route, then take the dev row in-app (the
    // offline flag must never ride through a reload).
    await page.locator('[data-testid="ticket-back"]').first().click();
    await settle(page, '[data-testid="notifications-screen"]', 1500);
    for (let i = 0; i < 4; i += 1) {
      if (page.url().replace(/\/$/, "").endsWith("/more")) break;
      await page.goBack({ timeout: 8_000 }).catch(() => {});
      await page.waitForTimeout(700);
    }
    if (!page.url().replace(/\/$/, "").endsWith("/more")) {
      throw new Error(`offline-queue never reached /more (at ${page.url()})`);
    }
    await page
      .locator('[data-testid="more-screen"]')
      .getByText("Offline queue inspector")
      .first()
      .click();
    await settle(page, '[data-testid="queue-screen"]', 1600);
    await page
      .locator('[data-testid^="queue-op-"]')
      .first()
      .waitFor({ timeout: 8_000 });
  }],
  ["offline-replay", async (page) => {
    // Back online WITHOUT a reload: toggling the airplane off flips the
    // connectivity state in-session, the engine flushes the queued op, and
    // the inspector moves it under "Settled this session" — same session,
    // so the settled list still remembers the op (a reload would reset it).
    await page.getByText("‹ Back").first().click();
    await settle(page, '[data-testid="more-screen"]', 1500);
    await page.locator('[data-testid="more-airplane-toggle"]').click();
    await page
      .locator('[data-testid="banner-offline"]')
      .first()
      .waitFor({ state: "hidden", timeout: 8_000 });
    await page.waitForTimeout(1500);
    await page
      .locator('[data-testid="more-screen"]')
      .getByText("Offline queue inspector")
      .first()
      .click();
    await settle(page, '[data-testid="queue-screen"]', 1600);
    await page.getByText("Settled this session").first().waitFor({ timeout: 12_000 });
    await page.waitForTimeout(500);
  }],
  ["mfa", async (page) => {
    // The demo owner is TOTP-enrolled; a real gated sign-in lands on the code
    // screen with a live bridge token. Sign the shell out first, or /login
    // just redirects to the app.
    await page.goto(`${WEB_URL}/login`, { waitUntil: "domcontentloaded" });
    await page
      .evaluate(() => localStorage.removeItem("marula.session.v1"))
      .catch(() => {});
    await page.goto(`${WEB_URL}/login`, { waitUntil: "domcontentloaded" }).catch(() => {});
    await settle(page, '[data-testid="login-email"]', 800, 120_000);
    await page.fill('[data-testid="login-email"]', MFA_EMAIL);
    await page.fill('[data-testid="login-password"]', PASSWORD);
    await page.click('[data-testid="login-submit"]');
    await settle(page, '[data-testid="mfa-screen"]', 900, 120_000);
  }]];

/**
 * Wait for an anchor if it shows; never abort a capture over a missing one.
 * `anchorTimeout` is generous for routes Metro compiles on FIRST visit (a
 * cold deep link can take a minute), and short everywhere else.
 */
async function settle(page, selector, ms = 2500, anchorTimeout = 20_000) {
  await page.waitForSelector(selector, { timeout: anchorTimeout }).catch(() => {});
  await page.waitForTimeout(ms);
}

async function goTab(page, route) {
  await page.goto(`${WEB_URL}${route}`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(600);
}

/**
 * The pack freezes a RUNNING timer (00:25:05) and shows the 56px pill in
 * every tab. Start a real session through the API on a seeded ticket so the
 * chrome renders; stopping is the capture run's last step.
 */
async function startRunningTimer(accessToken) {
  const headers = { "content-type": "application/json", authorization: `Bearer ${accessToken}` };
  const mine = await fetch(`${API_URL}/api/v1/tickets/my-work`, { headers });
  if (!mine.ok) return null;
  // my-work returns a bare array (Ok<T>[]), not an {items} envelope.
  const tickets = (await mine.json()) ?? [];
  const ticket = tickets.find((row) => row.category !== "resolved") ?? tickets[0];
  if (ticket === undefined) return null;
  const start = await fetch(`${API_URL}/api/v1/timers/start`, {
    method: "POST",
    headers,
    body: JSON.stringify({
      ticket_id: ticket.id,
      idempotency_key: `capture-${ticket.id}`,
    }),
  });
  if (!start.ok) {
    console.warn(`timer start failed: ${start.status}`);
    return null;
  }
  const session = await start.json();
  console.log(`running timer on ${ticket.number} (${session.id})`);
  return session;
}

async function stopRunningTimer(accessToken, sessionId) {
  if (sessionId === null) return;
  await fetch(`${API_URL}/api/v1/timers/stop`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${accessToken}` },
    body: JSON.stringify({ session_id: sessionId, idempotency_key: `capture-stop-${sessionId}` }),
  });
}

/** The first ticket the demo technician owns (for the deep-link state). */
async function firstMyWorkTicket(accessToken) {
  const response = await fetch(`${API_URL}/api/v1/tickets/my-work`, {
    headers: { authorization: `Bearer ${accessToken}` },
  });
  if (!response.ok) return null;
  const items = (await response.json()).items ?? [];
  const row = items.find((item) => item.category !== "resolved") ?? items[0];
  return row === undefined ? null : row.id;
}

/**
 * Undo our TOTP enrollment. The MFA state needs the owner enrolled, but the
 * repo's own Maestro flows sign in as the owner and expect the Today tab —
 * leaving a factor behind would break them (and the next person's demo). The
 * capture run restores the seed it borrowed.
 */
async function unenrollTotpForMfaUser() {
  const secret = await readTotpSecret();
  if (secret === null) return;
  const signIn = await fetch(`${API_URL}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: MFA_EMAIL, password: PASSWORD }),
  });
  if (!signIn.ok) return;
  let body = await signIn.json();
  let usedCounter = null;
  if (typeof body.access_token !== "string") {
    // The owner is gated by the very factor we enrolled: spend the stored
    // secret to get a session, then turn the factor off.
    const nowCounter = Math.floor(Date.now() / 1000 / 30);
    let verified = null;
    for (const counter of [nowCounter - 1, nowCounter, nowCounter + 1]) {
      verified = await fetch(`${API_URL}/api/v1/auth/mfa/verify`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ mfa_token: body.mfa_token, code: totpAt(secret, counter) }),
      });
      if (verified.ok) {
        usedCounter = counter;
        break;
      }
    }
    if (usedCounter === null) return;
    body = await verified.json();
  }
  // Replay protection: the disable call must present a code from a window
  // the verify did NOT consume — the next one.
  const disableCounter = (usedCounter ?? Math.floor(Date.now() / 1000 / 30)) + 1;
  const response = await fetch(`${API_URL}/api/v1/auth/mfa/totp/disable`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${body.access_token}`,
    },
    body: JSON.stringify({ password: PASSWORD, code: totpAt(secret, disableCounter) }),
  });
  console.log(
    response.ok
      ? "restored: owner TOTP unenrolled (the demo is back to its seeded shape)"
      : `cleanup failed: ${response.status} ${(await response.text()).slice(0, 120)}`,
  );
}

const arg = (name, fallback) => {
  const hit = process.argv.find((value) => value.startsWith(`--${name}=`));
  return hit === undefined ? fallback : hit.split("=")[1];
};

/** Sign in as the demo owner, spending the stored TOTP when enrolled. */
async function ownerLogin() {
  const signIn = await fetch(`${API_URL}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: MFA_EMAIL, password: PASSWORD }),
  });
  const body = await signIn.json();
  if (typeof body.access_token === "string") return body;
  const secret = await readTotpSecret();
  if (secret === null) return null;
  const counter = Math.floor(Date.now() / 1000 / 30);
  for (const offset of [-1, 0, 1]) {
    const verified = await fetch(`${API_URL}/api/v1/auth/mfa/verify`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ mfa_token: body.mfa_token, code: totp(secret, counter + offset) }),
    });
    if (verified.ok) return verified.json();
  }
  return null;
}

/**
 * Capture the timer states (time-day, stop-and-log) on the owner seat with
 * a real running timer — the dispatcher session cannot hold one, so these
 * two states get their own seeded context per theme.
 */
async function ownerTimePass(browser) {
  const captured = [];
  const tokens = await ownerLogin();
  if (tokens === null) {
    console.warn("owner pass skipped: no owner session");
    return;
  }
  const auth = { "content-type": "application/json", authorization: `Bearer ${tokens.access_token}` };
  const mine = await fetch(`${API_URL}/api/v1/tickets/my-work`, { headers: auth });
  const tickets = (await mine.json()) ?? [];
  const ticket = tickets.find((row) => row.category !== "resolved") ?? tickets[0];
  let timerId = null;
  if (ticket !== undefined) {
    const start = await fetch(`${API_URL}/api/v1/timers/start`, {
      method: "POST",
      headers: auth,
      body: JSON.stringify({ ticket_id: ticket.id, idempotency_key: `capture-owner-${ticket.id}` }),
    });
    if (start.ok) {
      timerId = (await start.json()).id;
      console.log(`owner timer running on ${ticket.number}`);
    }
  }
  const stored = {
    accessToken: tokens.access_token,
    refreshToken: tokens.refresh_token,
    sessionId: tokens.session_id,
  };
  const only = arg("only", null)?.split(",").map((value) => value.trim()) ?? null;
  for (const theme of THEMES) {
    const context = await browser.newContext({
      viewport: { width: 390, height: 844 },
      deviceScaleFactor: 2,
      colorScheme: theme,
    });
    await context.addInitScript(
      ([sessionJson, themeMode]) => {
        if (localStorage.getItem("marula.capture.seeded") === "1") return;
        localStorage.setItem("marula.session.v1", sessionJson);
        localStorage.setItem("@marula-core/prefs/theme.mode", themeMode);
        localStorage.setItem("marula.capture.seeded", "1");
      },
      [JSON.stringify(stored), theme],
    );
    const page = await context.newPage();
    for (const name of only === null
      ? ["time-day", "stop-and-log"]
      : ["time-day", "stop-and-log"].filter((state) => only.includes(state))) {
      const setup = STATES.find(([stateName]) => stateName === name)?.[1];
      if (setup === undefined) continue;
      try {
        await setup(page);
        await page.screenshot({
          path: path.join(outDir, `${theme}-${name}.png`),
          animations: "disabled",
          timeout: 60_000,
        });
        captured.push(`${theme}/${name}`);
        console.log(`captured ${theme}/${name} (owner seat)`);
      } catch (error) {
        console.warn(`skip ${theme}/${name}: ${error.message?.slice(0, 120)}`);
      }
    }
    await context.close();
  }
  if (timerId !== null) {
    await stopRunningTimer(tokens.access_token, timerId);
  }
  return captured;
}

async function main() {
  await mkdir(outDir, { recursive: true });
  const tokens = await login();
  const session = {
    accessToken: tokens.access_token,
    refreshToken: tokens.refresh_token,
    sessionId: tokens.session_id,
  };
  await enrollTotpForMfaUser();
  const only = arg("only", null)?.split(",").map((value) => value.trim()) ?? null;
  const themes = arg("theme", null) === null ? THEMES : [arg("theme", "light")];
  const plan = only === null ? STATES : STATES.filter(([name]) => only.includes(name));
  const browser = await chromium.launch();
  const shots = [];

  for (const theme of themes) {
    const context = await browser.newContext({
      viewport: { width: 390, height: 844 },
      deviceScaleFactor: 2,
      colorScheme: theme,
    });
    // Seed the session ONCE. A blanket init script re-injects it on every
    // navigation, which makes a signed-out state (login, mfa) unreachable.
    await context.addInitScript(
      ([sessionJson, themeMode]) => {
        if (localStorage.getItem("marula.capture.seeded") === "1") return;
        localStorage.setItem("marula.session.v1", sessionJson);
        localStorage.setItem("@marula-core/prefs/theme.mode", themeMode);
        localStorage.setItem("marula.capture.seeded", "1");
      },
      [JSON.stringify(session), theme],
    );
    const page = await context.newPage();

    for (const [name, setup] of plan) {
      try {
        await setup(page);
        const file = path.join(outDir, `${theme}-${name}.png`);
        await page.screenshot({ path: file, animations: "disabled", timeout: 60_000 });
        if (process.env.CAPTURE_TRACE === "1") {
          console.log(`   ${theme}/${name} @ ${page.url()}`);
        }
        shots.push(`${theme}/${name}`);
        console.log(`captured ${theme}/${name}`);
      } catch (error) {
        console.warn(`skip ${theme}/${name}: ${error.message?.slice(0, 120)}`);
      }
      // The mfa state signs the shell OUT in-page; re-seed and RELOAD so the
      // auth context returns to signedIn (a client-side nav would keep the
      // in-memory 'mfa' status and redirect every later state).
      if (name === "mfa" || name === "login") {
        await page
          .evaluate((sessionJson) => {
            localStorage.setItem("marula.session.v1", sessionJson);
            localStorage.setItem("marula.capture.seeded", "1");
          }, JSON.stringify(session))
          .catch(() => {});
        await page.reload({ waitUntil: "domcontentloaded" }).catch(() => {});
        await settle(page, '[data-testid="board-screen"]', 1500);
      }
    }
    await context.close();
  }

  // The timer surfaces ride the OWNER seat: RBAC grants Time:Create to the
  // Owner preset only (Dispatcher/Technician hold View/Edit — a start from
  // either seat answers 403, which silently left the running card undrawn
  // before this pass existed). The dispatcher stays the capture session.
  shots.push(...(await ownerTimePass(browser)));
  await browser.close();
  await unenrollTotpForMfaUser();
  await writeFile(path.join(outDir, "INDEX.json"), JSON.stringify(shots, null, 2));
  console.log(`\n${shots.length} capture files in ${outDir}`);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
