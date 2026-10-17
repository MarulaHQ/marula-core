// p47a21-loop-meter.mjs — does /tickets re-render forever, and is THAT what
// kills client-side navigation?
//
// Wave 47 lane A21. The nav probe (e2e/p47a21-nav-probe.mjs) showed that from
// /tickets no transition ever reaches `history.pushState`, while every history
// entry point works. The hypothesis this meter tests: the ledger is stuck in a
// render loop, and a pegged render loop starves the App Router's
// `startTransition`, so a `router.push` never commits.
//
// It measures the SAME page three ways, so the difference is the finding
// rather than an assertion:
//
//   A. /tickets, no saved surface document  -> the demo/fresh-account shape
//   B. /tickets, a saved surface document   -> `view()` returns a stable object
//   C. /assets                              -> the control A15 measured working
//
// Per phase it records:
//   mutations  — MutationObserver count over the window (render churn);
//   frames     — requestAnimationFrame ticks (main-thread responsiveness);
//   errors     — console errors, including React's "Maximum update depth";
//   pushState  — how many history.pushState calls the page made;
//   nav        — whether a direct router.push("/clients") moves the URL.
//
// Run: node e2e/p47a21-loop-meter.mjs   (stack up, web built + served)

import { chromium } from "playwright";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const require = createRequire(import.meta.url);
const { resolveEvidenceDir } = require("./helpers/evidence-core.cjs");

const env = Object.fromEntries(
  readFileSync("e2e/.e2e-stack.env", "utf8")
    .split("\n")
    .filter((l) => l.includes("="))
    .map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]),
);
const API = env.E2E_API_URL.replace(/\/+$/, "");
const WEB = env.E2E_WEB_URL;
const REPO = process.cwd();
const OUT = resolveEvidenceDir(
  { override: process.env.E2E_EVIDENCE_DIR, branch: "p47-tickets-route-nav-pin", sha: "", runId: env.MARULA_RUN_ID ?? "" },
  resolve(REPO),
);
mkdirSync(OUT, { recursive: true });

/** Counts mutations, rAF ticks and pushState calls for `ms`, then tears the
 *  observers down so phases cannot bleed into each other. */
const MEASURE = `
async (ms) => {
  const state = { mutations: 0, frames: 0, pushState: 0 };
  const obs = new MutationObserver((records) => { state.mutations += records.length; });
  obs.observe(document.documentElement, { subtree: true, childList: true, attributes: true, characterData: true });
  const original = window.history.pushState;
  window.history.pushState = function (...args) { state.pushState += 1; return original.apply(this, args); };
  let running = true;
  const tick = () => { if (!running) return; state.frames += 1; requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
  await new Promise((r) => setTimeout(r, ms));
  running = false;
  obs.disconnect();
  window.history.pushState = original;
  return state;
}
`;
const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
// A settings WRITE needs the bearer token; the session cookie the page uses is
// not enough, and a request that 401s here fails silently unless it is checked
// (the nav probe shipped with that bug, which is why its tour stayed up).
const login = await ctx.request.post(`${API}/api/v1/auth/login`, {
  data: { email: "owner@demo.local", password: "demo-password-123" },
});
const TOKEN = (await login.json()).access_token;
if (typeof TOKEN !== "string" || TOKEN === "") throw new Error("login returned no access_token");
const auth = { authorization: `Bearer ${TOKEN}` };
const disabled = await ctx.request.put(`${API}/api/v1/settings/user`, {
  headers: auth,
  data: { key: "tours.disabled", value: true },
});
if (!disabled.ok()) throw new Error(`could not disable tours (${disabled.status()})`);

// This meter rewrites the demo owner's surface document, which every other spec
// shares. Read it once so the run can put it back: an instrument that leaves the
// demo org in a state no other lane expects is a flake generator.
const settingsRows = await ctx.request.get(`${API}/api/v1/settings/user`, { headers: auth });
const rows = await settingsRows.json();
const SURFACE_KEY = "appearance.data_surfaces";
const originalSurface = rows.find((row) => row.key === SURFACE_KEY)?.value ?? "";
const restoreSurface = async () => {
  await ctx.request.put(`${API}/api/v1/settings/user`, {
    headers: auth,
    data: { key: SURFACE_KEY, value: originalSurface },
  });
};

const page = await ctx.newPage();
const consoleErrors = [];
page.on("console", (m) => {
  if (m.type() === "error") consoleErrors.push(m.text().slice(0, 300));
});
page.on("pageerror", (e) => consoleErrors.push(`[pageerror] ${e.message}`.slice(0, 300)));

/** The saved document this meter writes for phase B. Real keys only: the
 *  parser refuses a shape it does not recognise, which is the desired failure. */
const SAVED_SURFACE = JSON.stringify({
  "tickets.list": { view: "mine", columns: [], filters: [], sort: { field: "sla", dir: "asc" } },
});
/** Phase D: a saved document that keeps `columns`/`filters` stable but omits
 *  `sort`, so the page falls back to a fresh `{ field: "sla", dir: "asc" }`
 *  literal on every render. It isolates WHICH churning identity matters. */
const SAVED_NO_SORT = JSON.stringify({
  "tickets.list": { view: "mine", columns: [], filters: [] },
});

async function setSurface(value) {
  const res = await ctx.request.put(`${API}/api/v1/settings/user`, {
    headers: auth,
    data: { key: "appearance.data_surfaces", value },
  });
  if (!res.ok()) throw new Error(`could not write the surface document (${res.status()})`);
}

