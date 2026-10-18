// p47-conformance.mjs — Wave 47 lane A13, `p47-vision-captures`.
//
// The rule-level instrument. `check-pack-geometry.py` compares declared numbers
// and `p41-pack-diff.py` scores ink; neither can test a *rule*. The pack states
// its rules in prose per area (`DESIGN-NOTES-*.md`) and ships the HTML each
// surface was rendered from (`source/*.html`). This renders BOTH sides and diffs
// the same structural signature, so a difference cannot hide behind a class name.
//
// The anchor is the mockup's own state selector. Every one of the thirteen
// mockups carries `<select id="jump" aria-label="Jump to state">`, and the
// selector lives in the preview harness (`div.viewer`) which sits outside the
// surface. The surface is the harness's next sibling's `<main>` (the screen for
// the two mobile files). That is a declared relationship, not a geometric guess,
// so it locates the surface and excludes the chrome on every mockup at once.
//
// For each surface and each declared state this extracts, from the pack's
// mockup and from our route, the same signature:
//
//   TOOLBAR      every control in the band between the page title and the table,
//                in reading order, plus the right-hand group split out (the
//                group §2.1 fixes as Columns, Filter, Export).
//   COLUMNS      the header row's cells, in order.
//   ROW          the first body row's cell count and text shape.
//   FOOTER       the visible text below the table.
//   STATE        density attribute, open panels, empty and error shapes.
//   REGIONS      the box each region occupies, normalised to the canvas.
//
// Usage:
//   node e2e/p47-conformance.mjs                       # pack + app, mapped states
//   node e2e/p47-conformance.mjs --pack-only           # no stack needed
//   node e2e/p47-conformance.mjs --only tickets-list
//   node e2e/p47-conformance.mjs --pack-only --themes  # pack side, both themes
//
// The pack side needs no stack (`--pack-only` is the yardstick). The app side
// reads `e2e/.e2e-stack.env` and expects the web bundle up.
import { chromium } from "playwright";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const LANE = "design/qa-p47-vision-captures";
const OUT = resolve(LANE, "conformance");
mkdirSync(OUT, { recursive: true });

const PACK =
  process.env.MARULA_PACK_ROOT ??
  "/Users/iordn/Documents/GitHub/Marula/brand/packs/core-surface-pack";
const CANVAS = { width: 1600, height: 994 };
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";

const arg = (name, dflt) => {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? dflt : process.argv[i + 1];
};
const ONLY = arg("only", null);
const PACK_ONLY = process.argv.includes("--pack-only");
const WITH_THEMES = process.argv.includes("--themes");

/* The thirteen mockups the pack ships, each with the route that renders it in
 * our app (null where we have no equivalent — the two mobile files are native
 * screens, and the app's native half is not a browser surface). `surface` is the
 * app-side preference key when the route reads `useDataSurfacePrefs`. */
const SURFACES = [
  { id: "tickets-list", source: "tickets-list.html", route: "/tickets", surface: "tickets.list", themes: ["neutral", "dark"] },
  { id: "clients", source: "clients.html", route: "/clients", surface: "clients.list", themes: ["neutral", "dark"] },
  { id: "approvals", source: "approvals.html", route: "/approvals", themes: ["neutral", "dark"] },
  { id: "today", source: "today.html", route: "/today", themes: ["neutral", "dark"] },
  { id: "dispatch", source: "dispatch.html", route: "/dispatch", themes: ["neutral", "dark"] },
  { id: "settings", source: "settings.html", route: "/settings", themes: ["neutral", "dark"] },
  { id: "quoting", source: "quoting.html", route: "/sales/quotes", themes: ["neutral", "dark"] },
  { id: "knowledge", source: "knowledge-vault.html", route: "/documents", themes: ["neutral", "dark"] },
  { id: "workflow", source: "workflow-builder.html", route: "/automations", themes: ["neutral", "dark"] },
  { id: "portal", source: "portal.html", route: "/portal", themes: ["sand", "dark"] },
  { id: "auth", source: "auth.html", route: "/login", themes: ["neutral", "dark"] },
  { id: "mobile-core", source: "mobile-core.html", route: null, themes: ["light", "dark"] },
  { id: "mobile-scan", source: "mobile-scan.html", route: null, themes: ["light", "dark"] },
].filter((s) => ONLY === null || s.id === ONLY);

