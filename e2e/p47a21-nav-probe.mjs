// p47a21-nav-probe.mjs — WHERE is client-side navigation pinned on `/tickets`?
//
// Wave 47 lane A21 (`p47-tickets-route-nav-pin`). A15 measured that every
// client-side transition FROM `/tickets` is a no-op: chords, a rail-link
// click, a programmatic `router.push`, and even `router.refresh()`. This probe
// asks the next question: WHICH LAYER eats it. It instruments the browser
// BEFORE app code runs and then, for each of several navigation attempts,
// records:
//
//   history  — every pushState/replaceState the page makes (Next.js patches
//              both, so the patch's own calls are visible here);
//   rsc      — every fetch the router issues for a navigation, with its
//              response status (`?_rsc=`), i.e. whether the transition even
//              reached the network;
//   keys     — what the window's keydown path saw, and whether the event was
//              defaultPrevented by the time the LAST window listener ran
//              (i.e. did ShortcutLayer's dispatcher actually fire?);
//   clicks   — the same for a click on the rail's Clients link;
//   errors   — console errors, page errors, unhandled rejections;
//   env      — body/documentElement pointer + overflow state, and what
//              `document.elementFromPoint` returns at the rail link's centre
//              (an invisible interceptor would show up here).
//
// It then repeats the two signal attempts from `/assets`, which A15 measured
// as working, so the same probe shows the DIFFERENCE rather than an assertion.
//
// Run: node e2e/p47a21-nav-probe.mjs      (stack up: scripts/e2e-stack.sh up,
//                                          web built + served on E2E_WEB_URL)
// Writes design/qa-<branch>/nav-probe.json — no hard-coded lane path.

import { chromium } from "playwright";
import { execFileSync } from "node:child_process";
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
const git = (args) => execFileSync("git", ["-C", REPO, ...args], { encoding: "utf8" }).trim();
const BRANCH = git(["rev-parse", "--abbrev-ref", "HEAD"]);
const OUT = resolveEvidenceDir(
  {
    override: process.env.E2E_EVIDENCE_DIR,
    branch: BRANCH,
    sha: git(["rev-parse", "--short", "HEAD"]),
    runId: env.MARULA_RUN_ID ?? "",
  },
  resolve(REPO),
);
mkdirSync(OUT, { recursive: true });

/** Installed before ANY app code: records what the page does with history,
 *  fetch, keys, clicks and errors. */
const INSTRUMENT = `
window.__a21 = { history: [], historyAll: [], rsc: [], keys: [], clicks: [], errors: [], lastKey: null, lastClick: null };
(() => {
  const wrap = (name) => {
    const original = window.history[name].bind(window.history);
    window.history[name] = function (data, unused, url) {
      try {
        const entry = {
          call: name,
          url: url === undefined ? null : String(url),
          tagged: Boolean(data && (data.__NA || data._N)),
          carriesTree: Boolean(data && data.__PRIVATE_NEXTJS_INTERNALS_TREE),
          stack: new Error("trace").stack.split("\\n").slice(1, 6).join(" | "),
        };
        window.__a21.history.push(entry);
        // The never-cleared log: an untagged call at LOAD time is the
        // trigger for Next's ACTION_RESTORE-with-no-tree path.
        window.__a21.historyAll.push({ ...entry, at: Date.now() });
      } catch (e) { /* recording must never break the page */ }
      return original(data, unused, url);
    };
  };
  wrap("pushState");
  wrap("replaceState");

  const originalFetch = window.fetch.bind(window);
  window.fetch = function (input, init) {
    let url = "";
    try { url = typeof input === "string" ? input : input && input.url ? input.url : String(input); } catch (e) {}
    const state = { url, headers: {}, status: null, ok: null, error: null, at: Date.now() };
    try {
      const h = init && init.headers ? init.headers : (input && input.headers) || {};
      for (const k of Object.keys(h)) state.headers[k] = String(h[k]);
    } catch (e) {}
    if (url.includes("_rsc=")) window.__a21.rsc.push(state);
    return originalFetch(input, init).then(
      (res) => { state.status = res.status; state.ok = res.ok; state.type = res.type; return res; },
      (err) => { state.error = String(err && err.message ? err.message : err); throw err; },
    );
  };

  // A bubble listener at window is registered LAST (after load), so it sees
  // the event after the app's own window listeners have run: defaultPrevented
  // true means ShortcutLayer's dispatcher matched and ran.
  window.__a21.installKeyTail = () => {
    window.addEventListener("keydown", (e) => {
      window.__a21.keys.push({
        key: e.key, code: e.code, alt: e.altKey, meta: e.metaKey, ctrl: e.ctrlKey, shift: e.shiftKey,
        defaultPrevented: e.defaultPrevented, cancelBubble: e.cancelBubble,
      });
    });
  };
  window.__a21.installClickTail = () => {
    document.addEventListener("click", (e) => {
      const t = e.target;
      const desc = t && t.closest ? (t.closest("a") ? "a[" + (t.closest("a").getAttribute("href") || "") + "]" : t.tagName) : String(t);
      window.__a21.lastClick = { target: desc, defaultPrevented: e.defaultPrevented, trusted: e.isTrusted };
    }, true);
    document.addEventListener("click", (e) => {
      const t = e.target;
      const desc = t && t.closest ? (t.closest("a") ? "a[" + (t.closest("a").getAttribute("href") || "") + "]" : t.tagName) : String(t);
      window.__a21.clicks.push({ target: desc, defaultPrevented: e.defaultPrevented, trusted: e.isTrusted });
    });
  };
  window.addEventListener("error", (e) => {
    window.__a21.errors.push({ kind: "error", message: String(e.message), source: e.filename, line: e.lineno });
  });
  window.addEventListener("unhandledrejection", (e) => {
    window.__a21.errors.push({ kind: "unhandledrejection", message: String(e.reason && e.reason.message ? e.reason.message : e.reason) });
  });
})();
`;

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
await ctx.addInitScript(INSTRUMENT);
const login = await ctx.request.post(`${API}/api/v1/auth/login`, {
  data: { email: "owner@demo.local", password: "demo-password-123" },
});
// A settings WRITE needs the bearer token. Without it this PUT answered 401 and
// nothing checked, so the tour-disable never applied and a tour overlay sat on
// the page for every run — the predecessor's probe shipped with exactly that.
const token = (await login.json()).access_token;
const toursOff = await ctx.request.put(`${API}/api/v1/settings/user`, {
  headers: { authorization: `Bearer ${token}` },
  data: { key: "tours.disabled", value: true },
});
if (!toursOff.ok()) throw new Error(`could not disable tours (${toursOff.status()})`);
const page = await ctx.newPage();

