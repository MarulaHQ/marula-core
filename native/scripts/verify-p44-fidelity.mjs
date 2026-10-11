/**
 * Wave 44 lane 11 (p44-ui-mobile-fidelity) — mechanical verification of the
 * fixed behaviors, driven live against the lane stack's expo web render.
 *
 * Each lane fix gets a DOM-level assertion here: a dead control that now
 * writes, a fixture that is now real data, a lie that is now honest. The
 * capture battery proves the screens DRAW; this probe proves the FIXES
 * behave. Run AFTER `bash scripts/e2e-stack.sh up` + expo web on the
 * stack's web port (same usage contract as capture-pack.mjs).
 *
 *   node native/scripts/verify-p44-fidelity.mjs
 */
import { readFile } from "node:fs/promises";
import { createHmac, randomUUID } from "node:crypto";
import { Buffer } from "node:buffer";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");

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
const EMAIL = "dispatcher@demo.local";
const PASSWORD = "demo-password-123";

const results = [];
function check(name, ok, detail = "") {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${ok || detail === "" ? "" : ` — ${detail}`}`);
}

/** RFC 6238 TOTP over the demo owner's enrolled secret (the battery's code). */
function probeTotp(base32Secret, at = Date.now()) {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  let bits = "";
  for (const char of base32Secret.replace(/=+$/, "").toUpperCase()) {
    const index = alphabet.indexOf(char);
    if (index < 0) continue;
    bits += index.toString(2).padStart(5, "0");
  }
  const bytes = [];
  for (let i = 0; i + 8 <= bits.length; i += 8) bytes.push(parseInt(bits.slice(i, i + 8), 2));
  const counter = Math.floor(at / 30_000);
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

async function settle(page, selector, ms = 1200, anchorTimeout = 20_000) {
  await page.waitForSelector(selector, { timeout: anchorTimeout }).catch(() => {});
  await page.waitForTimeout(ms);
}

// ---- sign in through the API, seed the shell session (capture-pack pattern)
const response = await fetch(
  `http://localhost:${stack.E2E_API_PORT ?? "28483"}/api/v1/auth/login`,
  {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: EMAIL, password: PASSWORD }),
  },
);
if (!response.ok) throw new Error(`login failed: ${response.status}`);
const tokens = await response.json();
// The battery's own transform: the shell stores camelCase, the API speaks
// snake — seed the shape the app reads (secure-store's StoredSession).
const session = {
  accessToken: tokens.access_token,
  refreshToken: tokens.refresh_token,
  sessionId: tokens.session_id,
};
const api = `http://localhost:${stack.E2E_API_PORT ?? "28483"}/api/v1`;
const authHeaders = {
  "content-type": "application/json",
  authorization: `Bearer ${session.accessToken}`,
};

// The running timer lives in the owner-seeded block below (Time:Create is
// an Owner-seat grant — see the RBAC note at the time checks).

const browser = await chromium.launch();
// The battery's own seeding contract: an init script that re-injects the
// session until a signed-out state clears it. A one-shot evaluate races the
// auth hydration on the first real navigation.
const context = await browser.newContext({
  viewport: { width: 390, height: 844 },
  deviceScaleFactor: 2,
  colorScheme: "light",
});
await context.addInitScript(
  (sessionJson) => {
    if (localStorage.getItem("marula.capture.seeded") === "1") return;
    localStorage.setItem("marula.session.v1", sessionJson);
    localStorage.setItem("marula.capture.seeded", "1");
  },
  JSON.stringify(session),
);
const page = await context.newPage();
page.setDefaultTimeout(20_000);

// ---- 1. More footer: the real device name, never the "Pixel 8" fixture
await page.goto(`${WEB_URL}/more`, { waitUntil: "domcontentloaded" });
await settle(page, '[data-testid="more-screen"]', 1500);
const footer = await page.getByText(/^Marula [0-9.]+ · /).first().textContent().catch(() => "");
check("more footer names a real device", footer !== "" && !footer.includes("Pixel 8"), `footer="${footer}"`);

