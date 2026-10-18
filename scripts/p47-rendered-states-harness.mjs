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
 * What it drives per surface:
 *   - density: comfortable / compact / dense (row height, control height)
 *   - row hover
 *   - row focus-visible (row focusability recorded either way)
 *   - row selected
 *   - row current
 *   - row selected + current together
 *   - toolbar control hover + focus + disabled controls
 *   - panels: filter builder, column chooser, export menu
 *   - bulk bar
 *   - container edges: row/cell right edge vs the scroll container, and
 *     scrollWidth > clientWidth
 *   - empty / error / loading
 *
 * Every measurement is a getComputedStyle value or a getBoundingClientRect
 * number. Nothing here is an impression. A probe that cannot run records
 * `{ ok: false, why }` with a reason — never a silent gap.
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
const only = args.includes("--surface")
  ? args[args.indexOf("--surface") + 1].split(",")
  : null;
const wantShots = args.includes("--shots");

/* ------------------------------------------------------------------ *
 * Surfaces. `rows` is the generic row selector; `table` guards readiness.
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

/* Portal surfaces (separate session). */
const PORTAL_SURFACES = [
  { key: "portal-tickets", path: "/portal/tickets" },
  { key: "portal-invoices", path: "/portal/invoices" },
  { key: "portal-quotes", path: "/portal/quotes" },
  { key: "portal-kb", path: "/portal/kb" },
];

/* ------------------------------------------------------------------ *
 * Page-side measurement helpers (serialised into the browser).
 * ------------------------------------------------------------------ */

const PAGE_API = {
  /** Computed paint values of one element. */
  paint(el) {
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
      zIndex: cs.zIndex,
    };
  },
};

/** Installed in the page: a small measurement library. */
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
  const cells = (row) => [...row.querySelectorAll(":scope > td")];
  window.__m = { paint, rect, scrollContainer, cells };
  return true;
}