const consoleLines = [];
const netLog = [];
page.on("console", (m) => {
  if (m.type() === "error" || m.type() === "warning") consoleLines.push(`[${m.type()}] ${m.text()}`);
});
page.on("pageerror", (e) => consoleLines.push(`[pageerror] ${e.message}`));
// Playwright's own view of the network: independent of the page's fetch patch,
// so "no request was issued" is measured at the browser, not inferred.
page.on("request", (r) => {
  const u = r.url();
  if (u.includes("_rsc=") || u.startsWith(WEB)) {
    netLog.push({ kind: "request", url: u.replace(WEB, ""), headers: r.headers(), at: Date.now() });
  }
});
page.on("requestfailed", (r) => {
  const u = r.url();
  if (u.startsWith(WEB)) netLog.push({ kind: "failed", url: u.replace(WEB, ""), failure: r.failure()?.errorText, at: Date.now() });
});
page.on("response", (r) => {
  const u = r.url();
  if (u.includes("_rsc=")) netLog.push({ kind: "response", url: u.replace(WEB, ""), status: r.status(), at: Date.now() });
});
page.on("framenavigated", (f) => {
  if (f === page.mainFrame()) netLog.push({ kind: "framenavigated", url: f.url().replace(WEB, ""), at: Date.now() });
});

/** Open a surface, let it settle, and put the probe's listeners last. */
async function open(path) {
  await page.goto(`${WEB}${path}`, { waitUntil: "domcontentloaded" });
  await page.getByTestId("top-bar").waitFor({ timeout: 30_000 });
  await page.waitForTimeout(2500); // rows + SLA fan-out + a possible tour window
  await page.evaluate(() => {
    window.__a21.history.length = 0;
    window.__a21.rsc.length = 0;
    window.__a21.keys.length = 0;
    window.__a21.clicks.length = 0;
    window.__a21.errors.length = 0;
    window.__a21.installKeyTail();
    window.__a21.installClickTail();
  });
}

/** The router's own view of the current tree, plus the encode step the
 *  reducer performs before its fetch: `encodeURIComponent(JSON.stringify(
 *  stripClientOnlyData(tree)))`. If the tree is missing or the strip/encode
 *  throws, THIS is the line that would throw it, and the probe shows it. */