// ---- 2. More on-call pill: derived, so it is either a real pill or absent
const pillCount = await page.getByText("On call tonight").count();
check("more on-call pill present at most once (derived, not hardcoded)", pillCount <= 1, `count=${pillCount}`);

// ---- 3. Board chips count real rows, and the scope actually filters
await page.goto(`${WEB_URL}/board`, { waitUntil: "domcontentloaded" });
await settle(page, '[data-testid="board-screen"]', 2000);
// The chips live in the scope row the filter disc opens — Mine is the
// default, so the row is hidden until the disc is pressed.
await page.getByLabel("Filter the queue").first().click().catch(() => {});
await page.waitForTimeout(800);
const chipTexts = await page.locator('[data-testid^="board-queue-"]').allTextContents().catch(() => []);
const fixtures = chipTexts.some((text) => text.includes("12") || text.includes("47"));
check("board chips are rendered (no fixture counts 12/47)", chipTexts.length >= 3 && !fixtures, `chips=${JSON.stringify(chipTexts)}`);
const mineChips = chipTexts.find((text) => text.startsWith("Mine"));
const mineCount = Number(/(\d+)/.exec(mineChips ?? "")?.[1] ?? "-1");
const mineRows = await page.locator('[data-testid^="ticket-row-"]').count();
check("mine scope rows match the chip count", mineCount >= 0 && mineRows === mineCount, `chip=${mineCount} rows=${mineRows}`);
// The scope switch: take Unassigned, the list re-scopes.
const unassignedChip = page.locator('[data-testid="board-queue-unassigned"]');
if ((await unassignedChip.count()) > 0) {
  await unassignedChip.first().click();
  await page.waitForTimeout(900);
  const unassignedCount = Number(
    /(\d+)/.exec((await unassignedChip.first().textContent()) ?? "")?.[1] ?? "-1",
  );
  const unassignedRows = await page.locator('[data-testid^="ticket-row-"]').count();
  check("unassigned scope re-scopes the list to the queue", unassignedRows === unassignedCount, `chip=${unassignedCount} rows=${unassignedRows}`);
} else {
  check("unassigned scope re-scopes the list to the queue", false, "chip missing after disc open");
}

// ---- 4. Ticket detail: real client/contact data, no Acme/JOHN-LAPTOP fixture
await page.goto(`${WEB_URL}/board`, { waitUntil: "domcontentloaded" });
await settle(page, '[data-testid="ticket-list"]', 1500);
const firstRow = page.locator('[data-testid^="ticket-row-"]').first();
if ((await firstRow.count()) > 0) {
  await firstRow.click();
  await settle(page, '[data-testid="ticket-action-note"]', 2500, 120_000);
  const meta = await page.getByText("JOHN-LAPTOP").count();
  check("ticket meta line carries no JOHN-LAPTOP fixture", meta === 0, `found=${meta}`);
  const fakeAuthor = await page.getByText("Tumi L.").count();
  check("conversation carries no fake author", fakeAuthor === 0, `found=${fakeAuthor}`);
  const systemLine = await page.getByText("Status changed to In progress").count();
  check("conversation carries no hardcoded system line", systemLine === 0, `found=${systemLine}`);
} else {
  check("ticket detail rendered a row", false, "no rows on board");
}

// ---- 5. Login: the Microsoft button states the truth (never routes to /mfa)
await page.goto(`${WEB_URL}/login`, { waitUntil: "domcontentloaded" });
await page.evaluate(() => localStorage.removeItem("marula.session.v1")).catch(() => {});
await page.reload({ waitUntil: "domcontentloaded" }).catch(() => {});
await settle(page, '[data-testid="login-email"]', 1200);
const sso = page.locator('[data-testid="login-sso"]');
if ((await sso.count()) > 0) {
  await sso.first().click();
  await page.waitForTimeout(1200);
  const url = page.url();
  const note = await page.getByText(/Microsoft sign-in/).first().textContent().catch(() => "");
  const said = note !== null && note !== "";
  check("sso button does not fake-route to /mfa", !url.includes("/mfa"), `url=${url}`);
  check("sso button names the org's real state", said, `note="${note ?? ""}"`);
} else {
  check("sso button rendered", false, "missing");
}