async function session() {
  const envPath = resolve("e2e/.e2e-stack.env");
  if (!existsSync(envPath))
    throw new Error("e2e/.e2e-stack.env missing — boot the stack or pass --pack-only");
  const env = Object.fromEntries(
    readFileSync(envPath, "utf8")
      .split("\n")
      .filter((l) => l.includes("="))
      .map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]),
  );
  const api = (env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
  const res = await fetch(`${api}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: EMAIL, password: PASSWORD }),
  });
  if (!res.ok) throw new Error(`login failed (${res.status})`);
  return { tokens: await res.json(), web: env.E2E_WEB_URL ?? "http://localhost:3000", api };
}

/* ---------------------------------------------------------------------------
 * The signature. One function, run inside both pages, so the two sides are
 * measured by identical code — the whole reason this works where a screenshot
 * diff does not.
 * ---------------------------------------------------------------------- */
const SIGNATURE = () => {
  const vis = (el) => {
    const st = getComputedStyle(el);
    if (st.display === "none" || st.visibility === "hidden" || parseFloat(st.opacity) < 0.05) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };
  const txt = (el) => (el.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 60);
  const round = (n) => Math.round(n);

  /* --- the anchor ------------------------------------------------------- */
  // The pack asserts it by selector: `#jump` is the state switcher, it lives in
  // the harness (`div.viewer`), and the surface is the harness's next sibling's
  // `<main>` / the phone screen. Our app has no `#jump`, so it falls through to
  // the app shell's own `<main>`. Either way the chrome is excluded by being
  // outside the declared container, not by a height guess.
  const jump = document.querySelector("#jump");
  const viewer = jump && jump.closest(".viewer");
  const frame = viewer ? viewer.nextElementSibling || viewer.parentElement : null;
  // Inside the frame, prefer the declared content landmark. A few states have
  // none (portal's login is full-bleed), so the frame itself is the surface —
  // never `body`, which would readmit the harness the frame exists to exclude.
  const surface =
    (frame && (frame.querySelector("main") || frame.querySelector(".scr") || frame)) ||
    document.querySelector("#m-main-content") ||
    document.querySelector("main.page") ||
    document.querySelector("main") ||
    document.body;
  const surfaceBox = surface.getBoundingClientRect();

  /* --- the list --------------------------------------------------------- */
  // The pack draws its lists as div rows, not `<table>`, and names them
  // differently per surface: `#tbl`, `.tbl`, `.tbody`, or a header row `.hd`
  // over same-class rows. The detector takes the declared container when it
  // exists and otherwise the surface's densest cluster of same-class siblings.
  const declaredList =
    surface.querySelector("#tbl") ||
    surface.querySelector(".tbl") ||
    surface.querySelector("table") ||
    surface.querySelector(".tbody") ||
    null;
  const clusterList = (() => {
    let best = null;
    let bestN = 0;
    for (const el of surface.querySelectorAll("div, ul, section")) {
      const kids = [...el.children];
      if (kids.length < 3) continue;
      const counts = {};
      for (const k of kids) {
        const cls = typeof k.className === "string" ? k.className.split(/\s+/)[0] : "";
        if (cls) counts[cls] = (counts[cls] ?? 0) + 1;
      }
      const n = Math.max(0, ...Object.values(counts));
      if (n > bestN) {
        bestN = n;
        best = el;
      }
    }
    return bestN >= 4 ? best : null;
  })();
  const tbl = declaredList || clusterList;
  const tableBox = tbl ? tbl.getBoundingClientRect() : null;
  const listTop = tableBox ? tableBox.top : surfaceBox.top + 200;

  /* --- the toolbar ------------------------------------------------------ */
  // Every control inside the surface that sits above the list's top edge, then
  // the lowest band of those (the toolbar row immediately above the list — the
  // page header's own buttons sit far higher and drop out). The pack's `#tb` is
  // used when present; otherwise the band is read structurally.
  const controls = [];
  const seen = new Set();
  for (const el of surface.querySelectorAll(
    "button, a[href], [role=button], [role=radio], [role=checkbox], select, input",
  )) {
    if (!vis(el) || seen.has(el)) continue;
    const r = el.getBoundingClientRect();
    if (r.bottom > listTop + 2) continue;
    if (r.width < 8 || r.height < 8) continue;
    const label =
      txt(el) || el.getAttribute("aria-label") || el.getAttribute("placeholder") || el.getAttribute("title") || "";
    if (label === "") continue;
    controls.push({ label, x: round(r.left), y: round(r.top), w: round(r.width), h: round(r.height) });
    seen.add(el);
  }
  let toolbar = [];
  if (controls.length > 0) {
    const lowest = Math.max(...controls.map((c) => c.y));
    toolbar = controls.filter((c) => lowest - c.y <= 64);
    toolbar.sort((a, b) => (Math.abs(a.y - b.y) > 12 ? a.y - b.y : a.x - b.x));
  }
  const midX = surfaceBox.left + surfaceBox.width / 2;
  // The table-action controls, with the side of the strip each sits on. §2.1
  // fixes their wording and their side (Add filter with the chips; Columns,
  // Filter, Export right-aligned), and chip width otherwise swamps a plain
  // mid-line split.
  const ACTION = /^\+?\s*(Add filter|Columns|Filter|Export)/;
  const toolbarActions = toolbar
    .filter((c) => ACTION.test(c.label))
    .map((c) => ({ label: c.label, side: c.x + c.w / 2 >= midX ? "right" : "left" }));
  const toolbarRight = toolbar
    .filter((c) => c.x + c.w / 2 >= midX)
    .map((c) => c.label);

  /* --- columns and first row ------------------------------------------- */
  const columns = [];
  const row = [];
  if (tbl) {
    // Prefer a real table head; the div lists carry a `.hd` row instead.
    const headRow =
      tbl.querySelector("thead tr") ||
      [...tbl.children].find(
        (c) => typeof c.className === "string" && /(^|\s)(hd|head|th)(\s|$)/.test(c.className),
      ) ||
      null;
    if (headRow) {
      const cells = headRow.querySelectorAll("th").length
        ? [...headRow.querySelectorAll("th")]
        : [...headRow.children];
      for (const th of cells) {
        if (!vis(th)) continue;
        const r = th.getBoundingClientRect();
        columns.push({ label: txt(th), x: round(r.left), w: round(r.width) });
      }
      columns.sort((a, b) => a.x - b.x);
    }
    const bodyRow =
      tbl.querySelector("tbody tr") ||
      [...tbl.children].find((c) => c !== headRow && vis(c) && c.getBoundingClientRect().height > 8) ||
      null;
    if (bodyRow) {
      const cells = bodyRow.querySelectorAll("td").length
        ? [...bodyRow.querySelectorAll("td")]
        : [...bodyRow.children];
      for (const td of cells) {
        if (!vis(td)) continue;
        const r = td.getBoundingClientRect();
        row.push({ text: txt(td), x: round(r.left), w: round(r.width) });
      }
      row.sort((a, b) => a.x - b.x);
    }
  }

  /* --- footer ----------------------------------------------------------- */
  const footerEl =
    surface.querySelector("#ft") ||
    surface.querySelector(".ft2") ||
    surface.querySelector(".m-surface-footer") ||
    surface.querySelector("[data-testid=surface-footer]") ||
    null;
  const footerText = footerEl && vis(footerEl) ? txt(footerEl) : "";
  const footerBox = footerEl && vis(footerEl) ? footerEl.getBoundingClientRect() : null;

  /* --- title ------------------------------------------------------------ */
  const h = surface.querySelector("h1") || document.querySelector("h1");
  let title = null;
  if (h && vis(h)) {
    const st = getComputedStyle(h);
    const r = h.getBoundingClientRect();
    title = { text: txt(h), size: parseFloat(st.fontSize), weight: st.fontWeight, x: round(r.left), y: round(r.top) };
  }

  /* --- interaction state ------------------------------------------------ */
  const openPanels = [...surface.querySelectorAll('[role=dialog], [role=menu], [role=listbox]')].filter(vis).length +
    [...document.querySelectorAll("#modal > *, #overlays > *, #sheets > *")].filter(vis).length;
  const density =
    document.documentElement.getAttribute("data-density") ||
    document.body?.getAttribute("data-density") ||
    surface.getAttribute("data-density") ||
    null;
  const emptyNode =
    surface.querySelector('[data-testid*=empty], .m-empty, [class*=empty]') ||
    null;
  const emptyText = emptyNode && vis(emptyNode) ? txt(emptyNode).slice(0, 100) : null;
  const bodyText = surface.innerText.replace(/\s+/g, " ").trim();
  const empty = emptyText || (/no .{0,30}match|nothing is |no tickets yet|nothing waiting|everything open has/i.test(bodyText) ? bodyText.slice(0, 100) : null);

  return {
    canvas: { width: window.innerWidth, height: window.innerHeight },
    surfaceBox: { x: round(surfaceBox.left), y: round(surfaceBox.top), w: round(surfaceBox.width), h: round(surfaceBox.height) },
    title,
    toolbar: toolbar.map((c) => c.label),
    toolbarActions,
    toolbarRight,
    columns: columns.map((c) => c.label),
    firstRow: row.map((c) => c.text),
    rowCells: row.length,
    footer: footerText,
    footerBox: footerBox ? { x: round(footerBox.left), y: round(footerBox.top), w: round(footerBox.width), h: round(footerBox.height) } : null,
    tableBox: tableBox ? { x: round(tableBox.left), y: round(tableBox.top), w: round(tableBox.width), h: round(tableBox.height) } : null,
    state: { density, openPanels, empty },
  };
};

/* ---------------------------------------------------------------------------
 * Pack side: open each mockup, walk its declared themes and states.
 * ---------------------------------------------------------------------- */
async function readPack(browser, surface) {
  const source = resolve(PACK, "source", surface.source);
  if (!existsSync(source)) return { error: `no pack source at ${source}` };
  const page = await browser.newPage({ viewport: CANVAS });
  await page.goto(`file://${source}`, { waitUntil: "load", timeout: 60_000 });
  await page.waitForTimeout(900);

  const declared = await page.evaluate(() => ({
    states: [...document.querySelectorAll("#jump option")].map((o) => ({
      id: o.value,
      title: o.textContent.replace(/^\s*\d+\s*·\s*/, "").replace(/^[QKC]\d*\s*·?\s*/, "").trim(),
    })),
    themes: [...document.querySelectorAll("[data-theme-set]")].map((b) => b.dataset.themeSet),
  }));

  const themes = WITH_THEMES ? declared.themes : [declared.themes[0]].filter(Boolean);
  const byState = [];
  for (const st of declared.states) {
    const rec = { id: st.id, title: st.title, themes: {} };
    for (const theme of themes) {
      if (theme) {
        await page.evaluate((t) => {
          const b = document.querySelector(`[data-theme-set="${t}"]`);
          if (b) b.click();
        }, theme);
      }
      await page.evaluate((v) => {
        const j = document.querySelector("#jump");
        if (!j) return;
        j.value = v;
        j.dispatchEvent(new Event("change", { bubbles: true }));
      }, st.id);
      await page.waitForTimeout(220);
      rec.themes[theme ?? "default"] = await page.evaluate(SIGNATURE);
    }
    byState.push(rec);
  }
  await page.close();
  return { declared, page: byState };
}

/* ---------------------------------------------------------------------------
 * App side: per-state drivers.
 *
 * A driver says how to reach the equivalent of one pack state in our app, or
 * that there is no equivalent. It is deliberately data, not a maze of `if`s:
 * the audit's finding is the list of states with no counterpart, so a missing
 * entry is a first-class outcome (`no-equivalent` / `not-driven`) rather than a
 * silent skip.
 *
 *   status   "drive" (default) | "no-equivalent" | "not-driven"
 *   reason   why, for the two non-drive statuses
 *   prefs    { [surfaceKey]: { view?, filters?, sort? } } written to the user's
 *            settings before navigation (list state lives behind the API)
 *   density  "comfortable" | "compact" | "dense"
 *   actions  an ordered mini-language executed after load:
 *              "clickText:<text>"  "clickAria:<label>"  "checkRow:0"
 *   route    an override for this state
 *   abort    a URL glob whose requests are aborted (the error state)
 * ---------------------------------------------------------------------- */
const DRIVERS = {
  "tickets-list": {
    "01-default": { status: "drive" },
    "02-filtered": {
      prefs: { "tickets.list": { view: "all", filters: [{ field: "title", op: "contains", value: "e" }] } },
      actions: ["clickText:All tickets"],
    },
    "03-filter-builder": {
      prefs: { "tickets.list": { view: "all", filters: [{ field: "title", op: "contains", value: "e" }] } },
      actions: ["clickText:All tickets", "clickText:Filter"],
    },
    "04-column-chooser": { actions: ["clickText:Columns"] },
    "05-bulk-selected": { actions: ["checkRow:0"] },
    "06-bulk-move-conflict": { status: "not-driven", reason: "needs a multi-board selection and the move dialog; the modal is a workbench flow, not reachable from the list toolbar alone" },
    "07-export-large": { actions: ["clickText:Export"] },
    "08-density-compact": { density: "compact" },
    "09-density-dense": { density: "dense" },
    "10-empty-no-results": {
      prefs: { "tickets.list": { view: "all", filters: [{ field: "title", op: "contains", value: "zzzznomatch" }] } },
    },
    "11-empty-quiet-queue": { status: "not-driven", reason: "the quiet queue needs an account with nothing assigned; forcing it means mutating seeded data" },
    "12-load-error": { abort: "**/api/v1/tickets*" },
  },
  clients: {
    "01-clients-list": { status: "drive" },
    "02-list-possible-duplicate": { status: "not-driven", reason: "the duplicate strip renders only with a dedupe AI grant and a real duplicate pair in seed" },
    "03-client-overview": { status: "not-driven", reason: "the client record is a different route (/clients/[id]); a landing comparison has no state to drive" },
    "04-contacts": { status: "no-equivalent", reason: "the app's client record is a route, not a pack state; contacts are not a list state on /clients" },
    "05-sites-and-devices": { status: "no-equivalent", reason: "same — a record tab, not a list state" },
    "06-contracts-and-billing": { status: "no-equivalent", reason: "same — a record tab, not a list state" },
    "07-empty-client": { status: "no-equivalent", reason: "a half-finished client is a record shape, not a list state on /clients" },
  },
  approvals: {
    "01-queue-no-grant": { status: "drive" },
    "*": {
      status: "not-driven",
      reason: "the approvals queue's later states are inline verdicts, a bulk pass and invoice detail — interactions the landing capture does not reach",
    },
  },
  today: {
    "01-today-default": { status: "drive" },
    "*": {
      status: "not-driven",
      reason: "today's other states are quick-log sheets and a logged-day result; reaching them needs a timer/day flow the audit does not run",
    },
  },
  dispatch: {
    "01-board-rest": { status: "drive" },
    "*": {
      status: "no-equivalent",
      reason: "the dispatch mockup's remaining states are drag, plan, travel and overlap flows on a board surface; the app's /dispatch is a board too, but the audit does not drive drag/plan states",
    },
  },
  settings: {
    "01-settings-shell": { status: "drive" },
    "*": {
      status: "no-equivalent",
      reason: "each later settings state is a different sub-page (/settings/statuses, /settings/ai-grants, …), not a state of the settings shell",
    },
  },
  quoting: {
    Q01: { status: "drive" },
    "*": {
      status: "no-equivalent",
      reason: "Q02–C06 are the AI 'describe it' flows and the quote/contract editors; /sales/quotes renders the list only",
    },
  },
  knowledge: {
    K01: { status: "drive" },
    "*": {
      status: "no-equivalent",
      reason: "K02–K16 are record, vault, secret-reveal and audit states under /kb and /passwords, not states of the knowledge home",
    },
  },
  workflow: {
    W01: { status: "drive" },
    "*": {
      status: "not-driven",
      reason: "W02–W09 are canvas selections, run states and a describe-a-change proposal; the run states need a live run",
    },
  },
  portal: {
    "01-login": { status: "drive" },
    "*": {
      status: "not-driven",
      reason: "portal states 02–12 sit behind a portal staff seat (portal@acme.demo); the audit drives the staff session only",
    },
  },
  auth: {
    "01-login": { status: "drive" },
    "*": {
      status: "not-driven",
      reason: "the later auth states (mfa, magic link, invite, wizard) are separate flows reached by tokens/links the audit does not mint",
    },
  },
  "mobile-core": { "*": { status: "no-equivalent", reason: "the app's mobile half is a native screen (native/app), not a browser route; the web bundle cannot render it" } },
  "mobile-scan": { "*": { status: "no-equivalent", reason: "same — a native scanner screen, no web route" } },
};

function driverFor(surfaceId, stateId) {
  const table = DRIVERS[surfaceId];
  if (!table) return { status: "not-driven", reason: "no driver table for this surface" };
  return table[stateId] ?? table["*"] ?? { status: "not-driven", reason: "no driver authored for this state" };
}

async function putSetting(ctx, api, tokens, key, value) {
  const res = await ctx.request.put(`${api}/api/v1/settings/user`, {
    data: { key, value },
    headers: { authorization: `Bearer ${tokens.access_token}` },
  });
  return res.ok();
}

async function runActions(page, actions) {
  for (const action of actions ?? []) {
    const [verb, rest] = action.split(":");
    try {
      if (verb === "clickText") {
        await page
          .locator("button, [role=radio], [role=tab], [role=button], a")
          .filter({ hasText: rest })
          .first()
          .click({ timeout: 4000 });
      } else if (verb === "clickAria") {
        await page.locator(`[aria-label="${rest}"]`).first().click({ timeout: 4000 });
      } else if (verb === "checkRow") {
        await page.locator("tbody tr").first().locator('input[type="checkbox"], [role="checkbox"], button').first().click({ timeout: 4000 });
      }
    } catch {
      // An action that does not land is itself a finding: the driver recorded
      // the attempt and the state reads as reached-but-empty. Do not throw.
      return { action, ok: false };
    }
    await page.waitForTimeout(250);
  }
  return { ok: true };
}

async function readApp(browser, ctx, session_, surface, stateId) {
  const driver = driverFor(surface.id, stateId);
  if (driver.status === "no-equivalent" || driver.status === "not-driven") {
    return { status: driver.status, reason: driver.reason };
  }
  if (surface.route === null) {
    return { status: "no-equivalent", reason: "no web route for this surface" };
  }
  const { tokens, api, web } = session_;
  // List state lives behind the user's settings, not the DOM: reset the whole
  // document (so 01-default is genuinely default and one driver cannot bleed
  // into the next) and write this driver's entry under the surface's own key.
  const doc = {};
  for (const [surfaceKey, entry] of Object.entries(driver.prefs ?? {})) {
    doc[surfaceKey] = { ...entry, columns: entry.columns ?? [], filters: entry.filters ?? [] };
  }
  await putSetting(ctx, api, tokens, "appearance.data_surfaces", JSON.stringify(doc));
  await putSetting(ctx, api, tokens, "appearance.density", driver.density ?? "comfortable");

  const page = await ctx.newPage();
  if (driver.abort) await page.route(driver.abort, (r) => r.abort());
  await page
    .goto(`${web}${driver.route ?? surface.route}`, { waitUntil: "domcontentloaded", timeout: 60_000 })
    .catch(() => {});
  await page.waitForLoadState("networkidle", { timeout: 15_000 }).catch(() => {});
  await page.waitForTimeout(1200);
  const actionResult = await runActions(page, driver.actions);
  const sig = await page.evaluate(SIGNATURE);
  await page.close();
  return { status: "drive", signature: sig, actionResult };
}

/* ---------------------------------------------------------------------------
 * Rule diffs. Each compares one field and records both sides so a reader can
 * judge, not just a boolean.
 * ---------------------------------------------------------------------- */
function ruleDiffs(pack, app) {
  const out = [];
  const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  // Counts are data, not anatomy: "Export 12" and "Export 40,213" name the same
  // control. Normalise a trailing count so the rule reads the SHAPE of the group.
  const norm = (a) => a.label.replace(/^[^A-Za-z]+/, "").replace(/[\s\d,]+$/, "");
  if (!pack || !app) return out;
  const packActions = pack.toolbarActions.map(norm);
  const appActions = app.toolbarActions.map(norm);
  if (!eq(packActions, appActions)) {
    out.push({ rule: "table-action order (§2.1)", pack: packActions, app: appActions });
  }
  const packSides = pack.toolbarActions.map((a) => `${norm(a)}[${a.side}]`);
  const appSides = app.toolbarActions.map((a) => `${norm(a)}[${a.side}]`);
  if (!eq(packSides, appSides)) {
    out.push({ rule: "table-action side (§2.1)", pack: packSides, app: appSides });
  }
  const packCols = pack.columns.map((c) => c.replace(/[⠿↕▲▼↑↓]/g, "").trim());
  const appCols = app.columns.map((c) => c.replace(/[⠿↕▲▼↑↓]/g, "").trim());
  if (!eq(packCols, appCols)) {
    out.push({ rule: "column order/labels", pack: packCols, app: appCols });
  }
  if (pack.title && app.title && Math.abs(pack.title.size - app.title.size) > 0.5) {
    out.push({ rule: "title font-size", pack: pack.title.size, app: app.title.size });
  }
  if ((pack.footer ? 1 : 0) !== (app.footer ? 1 : 0)) {
    out.push({ rule: "footer present", pack: Boolean(pack.footer), app: Boolean(app.footer) });
  }
  return out;
}

/* ---------------------------------------------------------------------------
 * Run.
 * ---------------------------------------------------------------------- */
const browser = await chromium.launch({ });
const report = {
  generatedAt: new Date().toISOString(),
  pack: PACK,
  canvas: CANVAS,
  withThemes: WITH_THEMES,
  surfaces: [],
};

let appCtx = null;
let sess = null;
if (!PACK_ONLY) {
  sess = await session();
  appCtx = await browser.newContext({ viewport: CANVAS });
  // The app reads its session from both a cookie and localStorage; set both, the
  // way every capture harness in this repo does, so a page that lands on /login
  // is an auth failure rather than a silent empty capture.
  await appCtx.request.post(`${sess.api}/api/v1/auth/login`, {
    data: { email: EMAIL, password: PASSWORD },
  });
  await appCtx.addInitScript(
    ([raw, exp, theme]) => {
      window.localStorage.setItem("marula-tokens", raw);
      window.localStorage.setItem("marula-tokens-exp", exp);
      window.localStorage.setItem("marula-theme", theme);
      window.localStorage.setItem("marula-theme-preset", "marula");
    },
    [JSON.stringify(sess.tokens), String(Date.now() + sess.tokens.expires_in * 1000 - 30_000), "light"],
  );
}

for (const surface of SURFACES) {
  const packSide = await readPack(browser, surface);
  if (packSide.error) {
    report.surfaces.push({ id: surface.id, error: packSide.error });
    continue;
  }
  const record = {
    id: surface.id,
    source: surface.source,
    route: surface.route,
    themes: packSide.declared.themes,
    stateCount: packSide.page.length,
    states: [],
  };
  for (const st of packSide.page) {
    const packSig = st.themes[Object.keys(st.themes)[0]] ?? null;
    const entry = { id: st.id, title: st.title, packThemes: st.themes, app: null, deltas: [] };
    if (!PACK_ONLY) {
      const app = await readApp(browser, appCtx, sess, surface, st.id);
      entry.app = app;
      if (app.status === "drive") {
        entry.deltas = ruleDiffs(packSig, app.signature);
      }
    }
    record.states.push(entry);
  }
  report.surfaces.push(record);
  const states = record.states.length;
  const driven = record.states.filter((s) => s.app?.status === "drive").length;
  const noEq = record.states.filter((s) => s.app?.status === "no-equivalent").length;
  const notDriven = record.states.filter((s) => s.app?.status === "not-driven").length;
  console.log(
    `[${surface.id}] ${states} pack states | app: ${driven} driven, ${noEq} no-equivalent, ${notDriven} not-driven`,
  );
}

writeFileSync(resolve(OUT, "CONFORMANCE.json"), JSON.stringify(report, null, 2));
const totals = report.surfaces.reduce(
  (acc, s) => {
    if (s.error) return acc;
    acc.states += s.stateCount;
    for (const st of s.states) {
      const a = st.app?.status;
      if (a === "drive") acc.driven += 1;
      else if (a === "no-equivalent") acc.noEq += 1;
      else if (a === "not-driven") acc.notDriven += 1;
    }
    acc.deltas += s.states.reduce((n, st) => n + st.deltas.length, 0);
    return acc;
  },
  { states: 0, driven: 0, noEq: 0, notDriven: 0, deltas: 0 },
);
console.log(
  `\n[conformance] ${report.surfaces.length} surfaces, ${totals.states} pack states, ` +
    `${totals.driven} app-driven, ${totals.noEq} no-equivalent, ${totals.notDriven} not-driven, ${totals.deltas} rule deltas`,
);
console.log(`[conformance] -> ${OUT}/CONFORMANCE.json`);
if (appCtx) await appCtx.close();
await browser.close();
