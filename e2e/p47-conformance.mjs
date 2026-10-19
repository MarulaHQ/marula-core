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
    // The app declares its own tables by test id (`travel-drafts-table`,
    // `unassigned-table`, …); that is the same declared relationship the pack's
    // `#tbl` is, so it is read the same way rather than left to the cluster guess.
    surface.querySelector("[data-testid$=table]") ||
    surface.querySelector("[data-testid$=-table]") ||
    surface.querySelector('[role="table"], [role="grid"]') ||
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
        (c) => typeof c.className === "string" && /(^|\s)(hd|head|th|h)(\s|$)/.test(c.className),
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

  const openDialogs = [...document.querySelectorAll('[role=dialog]')]
    .filter(vis)
    .map((d) => (d.getAttribute("aria-label") || txt(d).slice(0, 40) || "").trim());
  const dialogText = [...document.querySelectorAll('[role=dialog]')]
    .filter(vis)
    .map((d) => txt(d).slice(0, 220))
    .join(" | ");

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
    hasList: Boolean(tbl),
    openDialogs,
    dialogText,
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
    "06-bulk-move-conflict": {
      actions: [
        "checkRow:0",
        "selectAria:Board to move the selected tickets to=#1",
        "clickText:Move",
        "wait:400",
      ],
      note: "a selection moved to another board through the real bulk bar",
    },
    "07-export-large": { actions: ["clickText:Export"] },
    "08-density-compact": { density: "compact" },
    "09-density-dense": { density: "dense" },
    "10-empty-no-results": {
      prefs: { "tickets.list": { view: "all", filters: [{ field: "title", op: "contains", value: "zzzznomatch" }] } },
    },
    "11-empty-quiet-queue": {
      prefs: { "tickets.list": { view: "unassigned" } },
      actions: ["clickText:Unassigned"],
      note: "the Unassigned view: 'Everything open has an owner.' is the app's quiet-queue copy",
    },
    "12-load-error": { abort: "**/api/v1/tickets*" },
  },
  clients: {
    "01-clients-list": { status: "drive" },
    "02-list-possible-duplicate": {
      ai: true,
      note: "the dedupe strip is gated on the decision_client_dedupe grant, which the ai:true driver enables",
    },
    "03-client-overview": {
      route: "/clients/{id}",
      routeFrom: { path: "/api/v1/clients" },
      note: "the client record frame, Overview tab",
    },
    "04-contacts": {
      route: "/clients/{id}",
      routeFrom: { path: "/api/v1/clients" },
      actions: ["clickText:Contacts"],
      note: "the record frame's Contacts tab",
    },
    "05-sites-and-devices": {
      route: "/clients/{id}",
      routeFrom: { path: "/api/v1/clients" },
      actions: ["clickText:Sites and devices"],
      note: "the record frame's Sites and devices tab",
    },
    "06-contracts-and-billing": {
      route: "/clients/{id}",
      routeFrom: { path: "/api/v1/clients" },
      actions: ["clickText:Contracts and billing"],
      note: "the record frame's Contracts and billing tab",
    },
    "07-empty-client": {
      status: "not-reachable",
      reason:
        "a half-finished client is a record shape (missing fields), and the demo seed has no such client; forcing one means mutating seeded data. /clients/[id] always renders the full frame",
    },
  },
  approvals: {
    "01-queue-no-grant": { status: "drive" },
    "02-queue-flagged": { actions: ["clickText:Flagged"], note: "the queue filtered to flagged entries" },
    "03-bulk-pass": {
      actions: ["clickTestId:approvals-select-easy", "wait:500"],
      note: "the staged bulk pass: the easy entries selected and the action bar raised, left unconfirmed so the later verdict states keep their preconditions. The link renders only while nothing is selected, and only when the seed carries entries the app classes as easy",
    },
    "04-review-flagged": { actions: ["clickText:Review all"], note: "Review all narrows the queue to the flagged entries" },
    "05-reject-dialog": {
      actions: ["clickText:Reject…"],
      note: "the reject panel with its required reason",
    },
    "06-nothing-flagged": {
      status: "not-reachable",
      reason:
        "a clean result needs a queue with no flagged entries; the demo seed carries one flagged entry and clearing it is a data mutation, not a drive",
    },
    "07-everything-approved": {
      actions: ["clickTestId:approvals-select-easy", "wait:400", "clickTestId:bulk-approve", "wait:1100", "clickText:Approved", "wait:600"],
      note: "the queue after a full pass, read from the Approved tab",
    },
    "08-invoice-detail": {
      route: "/billing/{id}",
      routeFrom: { path: "/api/v1/invoices" },
      note: "the invoice record, overdue",
    },
    "09-record-payment": {
      route: "/billing/{id}",
      routeFrom: { path: "/api/v1/invoices" },
      actions: ["clickText:Record payment", "wait:400"],
      note: "the record-payment drawer",
    },
    "10-invoice-paid": {
      route: "/billing/{id}",
      routeFrom: { path: "/api/v1/invoices" },
      actions: ["clickTestId:record-payment", "wait:400", "clickTestId:record-payment-save", "wait:900"],
      note: "the payment submitted; whether the invoice settles is what the capture reports",
    },
  },
  today: {
    "01-today-default": { status: "drive" },
    "02-quick-log-default": { actions: ["clickText:Quick log", "wait:500"], note: "the Quick log dialog at its defaults" },
    "03-quick-log-corrected": {
      actions: ["clickText:Quick log", "wait:500", "fillTestId:quicklog-minutes=45", "wait:300"],
      note: "the minutes corrected into the gap",
    },
    "04-quick-log-non-billable": {
      actions: ["clickText:Quick log", "wait:500", "clickTestId:quicklog-billable", "wait:400"],
      note: "non-billable, which reveals the reason field",
    },
    "05-quick-log-error": {
      actions: ["clickText:Quick log", "wait:600", "clickSelector:[data-testid=quicklog-work-types] button", "wait:300", "clickTestId:quicklog-submit", "wait:600"],
      note: "submit at the defaults; whether the collides-with-a-logged-entry error fires is what the capture reports",
    },
    "06-after-logging": {
      actions: ["clickText:Quick log", "wait:600", "fillTestId:quicklog-minutes=45", "wait:200", "clickSelector:[data-testid=quicklog-work-types] button", "wait:300", "clickTestId:quicklog-submit", "wait:1400"],
      note: "the day after a logged entry",
    },
    "07-quiet-day": {
      status: "not-reachable",
      reason:
        "a quiet day needs a day with no attention cards; the demo seed loads today with work, and emptying it means mutating seeded data",
    },
  },
  settings: {
    "01-settings-shell": { status: "drive" },
    "02-statuses-list": { route: "/settings/workflow", note: "Workflow → the status list with toggle rows" },
    "03-bulk-and-unsaved": {
      route: "/settings/workflow",
      actions: ["clickText:New status", "wait:400"],
      note: "an unsaved change in the form",
    },
    "04-form-validation": {
      route: "/settings/workflow",
      actions: ["clickText:New status", "wait:500", "clickText:Add step", "wait:400"],
      note: "the new-status form opened and driven toward its validation",
    },
    "05-save-failed": {
      route: "/settings/workflow",
      abort: "**/api/v1/workflow/**",
      actions: ["clickText:New status", "wait:500", "clickText:Save transitions", "wait:900"],
      note: "a change staged, then the save path aborted so a failure is what renders",
    },
    "06-delete-confirm": {
      status: "not-reachable",
      reason:
        "the app's /settings/workflow offers toggles, Import and Add step but no destructive delete with a remap; there is no confirm dialog to open",
    },
    "07-import-drawer": { route: "/settings/workflow", actions: ["clickText:Import", "wait:500"], note: "the import drawer" },
    "08-empty-list": {
      status: "not-reachable",
      reason: "the status list is seeded; emptying it means mutating seeded data",
    },
    "09-toggle-rows": { route: "/settings/workflow", note: "the toggle rows and the danger zone" },
    "10-ai-grants": { route: "/settings/ai", note: "the master switch and the per-surface grant matrix" },
    "11-shadow-list": {
      route: "/settings/ai/shadow",
      actions: ["clickText:What it would have said", "wait:400"],
      note: "the shadow list",
    },
    "12-shadow-runlog-promote": {
      route: "/settings/ai/shadow",
      actions: ["clickText:Run log", "wait:400"],
      note: "the run log; the promote control lives here",
    },
  },
  dispatch: {
    "01-board-rest": { status: "drive", note: "the day board at rest, no AI grant" },
    "02-board-ai-granted": { ai: true, note: "org ai.features_enabled + role grants; the board draws its proposed state" },
    "03-drag-in-flight": {
      actions: [
        "clickText:Day",
        "dragHold:.dp-qcard[data-q=\"18441\"]->.dp-lane[data-tech=\"AM\"] .dp-lt@0.90",
        "wait:150",
      ],
      note: "pointer pick-up held mid-flight over a free slot in Alex M.'s lane",
    },
    "04-drag-blocked": {
      actions: [
        "clickText:Day",
        "dragHold:.dp-qcard[data-q=\"18441\"]->.dp-lane[data-tech=\"JT\"] .dp-lt@0.70",
        "wait:150",
      ],
      note: "held over Jordan T.'s 14:00-18:00 absence, the pack's blocked ghost",
    },
    "05-drop-rejected": {
      actions: [
        "clickText:Day",
        "drag:.dp-qcard[data-q=\"18441\"]->.dp-lane[data-tech=\"JT\"] .dp-lt@0.70",
        "wait:300",
      ],
      note: "released over the absence; the app flashes the lane and toasts",
    },
    "06-keyboard-cursor": {
      actions: [
        "clickText:Day",
        "clickSelector:.dp-qcard[data-q=\"18441\"]",
        "pressOn:Enter@.dp-qcard[data-q=\"18441\"]",
        "press:ArrowRight",
        "wait:150",
      ],
      note: "Keyboard pick-up: Enter on a queue card, then arrow the cursor",
    },
    "07-collisions-banner": {
      route: "/dispatch?demo=collision",
      note: "the app's own collision day: Alex M.'s core switch overruns the cabling audit",
    },
    "08-density-compact": { density: "compact", actions: ["clickText:Day"] },
    "09-density-dense": { density: "dense", actions: ["clickText:Day"] },
    "10-week-view": { actions: ["clickText:Week"], note: "the Week matrix" },
    "11-unassigned-queue": { actions: ["clickText:Unassigned"], note: "the Unassigned queue table" },
    "12-plan-scope": {
      ai: true,
      actions: ["clickText:Day", "clickText:Plan with AI"],
      note: "the scope chooser dialog",
    },
    "13-plan-review": {
      ai: true,
      actions: ["clickText:Day", "clickText:Plan with AI", "clickTextAny:Everything in the unassigned queue", "clickText:Propose a plan"],
      note: "the proposed plan in the right panel",
    },
    "14-plan-resize": {
      ai: true,
      actions: [
        "clickText:Day",
        "clickText:Plan with AI",
        "clickTextAny:Everything in the unassigned queue",
        "clickText:Propose a plan",
        "clickAria:15 minutes later",
        "wait:150",
      ],
      note: "a proposed row nudged onto a different slot (the app's nudge twin of resize)",
    },
    "15-plan-split": {
      ai: true,
      actions: [
        "clickText:Day",
        "clickText:Plan with AI",
        "clickTextAny:Everything in the unassigned queue",
        "clickText:Propose a plan",
        "pressOn:s@[data-testid=plan-panel]",
        "wait:200",
      ],
      note: "the plan panel's S key: the app splits a placed block, not a proposed row; recorded as reached-but-different",
    },
    "16-plan-overlap": {
      ai: true,
      actions: [
        "clickText:Day",
        "clickText:Plan with AI",
        "clickTextAny:Everything in the unassigned queue",
        "clickText:Propose a plan",
        "clickAria:15 minutes earlier",
        "clickAria:15 minutes earlier",
        "clickAria:15 minutes earlier",
        "clickAria:15 minutes earlier",
        "clickAria:15 minutes earlier",
        "clickAria:15 minutes earlier",
        "wait:200",
      ],
      note: "a proposed row nudged back until it overlaps a placed job",
    },
    "17-plan-approval-failed": {
      status: "not-reachable",
      reason:
        "the app's plan cannot fail: `confirmPlan` (apps/web/app/dispatch/page.tsx:818-847) always books and never sets `plan.failed`, which is written only as null (:812). The 'Nothing was booked' branch at :1517-1533 is drawn but unreachable",
    },
    "18-planning-loaded": { route: "/dispatch/planning", note: "seven-day capacity, loaded" },
    "19-planning-loading": {
      route: "/dispatch/planning",
      abort: "**/api/v1/dispatch/capacity*",
      note: "capacity aborted so the skeleton stays on screen",
    },
    "20-travel-drafts": { route: "/dispatch/travel", note: "the Drafts table" },
    "21-travel-form": {
      route: "/dispatch/travel",
      actions: ["clickText:New travel draft"],
      note: "the drawer form",
    },
    "22-travel-reconciliation": {
      route: "/dispatch/travel",
      actions: ["clickText:Reconciliation"],
      note: "the 14-day reconciliation",
    },
    "23-drag-overlap": {
      actions: [
        "clickText:Day",
        "dragHold:.dp-qcard[data-q=\"18441\"]->.dp-lane[data-tech=\"TL\"] .dp-lt@0.606",
        "wait:150",
      ],
      note: "held over Tumi L.'s 13:30 job: the pack's occupied-slot ghost",
    },
    "24-overlap-reason": {
      actions: [
        "clickText:Day",
        "drag:.dp-qcard[data-q=\"18441\"]->.dp-lane[data-tech=\"TL\"] .dp-lt@0.606",
        "wait:400",
      ],
      note: "released over an occupied slot: the one-question overlap card",
    },
    "25-stacked-lanes": {
      actions: [
        "clickText:Day",
        "drag:.dp-qcard[data-q=\"18441\"]->.dp-lane[data-tech=\"TL\"] .dp-lt@0.606",
        "wait:400",
        "press:1",
        "wait:400",
      ],
      note: "overlap approved by key 1, so the lane stacks two rows",
    },
    "26-banner-and-stacks": {
      route: "/dispatch?demo=collision",
      actions: [
        "clickText:Day",
        "drag:.dp-qcard[data-q=\"18441\"]->.dp-lane[data-tech=\"RN\"] .dp-lt@0.60",
        "wait:400",
        "press:1",
        "wait:400",
      ],
      note: "the collision day plus an approved overlap, so one is flagged and one is quiet",
    },
    "27-plan-overlap-approved": {
      ai: true,
      actions: [
        "clickText:Day",
        "clickText:Plan with AI",
        "clickTextAny:Everything in the unassigned queue",
        "clickText:Propose a plan",
        "clickAria:15 minutes earlier",
        "clickAria:15 minutes earlier",
        "clickAria:15 minutes earlier",
        "clickAria:15 minutes earlier",
        "clickAria:15 minutes earlier",
        "clickAria:15 minutes earlier",
        "wait:200",
        "clickText:Waiting on the client",
        "wait:200",
      ],
      note: "the nudged overlap given a reason, so the row reads approved",
    },
    "28-settings-overlap-policy": {
      actions: ["clickText:Day", "clickAria:Dispatch settings"],
      note: "the overlapping-work policy modal",
    },
    "29-overlap-different-sites": {
      actions: [
        "clickText:Day",
        "dragHold:.dp-qcard[data-q=\"18441\"]->.dp-lane[data-tech=\"TL\"] .dp-lt@0.606",
        "wait:150",
      ],
      note: "Harbour Dental (zone 3) held over an Acme (zone 1) job: the cross-site warning",
    },
  },
  quoting: {
    Q01: { status: "drive", note: "the quotes list" },
    Q02: {
      actions: ["clickTestId:quotes-new"],
      note: "the app's New quote opens the Quote builder dialog directly; there is no Blank quote / From a template / Describe it menu",
    },
    Q03: {
      status: "not-reachable",
      reason:
        "the app has no Describe-it panel. `apps/web/app/sales/quotes/[id]/page.tsx:15` states it as a non-purpose: 'no AI describe-it panel, no clause library, no contract editor'. Q03–Q11, Q14 and C02–C05 are that flow",
    },
    Q04: { status: "not-reachable", reason: "same Describe-it panel; not built in the app" },
    Q05: { status: "not-reachable", reason: "same; no voice capture surface exists" },
    Q06: { status: "not-reachable", reason: "same; no notes-upload surface exists" },
    Q07: { status: "not-reachable", reason: "same; no reading/progress panel exists" },
    Q08: { status: "not-reachable", reason: "same; the app has no what-I-understood check step" },
    Q09: { status: "not-reachable", reason: "same; the app has no draft-from-notes state" },
    Q10: { status: "not-reachable", reason: "same; the app has no source-notes provenance tab" },
    Q11: { status: "not-reachable", reason: "same; the app has no change-by-describing proposal" },
    Q12: { status: "drive", note: "the quotes list with no AI grant: the pack's own no-grant state" },
    Q13: {
      route: "/sales/quotes/{id}",
      routeFrom: { path: "/api/v1/quotes" },
      note: "the hand-built quote editor; reached by id because the list rows do not navigate",
    },
    Q14: { status: "not-reachable", reason: "same Describe-it flow; the app has no could-not-build panel" },
    C01: { route: "/contracts", note: "the contracts list" },
    C02: { status: "not-reachable", reason: "no contract Describe-it panel in the app" },
    C03: { status: "not-reachable", reason: "no contract what-I-understood state in the app" },
    C04: { status: "not-reachable", reason: "no contract draft editor in the app" },
    C05: {
      status: "not-reachable",
      reason:
        "no renewal-from-a-contract describe flow; /contracts/[id] carries a Start renewal action (apps/web/app/contracts/[id]/page.tsx:610) that opens no panel the audit can compare",
    },
    C06: {
      status: "not-reachable",
      reason:
        "the app has no contract editor; /contracts/[id] is a read-only record with change orders (apps/web/app/contracts/[id]/page.tsx)",
    },
  },
  knowledge: {
    K01: { route: "/kb", note: "the knowledge base home" },
    K02: {
      route: "/clients/{id}",
      routeFrom: { path: "/api/v1/clients" },
      note: "the client record frame: Overview, Contacts, Sites and devices, Contracts and billing",
    },
    K03: { route: "/assets", note: "the configuration ledger (Devices / Config items / Merge queue)" },
    K04: {
      route: "/assets/{id}",
      routeFrom: { path: "/api/v1/assets" },
      note: "a device record, the app's nearest shape to the pack's server page",
    },
    K05: {
      status: "not-reachable",
      reason: "the app has no rack record with a U-elevation page; /assets/[id] is a device record, not a rack",
    },
    K06: {
      status: "not-reachable",
      reason: "the app has no site-and-network record page (no VLAN table, Wi-Fi keys or floor plan route)",
    },
    K07: { route: "/passwords", actions: ["clickText:Credentials"], note: "the client vault's credential table" },
    K08: {
      route: "/passwords",
      actions: ["clickText:Credentials"],
      note: "masked credential values in the table, the pack's hidden-secret shape",
    },
    K09: {
      route: "/passwords",
      actions: ["clickText:Credentials", "clickSelector:[data-testid^=\"vault-reveal-\"]"],
      note: "the reveal-confirmation dialog opened from the credential row",
    },
    K10: {
      route: "/passwords",
      actions: [
        "clickText:Credentials",
        "clickSelector:[data-testid^=\"vault-reveal-\"]",
        "clickTestId:vault-reveal-confirm",
        "wait:700",
      ],
      note: "the reveal confirmed; whether a value is shown is what the capture reports",
    },
    K11: {
      route: "/kb/secrets",
      actions: ["clickText:Org-level"],
      note: "the org vault admin view",
    },
    K12: {
      route: "/kb/secrets",
      actions: ["clickText:Category grants"],
      note: "the category-access grant surface",
    },
    K13: {
      status: "not-reachable",
      reason: "the technician view of the org vault needs a technician seat; this audit drives the owner session only",
    },
    K14: { route: "/audit", note: "the audit trail ledger" },
    K15: {
      route: "/kb",
      actions: ["pressOn:Control+k@body", "wait:400"],
      note: "the command palette (search launcher), the pack's find-a-password state",
    },
    K16: {
      status: "not-reachable",
      reason: "the app has no add-a-two-step-code dialog (no TOTP enrolment UI on the vault record)",
    },
  },
  workflow: {
    W01: {
      route: "/automations/canvas",
      actions: ["wait:600"],
      note: "the visual canvas (React Flow). The app's default document is one entry node",
    },
    W02: {
      route: "/automations/canvas",
      actions: ["clickTestId:rf__node-entry", "wait:400"],
      note: "a node selected, showing the inspector panel",
    },
    W03: {
      route: "/automations",
      actions: ["clickText:Edit", "wait:700"],
      note: "the app's trigger editor is a form on the rule, not a selected canvas node; recorded as reached-but-different",
    },
    W04: {
      route: "/automations",
      actions: ["clickText:New rule", "wait:700"],
      note: "the app's wait-for-event is a step in the rule form, not a canvas node selection",
    },
    W05: {
      route: "/automations/runs",
      actions: ["clickText:Waiting", "wait:400"],
      note: "the runs list filtered to Waiting; whether a run exists is what the capture reports",
    },
    W06: {
      route: "/automations/runs",
      actions: ["clickText:Failed", "wait:400"],
      note: "the runs list filtered to Failed",
    },
    W07: {
      route: "/automations",
      actions: ["clickText:New rule", "wait:500"],
      note: "the app's describe-a-change proposal is the prose draft on the rule form (automation-prose-draft)",
    },
    W08: { route: "/automations/canvas", actions: ["wait:600"], note: "a new, empty workflow — the app's default canvas document" },
    W09: {
      route: "/automations/canvas",
      actions: ["clickText:Steps", "wait:500"],
      note: "the Steps view of the same document",
    },
  },
  portal: {
    "01-login": { status: "drive", anon: true },
    "02-tickets": { seat: "portal@acme.demo", route: "/portal/tickets", note: "the portal's ticket list for the Acme seat" },
    "03-ticket-reply": {
      seat: "portal@acme.demo",
      route: "/portal/tickets",
      actions: ["clickSelector:[data-testid^=\"portal-ticket-\"]", "wait:700"],
      note: "the first ticket opened, with its reply box",
    },
    "04-ticket-nothing-yet": {
      seat: "portal@acme.demo",
      route: "/portal/tickets",
      actions: ["clickSelector:[data-testid^=\"portal-ticket-\"]", "wait:700"],
      note: "a ticket opened from the list; the newest one is the closest to a brand-new ticket",
    },
    "05-invoices": { seat: "portal@acme.demo", route: "/portal/invoices", note: "the portal invoices" },
    "06-quote": { seat: "portal@acme.demo", route: "/portal/quotes", note: "the quotes awaiting the client's answer" },
    "07-accept-quote": {
      seat: "portal@acme.demo",
      route: "/portal/quotes",
      actions: ["clickSelector:[data-testid^=\"portal-quote-\"]", "wait:600", "clickTestId:portal-quote-accept", "wait:600"],
      note: "opened the quote, then accepting; the validation error is what the capture reports",
    },
    "08-feedback": { seat: "portal@acme.demo", route: "/portal/csat", note: "the feedback surface" },
    "09-schedule": { seat: "portal@acme.demo", route: "/portal/schedule", note: "schedule and ask for a visit" },
    "10-help": { seat: "portal@acme.demo", route: "/portal/kb", note: "the portal help pages" },
    "11-empty-tickets": {
      seat: "portal@globex.demo",
      route: "/portal/quotes",
      note: "the Globex seat: its draft quote is deliberately invisible, so the quotes page draws the typed empty state",
    },
    "12-appearance": { seat: "portal@acme.demo", route: "/portal/tickets", note: "the portal's own appearance, per person" },
  },
  auth: {
    "01-login": { status: "drive", anon: true },
    "02-login-error": {
      anon: true,
      actions: ["fillSelector:#login-email=owner@demo.local", "fillSelector:#login-password=wrong-password", "clickTestId:login-submit", "wait:1200"],
      note: "the form submitted with a wrong password",
    },
    "03-mfa": { anon: true, route: "/mfa", note: "the six-digit code step" },
    "04-magic-link": { anon: true, route: "/magic-link", note: "check your email" },
    "05-password-reset": { anon: true, route: "/password-reset", note: "choose a new password" },
    "06-sso": { anon: true, route: "/sso", note: "sign in with a company account" },
    "07-verify-email": { anon: true, route: "/verify-email", note: "verify your email" },
    "08-accept-invite": { anon: true, route: "/accept-invite", note: "accept an invitation" },
    "09-welcome-invite-people": { anon: true, route: "/welcome", note: "the org wizard" },
    "10-welcome-all-steps": { anon: true, route: "/welcome", actions: ["wait:400"], note: "the org wizard's step set" },
    "11-onboarding-second-step": { anon: true, route: "/onboarding", note: "the per-person flow" },
    "12-onboarding-all-steps": { anon: true, route: "/onboarding", actions: ["wait:400"], note: "the per-person flow's step set" },
    "13-skipped-and-resumed": {
      status: "not-reachable",
      reason: "the skipped-and-resumed state is the wizard mid-flight with prior answers; /welcome and /onboarding start from step one in a clean session",
    },
    "14-never-show-again": { anon: true, route: "/onboarding", actions: ["wait:400"], note: "the never-show-again choice on the last step" },
  },
  "mobile-core": {
    "*": {
      status: "n/a",
      reason:
        "the app's mobile half is a native build (native/app, Expo) and this harness drives the web bundle; the pack's 11 core screens are drawn and the app has the surface, but a browser cannot render it. Instrument coverage, not a pack or product gap",
    },
  },
  "mobile-scan": {
    "*": {
      status: "n/a",
      reason:
        "same: the scan flow is native (native/app), so the browser harness cannot reach the pack's 10 scanner states. Instrument coverage, not a pack or product gap",
    },
  },
};

