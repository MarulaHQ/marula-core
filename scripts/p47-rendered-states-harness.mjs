#!/usr/bin/env node
/**
 * p47-rendered-states-harness — the rendered-state instrument for lane
 * `p47-rendered-states`.
 *
 * The structural audit compared default/landing states. This harness drives
 * the STATE MODIFIERS and records COMPUTED VALUES, so a defect that only
 * appears when two states meet (selected + current on the same row) is
 * measurable rather than inferred.
 *
 * Per surface it drives: the three density steps; row hover; row
 * focus-visible; row selected; row current; both states at once; the toolbar
 * controls (hover, focus, disabled); the filter builder, column chooser and
 * export menu; the bulk bar; empty; error; loading; and the row's right edge
 * against its scroll container.
 *
 * Every measurement is a getComputedStyle value or a getBoundingClientRect
 * number. A probe that cannot run records `{ ok: false, why }` with a reason —
 * never a silent gap, and never the word "inconclusive".
 *
 * Usage (after `scripts/e2e-stack.sh up` and `scripts/e2e-build-web.sh`):
 *   node scripts/p47-rendered-states-harness.mjs                 # all surfaces
 *   node scripts/p47-rendered-states-harness.mjs --surface approvals,billing
 *   node scripts/p47-rendered-states-harness.mjs --shots         # + PNGs
 */

import { chromium } from "playwright";
import { mkdirSync, readFileSync, existsSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..");
const OUT = resolve(ROOT, "design/qa-p47-rendered-states");
const SHOTS = resolve(OUT, "shots");
mkdirSync(SHOTS, { recursive: true });

const envPath = resolve(ROOT, "e2e/.e2e-stack.env");
if (!existsSync(envPath)) {
  throw new Error(
    "e2e/.e2e-stack.env is missing. Run scripts/e2e-stack.sh up first — the lane " +
      "boot routes through that script; ports are never set by hand.",
  );
}
const env = Object.fromEntries(
  readFileSync(envPath, "utf8")
    .split("\n")
    .filter((l) => l.includes("="))
    .map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]),
);
const WEB = process.env.E2E_WEB_URL ?? env.E2E_WEB_URL ?? "http://localhost:3000";
const API = (env.E2E_API_URL ?? "http://localhost:8080/").replace(/\/+$/, "");
const VIEWPORT = { width: 1600, height: 994 };
const STAFF = { email: "owner@demo.local", password: "demo-password-123" };

const args = process.argv.slice(2);
const only = args.includes("--surface") ? args[args.indexOf("--surface") + 1].split(",") : null;
const wantShots = args.includes("--shots");
const SHOT_KEYS = ["approvals", "billing", "tickets", "clients", "contracts", "quotes"];

/* ------------------------------------------------------------------ *
 * Surfaces
 * ------------------------------------------------------------------ */
const SURFACES = [
  { key: "approvals", path: "/approvals", note: "money.css row modifiers — the proving defect" },
  { key: "billing", path: "/billing", note: "same stylesheet family as approvals" },
  { key: "billing-credit-notes", path: "/billing/credit-notes" },
  { key: "tickets", path: "/tickets", note: "the template surface; kit-table row state" },
  { key: "clients", path: "/clients" },
  { key: "assets", path: "/assets" },
  { key: "stock", path: "/stock" },
  { key: "stock-tools", path: "/stock/tools" },
  { key: "inventory", path: "/inventory" },
  { key: "purchasing", path: "/purchasing" },
  { key: "purchasing-vendors", path: "/purchasing/vendors" },
  { key: "contracts", path: "/contracts" },
  { key: "projects", path: "/projects" },
  { key: "audit", path: "/audit" },
  { key: "provenance", path: "/provenance" },
  { key: "documents", path: "/documents", note: "knowledge/documents list" },
  { key: "kb", path: "/kb" },
  { key: "quotes", path: "/sales/quotes", note: "quoting" },
  { key: "sales-catalog", path: "/sales/catalog" },
  { key: "dispatch", path: "/dispatch" },
  { key: "today", path: "/today" },
  { key: "settings-team", path: "/settings/team" },
  { key: "settings-tags", path: "/settings/tags" },
  { key: "settings-boards", path: "/settings/boards" },
  { key: "settings-api-keys", path: "/settings/api-keys" },
  { key: "reports", path: "/reports" },
  { key: "scorecards", path: "/scorecards" },
  { key: "alerts", path: "/alerts" },
  { key: "imports", path: "/imports" },
  { key: "passwords", path: "/passwords" },
];

const PORTAL_SURFACES = [
  { key: "portal-tickets", path: "/portal/tickets" },
  { key: "portal-invoices", path: "/portal/invoices" },
  { key: "portal-quotes", path: "/portal/quotes" },
  { key: "portal-kb", path: "/portal/kb" },
];