// The login check signed the shell out (the battery does the same for its
// login state): re-seed and reload so the checks after it keep a session.
await page
  .evaluate((sessionJson) => {
    localStorage.setItem("marula.session.v1", sessionJson);
    localStorage.setItem("marula.capture.seeded", "1");
  }, JSON.stringify(session))
  .catch(() => {});
await page.reload({ waitUntil: "domcontentloaded" }).catch(() => {});

// ---- 6. Scan: the torch disc is a real control (gold when on, state flips)
await page.goto(`${WEB_URL}/scan`, { waitUntil: "domcontentloaded" });
await settle(page, '[data-testid="scan-screen"]', 4000, 120_000);
const torch = page.locator('[data-testid="scan-torch"]');
if ((await torch.count()) > 0) {
  const before = await torch.first().getAttribute("aria-label");
  await torch.first().click();
  await page.waitForTimeout(400);
  const after = await torch.first().getAttribute("aria-label");
  check("torch disc toggles state", before !== after, `label ${before} -> ${after}`);
} else {
  check("torch disc rendered", false, "missing");
}

const recentLabel = await page.getByText("Try a code").count();
check("scan quick payloads are labeled honestly (not 'Recent')", recentLabel === 1, `found=${recentLabel}`);

// ---- 7. Scan: the ambiguity picker preselects and names its choice
await page.goto(`${WEB_URL}/scan`, { waitUntil: "domcontentloaded" });
await settle(page, '[data-testid="scan-input"]', 900);
await page.fill('[data-testid="scan-input"]', "DOCK-01").catch(() => {});
await page.click('[data-testid="scan-resolve"]').catch(() => {});
const pickSheet = page.locator('[data-testid="scan-pick-sheet"]');
await pickSheet.waitFor({ timeout: 12_000 }).catch(() => {});
if ((await pickSheet.count()) > 0) {
  // The preselection contract, read through the primary's label: it names
  // the resolver's first row (the best match) before any tap, and flips to
  // whichever card is tapped last — reversible until Continue.
  const cardTitles = await page
    .locator('[role="radio"]')
    .evaluateAll((nodes) => nodes.map((node) => (node.textContent ?? "").trim()));
  const continueLabel = await page
    .locator('[data-testid="scan-pick-continue"]')
    .textContent()
    .catch(() => "");
  // The card's text is title + detail, so the label's name is a prefix.
  const names = (index) => {
    const named = /Continue with (.+)$/.exec(continueLabel ?? "");
    const title = cardTitles[index];
    return named !== null && title !== undefined && title.startsWith(named[1] ?? "");
  };
  const namesFirst = cardTitles.length >= 2 && names(0);
  check("ambiguity picker preselects the best match (primary names it)", namesFirst, `cards=${JSON.stringify(cardTitles)} label="${continueLabel}"`);
  await page.locator('[role="radio"]').nth(1).click();
  await page.waitForTimeout(400);
  const afterLabel = await page
    .locator('[data-testid="scan-pick-continue"]')
    .textContent()
    .catch(() => "");
  const afterNamed = /Continue with (.+)$/.exec(afterLabel ?? "");
  const flips =
    afterNamed !== null &&
    cardTitles[1] !== undefined &&
    cardTitles[1].startsWith(afterNamed[1] ?? "");
  check("picker choice is reversible until Continue", flips, `label="${afterLabel}"`);
} else {
  check("ambiguity picker opened", false, "sheet missing — seed may lack DOCK-01 candidates");
}