function driverFor(surfaceId, stateId) {
  const table = DRIVERS[surfaceId];
  if (!table) return { status: "not-driven", reason: "no driver table for this surface" };
  return table[stateId] ?? table["*"] ?? { status: "not-driven", reason: "no driver authored for this state" };
}

async function putSetting(ctx, api, tokens, key, value, scope = "user") {
  const res = await ctx.request.put(`${api}/api/v1/settings/${scope}`, {
    data: { key, value },
    headers: { authorization: `Bearer ${tokens.access_token}` },
  });
  return res.ok();
}

/* AI grants are org settings, not user settings: enabling features and granting
 * a surface to the owner's role is what makes the pack's "AI granted" states
 * exist at all on /dispatch, /approvals, /sales/quotes and /automations. */
async function enableAi(ctx, api, tokens) {
  const auth = { authorization: `Bearer ${tokens.access_token}` };
  await ctx.request.put(`${api}/api/v1/settings/ai/decision/features`, {
    data: { enabled: true, idempotency_key: `features:p47:${Date.now()}` },
    headers: auth,
  });
  const roles = await ctx.request.get(`${api}/api/v1/roles`, { headers: auth });
  let roleId = null;
  if (roles.ok()) {
    const body = await roles.json();
    const rows = Array.isArray(body) ? body : (body.items ?? body.roles ?? []);
    roleId = rows[0]?.id ?? null;
  }
  for (const surface of ["decision_dispatch", "decision_client_dedupe", "decision_tool_routing", "general_chat"]) {
    await ctx.request.put(`${api}/api/v1/settings/ai/grants`, {
      data: {
        surface_key: surface,
        grantee_kind: "role",
        role_id: roleId ?? undefined,
        idempotency_key: `grant:p47:${surface}:${Date.now()}`,
      },
      headers: auth,
    });
  }
  return { featuresEnabled: true, roleId };
}

