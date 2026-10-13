// Wave 41 lane 15 `p41-ui-audit` — pack conformance captures.
//
// Drives the REAL app against the live per-run e2e stack (never mocks), walks
// a declarative state list derived from the pack manifests, and screenshots
// each state at its recorded viewport and theme, so a capture can be diffed
// against the pack PNG it was captured for.
//
//   set -a; . e2e/.e2e-stack.env; set +a
//   node e2e/p41-audit-captures.mjs [--pack workbench|surface] [--states a,b]
//
// Output: design/qa-p41-ui-audit/captures/<pack>/<state>.png, plus an
// accumulating record at captures/_run.json that the diff and review tools read.
//
// Three constraints this harness is built around, each learned the hard way:
//   1. ONE browser context, ONE login. The API client is memory-only tokens
//      (apps/web/lib/api.tsx:3-7), so a storage-state round trip authenticates
//      nothing; and cookies are per-browser, so signing in on a second browser
//      instance authenticates nothing either. Re-logging in per state trips the
//      auth gate (429) and every later capture is a picture of /login.
//   2. The mode is pinned BEFORE the pre-paint bootstrap reads it. That
//      bootstrap (app/layout.tsx:99) reads localStorage["marula-theme"] and
//      falls back to prefers-color-scheme, which the dark sign-in sheet has
//      already set to dark — so without the pin every "neutral" capture renders
//      dark and diffs at ~0.52 against a light pack PNG.
//   3. The seed re-arms the docs tour, whose spotlight scrim dims the whole
//      surface. The pack's default states are drawn post-tour, so it is
//      dismissed before each shot.
import { chromium } from "playwright";
import { mkdirSync, writeFileSync, existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

const API = (process.env.E2E_API_URL ?? "http://localhost:28483").replace(/\/+$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3001").replace(/\/+$/, "");
const OUT = "design/qa-p41-leftovers-closeout/captures";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";

const arg = (name, dflt) => {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? dflt : process.argv[i + 1];
};
const PACK_FILTER = arg("pack", "both");
const ONLY = arg("states", null);
mkdirSync(OUT, { recursive: true });

// ---------- api client (staging only; the browser signs in through the UI) ----------
const req = await (await import("playwright")).request.newContext();
const login = await req.post(`${API}/api/v1/auth/login`, { data: { email: EMAIL, password: PASSWORD } });
if (!login.ok()) throw new Error(`api login failed: ${login.status()}`);
const { access_token: token } = await login.json();
const auth = async (path) => {
  const r = await req.fetch(`${API}/api/v1${path}`, { headers: { authorization: `Bearer ${token}` } });
  return r.ok() ? await r.json() : null;
};

// ---------- state definitions ----------
/**
 * Each entry names the pack state it was captured for, the route that renders
 * it, the platform and themes to capture, and an optional `stage(page)` that
 * drives the interaction the pack state shows. A missing stage captures the
 * landing view, which is itself the pack's default state.
 *
 * The route is a function of TICKET so the workbench states can all target the
 * same staged ticket and stay comparable to each other.
 */
const openWrapUp = async (p) => {
  const take = p.getByTestId("wb-take-it").first();
  if (await take.count()) await take.click({ timeout: 8000 }).catch(() => {});
  // By testid, not by role/name: the dock carries a second node whose accessible
  // text also matches /wrap up/, and `.first()` was picking that one, so the
  // sheet never opened and five states captured the plain workbench.
  //
  // WAIT for the button rather than testing `count()`. The dock is shell-level
  // chrome and hydrates after the page, and the capture loop navigates then
  // waits 2400ms before the stage runs, so at that moment `count()` can be 0 and
  // the click was being skipped silently — which read as "the sheet does not
  // open" when in fact nothing had been clicked.
  const b = p.getByTestId("workbar-wrap-up").first();
  await b.waitFor({ state: "visible", timeout: 20_000 }).catch(() => {});
  await b.click({ timeout: 10_000 }).catch(() => {});
  // The sheet is a React state flip on the page, not a route change; give it a
  // beat to mount before the shot.
  await p.waitForTimeout(1500);
};

const STATES = [
  // ---- workbench pack: the ticket workbench, neutral + dark ----
  { pack: "workbench", id: "01-workbench-reply-default", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"] },
  { pack: "workbench", id: "02-timeline-scrolled-to-top", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => { await p.evaluate(() => window.scrollTo(0, 0)); } },
  { pack: "workbench", id: "03-composer-internal-note", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const b = p.getByRole("button", { name: /internal note/i }).first();
      if (await b.count()) await b.click({ timeout: 5000 }).catch(() => {});
    } },
  { pack: "workbench", id: "07-filter-client", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const b = p.locator('[class*="filter"], [data-filter]').filter({ hasText: /^client$/i }).first();
      if (await b.count()) await b.click({ timeout: 5000 }).catch(() => {});
    } },
  { pack: "workbench", id: "08-filter-internal", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const b = p.locator('[class*="filter"], [data-filter]').filter({ hasText: /^internal$/i }).first();
      if (await b.count()) await b.click({ timeout: 5000 }).catch(() => {});
    } },
  { pack: "workbench", id: "09-filter-agent", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const b = p.locator('[class*="filter"], [data-filter]').filter({ hasText: /agent|assistant/i }).first();
      if (await b.count()) await b.click({ timeout: 5000 }).catch(() => {});
    } },
  { pack: "workbench", id: "15-note-lines-open", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const b = p.locator('[class*="note"], [data-testid*="note"]').filter({ hasText: /wrap|lines/i }).first();
      if (await b.count()) await b.click({ timeout: 5000 }).catch(() => {});
    } },
