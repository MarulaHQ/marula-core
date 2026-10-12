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
const OUT = "design/qa-p41-ui-audit/captures";
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
  { pack: "workbench", id: "18-wrap-up-resolve", route: (t) => `/tickets/${t}`, platform: "desktop", theme: ["neutral", "dark"], stage: async (p) => {
      const b = p.getByRole("button", { name: /wrap up/i }).first();
      if (await b.count()) await b.click({ timeout: 6000 }).catch(() => {});
    } },

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
const TICKET = list[0]?.id;
if (!TICKET) throw new Error("no ticket to stage against");
console.log(`staging against ticket ${TICKET}`);

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

      // Pin the mode before the bootstrap reads it (constraint 2 above).
      await target.evaluate(({ mode, preset }) => {
        try {
          localStorage.setItem("marula-theme", mode);
          if (preset) localStorage.setItem("marula-theme-preset", preset);
          else localStorage.removeItem("marula-theme-preset");
        } catch {}
      }, { mode: mode === "dark" ? "dark" : "light", preset });

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

      if (st.stage) { await st.stage(target); await target.waitForTimeout(1500); }
      const tour = await dismissTour(target);

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