const TREE_CHECK = `
(() => {
  const state = window.history.state;
  const tree = state && state.__PRIVATE_NEXTJS_INTERNALS_TREE;
  let encoded = null, encodeError = null, stripped = null, stripError = null;
  try { encoded = encodeURIComponent(JSON.stringify(tree)).length; } catch (e) { encodeError = String(e && e.message); }
  try {
    const strip = (node) => {
      const [segment, parallelRoutes, _url, marker, isRoot, hasLoading] = node;
      const out = { segment, parallelRoutes, marker, isRoot, hasLoading };
      const cleaned = {};
      for (const [key, child] of Object.entries(parallelRoutes)) cleaned[key] = strip(child);
      out.cleanedCount = Object.keys(cleaned).length;
      return out;
    };
    stripped = strip(tree);
  } catch (e) { stripError = String(e && e.message); }
  return {
    hasState: Boolean(state),
    stateKeys: state ? Object.keys(state) : null,
    tagged: Boolean(state && (state.__NA || state._N)),
    hasTree: tree !== undefined && tree !== null,
    treeEncodeOk: encoded !== null,
    treeEncodedLength: encoded,
    treeEncodeError: encodeError,
    treeStripError: stripError,
  };
})()
`;

let netMark = 0;

async function snapshot(label, { path, after }) {
  const data = await page.evaluate(() => ({
    url: window.location.href,
    history: window.__a21.history,
    rsc: window.__a21.rsc,
    keys: window.__a21.keys,
    clicks: window.__a21.clicks,
    errors: window.__a21.errors,
    hasNextRouter: Boolean(window.next && window.next.router),
    bodyPointerEvents: document.body.style.pointerEvents,
    bodyOverflow: document.body.style.overflow,
    htmlOverflow: document.documentElement.style.overflow,
    histState: (() => {
      try { return Object.keys(window.history.state || {}); } catch (e) { return []; }
    })(),
  }));
  data.tree = await page.evaluate(TREE_CHECK);
  const net = netLog.slice(netMark);
  netMark = netLog.length;
  console.log(`--- ${label} (${path}) ---`);
  console.log(`  url            : ${data.url}  ${after ? "(after)" : ""}`);
  console.log(`  next.router    : ${data.hasNextRouter}`);
  console.log(`  body pointerEv : ${JSON.stringify(data.bodyPointerEvents)}  overflow=${JSON.stringify(data.bodyOverflow)}`);
  console.log(`  history calls  : ${data.history.length}`);
  for (const h of data.history) console.log(`      ${h.call} ${h.url} tagged=${h.tagged} carriesTree=${h.carriesTree}`);
  console.log(`  router state   : ${JSON.stringify(data.tree)}`);
  console.log(`  net (attempt)  : ${net.length}`);
  for (const n of net) console.log(`      ${n.kind} ${n.url} ${n.status ?? n.failure ?? ""}`);
  console.log(`  rsc fetches    : ${data.rsc.length}`);
  for (const r of data.rsc) console.log(`      ${r.url.replace(WEB, "")} status=${r.status} err=${r.error}`);
  console.log(`  keys seen      : ${data.keys.length}`);
  for (const k of data.keys) console.log(`      ${k.key} (${k.code}) alt=${k.alt} meta=${k.meta} ctrl=${k.ctrl} defaultPrevented=${k.defaultPrevented}`);
  console.log(`  clicks seen    : ${data.clicks.length}`);
  for (const c of data.clicks) console.log(`      ${c.target} defaultPrevented=${c.defaultPrevented}`);
  console.log(`  errors         : ${data.errors.length}`);
  for (const e of data.errors) console.log(`      ${e.kind}: ${e.message}`);
  return data;
}

const run = { branch: BRANCH, sha: git(["rev-parse", "--short", "HEAD"]), web: WEB, attempts: [], console: consoleLines };

async function attempt(label, path, act, { late = false } = {}) {
  await open(path);
  const before = await snapshot(`${label} / before`, { path, after: false });
  await act();
  await page.waitForTimeout(3000);
  const after = await snapshot(`${label} / after`, { path, after: true });
  let lateUrl = null;
  if (late) {
    // A slow starved request would land eventually; a dead router never does.
    await page.waitForTimeout(25000);
    lateUrl = new URL(page.url()).pathname;
    console.log(`  url @+28s      : ${lateUrl}`);
  }
  run.attempts.push({ label, path, before, after, lateUrl, navigated: before.url !== after.url || (lateUrl !== null && lateUrl !== path) });
  console.log(`  => navigated: ${before.url !== after.url}  (late: ${lateUrl ?? "n/a"})`);
  return before.url !== after.url;
}

/** What sits on top at the rail's CLIENTS link — an invisible interceptor
 *  would be named here. */