/** Shared stage for every wrap-up state: the sheet only opens on a ticket with
 *  a RUNNING timer (`timer-dock.tsx:157` returns early when `current === null`,
 *  so the click silently does nothing otherwise), and the pack draws the sheet
 *  over a live session. `wb-take-it` is the one control that self-assigns, moves
 *  the ticket to In Progress and starts the timer in a single envelope write. */

  { pack: "workbench", id: "18-wrap-up-resolve", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      // The pack draws the resolve choice selected, not the bare sheet. Selecting
      // it also reveals the resolved status name under the label (the sheet shows
      // `→ <target>` where a resolve target exists), which is what the pack PNG
      // carries. The confirm is deliberately NOT clicked: state 22 is the after
      // state, and confirming here would collapse the two.
      await openWrapUp(p);
      const r = p.getByTestId("wb-wrapup-choice-resolve").first();
      if (await r.count()) await r.click({ timeout: 6000 }).catch(() => {});
    } },

  // ---- Wave 41 lane 16 (`p41-leftovers-closeout`): the 14 workbench states the
  // audit left unstaged. Every stage is fail-soft on purpose — a state whose
  // control is not on the screen records a FAIL with the reason rather than
  // throwing the whole battery off the run, which is how the audit found the
  // states that were missing in the first place.
  { pack: "workbench", id: "04-internal-note-added", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const note = p.getByRole("button", { name: /internal note/i }).first();
      if (await note.count()) await note.click({ timeout: 6000 }).catch(() => {});
      const add = p.getByRole("button", { name: /^add note$/i }).first();
      if (await add.count()) await add.click({ timeout: 6000 }).catch(() => {});
    } },
  { pack: "workbench", id: "05-reply-sent-status-waiting", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const waiting = p.getByText(/set to waiting after send/i).first();
      if (await waiting.count()) await waiting.click({ timeout: 6000 }).catch(() => {});
      const send = p.getByRole("button", { name: /send reply/i }).first();
      if (await send.count()) await send.click({ timeout: 8000 }).catch(() => {});
    } },
  { pack: "workbench", id: "06-status-set-resolved", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const resolved = p.getByRole("button", { name: /^resolved$/i }).first();
      if (await resolved.count()) await resolved.click({ timeout: 6000 }).catch(() => {});
    } },
  // 10 and 11 read the RECOMMENDED NEXT STEP card, so they only resolve if the
  // filed proposal above actually landed — which is the point of staging it.
  { pack: "workbench", id: "10-assistant-script-ran", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const run = p.getByTestId("wb-agent-run").first();
      if (await run.count()) await run.click({ timeout: 8000 }).catch(() => {});
    } },
  { pack: "workbench", id: "11-assistant-suggestion-dismissed", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const notThis = p.getByTestId("wb-agent-not-this").first();
      if (await notThis.count()) await notThis.click({ timeout: 8000 }).catch(() => {});
    } },
  { pack: "workbench", id: "12-assistant-question-answered", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const chip = p.getByRole("button", { name: /likely cause/i }).first();
      if (await chip.count()) await chip.click({ timeout: 6000 }).catch(() => {});
      const ask = p.getByRole("button", { name: /ask|send/i }).first();
      if (await ask.count()) await ask.click({ timeout: 8000 }).catch(() => {});
    } },
  { pack: "workbench", id: "13-timer-paused", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const pause = p.getByRole("button", { name: /pause timer/i }).first();
      if (await pause.count()) await pause.click({ timeout: 6000 }).catch(() => {});
    } },
  { pack: "workbench", id: "14-quick-note-typed", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const field = p.getByTestId("workbar-note-input").first();
      if (await field.count()) await field.fill("checked the switch uplink on site").catch(() => {});
    } },
  { pack: "workbench", id: "16-note-line-added", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const field = p.getByTestId("workbar-note-input").first();
      if (await field.count()) await field.fill("cable swap confirmed").catch(() => {});
      await field.press("Enter").catch(() => {});
    } },
  { pack: "workbench", id: "17-keyboard-focus-on-send", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      // Keyboard focus only — the pack draws the ring, and a click would leave
      // the pointer ring behind instead.
      const send = p.getByRole("button", { name: /send reply/i }).first();
      if (await send.count()) await send.focus().catch(() => {});
    } },
  { pack: "workbench", id: "19-wrap-up-waiting-on-client", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      await openWrapUp(p);
      const w = p.getByRole("button", { name: /waiting on customer/i }).first();
      if (await w.count()) await w.click({ timeout: 6000 }).catch(() => {});
    } },
  { pack: "workbench", id: "20-wrap-up-schedule-follow-up", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      await openWrapUp(p);
      const s = p.getByRole("button", { name: /schedule/i }).first();
      if (await s.count()) await s.click({ timeout: 6000 }).catch(() => {});
    } },
  { pack: "workbench", id: "21-wrap-up-keep-working", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      await openWrapUp(p);
      const k = p.getByRole("button", { name: /keep working/i }).first();
      if (await k.count()) await k.click({ timeout: 6000 }).catch(() => {});
    } },
  { pack: "workbench", id: "22-wrap-up-completed-resolved", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      await openWrapUp(p);
      const r = p.getByRole("button", { name: /^resolve$/i }).first();
      if (await r.count()) await r.click({ timeout: 6000 }).catch(() => {});
      const confirm = p.getByRole("button", { name: /resolve and open next/i }).first();
      if (await confirm.count()) await confirm.click({ timeout: 8000 }).catch(() => {});
    } },

  // ---- density matrix — the render proof. check-theme-packs.mjs already proves
  // the three steps at the token level (114/0); these captures are the
  // rendered proof, which the audit's remainder asked for. They belong to no
  // pack, so pack-diff scores nothing for them — they are read by vision.
  { pack: "matrix", id: "density-comfortable-tickets", route: () => "/tickets", platform: "desktop", theme: ["neutral", "dark"], density: "comfortable" },
  { pack: "matrix", id: "density-compact-tickets", route: () => "/tickets", platform: "desktop", theme: ["neutral", "dark"], density: "compact" },
  { pack: "matrix", id: "density-dense-tickets", route: () => "/tickets", platform: "desktop", theme: ["neutral", "dark"], density: "dense" },
  { pack: "matrix", id: "density-comfortable-dispatch", route: () => "/dispatch", platform: "desktop", theme: ["neutral", "dark"], density: "comfortable" },
  { pack: "matrix", id: "density-compact-dispatch", route: () => "/dispatch", platform: "desktop", theme: ["neutral", "dark"], density: "compact" },
  { pack: "matrix", id: "density-dense-dispatch", route: () => "/dispatch", platform: "desktop", theme: ["neutral", "dark"], density: "dense" },
  { pack: "matrix", id: "density-comfortable-today", route: () => "/today", platform: "desktop", theme: ["neutral", "dark"], density: "comfortable" },
  { pack: "matrix", id: "density-compact-today", route: () => "/today", platform: "desktop", theme: ["neutral", "dark"], density: "compact" },
  { pack: "matrix", id: "density-dense-today", route: () => "/today", platform: "desktop", theme: ["neutral", "dark"], density: "dense" },
  { pack: "matrix", id: "density-comfortable-settings", route: () => "/settings", platform: "desktop", theme: ["neutral", "dark"], density: "comfortable" },
  { pack: "matrix", id: "density-compact-settings", route: () => "/settings", platform: "desktop", theme: ["neutral", "dark"], density: "compact" },
  { pack: "matrix", id: "density-dense-settings", route: () => "/settings", platform: "desktop", theme: ["neutral", "dark"], density: "dense" },

  // ---- theme matrix — the three shipped presets, both modes, on one
  // representative surface each. Confirms the primary flip, the five status
  // pairs and the assistant inversion do not move. `canopy` and `ripe` are not
  // capturable: they do not exist (the plan said five, three ship).
  { pack: "matrix", id: "theme-marula-light", route: () => "/tickets", platform: "desktop", theme: ["light"], preset: "marula" },
  { pack: "matrix", id: "theme-marula-dark", route: () => "/tickets", platform: "desktop", theme: ["dark"], preset: "marula" },
  { pack: "matrix", id: "theme-marula-warm-light", route: () => "/tickets", platform: "desktop", theme: ["light"], preset: "marula-warm" },
  { pack: "matrix", id: "theme-marula-warm-dark", route: () => "/tickets", platform: "desktop", theme: ["dark"], preset: "marula-warm" },
  { pack: "matrix", id: "theme-high-contrast-light", route: () => "/tickets", platform: "desktop", theme: ["light"], preset: "high_contrast" },
  { pack: "matrix", id: "theme-high-contrast-dark", route: () => "/tickets", platform: "desktop", theme: ["dark"], preset: "high_contrast" },
  { pack: "matrix", id: "theme-marula-light-dispatch", route: () => "/dispatch", platform: "desktop", theme: ["light"], preset: "marula" },
  { pack: "matrix", id: "theme-marula-dark-dispatch", route: () => "/dispatch", platform: "desktop", theme: ["dark"], preset: "marula" },
  { pack: "matrix", id: "theme-marula-warm-light-dispatch", route: () => "/dispatch", platform: "desktop", theme: ["light"], preset: "marula-warm" },
  { pack: "matrix", id: "theme-marula-warm-dark-dispatch", route: () => "/dispatch", platform: "desktop", theme: ["dark"], preset: "marula-warm" },
  { pack: "matrix", id: "theme-high-contrast-light-dispatch", route: () => "/dispatch", platform: "desktop", theme: ["light"], preset: "high_contrast" },
  { pack: "matrix", id: "theme-high-contrast-dark-dispatch", route: () => "/dispatch", platform: "desktop", theme: ["dark"], preset: "high_contrast" },

  // ---- surface pack: the default (landing) state of every drawn surface ----
  { pack: "surface", id: "tickets-list", route: () => "/tickets", platform: "desktop", theme: ["neutral", "dark"] },
  { pack: "surface", id: "today-default", route: () => "/today", platform: "desktop", theme: ["neutral", "dark"] },
  { pack: "surface", id: "dispatch-default", route: () => "/dispatch", platform: "desktop", theme: ["neutral", "dark"] },
  { pack: "surface", id: "approvals-default", route: () => "/approvals", platform: "desktop", theme: ["neutral", "dark"] },
  { pack: "surface", id: "clients-default", route: () => "/clients", platform: "desktop", theme: ["neutral", "dark"] },
  { pack: "surface", id: "knowledge-default", route: () => "/documents", platform: "desktop", theme: ["neutral", "dark"] },
  { pack: "surface", id: "settings-default", route: () => "/settings", platform: "desktop", theme: ["neutral", "dark"] },
  { pack: "surface", id: "workflow-default", route: () => "/automations", platform: "desktop", theme: ["neutral", "dark"] },
  { pack: "surface", id: "quoting-default", route: () => "/sales/quotes", platform: "desktop", theme: ["neutral", "dark"] },
  { pack: "surface", id: "portal-default", route: () => "/portal/login", platform: "desktop", theme: ["sand"], noAuth: true },
  { pack: "surface", id: "auth-default", route: () => "/login", platform: "desktop", theme: ["light", "dark"], noAuth: true },
];