/* ------------------------------------------------------------------ *
 * Page-side measurement library (installed once per page).
 * ------------------------------------------------------------------ */
function installMeasure() {
  const paint = (el) => {
    const cs = getComputedStyle(el);
    return {
      backgroundColor: cs.backgroundColor,
      backgroundImage: cs.backgroundImage,
      boxShadow: cs.boxShadow,
      outline: `${cs.outlineWidth} ${cs.outlineStyle} ${cs.outlineColor}`,
      borderTop: `${cs.borderTopWidth} ${cs.borderTopStyle} ${cs.borderTopColor}`,
      borderRight: `${cs.borderRightWidth} ${cs.borderRightStyle} ${cs.borderRightColor}`,
      borderBottom: `${cs.borderBottomWidth} ${cs.borderBottomStyle} ${cs.borderBottomColor}`,
      borderLeft: `${cs.borderLeftWidth} ${cs.borderLeftStyle} ${cs.borderLeftColor}`,
      height: cs.height,
      position: cs.position,
    };
  };
  const rect = (el) => {
    const r = el.getBoundingClientRect();
    return { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height), right: Math.round(r.right) };
  };
  const scrollContainer = (el) => {
    let p = el.parentElement;
    while (p) {
      const cs = getComputedStyle(p);
      if (/(auto|scroll)/.test(cs.overflowX) || /(auto|scroll)/.test(cs.overflow)) return p;
      p = p.parentElement;
    }
    return document.documentElement;
  };
  window.__m = { paint, rect, scrollContainer };
  return true;
}

/** Build a state record for one row. `preferredIndex` selects which row; the
 *  caller passes the row it actually put into the state, so a selection made on
 *  row 0 is never read back from row 1. */
const MEASURE_ROW = ({ rowSelector, index: preferredIndex }) => {
  const m = window.__m;
  const rows = [...document.querySelectorAll(rowSelector)].filter(
    (r) => r.querySelector("td") && !r.querySelector(":scope > th"),
  );
  if (!rows.length) return { ok: false, why: `no data rows matched ${rowSelector}` };
  const idx = Math.min(preferredIndex ?? 1, rows.length - 1);
  const row = rows[idx];
  const cont = m.scrollContainer(row);
  const cr = m.rect(cont);
  const rr = m.rect(row);
  const cellRecords = [...row.querySelectorAll(":scope > td")].map((c, i) => {
    const p = m.paint(c);
    const r = m.rect(c);
    return {
      i,
      boxShadow: p.boxShadow,
      backgroundColor: p.backgroundColor,
      borderTop: p.borderTop,
      borderRight: p.borderRight,
      borderBottom: p.borderBottom,
      borderLeft: p.borderLeft,
      outline: p.outline,
      right: r.right,
      crossesContainerRight: r.right > cr.right + 1,
    };
  });
  const rowPaint = m.paint(row);
  const prev = document.activeElement;
  row.focus();
  const rowFocusable = document.activeElement === row;
  if (prev && prev.focus) prev.focus();
  return {
    ok: true,
    rowIndex: idx,
    rowCount: rows.length,
    rowClass: row.className,
    rowAttrs: {
      dataSelected: row.getAttribute("data-selected"),
      dataCurrent: row.getAttribute("data-current"),
      ariaCurrent: row.getAttribute("aria-current"),
      ariaSelected: row.getAttribute("aria-selected"),
      tabIndex: row.getAttribute("tabindex"),
    },
    row: { ...rowPaint, rect: rr },
    rowFocusable,
    cells: cellRecords,
    cellCount: cellRecords.length,
    distinctCellBoxShadows: [...new Set(cellRecords.map((c) => c.boxShadow))],
    distinctCellBackgrounds: [...new Set(cellRecords.map((c) => c.backgroundColor))],
    cellsCrossingContainerRight: cellRecords.filter((c) => c.crossesContainerRight).length,
    container: {
      rect: cr,
      scrollWidth: cont.scrollWidth,
      clientWidth: cont.clientWidth,
      overflows: cont.scrollWidth > cont.clientWidth + 1,
    },
    rowCrossesContainerRight: rr.right > cr.right + 1,
    focusablesInRow: [...row.querySelectorAll("a,button,input,select,textarea,[tabindex]")].map(
      (e) => e.tagName + (e.getAttribute("data-testid") ? `#${e.getAttribute("data-testid")}` : ""),
    ),
  };
};

/* ------------------------------------------------------------------ *
 * Driver
 * ------------------------------------------------------------------ */