async function phase(label, path) {
  const errorsBefore = consoleErrors.length;
  let rsc = 0;
  let requests = 0;
  const count = (r) => {
    requests += 1;
    if (r.url().includes("_rsc=")) rsc += 1;
  };
  page.on("request", count);
  await page.goto(`${WEB}${path}`, { waitUntil: "domcontentloaded" });
  await page.getByTestId("top-bar").waitFor({ timeout: 30_000 });
  await page.waitForTimeout(2500); // rows, SLA fan-out, tour window
  const rscBeforeWindow = rsc;
  const measured = await page.evaluate(`(${MEASURE})(2000)`);
  // How the surface actually looks, so "pinned" is never confused with
  // "the page failed to render".
  const shape = await page.evaluate(() => ({
    rows: document.querySelectorAll("tbody tr").length,
    bodyText: (document.body.innerText || "").replace(/\s+/g, " ").trim().slice(0, 120),
    testids: Array.from(document.querySelectorAll("[data-testid]")).length,
  }));
  // The requests the page makes while nobody is touching it are the churn
  // metric: a settled surface issues none, a re-rendering one re-prefetches
  // its rows forever.
  const requestsInWindow = requests - rscBeforeWindow * 0 - 0;
  const rscInWindow = rsc - rscBeforeWindow;
  const pushed = await page.evaluate(() => {
    const r = window.next && window.next.router;
    if (!r) return "no window.next.router";
    try { r.push("/clients"); return "called"; } catch (e) { return `threw: ${e.message}`; }
  });
  await page.waitForTimeout(1500);
  page.off("request", count);
  const after = new URL(page.url()).pathname;
  const row = {
    label,
    path,
    mutations: measured.mutations,
    frames: measured.frames,
    rscPrefetchesInWindow: rscInWindow,
    pushStateDuringMeasure: measured.pushState,
    routerPush: pushed,
    urlAfterPush: after,
    navigated: after === "/clients",
    shape,
    newErrors: consoleErrors.slice(errorsBefore),
  };
  console.log(
    `${label.padEnd(34)} rows=${String(shape.rows).padStart(2)} rscIn2s=${String(row.rscPrefetchesInWindow).padStart(5)}` +
      ` frames=${String(row.frames).padStart(4)} nav=${row.navigated ? "YES -> " + after : "no"}`,
  );
  console.log(`    body: ${shape.bodyText}`);
  for (const e of row.newErrors.slice(0, 3)) console.log(`    ! ${e}`);
  void requestsInWindow;
  return row;
}

const results = [];

// Order is flipped on demand so a passing run cannot be an artefact of which
// phase warmed the browser first.
const REVERSE = process.env.A21_REVERSE === "1";

if (!REVERSE) {
  // A. the shape the demo and a fresh account have: nothing saved for this
  // surface, so `view()` fabricates a fresh object on every render.
  await setSurface("");
  await page.goto(`${WEB}/tickets`, { waitUntil: "domcontentloaded" });
  await page.getByTestId("top-bar").waitFor({ timeout: 30_000 });
  results.push(await phase("A /tickets no saved surface", "/tickets"));
}

// B. the same page with a saved document, so `view()` hands back the stored
// object and every `useMemo` keyed on it keeps its identity across renders.
await setSurface(SAVED_SURFACE);
results.push(await phase("B /tickets saved surface", "/tickets"));

// D. a saved document with stable columns/filters but NO sort, so only the
// page's own default-sort literal churns.
await setSurface(SAVED_NO_SORT);
results.push(await phase("D /tickets saved, no sort", "/tickets"));

// C. the control.
await setSurface(SAVED_SURFACE);
results.push(await phase("C /assets (control)", "/assets"));

if (REVERSE) {
  await setSurface("");
  await page.goto(`${WEB}/tickets`, { waitUntil: "domcontentloaded" });
  await page.getByTestId("top-bar").waitFor({ timeout: 30_000 });
  results.push(await phase("A /tickets no saved surface", "/tickets"));
}

const byLabel = (prefix) => results.find((r) => r.label.startsWith(prefix));
const A = byLabel("A ");
const B = byLabel("B ");
const C = byLabel("C ");
const verdict = {
  branch: "p47-tickets-route-nav-pin",
  web: WEB,
  reverse: REVERSE,
  savedSurface: SAVED_SURFACE,
  results,
  // Both directions are recorded, because this file is run before AND after the
  // fix and the same fields have to be readable in both:
  //   pinnedPhases       — which shapes could not navigate (empty after a fix)
  //   allPhasesNavigate  — the property the fix restores
  pinnedPhases: results.filter((r) => !r.navigated).map((r) => r.label),
  allPhasesNavigate: results.every((r) => r.navigated),
  // The pre-fix signature, kept so a future regression is recognisable: the
  // unsaved shape pinned, the saved shape and the control fine.
  preFixSignature:
    Boolean(A && B && C) && A.navigated === false && B.navigated === true && C.navigated === true,
};
writeFileSync(join(OUT, REVERSE ? "loop-meter-reversed.json" : "loop-meter.json"), JSON.stringify(verdict, null, 2));
console.log(`\npinned phases: ${verdict.pinnedPhases.join(", ") || "none"} | all navigate: ${verdict.allPhasesNavigate}`);
await restoreSurface();
await browser.close();