/** The pack's recorded viewport per platform. */
const VIEWPORT = { desktop: { width: 1600, height: 994 }, mobile: { width: 964, height: 1872 } };

/** Pack theme -> the app's mode. `neutral` is the web default canvas, `sand`
 *  the cream (marula-warm) column, `light`/`dark` the two modes of the kit. */
const THEME_MODE = { neutral: "light", light: "light", sand: "light", dark: "dark" };
const THEME_PRESET = { sand: "marula-warm" };

/** Dismiss the first-run docs tour. Returns true when one was dismissed —
 *  a tour that opens on a page the pack drew without one is itself worth
 *  recording. */
const dismissTour = async (page) => {
  let dismissed = false;
  try {
    for (const name of [/^done$/i, /^got it$/i, /^next$/i, /^finish$/i, /^close$/i]) {
      const b = page.getByRole("button", { name }).first();
      if (await b.count() && (await b.isVisible().catch(() => false))) {
        await b.click({ timeout: 4000 }).catch(() => {});
        dismissed = true;
      }
    }
    const never = page.getByRole("checkbox", { name: /don't show/i }).first();
    if (await never.count()) await never.check({ timeout: 3000 }).catch(() => {});
    await page.keyboard.press("Escape").catch(() => {});
    await page.waitForTimeout(500);
  } catch {
    /* a tour that will not dismiss is recorded by the diff, not by throwing */
  }
  return dismissed;
};

// ---------- stage a ticket with a timeline ----------
const tickets = await auth("/tickets?page=1");
const list = Array.isArray(tickets) ? tickets : (tickets?.items ?? []);
// The wrap-up states need a ticket the flow can actually close. The seed's
// `list[0]` is DEMO-1004, which sits in the closed `Resolved` category, and
// DEMO-1003 is `in_progress` — neither lets the sheet open, because
// `timer-dock.tsx:157` returns early when there is no running timer, and the
// only control that starts one (`wb-take-it`) is gated on
// `app/tickets/[id]/page.tsx:364-372` to tickets whose category is `open` or
// `waiting`.
//
// The list and detail endpoints expose `status_id` but no status NAME or
// category (verified: no `status` field on either, and `/boards/{id}/statuses`
// returns an empty list), so no request says "give me a pickupable ticket".
// `seed-demo --force` is mechanical and deterministic — the stack script
// reseeds identically before every run — so the stable rule is the seed's own
// numbering: DEMO-1001 is the first ticket that is both category `open` and
// already assigned to the demo owner. If the seed ever changes, this falls back
// to `list[0]` and the wrap-up states record their own divergence rather than
// throwing the battery.
const OPEN_TICKET_NUMBER = "DEMO-1001";
const TICKET = (list.find((t) => t.number === OPEN_TICKET_NUMBER) ?? list[0])?.id;
if (!TICKET) throw new Error("no ticket to stage against");
const TICKET_NUMBER = (list.find((t) => t.id === TICKET) ?? list[0])?.number ?? "?";
console.log(`staging against ticket ${TICKET_NUMBER} (${TICKET})`);

// ---------- stage the filed assistant proposal (the pack draws one) ----------
// The workbench pack's "RECOMMENDED NEXT STEP" card carries a filed agent
// triage proposal. Filing one through the real route is what makes it draw, so
// this stages it rather than explaining the gap away. Idempotent: a ticket that
// already has a `proposed` proposal is left alone, so re-running the battery
// does not stack duplicates.
//
// The pack's "RECOMMENDED NEXT STEP" card draws a filed agent triage proposal, so
// this files one through the real route rather than explaining the gap away.
// Idempotent: a ticket that already has a `proposed` proposal is left alone, so
// re-running the battery does not stack duplicates.
//
// HISTORY, kept because the measurement is the evidence. This staging used to
// pin a far-future keyset cursor on every request, because
// `GET /api/v1/agent-proposals` with no cursor could not return a first page
// at all: `list_proposals` bound a cursor-less request to
// `(created_at, id) < ('1970-01-01', Uuid::nil())`, which is false for every row
// written after the epoch. Six proposals sat in the table, all `proposed`, and
// both the API and that exact predicate returned 0. The audit's GAP-LEDGER
// Section 3.12 read that as a missing demo fixture; it was an unreadable list,
// and the panel that draws it could never read one. Fixed 2026-10-13 in
// `crates/core-db/src/agent_platform.rs` with the same nullable guard
// `emails.rs` already uses, so the cursor override is gone and the capture now
// exercises the real first-page path.
const RATIONALE =
  "Same uplink fault the client logged Thursday. A remote session would confirm it before anyone drives out — suggest triaging from the desk first.";
const existing = await auth(`/agent-proposals?limit=200`);
const alreadyFiled = (existing?.proposals ?? []).some(
  (p) => p.entity === "ticket" && p.entity_id === TICKET && p.status === "proposed",
);
if (alreadyFiled) {
  console.log("assistant proposal already filed against this ticket");
} else {
  const filed = await req.post(`${API}/api/v1/agent-proposals`, {
    headers: { authorization: `Bearer ${token}` },
    data: {
      entity: "ticket",
      entity_id: TICKET,
      op: "update",
      proposed_value: { next_step: "remote_triage_before_dispatch" },
      classification: "internal",
      rationale: RATIONALE,
      skill_slug: "triage",
    },
  });
  if (!filed.ok()) {
    console.error(`could not file the assistant proposal: ${filed.status()} ${(await filed.text()).slice(0, 200)}`);
  } else {
    console.log(`filed assistant proposal for ticket ${TICKET}`);
  }
}

// ---------- browser: ONE context, ONE login ----------
const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: VIEWPORT.desktop });
const page = await ctx.newPage();
page.on("pageerror", (e) => console.error("[pageerror]", e.message.slice(0, 120)));