/* The action mini-language. Every verb is a real user gesture; a verb that does
 * not land returns `{action, ok:false}` and the state still captures, so a
 * broken drive is recorded rather than thrown. */
async function runActions(page, actions) {
  for (const action of actions ?? []) {
    const [verb, rest] = action.split(":");
    const fail = () => ({ action, ok: false });
    try {
      if (verb === "clickText") {
        await page
          .locator("button, [role=radio], [role=tab], [role=button], a")
          .filter({ hasText: new RegExp(`^\\s*${rest.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`, "i") })
          .first()
          .click({ timeout: 4000 });
      } else if (verb === "clickAria") {
        await page.locator(`[aria-label="${rest}"]`).first().click({ timeout: 4000 });
      } else if (verb === "clickTextAny") {
        await page.getByText(rest, { exact: false }).first().click({ timeout: 4000 });
      } else if (verb === "clickTestId") {
        await page.locator(`[data-testid="${rest}"]`).first().click({ timeout: 4000 });
      } else if (verb === "clickSelector") {
        await page.locator(rest).first().click({ timeout: 4000 });
      } else if (verb === "clickNth") {
        const [sel, n] = rest.split("#");
        await page.locator(sel).nth(Number(n) || 0).click({ timeout: 4000 });
      } else if (verb === "checkRow") {
        await page.locator("tbody tr, [data-testid$=table] [role=row], .trow, .dp-trow, .dp-qcard")
          .nth(Number(rest) || 0)
          .locator('input[type="checkbox"], [role="checkbox"], button.dp-cb, input')
          .first()
          .click({ timeout: 4000 });
      } else if (verb === "press") {
        await page.keyboard.press(rest);
      } else if (verb === "pressOn") {
        const [key, sel] = rest.split("@");
        await page.locator(sel).first().focus().catch(() => {});
        await page.keyboard.press(key);
      } else if (verb === "wait") {
        await page.waitForTimeout(Number(rest) || 300);
      } else if (verb === "fillAria") {
        const [label, value] = rest.split("=");
        await page.locator(`[aria-label="${label}"]`).first().fill(value ?? "", { timeout: 4000 });
      } else if (verb === "fillSelector") {
        const at = rest.indexOf("=");
        const css = at === -1 ? rest : rest.slice(0, at);
        const val = at === -1 ? "" : rest.slice(at + 1);
        await page.locator(css).first().fill(val, { timeout: 4000 });
      } else if (verb === "fillTestId") {
        const [id, value] = rest.split("=");
        await page.locator(`[data-testid="${id}"]`).first().fill(value ?? "", { timeout: 4000 });
      } else if (verb === "fillPlaceholder") {
        const [ph, value] = rest.split("=");
        await page.locator(`[placeholder="${ph}"]`).first().fill(value ?? "", { timeout: 4000 });
      } else if (verb === "selectAria") {
        const [label, value] = rest.split("=");
        const target = value?.startsWith("#") ? { index: Number(value.slice(1)) } : value;
        await page.locator(`[aria-label="${label}"]`).first().selectOption(target, { timeout: 4000 });
      } else if (verb === "drag" || verb === "dragHold") {
        // A real pointer drag: hover the source, press, move in steps onto the
        // target, release. `to@fx,fy` targets a fraction of the target's box,
        // which is how a drop lands in a specific time window on a lane.
        // `dragHold` leaves the button down, so the mid-drag state is what the
        // signature captures.
        const [from, toSpec, steps] = rest.split("->");
        const [to, frac] = toSpec.split("@");
        const [fx, fy] = (frac ?? "0.5,0.5").split(",").map(Number);
        const a = page.locator(from).first();
        const b = page.locator(to).first();
        const ab = await a.boundingBox();
        const bb = await b.boundingBox();
        if (!ab || !bb) return fail();
        const tx = bb.x + bb.width * (Number.isFinite(fx) ? fx : 0.5);
        const ty = bb.y + bb.height * (Number.isFinite(fy) ? fy : 0.5);
        await page.mouse.move(ab.x + ab.width / 2, ab.y + ab.height / 2);
        await page.mouse.down();
        const n = Number(steps) || 12;
        for (let i = 1; i <= n; i += 1) {
          await page.mouse.move(
            ab.x + ab.width / 2 + ((tx - ab.x - ab.width / 2) * i) / n,
            ab.y + ab.height / 2 + ((ty - ab.y - ab.height / 2) * i) / n,
          );
          await page.waitForTimeout(16);
        }
        if (verb === "drag") await page.mouse.up();
      } else if (verb === "dragEnd") {
        await page.mouse.up();
      } else if (verb === "openFirstRow") {
        await page
          .locator("tbody tr a, table a, [data-testid$=table] a, .dp-trow a, main a[href]")
          .first()
          .click({ timeout: 4000 });
      } else if (verb === "hover") {
        await page.locator(rest).first().hover({ timeout: 4000 });
      } else {
        return { action, ok: false, reason: `unknown verb ${verb}` };
      }
    } catch {
      return fail();
    }
    await page.waitForTimeout(Number(process.env.P47_ACTION_WAIT ?? 260));
  }
  return { ok: true };
}

