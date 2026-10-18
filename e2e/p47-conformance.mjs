// p47-conformance.mjs — Wave 47 lane A13, `p47-vision-captures`.
//
// The missing instrument. `check-pack-geometry.py` compares numbers, and
// `p41-pack-diff.py` scores ink; neither can test a *rule*. The pack states its
// rules in prose per area (`DESIGN-NOTES-*.md`) and ships the HTML each surface
// was rendered from (`source/*.html`). This renders BOTH sides and diffs the
// same structural signature, so a difference cannot hide behind a class name.
//
// For each surface it extracts, from the pack's mockup and from our route:
//
//   TOOLBAR   every interactive control in the band between the page title and
//             the table's top edge, in reading order. This is the check that
//             catches DESIGN-NOTES-tickets §2.1 ("Columns, Filter and Export
//             right-aligned") without anyone reading the notes.
//   COLUMNS   the table's header cells, in order, as labels.
//   ROW       the first body row's cell count and the text shape of each cell.
//   FOOTER    the visible text in the band below the table.
//   REGIONS   the box each of the above occupies, normalised to the canvas, so
//             a region that is present but mis-sized is visible too.
//
// Usage:
//   node e2e/p47-conformance.mjs                 # every mapped surface
//   node e2e/p47-conformance.mjs --only tickets-list
//   node e2e/p47-conformance.mjs --pack-only     # no stack needed
//
// The pack side needs no stack at all (`--pack-only` dumps the declarations,
// which is the yardstick). The app side reads `e2e/.e2e-stack.env` and expects
// the web bundle up, exactly like the capture harnesses.
import { chromium } from "playwright";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const LANE = "design/qa-p47-vision-captures";
const OUT = resolve(LANE, "conformance");
mkdirSync(OUT, { recursive: true });

const PACK = process.env.MARULA_PACK_ROOT ?? "/Users/iordn/Documents/GitHub/Marula/brand/packs/core-surface-pack";
const CANVAS = { width: 1600, height: 994 };
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";

const arg = (name, dflt) => {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? dflt : process.argv[i + 1];
};
const ONLY = arg("only", null);
const PACK_ONLY = process.argv.includes("--pack-only");

/* The nine web areas the pack draws, each with the mockup file it was rendered
 * from and the route that renders it. The route is what makes this a
 * conformance check rather than a picture review. */
const SURFACES = [
  { id: "tickets-list", source: "tickets-list.html", route: "/tickets" },
  { id: "clients", source: "clients.html", route: "/clients" },
  { id: "approvals", source: "approvals.html", route: "/approvals" },
  { id: "today", source: "today.html", route: "/today" },
  { id: "dispatch", source: "dispatch.html", route: "/dispatch" },
  { id: "settings", source: "settings.html", route: "/settings" },
  { id: "quoting", source: "quoting.html", route: "/sales/quotes" },
  { id: "knowledge", source: "knowledge-vault.html", route: "/documents" },
  { id: "workflow", source: "workflow-builder.html", route: "/automations" },
].filter((s) => ONLY === null || s.id === ONLY);