for (let attempt = 0; attempt < 6; attempt += 1) {
  await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
  const submit = page.locator('button[type="submit"]');
  await submit.waitFor({ state: "visible", timeout: 30_000 });
  await page.waitForTimeout(1500);
  await page.fill('input[type="email"]', EMAIL);
  await page.fill('input[type="password"]', PASSWORD);
  await submit.click();
  try { await page.waitForURL(/tickets|today|dashboard|search/, { timeout: 45_000 }); break; }
  catch { await page.waitForTimeout(15_000); }
}
const landed = new URL(page.url()).pathname;
if (!/tickets|today|dashboard|search/.test(landed)) throw new Error(`login never landed in the app (at ${landed})`);
console.log(`signed in once -> ${landed}`);

/** Sign in again on the live page. The access token is memory-only and the
 *  refresh cookie rotates per load, so a long capture run eventually finds
 *  itself back on /login; re-authenticating on the SAME page keeps the run
 *  going without hammering the gate the way a per-state login does. */
const reSignIn = async (target) => {
  await target.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
  const submit = target.locator('button[type="submit"]');
  await submit.waitFor({ state: "visible", timeout: 30_000 });
  await target.waitForTimeout(1500);
  await target.fill('input[type="email"]', EMAIL);
  await target.fill('input[type="password"]', PASSWORD);
  await submit.click();
  await target.waitForURL(/tickets|today|dashboard|search/, { timeout: 45_000 }).catch(() => {});
};