// ---- 8. Scan: the unknown sheet offers the pack's ways out
await page.goto(`${WEB_URL}/scan`, { waitUntil: "domcontentloaded" });
await settle(page, '[data-testid="scan-input"]', 900);
await page.fill('[data-testid="scan-input"]', "NOPE-NOPE-000").catch(() => {});
await page.click('[data-testid="scan-resolve"]').catch(() => {});
await page.locator('[data-testid="scan-unknown-sheet"]').waitFor({ timeout: 12_000 }).catch(() => {});
const waysOut = await page.getByText("Scan again").count();
check("unknown sheet offers Scan again (three ways out)", waysOut === 1, `found=${waysOut}`);

// ---- 8b. Time tab: the running card's Pause/Resume writes for real
// RBAC note (found during this gate): the Dispatcher and Technician presets
// hold Time as View/Edit only — POST /timers/start needs Create, so the
// running-timer surfaces are exercisable on the Owner seat alone. The
// dispatcher stays the capture session (its scan + stock grants); the time
// checks get their own owner-seeded context, with the owner's TOTP enrolled
// through the battery's enroll contract.
const ownerContext = await browser.newContext({
  viewport: { width: 390, height: 844 },
  deviceScaleFactor: 2,
  colorScheme: "light",
});
{
  const ownerSignIn = await fetch(`${api}/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: "owner@demo.local", password: PASSWORD }),
  });
  const ownerBody = await ownerSignIn.json();
  let ownerTokens = ownerBody;
  if (typeof ownerBody.access_token !== "string") {
    // The demo owner is TOTP-enrolled (the battery enrolls it for its mfa
    // state). The secret lives in the capture file the battery wrote; when
    // it is absent, enroll fresh through the battery's contract.
    let secret = null;
    try {
      secret = JSON.parse(await readFile(path.join(root, ".kilo/capture-totp.json"), "utf8"))
        .secret_base32;
    } catch {
      secret = null;
    }
    if (secret === null) {
      const begin = await fetch(`${api}/auth/mfa/totp/enroll-begin`, {
        method: "POST",
        headers: { authorization: `Bearer ${ownerBody.mfa_token}` },
      });
      ({ secret_base32: secret } = await begin.json());
    }
    const verified = await fetch(`${api}/auth/mfa/verify`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ mfa_token: ownerBody.mfa_token, code: probeTotp(secret) }),
    }).then((r) => r.json());
    ownerTokens = verified;
  }
  const ownerStored = {
    accessToken: ownerTokens.access_token,
    refreshToken: ownerTokens.refresh_token,
    sessionId: ownerTokens.session_id,
  };
  const ownerHeaders = {
    "content-type": "application/json",
    authorization: `Bearer ${ownerStored.accessToken}`,
  };
  const ownerWork = await fetch(`${api}/tickets/my-work`, { headers: ownerHeaders });
  const ownerRows = (await ownerWork.json()) ?? [];
  const ownerTicket = ownerRows.find((row) => row.category !== "resolved") ?? ownerRows[0];
  let ownerTimerId = null;
  if (ownerTicket !== undefined) {
    const started = await fetch(`${api}/timers/start`, {
      method: "POST",
      headers: ownerHeaders,
      body: JSON.stringify({ ticket_id: ownerTicket.id, idempotency_key: crypto.randomUUID() }),
    });
    const startedBody = await started.json();
    ownerTimerId = startedBody?.id ?? null;
    if (!started.ok) console.log(`   owner timer start: ${started.status}`);
  }
  await ownerContext.addInitScript(
    (sessionJson) => {
      if (localStorage.getItem("marula.capture.seeded") === "1") return;
      localStorage.setItem("marula.session.v1", sessionJson);
      localStorage.setItem("marula.capture.seeded", "1");
    },
    JSON.stringify(ownerStored),
  );
  const ownerPage = await ownerContext.newPage();
  await ownerPage.goto(`${WEB_URL}/time`, { waitUntil: "domcontentloaded" });
  await settle(ownerPage, '[data-testid="time-screen"]', 1200);
  if (process.env.PROBE_DEBUG === "1") {
    console.log(
      `   [debug] owner url=${ownerPage.url()} timer=${ownerTimerId ?? "none"} body="${(await ownerPage.evaluate(() => document.body.innerText)).slice(0, 140).replace(/\n+/g, " | ")}"`,
    );
  }
  await settle(ownerPage, '[data-testid="time-running-card"]', 3500, 90_000);
  const pause = ownerPage.locator('[data-testid="time-pause"]');
  if ((await pause.count()) > 0) {
    await pause.first().click();
    await settle(ownerPage, '[data-testid="time-running-card"]', 3500);
    const resume = await ownerPage.locator('[data-testid="time-resume"]').count();
    check("time pause button writes (card flips to Resume)", resume === 1, `resume buttons=${resume}`);
    const back = ownerPage.locator('[data-testid="time-resume"]');
    if ((await back.count()) > 0) {
      await back.first().click();
      await settle(ownerPage, '[data-testid="time-running-card"]', 3500);
      const runningAgain = await ownerPage.locator('[data-testid="time-pause"]').count();
      check("time resume button writes (card flips back to Pause)", runningAgain === 1, `pause buttons=${runningAgain}`);
    }
  } else {
    check("time running card shows Pause", false, "pause button missing");
  }
  const stop = ownerPage
    .locator('[data-testid="time-running-card"] button', { hasText: "Stop and log" })
    .first();
  if ((await stop.count()) > 0) {
    await stop.click();
    await ownerPage.waitForTimeout(1200);
    const stopSheet = await ownerPage.locator('[data-testid="stop-log-sheet"]').count();
    check("stop and log opens the log sheet", stopSheet === 1, `sheets=${stopSheet}`);
  } else {
    check("stop and log primary rendered", false, "missing");
  }
  if (ownerTimerId !== null) {
    await fetch(`${api}/timers/stop`, {
      method: "POST",
      headers: ownerHeaders,
      body: JSON.stringify({ session_id: ownerTimerId }),
    }).catch(() => {});
  }
  await ownerContext.close();
}

// ---- 8c. Scan offline-save: the pack's scan-10 (the lane's new state)
// NAVIGATION RULE (the battery's offline-trio contract): forcedOffline is
// in-memory state — every move after the toggle is IN-APP, never a goto.
await page.goto(`${WEB_URL}/scan`, { waitUntil: "domcontentloaded" });
await settle(page, '[data-testid="scan-screen"]', 2000, 120_000);
await page.goto(`${WEB_URL}/more`, { waitUntil: "domcontentloaded" });
await settle(page, '[data-testid="more-airplane-toggle"]', 1500);
await page.locator('[data-testid="more-airplane-toggle"]').click();
await settle(page, '[data-testid="banner-offline"]', 2500);
await page.getByLabel("Scan, tab").first().click();
await settle(page, '[data-testid="scan-screen"]', 2000);
await page.fill('[data-testid="scan-input"]', "VAN-SHELF-42").catch(() => {});
await page.click('[data-testid="scan-resolve"]').catch(() => {});
const savedSheet = page.locator('[data-testid="scan-saved-sheet"]');
await savedSheet.waitFor({ timeout: 15_000 }).catch(() => {});
const savedVisible = (await savedSheet.count()) > 0;
const savedRow = await page.getByText(/Saved for later · \d+ waiting/).count();
check("offline scan is saved, not lost (Saved for later sheet + row)", savedVisible && savedRow >= 1, `sheet=${savedVisible} row=${savedRow}`);
// Back online in-session: the shelf drains and the code resolves by itself.
// Dismiss the saved sheet first — it covers the tab bar.
await page.locator('[data-testid="scan-saved-sheet"] button').first().click().catch(() => {});
await page.waitForTimeout(400);
await page.getByRole("tab", { name: /More/ }).first().click();
await settle(page, '[data-testid="more-airplane-toggle"]', 1500);
await page.locator('[data-testid="more-airplane-toggle"]').click();
await settle(page, '[data-testid="banner-offline"]', 1000, 8_000);
await page.waitForTimeout(2500);

await browser.close();

const failed = results.filter((row) => !row.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
if (failed.length > 0) process.exit(1);