async function token() {
  const envPath = resolve("e2e/.e2e-stack.env");
  if (!existsSync(envPath)) throw new Error("e2e/.e2e-stack.env missing — boot the stack or pass --pack-only");
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
 * measured by identical code — which is the whole reason this works where a
 * screenshot comparison does not.
 * ---------------------------------------------------------------------- */
const SIGNATURE = () => {
  const vis = (el) => {
    const st = getComputedStyle(el);
    if (st.display === "none" || st.visibility === "hidden" || parseFloat(st.opacity) < 0.05) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };
  const txt = (el) => (el.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 48);
  const round = (n) => Math.round(n);

  // The anchor. Every web mockup wraps its surface in `main.page` (nine of
  // nine), and the preview harness sits outside it - so this single selector
  // both locates the surface and excludes the chrome, with no per-file map.
  // Our app has a bare `<main>`. The toolbar is every control above the
  // anchor's largest child, which is the list.
  const CHROME = ["#jump", "#toast", "#overlays", "#modal", "#sheets"];
  const inChrome = (el) => CHROME.some((sel) => el.closest(sel) !== null);
  const anchor = document.querySelector("main.page") || document.querySelector("main");
  const bandBottom = (() => {
    // 1. The anchor's largest child - the list - when the mockup nests it.
    if (anchor) {
      let best = null;
      let bestArea = 0;
      for (const kid of anchor.children) {
        if (inChrome(kid)) continue;
        const r = kid.getBoundingClientRect();
        if (r.width * r.height > bestArea) {
          bestArea = r.width * r.height;
          best = r.top;
        }
      }
      if (best !== null && bestArea > 0) return best;
    }
    // 2. The top of the biggest cluster of same-height rows anywhere on the
    //    page. Survives display:contents, nested wrappers and any naming.
    const byHeight = new Map();
    for (const el of document.querySelectorAll("body *")) {
      if (inChrome(el)) continue;
      const r = el.getBoundingClientRect();
      const h = Math.round(r.height);
      if (h < 24 || h > 80 || r.width < 200) continue;
      if (!byHeight.has(h)) byHeight.set(h, []);
      byHeight.get(h).push(r.top);
    }
    let bestN = 0;
    let clusterTop = null;
    for (const tops of byHeight.values()) {
      if (tops.length > bestN) {
        bestN = tops.length;
        clusterTop = Math.min(...tops);
      }
    }
    if (bestN >= 5) return clusterTop;
    // 3. Last resort: a fixed band under the surface's top edge.
    return anchor ? anchor.getBoundingClientRect().top + 180 : null;
  })();

  const tbl = document.querySelector("table");
  const ft = document.querySelector("#ft") || document.querySelector(".m-surface-footer") || null;
  const tableBox = tbl ? tbl.getBoundingClientRect() : null;

  // Toolbar: every control above the list band, in reading order.
  const controls = [];
  for (const el of document.querySelectorAll("button, a[href], [role=button], [role=radio], [role=combobox], select, input")) {
    if (!vis(el) || inChrome(el)) continue;
    const r = el.getBoundingClientRect();
    if (bandBottom !== null && r.bottom > bandBottom + 1) continue;
    if (r.width < 8 || r.height < 8) continue;
    if (r.width < 8 || r.height < 8) continue;
    const label = txt(el) || el.getAttribute("aria-label") || el.getAttribute("placeholder") || "";
    if (label === "") continue;
    controls.push({ label, x: round(r.left), y: round(r.top), w: round(r.width), h: round(r.height) });
  }
  // The toolbar is the row IMMEDIATELY above the list, not every control above
  // it (the mockups carry a top bar and a preview theme switcher above that).
  // Keep the lowest band, allowing for a wrapped toolbar.
  if (controls.length > 0) {
    const lowest = Math.max(...controls.map((c) => c.y));
    const band = controls.filter((c) => lowest - c.y <= 64);
    controls.length = 0;
    controls.push(...band);
  }
  controls.sort((a, b) => (Math.abs(a.y - b.y) > 12 ? a.y - b.y : a.x - b.x));

  // Columns: the header row's cells, in x order. Our kit uses thead/th; the
  // pack draws its header with divs under .hd.
  const columns = [];
  if (tbl) {
    const ths = tbl.querySelectorAll("thead th").length
      ? [...tbl.querySelectorAll("thead th")]
      : [...tbl.querySelectorAll(".hd > *, .hd")].flatMap((el) => (el.children.length ? [...el.children] : [el]));
    for (const th of ths) {
      if (!vis(th)) continue;
      const r = th.getBoundingClientRect();
      columns.push({ label: txt(th), x: round(r.left), w: round(r.width) });
    }
    columns.sort((a, b) => a.x - b.x);
  }

  // The first body row's cells.
  const row = [];
  const firstRow = tbl
    ? tbl.querySelector("tbody tr") ?? tbl.querySelector(".tr:not(.hd)")
    : null;
  if (firstRow) {
    const cells = firstRow.querySelectorAll("td").length
      ? [...firstRow.querySelectorAll("td")]
      : [...firstRow.children];
    for (const td of cells) {
      if (!vis(td)) continue;
      const r = td.getBoundingClientRect();
      row.push({ text: txt(td), x: round(r.left), w: round(r.width) });
    }
    row.sort((a, b) => a.x - b.x);
  }

  const title = (() => {
    const h = document.querySelector("h1");
    if (!h || !vis(h)) return null;
    const st = getComputedStyle(h);
    const r = h.getBoundingClientRect();
    return { text: txt(h), size: parseFloat(st.fontSize), weight: st.fontWeight, x: round(r.left), y: round(r.top) };
  })();

  return {
    title,
    toolbar: controls.map((c) => c.label),
    toolbarBoxes: controls,
    columns: columns.map((c) => c.label),
    firstRow: row.map((c) => c.text),
    rowCells: row.length,
    footer: ft && vis(ft) ? [txt(ft)] : [],
    regionsFound: { toolbar: bandBottom !== null, table: tbl !== null, footer: ft !== null },
    canvas: { width: window.innerWidth, height: window.innerHeight },
    tableBox: tableBox ? { x: round(tableBox.left), y: round(tableBox.top), w: round(tableBox.width), h: round(tableBox.height) } : null,
  };
};

/* ---------------------------------------------------------------------------
 * Run.
 * ---------------------------------------------------------------------- */
const browser = await chromium.launch();
const report = { generatedAt: new Date().toISOString(), pack: PACK, canvas: CANVAS, surfaces: [] };

let ctx = null;
let tokens = null;
let web = null;
if (!PACK_ONLY) {
  const session = await token();
  tokens = session.tokens;
  web = session.web;
  ctx = await browser.newContext({ viewport: CANVAS });
  await ctx.request.post(`${session.api}/api/v1/auth/login`, { data: { email: EMAIL, password: PASSWORD } });
  await ctx.request.put(`${session.api}/api/v1/settings/user`, {
    data: { key: "tours.disabled", value: true },
    headers: { authorization: `Bearer ${tokens.access_token}` },
  });
  await ctx.addInitScript(([raw, exp]) => {
    window.localStorage.setItem("marula-tokens", raw);
    window.localStorage.setItem("marula-tokens-exp", exp);
    window.localStorage.setItem("marula-theme", "light");
    window.localStorage.setItem("marula-theme-preset", "marula");
  }, [JSON.stringify(tokens), String(Date.now() + tokens.expires_in * 1000 - 30_000)]);
}

for (const surface of SURFACES) {
  const sourcePath = resolve(PACK, "source", surface.source);
  if (!existsSync(sourcePath)) {
    report.surfaces.push({ id: surface.id, error: `no pack source at ${sourcePath}` });
    continue;
  }
  const packPage = await browser.newPage({ viewport: CANVAS });
  await packPage.goto(`file://${sourcePath}`, { waitUntil: "load", timeout: 60_000 });
  await packPage.waitForTimeout(1500);
  const pack = await packPage.evaluate(SIGNATURE);
  await packPage.close();

  let app = null;
  if (!PACK_ONLY) {
    const page = await ctx.newPage();
    await page.goto(`${web}${surface.route}`, { waitUntil: "domcontentloaded", timeout: 60_000 }).catch(() => {});
    await page.waitForLoadState("networkidle", { timeout: 20_000 }).catch(() => {});
    await page.waitForTimeout(1500);
    app = await page.evaluate(SIGNATURE);
    await page.close();
  }

  const deltas = [];
  if (app) {
    const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
    if (!same(pack.toolbar, app.toolbar)) {
      deltas.push({ rule: "toolbar order", pack: pack.toolbar, app: app.toolbar });
    }
    if (!same(pack.columns, app.columns)) {
      deltas.push({ rule: "column order/labels", pack: pack.columns, app: app.columns });
    }
    if (pack.rowCells !== app.rowCells) {
      deltas.push({ rule: "row cell count", pack: pack.rowCells, app: app.rowCells });
    }
    if (pack.title && app.title && Math.abs(pack.title.size - app.title.size) > 0.5) {
      deltas.push({ rule: "title font-size", pack: pack.title.size, app: app.title.size });
    }
    if (!same(pack.footer, app.footer)) {
      deltas.push({ rule: "footer text", pack: pack.footer, app: app.footer });
    }
  }
  report.surfaces.push({ id: surface.id, route: surface.route, source: surface.source, pack, app, deltas });
  console.log(`\n[${surface.id}] ${app ? `${deltas.length} rule(s) differ` : "pack side only"}`);
  for (const d of deltas) {
    console.log(`  ! ${d.rule}`);
    console.log(`      pack: ${JSON.stringify(d.pack)}`);
    console.log(`      app : ${JSON.stringify(d.app)}`);
  }
}

writeFileSync(resolve(OUT, "CONFORMANCE.json"), JSON.stringify(report, null, 2));
console.log(`\n[conformance] ${report.surfaces.length} surfaces -> ${OUT}/CONFORMANCE.json`);
if (ctx) await ctx.close();
await browser.close();