const browser = await chromium.launch();
const findings = {
  generatedAt: new Date().toISOString(),
  viewport: VIEWPORT,
  stack: { web: WEB, api: API },
  surfaces: {},
  portal: {},
};
const consoleErrors = [];
const pageErrors = [];
const crashes = [];

// Write after every surface, so a crash late in the run costs one surface
// instead of the whole matrix.
function flush() {
  findings.consoleErrors = consoleErrors;
  findings.pageErrors = pageErrors;
  findings.crashes = crashes;
  writeFileSync(resolve(OUT, "rendered-states.json"), JSON.stringify(findings, null, 2) + "\n");
}

// A rejected route handler used to kill the process and lose the run. Record
// the rejection and keep going.
process.on("unhandledRejection", (reason) => {
  const line = `unhandledRejection: ${reason?.message ?? reason}`;
  crashes.push(line);
  console.error(`  ! ${line}`);
  flush();
});
process.on("uncaughtException", (err) => {
  crashes.push(`uncaughtException: ${err?.message ?? err}`);
  console.error(`  ! uncaughtException: ${err?.message ?? err}`);
  flush();
  process.exit(0);
});

async function newSession() {
  const context = await browser.newContext({ viewport: VIEWPORT });
  const page = await context.newPage();
  const lists = [];
  page.on("response", async (res) => {
    const url = res.url();
    if (!/\/api\/v1\//.test(url)) return;
    if (!res.request().method().startsWith("GET")) return;
    const ct = res.headers()["content-type"] ?? "";
    if (!ct.includes("json")) return;
    let len = -1;
    try {
      const body = await res.json();
      len = Array.isArray(body) ? body.length : Array.isArray(body?.items) ? body.items.length : -1;
    } catch {
      /* not a JSON list */
    }
    lists.push({ url, status: res.status(), arrayLength: len });
  });
  page.on("console", (msg) => {
    if (msg.type() === "error") consoleErrors.push(`${page.url()} :: ${msg.text()}`);
  });
  page.on("pageerror", (e) => pageErrors.push(`${page.url()} :: ${e.message}`));
  return { context, page, lists };
}

async function staffLogin(page) {
  await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded", timeout: 60_000 });
  await page.fill("#login-email", STAFF.email, { timeout: 60_000 });
  await page.fill("#login-password", STAFF.password, { timeout: 60_000 });
  await page.click('[data-testid="login-submit"]', { timeout: 60_000 });
  await page.waitForFunction(() => !location.pathname.startsWith("/login"), null, { timeout: 60_000 });
}

async function portalLogin(page, email) {
  await page.goto(`${WEB}/portal/login`, { waitUntil: "domcontentloaded", timeout: 60_000 });
  await page.click('[data-testid="portal-use-password"]', { timeout: 60_000 });
  await page.fill("#portal-email", email, { timeout: 60_000 });
  await page.fill("#portal-password", STAFF.password, { timeout: 60_000 });
  await page.click('[data-testid="portal-password-submit"]', { timeout: 60_000 });
  await page.waitForFunction(() => !location.pathname.startsWith("/portal/login"), null, { timeout: 60_000 });
}

const ready = async (page) => {
  await page.waitForLoadState("domcontentloaded");
  await page.waitForTimeout(1100);
};

async function rowSelector(page) {
  for (const sel of ["table.m-table tbody tr", "table tbody tr", "[data-testid$='-row']", "tr"]) {
    const n = await page.locator(sel).filter({ has: page.locator("td") }).count();
    if (n > 0) return sel;
  }
  return null;
}

function pickRowSource(list) {
  const arrays = list.filter((l) => l.arrayLength > 0 && l.status === 200);
  if (!arrays.length) return null;
  arrays.sort((a, b) => b.arrayLength - a.arrayLength);
  return arrays[0];
}

async function probeSurface(page, surface, lists) {
  const rec = { key: surface.key, path: surface.path, note: surface.note ?? null, states: {} };
  const before = lists.length;
  const open = async () => {
    await page.goto(`${WEB}${surface.path}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
    await ready(page);
    await page.evaluate(installMeasure);
  };
  await open();

  const rowSel = await rowSelector(page);
  const observed = lists.slice(before);
  rec.listResponses = observed;
  rec.rowSource = pickRowSource(observed);

  if (!rowSel) {
    rec.states.noRows = { ok: false, why: "no table or row markup on the route — row instrument has nothing to read" };
    return rec;
  }
  rec.rowSelector = rowSel;

  const measure = async (label, index) => {
    try {
      rec.states[label] = await page.evaluate(MEASURE_ROW, { rowSelector: rowSel, index });
    } catch (e) {
      rec.states[label] = { ok: false, why: String(e.message ?? e) };
    }
  };
  const rowLoc = () => page.locator(rowSel).filter({ has: page.locator("td") });
  // The pointer sits somewhere after every click; park it off the table so a
  // hover state never masquerades as a selection or a cursor state.
  const parkPointer = async () => {
    await page.mouse.move(4, 4);
    await page.waitForTimeout(160);
  };

  // ---- baseline + hover --------------------------------------------------------
  await parkPointer();
  const rowCount = await rowLoc().count();
  const focusIndex = Math.min(1, rowCount - 1);
  await measure("row-default", focusIndex);
  try {
    await rowLoc().nth(focusIndex).hover({ timeout: 5000 });
    await page.waitForTimeout(140);
    await measure("row-hover", focusIndex);
  } catch (e) {
    rec.states["row-hover"] = { ok: false, why: String(e.message ?? e) };
  }
  await parkPointer();

  // ---- select: scan the rows for an enabled selection control -------------------
  // Two control shapes exist in the app: a raw `input[type=checkbox]`
  // (approvals, and the money cluster) and the kit `Checkbox`, which is a
  // `[role=checkbox]` button (the tickets ledger and its derivatives). A probe
  // that only looked for the input missed half the fleet. Row `focusIndex` is
  // tried first, so the row that is measured is the row that was selected.
  let selected = false;
  let selectHow = null;
  let selectedIndex = focusIndex;
  const order = [...new Set([focusIndex, 0, 2, 3, 4, 5])].filter((i) => i < rowCount);
  for (const i of order) {
    if (selected) break;
    const row = rowLoc().nth(i);
    const input = row.locator("input[type=checkbox]").first();
    if ((await input.count()) && !(await input.isDisabled())) {
      try {
        await input.check({ timeout: 4000, force: true });
        await page.waitForTimeout(250);
        await parkPointer();
        const live = await page.evaluate(MEASURE_ROW, { rowSelector: rowSel, index: i });
        const gotState = /selected|sel\b/.test(live.rowClass ?? "") || live.rowAttrs?.dataSelected === "true";
        selected = gotState;
        selectHow = `input[type=checkbox] in row ${i}${gotState ? "" : " (no state appeared)"}`;
        selectedIndex = i;
        rec.states["row-selected"] = { ...live, selectionMechanism: selectHow };
        break;
      } catch {
        /* next control */
      }
    }
    const roleCb = row.locator("[role=checkbox]").first();
    if ((await roleCb.count()) && (await roleCb.isVisible().catch(() => false))) {
      try {
        await roleCb.click({ timeout: 4000 });
        await page.waitForTimeout(250);
        await parkPointer();
        const live = await page.evaluate(MEASURE_ROW, { rowSelector: rowSel, index: i });
        const gotState = /selected|sel\b/.test(live.rowClass ?? "") || live.rowAttrs?.dataSelected === "true";
        if (gotState) {
          selected = true;
          selectHow = `[role=checkbox] in row ${i}`;
          selectedIndex = i;
          rec.states["row-selected"] = { ...live, selectionMechanism: selectHow };
          break;
        }
      } catch {
        /* next */
      }
    }
  }
  if (!selected) {
    try {
      await rowLoc().nth(focusIndex).locator("td").nth(1).click({ timeout: 4000, position: { x: 4, y: 4 } });
      await page.waitForTimeout(250);
      await parkPointer();
      const live = await page.evaluate(MEASURE_ROW, { rowSelector: rowSel, index: focusIndex });
      const gotState = /selected|sel\b/.test(live.rowClass ?? "") || live.rowAttrs?.dataSelected === "true";
      selected = gotState;
      selectedIndex = focusIndex;
      selectHow = gotState ? "row click (no checkbox on the surface)" : null;
      rec.states["row-selected"] = gotState
        ? { ...live, selectionMechanism: selectHow }
        : { ok: false, why: "no checkbox and a row click did not produce a selected state" };
    } catch (e) {
      rec.states["row-selected"] = { ok: false, why: `no checkbox; row click failed: ${e.message ?? e}` };
    }
  }

  // Does the surface even HAVE a selected state? A control can exist while the
  // state does not appear (a checkbox in a column the surface does not wire to
  // the row's own state), so this is recorded, not assumed.
  if (selected) {
    const s = rec.states["row-selected"];
    rec.states["row-selected"].stateVisible =
      s.row?.boxShadow !== "none" ||
      (s.row?.backgroundColor !== "rgba(0, 0, 0, 0)" && s.row?.backgroundColor !== "rgb(239, 238, 234)") ||
      (s.cells ?? []).some((c) => c.boxShadow !== "none") ||
      (s.cells ?? []).some((c) => c.backgroundColor !== "rgba(0, 0, 0, 0)");
    // The collision the pack defends against with `!important`: hover on top of
    // selected. Measured because a selected tint that hover replaces is a real
    // difference from the drawn pack, not a nicety.
    try {
      await rowLoc().nth(selectedIndex).hover({ timeout: 4000 });
      await page.waitForTimeout(160);
      await measure("row-selected-hovered", selectedIndex);
    } catch (e) {
      rec.states["row-selected-hovered"] = { ok: false, why: String(e.message ?? e) };
    }
    await parkPointer();
  }

  // ---- current: click the row body, away from its controls ---------------------
  try {
    const idx = selected ? selectedIndex : focusIndex;
    await rowLoc().nth(idx).locator("td").nth(2).click({ timeout: 4000, position: { x: 4, y: 4 } });
    await page.waitForTimeout(260);
    await parkPointer();
    await measure("row-current", idx);
    if (selected) rec.states["row-selected-and-current"] = await page.evaluate(MEASURE_ROW, { rowSelector: rowSel, index: selectedIndex });
  } catch (e) {
    rec.states["row-current"] = { ok: false, why: String(e.message ?? e) };
  }

  // ---- focus-visible -----------------------------------------------------------
  try {
    const idx = selected ? selectedIndex : focusIndex;
    const r = rowLoc().nth(idx);
    const tookRowFocus = await r.evaluate((el) => {
      const before = document.activeElement;
      el.focus();
      const took = document.activeElement === el;
      if (before && before.focus) before.focus();
      return took;
    });
    if (tookRowFocus) {
      await r.focus();
      await page.waitForTimeout(160);
      const m = await page.evaluate(MEASURE_ROW, { rowSelector: rowSel, index: idx });
      rec.states["row-focus-visible"] = { ...m, focusVisibleMatched: await r.evaluate((el) => el.matches(":focus-visible")) };
    } else {
      const ctl = r.locator("a,button,input,[tabindex]").first();
      if (await ctl.count()) {
        await ctl.focus();
        await page.waitForTimeout(160);
        const m = await page.evaluate(MEASURE_ROW, { rowSelector: rowSel, index: idx });
        rec.states["row-focus-visible"] = {
          ...m,
          focusTarget: "control-in-row",
          focusVisibleMatched: await ctl.evaluate((el) => el.matches(":focus-visible")),
          control: await ctl.evaluate((el) => window.__m.paint(el)),
          rowFocusable: false,
          why: "row is not focusable (no tabindex); focused the first control inside it instead",
        };
      } else {
        rec.states["row-focus-visible"] = { ok: false, why: "row is not focusable and holds no focusable control" };
      }
    }
  } catch (e) {
    rec.states["row-focus-visible"] = { ok: false, why: String(e.message ?? e) };
  }

  // ---- container edge ----------------------------------------------------------
  try {
    rec.states.containerEdge = await page.evaluate((sel) => {
      const m = window.__m;
      const rows = [...document.querySelectorAll(sel)].filter((r) => r.querySelector("td"));
      const row = rows[rows.length - 1] ?? rows[0];
      if (!row) return { ok: false, why: "no rows" };
      const cont = m.scrollContainer(row);
      const cr = m.rect(cont);
      const cells = [...row.querySelectorAll(":scope > td")].map((c, i) => ({ i, right: m.rect(c).right, w: m.rect(c).w }));
      return {
        ok: true,
        containerRect: cr,
        scrollWidth: cont.scrollWidth,
        clientWidth: cont.clientWidth,
        overflows: cont.scrollWidth > cont.clientWidth + 1,
        rowRight: m.rect(row).right,
        cells,
        cellsPastRight: cells.filter((c) => c.right > cr.right + 1).length,
        lastCellRight: cells.length ? cells[cells.length - 1].right : null,
      };
    }, rowSel);
  } catch (e) {
    rec.states.containerEdge = { ok: false, why: String(e.message ?? e) };
  }

  // ---- cell content clip -------------------------------------------------------
  // The row's box can sit inside its container while a cell still clips its own
  // content. With `overflow: hidden` and no `text-overflow: ellipsis`, the
  // reader gets a hard cut with no ellipsis to say so.
  try {
    rec.states.contentClip = await page.evaluate((sel) => {
      const rows = [...document.querySelectorAll(sel)].filter((r) => r.querySelector("td"));
      if (!rows.length) return { ok: false, why: "no rows" };
      const row = rows[Math.min(1, rows.length - 1)];
      const cells = [...row.querySelectorAll(":scope > td")];
      return {
        ok: true,
        cells: cells.map((c, i) => {
          const cs = getComputedStyle(c);
          return {
            i,
            clientWidth: c.clientWidth,
            scrollWidth: c.scrollWidth,
            clipped: c.scrollWidth > c.clientWidth + 1,
            overflowX: cs.overflowX,
            textOverflow: cs.textOverflow,
            text: (c.textContent ?? "").trim().slice(0, 50),
          };
        }),
        clippedCells: cells.filter((c) => c.scrollWidth > c.clientWidth + 1).length,
        clippedWithoutEllipsis: cells.filter(
          (c) => c.scrollWidth > c.clientWidth + 1 && getComputedStyle(c).textOverflow !== "ellipsis",
        ).length,
      };
    }, rowSel);
  } catch (e) {
    rec.states.contentClip = { ok: false, why: String(e.message ?? e) };
  }

  // ---- bulk bar -----------------------------------------------------------------
  try {
    rec.states.bulkBar = await page.evaluate(() => {
      const bar =
        document.querySelector("[data-testid='bulk-bar']") ??
        document.querySelector(".m-money-bulkbar") ??
        document.querySelector("[class*='bulk']");
      const headerRow = document.querySelector("thead tr");
      return {
        ok: true,
        present: !!bar,
        barClass: bar ? bar.className : null,
        barText: bar ? (bar.textContent ?? "").trim().slice(0, 140) : null,
        headerStillHasTh: headerRow ? headerRow.querySelectorAll("th").length > 0 : false,
      };
    });
  } catch (e) {
    rec.states.bulkBar = { ok: false, why: String(e.message ?? e) };
  }

  // ---- density ladder (mutate, then reload so the app re-reads it) --------------
  rec.states.density = {};
  for (const step of ["comfortable", "compact", "dense"]) {
    try {
      await page.evaluate((s) => localStorage.setItem("marula-density", s), step);
      await open();
      if (!(await rowSelector(page))) throw new Error("rows disappeared after reload");
      rec.states.density[step] = await page.evaluate((sel) => {
        const m = window.__m;
        const rows = [...document.querySelectorAll(sel)].filter((r) => r.querySelector("td"));
        const row = rows[Math.min(1, rows.length - 1)];
        const cell = row ? row.querySelector("td") : null;
        const inner = row ? row.querySelector("input,button,a") : null;
        return {
          ok: !!row,
          rootDensity: document.documentElement.getAttribute("data-density"),
          cellHeight: cell ? getComputedStyle(cell).height : null,
          rowHeight: row ? Math.round(row.getBoundingClientRect().height) : null,
          innerControl: inner ? { tag: inner.tagName, h: Math.round(inner.getBoundingClientRect().height) } : null,
          cellPx: cell ? getComputedStyle(cell).paddingLeft : null,
          rowVar: getComputedStyle(document.documentElement).getPropertyValue("--m-density-row").trim(),
          _rect: row ? m.rect(row) : null,
        };
      }, rowSel);
    } catch (e) {
      rec.states.density[step] = { ok: false, why: String(e.message ?? e) };
    }
  }
  await page.evaluate(() => localStorage.removeItem("marula-density"));
  await open();

  // ---- toolbar controls --------------------------------------------------------
  try {
    rec.states.toolbar = await page.evaluate(() => {
      const scope = document.querySelector("main") ?? document.body;
      const ctls = [...scope.querySelectorAll("button, [role=button], input, select")].filter((e) => e.offsetParent !== null);
      return {
        ok: true,
        count: ctls.length,
        disabled: ctls
          .filter((e) => e.disabled || e.getAttribute("aria-disabled") === "true")
          .map((e) => (e.textContent ?? "").trim().slice(0, 40) || e.tagName),
        controls: ctls.slice(0, 26).map((e) => ({
          tag: e.tagName,
          label: (e.textContent ?? "").trim().slice(0, 40),
          disabled: !!e.disabled,
          testid: e.getAttribute("data-testid"),
        })),
      };
    });
    const btn = page.locator("main button:visible, main [role=button]:visible").first();
    if (await btn.count()) {
      try {
        const beforePaint = await btn.evaluate((el) => window.__m.paint(el));
        await btn.hover({ timeout: 4000 });
        await page.waitForTimeout(140);
        const afterPaint = await btn.evaluate((el) => window.__m.paint(el));
        rec.states.toolbarHover = {
          ok: true,
          label: (await btn.textContent())?.trim().slice(0, 40) ?? null,
          before: beforePaint,
          after: afterPaint,
          changed: JSON.stringify(beforePaint) !== JSON.stringify(afterPaint),
        };
      } catch (e) {
        rec.states.toolbarHover = { ok: false, why: String(e.message ?? e) };
      }
      try {
        await btn.focus();
        rec.states.toolbarFocus = {
          ok: true,
          after: await btn.evaluate((el) => ({ ...window.__m.paint(el), focusVisible: el.matches(":focus-visible") })),
        };
      } catch (e) {
        rec.states.toolbarFocus = { ok: false, why: String(e.message ?? e) };
      }
    }
  } catch (e) {
    rec.states.toolbar = { ok: false, why: String(e.message ?? e) };
  }

  // ---- panels ------------------------------------------------------------------
  rec.states.panels = {};
  for (const p of [
    { name: "filter", testids: ["filter", "filter-button", "tickets-filter"], labels: ["Filter", "Add filter", "Filters"] },
    { name: "columns", testids: ["columns", "column-chooser"], labels: ["Columns"] },
    { name: "export", testids: ["export"], labels: ["Export"] },
  ]) {
    let how = null;
    for (const label of p.labels) {
      const b = page.getByRole("button", { name: new RegExp(`^${label}\\b`, "i") }).first();
      if ((await b.count()) && (await b.isVisible().catch(() => false))) {
        try {
          await b.click({ timeout: 3000 });
          how = `button:${label}`;
          break;
        } catch {
          /* next */
        }
      }
    }
    if (!how) {
      for (const t of p.testids) {
        const l = page.locator(`[data-testid="${t}"]`).first();
        if ((await l.count()) && (await l.isVisible().catch(() => false))) {
          try {
            await l.click({ timeout: 3000 });
            how = `testid:${t}`;
            break;
          } catch {
            /* next */
          }
        }
      }
    }
    if (!how) {
      rec.states.panels[p.name] = { ok: false, why: "no control for this panel was found on the surface" };
      continue;
    }
    await page.waitForTimeout(300);
    rec.states.panels[p.name] = await page.evaluate(
      (h) => {
        const m = window.__m;
        const pop =
          document.querySelector("[role=dialog]") ??
          document.querySelector("[role=menu]") ??
          document.querySelector("[data-radix-popper-content-wrapper]") ??
          document.querySelector("[class*='popover']");
        return {
          ok: true,
          opened: !!pop,
          how: h,
          tag: pop ? pop.tagName : null,
          className: pop ? pop.className : null,
          rect: pop ? m.rect(pop) : null,
          text: pop ? (pop.textContent ?? "").trim().slice(0, 200) : null,
        };
      },
      how,
    );
    await page.keyboard.press("Escape");
    await page.waitForTimeout(180);
  }

  // ---- empty -------------------------------------------------------------------
  try {
    await open();
    const search = page
      .locator(
        "main input[type=search], main input[placeholder*='Search' i], main input[placeholder*='Filter' i], main input[placeholder*='Find' i]",
      )
      .first();
    if (await search.count()) {
      await search.fill("zzz-nothing-matches-p47", { timeout: 5000 });
      await page.waitForTimeout(900);
      rec.states.empty = await page.evaluate((sel) => {
        const rows = [...document.querySelectorAll(sel)].filter((r) => r.querySelector("td"));
        const t = (document.body.innerText ?? "").toLowerCase();
        return {
          ok: true,
          drivenBy: "search input",
          rowCount: rows.length,
          saysNothing: /no |nothing|empty|no results|matches/.test(t.slice(0, 4000)),
          emptyTestid: document.querySelector("[data-testid*='empty']")?.getAttribute("data-testid") ?? null,
        };
      }, rowSel);
      await search.fill("");
      await page.waitForTimeout(400);
    } else {
      rec.states.empty = { ok: false, why: "no search input on the surface to drive an empty result" };
    }
  } catch (e) {
    rec.states.empty = { ok: false, why: String(e.message ?? e) };
  }

  // ---- loading, then error, on the route's own list request --------------------
  const src = rec.rowSource;
  if (!src) {
    rec.states.loading = { ok: false, why: "no JSON list response was observed on the route; nothing to delay" };
    rec.states.error = { ok: false, why: "no JSON list response was observed on the route; nothing to fail" };
  } else {
    // Match by pathname, so a reload with different query params still hits the
    // interceptor. A string route() would be an exact match and would silently
    // never fire.
    const sameEndpoint = (u) => {
      try {
        const a = new URL(typeof u === "string" ? u : u.toString());
        const b = new URL(src.url);
        return a.origin === b.origin && a.pathname === b.pathname;
      } catch {
        return false;
      }
    };
    try {
      await page.route(sameEndpoint, async (route) => {
        await new Promise((r) => setTimeout(r, 3000));
        // The navigation can abort the request while it is held; Playwright
        // then reports the route as already handled. Swallow it — the point of
        // this probe is what the page paints while the request is in flight.
        try {
          await route.continue();
        } catch {
          /* aborted by the navigation */
        }
      });
      const nav = page.goto(`${WEB}${surface.path}`, { waitUntil: "commit", timeout: 30_000 });
      await page.waitForTimeout(1100);
      await page.evaluate(installMeasure);
      rec.states.loading = await page.evaluate((sel) => {
        const rows = [...document.querySelectorAll(sel)].filter((r) => r.querySelector("td"));
        const skel = document.querySelector(".m-skel, [class*='skeleton'], [aria-busy='true'], [role=status]");
        return {
          ok: true,
          rowsRendered: rows.length,
          skeletonPresent: !!skel,
          skeletonClass: skel ? skel.className : null,
          text: (document.body.innerText ?? "").trim().slice(0, 160),
        };
      }, rowSel);
      await nav.catch(() => {});
      await page.unroute(sameEndpoint);
    } catch (e) {
      rec.states.loading = { ok: false, why: String(e.message ?? e) };
      await page.unroute(sameEndpoint).catch(() => {});
    }

    try {
      await page.route(sameEndpoint, (route) =>
        route.fulfill({ status: 500, contentType: "application/json", body: '{"error":"p47 probe"}' }).catch(() => {}),
      );
      await open();
      rec.states.error = await page.evaluate((sel) => {
        const rows = [...document.querySelectorAll(sel)].filter((r) => r.querySelector("td"));
        const alert = document.querySelector("[role=alert]");
        return {
          ok: true,
          rowsRendered: rows.length,
          alertText: alert ? (alert.textContent ?? "").trim().slice(0, 200) : null,
          bodyHead: (document.body.innerText ?? "").trim().slice(0, 200),
        };
      }, rowSel);
      await page.unroute(sameEndpoint);
    } catch (e) {
      rec.states.error = { ok: false, why: String(e.message ?? e) };
      await page.unroute(sameEndpoint).catch(() => {});
    }
  }

  return rec;
}

/* ------------------------------------------------------------------ *
 * Run
 * ------------------------------------------------------------------ */
console.log(`p47-rendered-states harness → ${WEB}`);
const { context, page, lists } = await newSession();
await staffLogin(page);
console.log("staff session established (owner@demo.local)");

const list = only ? SURFACES.filter((s) => only.includes(s.key)) : SURFACES;
for (const surface of list) {
  process.stdout.write(`  · ${surface.key} `);
  try {
    const rec = await probeSurface(page, surface, lists);
    findings.surfaces[surface.key] = rec;
    const st = rec.states ?? {};
    const both = st["row-selected-and-current"] ?? st["row-selected"];
    const flag =
      both?.ok && both.distinctCellBoxShadows?.length > 1 ? `  ← ${both.distinctCellBoxShadows.length} distinct cell shadows` : "";
    console.log(
      `ok (rows=${st["row-default"]?.rowCount ?? "?"}, sel=${st["row-selected"]?.ok ? "yes" : "no"}, cur=${st["row-current"]?.ok ? "yes" : "no"})${flag}`,
    );
    if (wantShots && SHOT_KEYS.includes(surface.key)) {
      // The probes above end on an injected 500, so reload to the surface's own
      // state before shooting; otherwise every shot would be of the probe, not
      // of the product.
      await page.goto(`${WEB}${surface.path}`, { waitUntil: "domcontentloaded", timeout: 60_000 }).catch(() => {});
      await page.waitForTimeout(1200);
      await page.screenshot({ path: resolve(SHOTS, `${surface.key}-state.png`) });
    }
  } catch (e) {
    findings.surfaces[surface.key] = { ok: false, error: String(e?.message ?? e) };
    console.log(`FAILED: ${e?.message ?? e}`);
  }
  flush();
}

const portalOnly = only ? !only.some((k) => SURFACES.some((s) => s.key === k)) : true;
if (portalOnly) {
  const ps = await newSession();
  try {
    await portalLogin(ps.page, "portal@acme.demo");
    console.log("portal session established (portal@acme.demo)");
    for (const surface of PORTAL_SURFACES) {
      process.stdout.write(`  · ${surface.key} `);
      try {
        findings.portal[surface.key] = await probeSurface(ps.page, surface, ps.lists);
        console.log("ok");
      } catch (e) {
        findings.portal[surface.key] = { ok: false, error: String(e?.message ?? e) };
        console.log(`FAILED: ${e?.message ?? e}`);
      }
      flush();
    }
  } catch (e) {
    findings.portal = { ok: false, error: `portal login failed: ${e?.message ?? e}` };
    console.log(`portal login FAILED: ${e?.message ?? e}`);
  }
  await ps.context.close();
}

findings.consoleErrors = consoleErrors;
findings.pageErrors = pageErrors;
writeFileSync(resolve(OUT, "rendered-states.json"), JSON.stringify(findings, null, 2) + "\n");
console.log(
  `\nwrote design/qa-p47-rendered-states/rendered-states.json (${consoleErrors.length} console errors, ${pageErrors.length} page errors)`,
);

await context.close();
await browser.close();