async function probeRail() {
  const info = await page.evaluate(() => {
    const link = document.querySelector('nav[aria-label="Main"] a[href="/clients"]');
    if (!link) return { found: false };
    const r = link.getBoundingClientRect();
    const x = r.left + r.width / 2;
    const y = r.top + r.height / 2;
    const top = document.elementFromPoint(x, y);
    const chain = [];
    let el = top;
    while (el && chain.length < 6) {
      const cs = getComputedStyle(el);
      chain.push(`${el.tagName.toLowerCase()}${el.getAttribute("data-testid") ? "[" + el.getAttribute("data-testid") + "]" : ""} pe=${cs.pointerEvents} z=${cs.zIndex} pos=${cs.position}`);
      el = el.parentElement;
    }
    return {
      found: true,
      railRect: { x: Math.round(x), y: Math.round(y) },
      linkIsTop: top === link || link.contains(top),
      topChain: chain,
      dialogs: Array.from(document.querySelectorAll('[role="dialog"],[aria-modal="true"],[data-testid$="overlay"]')).map(
        (d) => `${d.tagName.toLowerCase()}[${d.getAttribute("role") || ""}] testid=${d.getAttribute("data-testid") || ""} pe=${getComputedStyle(d).pointerEvents} hidden=${d.hasAttribute("hidden")}`,
      ),
    };
  });
  console.log(`  rail hit-test  : ${JSON.stringify(info, null, 2).split("\n").join("\n      ")}`);
  return info;
}

// --- 1. the pinned surface ----------------------------------------------
await open("/tickets");
run.rail = await probeRail();

await attempt("tickets: Alt+2 chord", "/tickets", async () => {
  await page.keyboard.press("Alt+2");
}, { late: true });

await attempt("tickets: rail Clients click", "/tickets", async () => {
  await page.locator('nav[aria-label="Main"] a[href="/clients"]').click({ force: true });
}, { late: true });

await attempt("tickets: programmatic router.push", "/tickets", async () => {
  const result = await page.evaluate(() => {
    const r = window.next && window.next.router;
    if (!r) return "no window.next.router";
    try { r.push("/clients"); return "pushed"; } catch (e) { return "threw: " + String(e && e.message); }
  });
  console.log(`  router.push    : ${result}`);
}, { late: true });

await attempt("tickets: external history.pushState", "/tickets", async () => {
  await page.evaluate(() => {
    window.history.pushState({ probe: "external" }, "", "/clients");
  });
});

await attempt("tickets: router.refresh", "/tickets", async () => {
  const result = await page.evaluate(() => {
    const r = window.next && window.next.router;
    if (!r || typeof r.refresh !== "function") return "no refresh";
    try { r.refresh(); return "refreshed"; } catch (e) { return "threw: " + String(e && e.message); }
  });
  console.log(`  router.refresh : ${result}`);
});

// --- 2. the control surface (A15 measured these working) ----------------
await attempt("assets: Alt+2 chord", "/assets", async () => {
  await page.keyboard.press("Alt+2");
});

await attempt("assets: rail Clients click", "/assets", async () => {
  await page.locator('nav[aria-label="Main"] a[href="/clients"]').click({ force: true });
});

// --- 3. the other direction: INTO /tickets, then OUT --------------------
{
  const inOk = await attempt("assets: Alt+1 chord into /tickets", "/assets", async () => {
    await page.keyboard.press("Alt+1");
  });
  console.log(`  into /tickets  : ${inOk ? "LANDED on /tickets" : "did NOT land"}`);
  if (inOk) {
    await attempt("tickets (arrived by chord): Alt+2 out", "/tickets", async () => {
      await page.keyboard.press("Alt+2");
    });
  }
}

// `historyAll` lives in the PAGE (it is never cleared, so it holds every
// pushState/replaceState since load, tagged or not). The predecessor's script
// referenced it as a Node variable at this point, which threw a ReferenceError
// and meant nav-probe.json was never written. Read it from the page instead.
const historyAll = await page.evaluate(() => window.__a21.historyAll ?? []);
writeFileSync(join(OUT, "nav-probe.json"), JSON.stringify({ ...run, historyAll, net: netLog }, null, 2));
console.log(`\nhistory calls since load (never cleared): ${historyAll.length}`);
for (const h of historyAll.slice(0, 15)) console.log(`  ${h.call} ${h.url} tagged=${h.tagged} carriesTree=${h.carriesTree} at=${h.at}`);
console.log(`\nconsole (error/warning): ${consoleLines.length}`);
for (const l of consoleLines.slice(0, 20)) console.log("  " + l);
console.log(`\nwrote ${join(OUT, "nav-probe.json")}`);
await browser.close();