/* A driver may name a record route by template (`/sales/quotes/{id}`) plus the
 * API collection that enumerates it. Some app lists do not link their rows (the
 * quotes table has no row navigation), so the id has to come from the API the
 * same list was built from rather than from a click. */
async function resolveRoute(ctx, api, tokens, driver, fallback) {
  if (!driver.routeFrom) return driver.route ?? fallback;
  const res = await ctx.request.get(`${api}${driver.routeFrom.path}`, {
    headers: { authorization: `Bearer ${tokens.access_token}` },
  });
  if (!res.ok()) return driver.route;
  const body = await res.json();
  const rows = Array.isArray(body) ? body : (body.items ?? body.data ?? []);
  const id = rows[0]?.[driver.routeFrom.pick ?? "id"];
  return id ? driver.route.replace("{id}", id) : driver.route;
}

/* The auth and portal-sign-in states only exist for a signed-out visitor: with
 * a session in localStorage /login bounces to the app and the form is never
 * drawn. Those drivers therefore take a context with no injected session. */
let anonCtx = null;
async function anonPage(browser) {
  if (!anonCtx) anonCtx = await browser.newContext({ viewport: CANVAS });
  return anonCtx.newPage();
}

async function readApp(browser, ctx, session_, surface, stateId) {
  const driver = driverFor(surface.id, stateId);
  if (driver.status && driver.status !== "drive") {
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
  for (const [key, value] of Object.entries(driver.org ?? {})) {
    await putSetting(ctx, api, tokens, key, value, "org");
  }
  if (driver.ai) await enableAi(ctx, api, tokens);

  const page = (driver.anon ? await anonPage(browser) : await ctx.newPage());
  if (driver.abort) await page.route(driver.abort, (r) => r.abort());
  const route = await resolveRoute(ctx, api, tokens, driver, surface.route);
  // The portal is a separate session: its own login route and its own token
  // store. A seat driver signs in through the portal's own form first, in the
  // same context, then walks to the state's own route.
  if (driver.seat) {
    await page.goto(`${web}/portal/login`, { waitUntil: "domcontentloaded", timeout: 60_000 }).catch(() => {});
    await page.waitForTimeout(900);
    await page.locator('[data-testid="portal-use-password"]').first().click({ timeout: 4000 }).catch(() => {});
    await page.locator("#portal-email").first().fill(driver.seat, { timeout: 4000 }).catch(() => {});
    await page.locator('input[type="password"]').first().fill("demo-password-123", { timeout: 4000 }).catch(() => {});
    await page.locator('[data-testid="portal-password-submit"]').first().click({ timeout: 4000 }).catch(() => {});
    await page.waitForTimeout(1600);
  }
  await page
    .goto(`${web}${route}`, { waitUntil: "domcontentloaded", timeout: 60_000 })
    .catch(() => {});
  await page.waitForLoadState("networkidle", { timeout: 15_000 }).catch(() => {});
  await page.waitForTimeout(1200);
  const actionResult = await runActions(page, driver.actions);
  const sig = await page.evaluate(SIGNATURE);
  await page.close();
  return { status: "drive", route, signature: sig, actionResult };
}

/* ---------------------------------------------------------------------------
 * Rule diffs. Each compares one field and records both sides so a reader can
 * judge, not just a boolean.
 * ---------------------------------------------------------------------- */
function ruleChecks(pack, app) {
  const out = [];
  const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  // Counts are data, not anatomy: "Export 12" and "Export 40,213" name the same
  // control. Normalise a trailing count so the rule reads the SHAPE of the group.
  const norm = (a) => a.label.replace(/^[^A-Za-z]+/, "").replace(/[\s\d,]+$/, "");
  if (!pack || !app) return out;
  const push = (rule, p, a, ok) => out.push({ rule, pack: p, app: a, ok });
  const packActions = pack.toolbarActions.map(norm);
  const appActions = app.toolbarActions.map(norm);
  push("toolbar order (§2.1)", packActions, appActions, eq(packActions, appActions));
  const packSides = pack.toolbarActions.map((a) => `${norm(a)}[${a.side}]`);
  const appSides = app.toolbarActions.map((a) => `${norm(a)}[${a.side}]`);
  push("toolbar side (§2.1)", packSides, appSides, eq(packSides, appSides));
  // Column and row rules only bind where BOTH sides drew a list. On a
  // record-shaped state (a client record, a secret, an auth wizard) one side has
  // no table head at all, and comparing the app's section headings against the
  // pack's absent header would read as a defect when it is the wrong object.
  const listPair = pack.hasList && app.hasList;
  const packCols = pack.columns.map((c) => c.replace(/[⠿↕▲▼↑↓]/g, "").trim());
  const appCols = app.columns.map((c) => c.replace(/[⠿↕▲▼↑↓]/g, "").trim());
  push(
    "column order/labels",
    listPair ? packCols : "n/a (no list on both sides)",
    listPair ? appCols : "n/a (no list on both sides)",
    listPair ? eq(packCols, appCols) : true,
  );
  push(
    "title font-size",
    pack.title ? Math.round(pack.title.size) : null,
    app.title ? Math.round(app.title.size) : null,
    Boolean(pack.title && app.title && Math.abs(pack.title.size - app.title.size) <= 0.5),
  );
  // Row cells: the COUNT is the structural signal. The cell text is recorded
  // beside it but is not a rule, because the two sides are seeded with different
  // records ("#18402" against "DEMO-1004") and a text diff would read as a
  // design delta when it is demo data.
  push(
    "first-row cells (count)",
    listPair ? pack.rowCells : "n/a (no list on both sides)",
    listPair ? app.rowCells : "n/a (no list on both sides)",
    listPair ? pack.rowCells === app.rowCells : true,
  );
  push(
    "first-row cell text",
    listPair ? pack.firstRow : "n/a",
    listPair ? app.firstRow : "n/a",
    true,
  );
  push(
    "list region present",
    pack.hasList,
    app.hasList,
    pack.hasList === app.hasList,
  );
  push(
    "footer present",
    Boolean(pack.footer),
    Boolean(app.footer),
    Boolean(pack.footer) === Boolean(app.footer),
  );
  return out;
}

const deltaDiffs = (checks) => checks.filter((c) => !c.ok);

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
    const driver = driverFor(surface.id, st.id);
    const entry = {
      id: st.id,
      title: st.title,
      driverNote: driver.note ?? null,
      packThemes: st.themes,
      app: null,
      deltas: [],
      rules: [],
    };
    if (!PACK_ONLY) {
      const app = await readApp(browser, appCtx, sess, surface, st.id);
      entry.app = app;
      if (app.status === "drive") {
        entry.rules = ruleChecks(packSig, app.signature);
        entry.deltas = deltaDiffs(entry.rules);
      }
    }
    record.states.push(entry);
  }
  report.surfaces.push(record);
  const states = record.states.length;
  const driven = record.states.filter((s) => s.app?.status === "drive").length;
  const noEq = record.states.filter((s) => s.app?.status === "no-equivalent").length;
  const notDriven = record.states.filter((s) => s.app?.status === "not-driven").length;
  const notReach = record.states.filter((s) => s.app?.status === "not-reachable").length;
  const na = record.states.filter((s) => s.app?.status === "n/a").length;
  console.log(
    `[${surface.id}] ${states} pack states | app: ${driven} driven, ${notReach} not-reachable, ${na} n/a, ${noEq} no-equivalent, ${notDriven} not-driven`,
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
      else if (a === "not-reachable") acc.notReach += 1;
      else if (a === "n/a") acc.na += 1;
      else if (a === "no-equivalent") acc.noEq += 1;
      else if (a === "not-driven") acc.notDriven += 1;
    }
    acc.deltas += s.states.reduce((n, st) => n + st.deltas.length, 0);
    return acc;
  },
  { states: 0, driven: 0, notReach: 0, na: 0, noEq: 0, notDriven: 0, deltas: 0 },
);
console.log(
  `\n[conformance] ${report.surfaces.length} surfaces, ${totals.states} pack states, ` +
    `${totals.driven} app-driven, ${totals.notReach} not-reachable, ${totals.na} n/a, ` +
    `${totals.noEq} no-equivalent, ${totals.notDriven} not-driven, ${totals.deltas} rule deltas`,
);
console.log(`[conformance] -> ${OUT}/CONFORMANCE.json`);
if (appCtx) await appCtx.close();
await browser.close();