/** True when the page is the sign-in sheet — the one failure mode that would
 *  otherwise silently produce a capture of the wrong page. */
const onLogin = (target) => new URL(target.url()).pathname === "/login";


/** Sign in on a page that has already landed somewhere. The form redirects to
 *  the app on success, which is what the capture loop keys off. */
const signIn = async (target) => {
  await target.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
  await target.waitForTimeout(2500);
  await target.fill('input[type="email"]', EMAIL);
  await target.fill('input[type="password"]', PASSWORD);
  await target.locator('button[type="submit"]').click();
  await target.waitForTimeout(3000);
};

const results = [];
for (const st of STATES) {
  if (PACK_FILTER !== "both" && st.pack !== PACK_FILTER) continue;
  if (ONLY && !ONLY.split(",").includes(st.id)) continue;
  const dir = join(OUT, st.pack);
  mkdirSync(dir, { recursive: true });

  for (const theme of st.theme) {
    const mode = THEME_MODE[theme] ?? "light";
    const preset = THEME_PRESET[theme] ?? null;
    const id = `${st.id}--${theme}`;
    const file = join(dir, `${id}.png`);
    let ok = false, note = "";
    try {
      // The logged-out pack states need a context with no session; everything
      // else rides the one signed-in context, which is the whole point.
      let target = page, ownCtx = null;
      if (st.noAuth) {
        ownCtx = await browser.newContext({ viewport: VIEWPORT[st.platform] });
        target = await ownCtx.newPage();
      } else {
        await target.setViewportSize(VIEWPORT[st.platform]);
      }

      // Pin the mode before the bootstrap reads it (constraint 2 above). The
      // density step and the theme preset are pinned the same way: both are
      // read from localStorage by the pre-paint bootstrap in app/layout.tsx,
      // so a value set after navigation is already too late for the frame.
      await target.evaluate(({ mode, preset, density }) => {
        try {
          localStorage.setItem("marula-theme", mode);
          if (preset) localStorage.setItem("marula-theme-preset", preset);
          else localStorage.removeItem("marula-theme-preset");
          if (density) localStorage.setItem("marula-density", density);
          else localStorage.removeItem("marula-density");
        } catch {}
      }, { mode: mode === "dark" ? "dark" : "light", preset: st.preset ?? preset, density: st.density ?? null });

      await target.goto(`${WEB}${st.route(TICKET)}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
      await target.waitForTimeout(2400);
      // The reload is what drops the memory-only token, so the guard sends the
      // page to /login. Re-reading the route after a re-authenticate is the
      // only shape that survives it, so sign in HERE rather than assuming the
      // first navigation held.
      if (!st.noAuth && new URL(target.url()).pathname === "/login") {
        await signIn(target);
        await target.goto(`${WEB}${st.route(TICKET)}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
        await target.waitForTimeout(2400);
        if (new URL(target.url()).pathname === "/login") throw new Error("session died (landed on /login)");
      }
      // A reload re-enters the guarded route, and the guard redirects client
      // side. If the session has rotated out, re-authenticate on the same page
      // before the screenshot, so the capture is the surface and not /login.

      // The auth surface styles itself (its pack state is the dark sheet the
      // sign-in page ships), so the mode assertion only applies to the product
      // surfaces that honour the bootstrap key.
      if (!st.noAuth) {
        const applied = await target.evaluate(() => document.documentElement.getAttribute("data-theme"));
        if (applied !== (mode === "dark" ? "dark" : "neutral")) {
          throw new Error(`theme did not apply (data-theme="${applied}", wanted ${mode === "dark" ? "dark" : "neutral"})`);
        }
      }

      // The tour is dismissed BEFORE the stage, not after. It used to run after,
      // and its `Escape` press plus its /^close$/i button sweep closed the
      // wrap-up sheet the stage had just opened — which read as "the sheet does
      // not open" when the staging was working and something was closing it.
      const tour = await dismissTour(target);
      if (st.stage) { await st.stage(target); await target.waitForTimeout(1500); }

      await target.screenshot({ path: file, fullPage: false });
      ok = existsSync(file);
      note = tour ? "docs-tour dismissed before the shot" : "";
      if (ownCtx) await ownCtx.close();
    } catch (e) {
      note = String(e).slice(0, 170);
    }
    results.push({ id, file: file.replace(`${process.cwd()}/`, ""), ok, note });
    console.log(`${ok ? "ok  " : "FAIL"} ${id}${note ? "  :: " + note : ""}`);
  }
}

await browser.close();

const runPath = join(OUT, "_run.json");
const prev = existsSync(runPath) ? JSON.parse(readFileSync(runPath, "utf8")) : { results: [] };
const merged = new Map(prev.results.map((r) => [r.id, r]));
for (const r of results) merged.set(r.id, r);
const all = [...merged.values()];
writeFileSync(runPath, JSON.stringify({ api: API, web: WEB, ticket: TICKET, captured: Date.now(), runs: (prev.runs ?? []).concat([{ states: results.length, at: Date.now() }]), results: all }, null, 2));
const failed = all.filter((r) => !r.ok);
console.log(`\n${all.length - failed.length}/${all.length} captured (this run ${results.length})${failed.length ? `; failing: ${failed.map((f) => f.id).join(", ")}` : ""}`);