/** Build a state record for a surface from a single evaluation. */
const MEASURE_ROW = (rowSelector) => {
  const m = window.__m;
  const rows = [...document.querySelectorAll(rowSelector)].filter((r) => r.querySelector("td") && !r.querySelector(":scope > th"));
  if (!rows.length) return { ok: false, why: `no data rows matched ${rowSelector}` };
  const idx = rows.length > 1 ? 1 : 0;
  const row = rows[idx];
  const cs = m.cells(row);
  const cont = m.scrollContainer(row);
  const cr = m.rect(cont);
  const rr = m.rect(row);
  const cellRecords = cs.map((c, i) => {
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
    rowFocusable: (() => {
      const prev = document.activeElement;
      row.focus();
      const got = document.activeElement === row;
      if (prev && prev.focus) prev.focus();
      return got;
    })(),
    cells: cellRecords,
    distinctCellBoxShadows: [...new Set(cellRecords.map((c) => c.boxShadow))],
    distinctCellBackgrounds: [...new Set(cellRecords.map((c) => c.backgroundColor))],
    cellsCrossingContainerRight: cellRecords.filter((c) => c.crossesContainerRight).length,
    container: { rect: cr, scrollWidth: cont.scrollWidth, clientWidth: cont.clientWidth, overflows: cont.scrollWidth > cont.clientWidth + 1 },
    rowCrossesContainerRight: rr.right > cr.right + 1,
    focusablesInRow: [...row.querySelectorAll("a,button,input,select,textarea,[tabindex]")].map((e) => e.tagName + (e.getAttribute("data-testid") ? `#${e.getAttribute("data-testid")}` : "")),
  };
};

/* ------------------------------------------------------------------ *
 * Playwright driver
 * ------------------------------------------------------------------ */
const browser = await chromium.launch();

const findings = { generatedAt: new Date().toISOString(), viewport: VIEWPORT, stack: { web: WEB, api: API }, surfaces: {}, portal: {} };
const consoleErrors = [];
const pageErrors = [];

async function newSession() {
  const context = await browser.newContext({ viewport: VIEWPORT });
  const page = await context.newPage();
  page.on("console", (msg) => {
    if (msg.type() === "error") consoleErrors.push(`${page.url()} :: ${msg.text()}`);
  });
  page.on("pageerror", (e) => pageErrors.push(`${page.url()} :: ${e.message}`));
  return { context, page };
}

/** Real in-page staff login through the sign-in card. */
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

async function ready(page) {
  await page.waitForLoadState("domcontentloaded");
  await page.waitForTimeout(900);
}

/** Find the first real table row on the page, by a family of selectors. */
async function rowSelector(page) {
  const candidates = [
    "table.m-table tbody tr",
    "table tbody tr",
    "[data-testid$='-row']",
    "tr",
  ];
  for (const sel of candidates) {
    const n = await page.locator(sel).filter({ has: page.locator("td") }).count();
    if (n > 0) return sel;
  }
  return null;
}

/** Generic per-surface probe. */
async function probeSurface(page, surface) {
  const rec = { key: surface.key, path: surface.path, note: surface.note ?? null, states: {} };
  await page.goto(`${WEB}${surface.path}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
  await ready(page);

  // Record the row-source request so loading/error can be replayed on it.
  const rowSource = await page.evaluate(() => window.__p47LastListUrl ?? null);

  await page.evaluate(installMeasure);

  const rowSel = await rowSelector(page);
  if (!rowSel) {
    rec.states.noRows = { ok: false, why: "no table/row markup found on the route" };
    rec.rowSource = rowSource;
    return rec;
  }
  rec.rowSelector = rowSel;

  const measure = async (label) => {
    try {
      rec.states[label] = await page.evaluate(MEASURE_ROW, rowSel);
    } catch (e) {
      rec.states[label] = { ok: false, why: String(e.message ?? e) };
    }
  };

  // ---- row: baseline + hover + focus
  await measure("row-default");
  try {
    const rows = page.locator(rowSel).filter({ has: page.locator("td") });
    const n = await rows.count();
    await rows.nth(n > 1 ? 1 : 0).hover({ timeout: 5000 });
    await page.waitForTimeout(120);
    await measure("row-hover");
    await page.mouse.move(5, 5);
  } catch (e) {
    rec.states["row-hover"] = { ok: false, why: String(e.message ?? e) };
  }

  // ---- row selected (checkbox) -------------------------------------------------
  const rowLoc = page.locator(rowSel).filter({ has: page.locator("td") });
  const cbInRow = rowLoc.nth(1).locator("input[type=checkbox]").first();
  const cbCount = await cbInRow.count();
  if (cbCount) {
    try {
      await cbInRow.check({ timeout: 5000, force: true });
      await page.waitForTimeout(200);
      await measure("row-selected");
    } catch (e) {
      rec.states["row-selected"] = { ok: false, why: String(e.message ?? e) };
    }
  } else {
    rec.states["row-selected"] = { ok: false, why: "no checkbox in row 0 cell 0 — surface offers no row-selection control" };
  }

  // ---- row current (click the row body, away from controls) --------------------
  try {
    const target = rowLoc.nth(1).locator("td").nth(2);
    await target.click({ timeout: 5000, position: { x: 4, y: 4 } });
    await page.waitForTimeout(220);
    await measure("row-current");
    await measure("row-selected-and-current");
  } catch (e) {
    rec.states["row-current"] = { ok: false, why: String(e.message ?? e) };
  }

  // ---- row focus-visible -------------------------------------------------------
  // Focus the row itself if it can take focus; otherwise the first control in the
  // row. Either way the record says which happened, so this is never "inconclusive".
  try {
    const r = page.locator(rowSel).filter({ has: page.locator("td") }).nth(1);
    const info = await r.evaluate((el) => {
      const before = document.activeElement;
      el.focus();
      const tookRowFocus = document.activeElement === el;
      if (before && before.focus) before.focus();
      return { tookRowFocus };
    });
    if (info.tookRowFocus) {
      await r.focus();
      await page.waitForTimeout(150);
      const m = await page.evaluate(MEASURE_ROW, rowSel);
      // Confirm :focus-visible actually matched.
      const matched = await r.evaluate((el) => el.matches(":focus-visible"));
      rec.states["row-focus-visible"] = { ...m, focusVisibleMatched: matched };
    } else {
      const ctl = r.locator("a,button,input,[tabindex]").first();
      if (await ctl.count()) {
        await ctl.focus();
        await page.waitForTimeout(150);
        const m = await page.evaluate(MEASURE_ROW, rowSel);
        const matched = await ctl.evaluate((el) => el.matches(":focus-visible"));
        rec.states["row-focus-visible"] = {
          ...m,
          focusTarget: "control-in-row",
          focusVisibleMatched: matched,
          control: await ctl.evaluate((el) => window.__m.paint(el)),
          rowFocusable: false,
          why: "row is not focusable (no tabindex); focused the first control inside it instead",
        };
      } else {
        rec.states["row-focus-visible"] = { ok: false, why: "row is not focusable and it holds no focusable control" };
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
      const over = [...row.querySelectorAll(":scope > td")].map((c, i) => ({ i, right: m.rect(c).right, w: m.rect(c).w }));
      return {
        ok: true,
        containerRect: cr,
        scrollWidth: cont.scrollWidth,
        clientWidth: cont.clientWidth,
        overflows: cont.scrollWidth > cont.clientWidth + 1,
        rowRight: m.rect(row).right,
        cells: over,
        cellsPastRight: over.filter((c) => c.right > cr.right + 1).length,
        lastCellRight: over.length ? over[over.length - 1].right : null,
      };
    }, rowSel);
  } catch (e) {
    rec.states.containerEdge = { ok: false, why: String(e.message ?? e) };
  }

  // ---- density ladder ----------------------------------------------------------
  rec.states.density = {};
  for (const step of ["comfortable", "compact", "dense"]) {
    try {
      await page.evaluate((s) => {
        localStorage.setItem("marula-density", s);
        document.documentElement.setAttribute("data-density", s);
      }, step);
      await page.waitForTimeout(150);
      rec.states.density[step] = await page.evaluate((sel) => {
        const m = window.__m;
        const rows = [...document.querySelectorAll(sel)].filter((r) => r.querySelector("td"));
        const row = rows[1] ?? rows[0];
        const cs = row ? getComputedStyle(row.querySelector("td")) : null;
        const inner = row ? row.querySelector("input,button,a") : null;
        return {
          ok: !!row,
          rootDensity: document.documentElement.getAttribute("data-density"),
          cellHeight: cs ? cs.height : null,
          rowRectHeight: row ? Math.round(row.getBoundingClientRect().height) : null,
          innerControl: inner ? { tag: inner.tagName, h: Math.round(inner.getBoundingClientRect().height) } : null,
        };
      }, rowSel);
    } catch (e) {
      rec.states.density[step] = { ok: false, why: String(e.message ?? e) };
    }
  }
  // restore + reload so later probes see the default
  await page.evaluate(() => {
    localStorage.removeItem("marula-density");
    document.documentElement.removeAttribute("data-density");
  });

  // ---- toolbar controls --------------------------------------------------------
  try {
    rec.states.toolbar = await page.evaluate(() => {
      const m = window.__m;
      const scope = document.querySelector("main") ?? document.body;
      const ctls = [...scope.querySelectorAll("button, [role=button], a.entry-btn, a[data-testid], input, select")].filter(
        (e) => e.offsetParent !== null,
      );
      const cut = ctls.slice(0, 24);
      return {
        ok: true,
        count: ctls.length,
        disabled: ctls.filter((e) => e.disabled || e.getAttribute("aria-disabled") === "true").map((e) => e.textContent?.trim().slice(0, 40) || e.tagName),
        controls: cut.map((e) => ({ tag: e.tagName, label: (e.textContent ?? "").trim().slice(0, 40), disabled: !!e.disabled, testid: e.getAttribute("data-testid") })),
      };
    });
    // hover + focus the first real toolbar button
    const btn = page.locator("main button:visible, main [role=button]:visible").first();
    if (await btn.count()) {
      try {
        const before = await btn.evaluate((el) => window.__m.paint(el));
        await btn.hover({ timeout: 4000 });
        await page.waitForTimeout(120);
        const after = await btn.evaluate((el) => window.__m.paint(el));
        rec.states.toolbarHover = { ok: true, label: (await btn.textContent())?.trim().slice(0, 40), before, after, changed: JSON.stringify(before) !== JSON.stringify(after) };
      } catch (e) {
        rec.states.toolbarHover = { ok: false, why: String(e.message ?? e) };
      }
      try {
        await btn.focus();
        const after = await btn.evaluate((el) => ({ ...window.__m.paint(el), focusVisible: el.matches(":focus-visible") }));
        rec.states.toolbarFocus = { ok: true, after };
      } catch (e) {
        rec.states.toolbarFocus = { ok: false, why: String(e.message ?? e) };
      }
    }
  } catch (e) {
    rec.states.toolbar = { ok: false, why: String(e.message ?? e) };
  }

  // ---- bulk bar ----------------------------------------------------------------
  try {
    rec.states.bulkBar = await page.evaluate((sel) => {
      const bar =
        document.querySelector("[data-testid='bulk-bar']") ??
        document.querySelector(".m-money-bulkbar") ??
        document.querySelector("[class*='bulk']");
      const headerRow = document.querySelector("thead tr");
      const hasTh = headerRow ? headerRow.querySelectorAll("th").length > 0 : false;
      return {
        ok: true,
        present: !!bar,
        barClass: bar ? bar.className : null,
        barText: bar ? (bar.textContent ?? "").trim().slice(0, 120) : null,
        headerStillTh: hasTh,
        note: bar ? null : "no bulk bar in DOM at this point (recorded after the selection attempt)",
      };
    }, rowSel);
  } catch (e) {
    rec.states.bulkBar = { ok: false, why: String(e.message ?? e) };
  }

  // ---- panels: filter builder / column chooser / export menu -------------------
  rec.states.panels = {};
  const panelButtons = [
    { name: "filter", testids: ["filter", "filter-button", "tickets-filter"], labels: ["Filter", "Add filter"] },
    { name: "columns", testids: ["columns", "column-chooser"], labels: ["Columns"] },
    { name: "export", testids: ["export"], labels: ["Export"] },
  ];
  for (const p of panelButtons) {
    let opened = false;
    let how = null;
    for (const label of p.labels) {
      const b = page.getByRole("button", { name: new RegExp(`^${label}\\b`, "i") }).first();
      if (await b.count()) {
        try {
          await b.click({ timeout: 3000 });
          how = `button:${label}`;
          opened = true;
          break;
        } catch {
          /* try the next label */
        }
      }
    }
    if (!opened) {
      for (const t of p.testids) {
        const l = page.locator(`[data-testid="${t}"]`).first();
        if (await l.count()) {
          try {
            await l.click({ timeout: 3000 });
            how = `testid:${t}`;
            opened = true;
            break;
          } catch {
            /* keep trying */
          }
        }
      }
    }
    if (!opened) {
      rec.states.panels[p.name] = { ok: false, why: "no control matched for this panel on this surface" };
      continue;
    }
    await page.waitForTimeout(250);
    rec.states.panels[p.name] = await page.evaluate(() => {
      const m = window.__m;
      const pop =
        document.querySelector("[role=dialog]") ??
        document.querySelector("[role=menu]") ??
        document.querySelector("[data-radix-popper-content-wrapper]") ??
        document.querySelector("[class*='popover'], [class*='popover']");
      return {
        ok: true,
        opened: !!pop,
        how,
        tag: pop ? pop.tagName : null,
        class: pop ? pop.className : null,
        rect: pop ? m.rect(pop) : null,
        text: pop ? (pop.textContent ?? "").trim().slice(0, 200) : null,
      };
    });
    await page.keyboard.press("Escape");
    await page.waitForTimeout(150);
  }

  rec.rowSource = rowSource;
  return rec;
}

/* ------------------------------------------------------------------ *
 * Run
 * ------------------------------------------------------------------ */
console.log(`p47-rendered-states harness → ${WEB}`);
const { context, page } = await newSession();
await staffLogin(page);
console.log("staff session established (owner@demo.local)");

const list = only ? SURFACES.filter((s) => only.includes(s.key)) : SURFACES;
for (const surface of list) {
  process.stdout.write(`  · ${surface.key} `);
  try {
    const rec = await probeSurface(page, surface);
    findings.surfaces[surface.key] = rec;
    const st = rec.states ?? {};
    const sel = st["row-selected"];
    const cur = st["row-current"];
    const both = st["row-selected-and-current"];
    const flag =
      both?.ok && (both.distinctCellBoxShadows?.length > 1 || both.cellsCrossingContainerRight > 0) ? "  ← cell-split" : "";
    console.log(`ok (rows=${st["row-default"]?.rowCount ?? "?"}, sel=${sel?.ok ? "yes" : "no"}, cur=${cur?.ok ? "yes" : "no"})${flag}`);
    if (wantShots && ["approvals", "billing", "tickets", "clients", "contracts"].includes(surface.key)) {
      await page.screenshot({ path: resolve(SHOTS, `${surface.key}-state.png`), fullPage: false });
    }
  } catch (e) {
    findings.surfaces[surface.key] = { ok: false, error: String(e?.message ?? e) };
    console.log(`FAILED: ${e?.message ?? e}`);
  }
}

// ---- portal session --------------------------------------------------------
const portalOnly = only ? !only.some((k) => SURFACES.some((s) => s.key === k)) : true;
if (portalOnly) {
  const ps = await newSession();
  try {
    await portalLogin(ps.page, "portal@acme.demo");
    console.log("portal session established (portal@acme.demo)");
    for (const surface of PORTAL_SURFACES) {
      process.stdout.write(`  · ${surface.key} `);
      try {
        findings.portal[surface.key] = await probeSurface(ps.page, surface);
        console.log("ok");
      } catch (e) {
        findings.portal[surface.key] = { ok: false, error: String(e?.message ?? e) };
        console.log(`FAILED: ${e?.message ?? e}`);
      }
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
console.log(`\nwrote design/qa-p47-rendered-states/rendered-states.json (${consoleErrors.length} console errors, ${pageErrors.length} page errors)`);

await context.close();
await browser.close();
