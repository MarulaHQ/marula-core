#!/usr/bin/env node
/**
 * p47-review-table — turn rendered-states.json into the REVIEW.md matrix.
 *
 * Deterministic: it reads only measured values and emits a markdown table plus
 * a defect list. It never phrases an impression, and a state that was not
 * driven prints as `not driven` with the reason the harness recorded.
 *
 * Usage: node scripts/p47-review-table.mjs            # prints markdown
 *        node scripts/p47-review-table.mjs --json      # summary object
 */

import { readFileSync, existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..");
const SRC = resolve(ROOT, "design/qa-p47-rendered-states/rendered-states.json");
if (!existsSync(SRC)) throw new Error(`${SRC} missing — run the harness first`);
const data = JSON.parse(readFileSync(SRC, "utf8"));

/** True when a cell paints a shadow, ring or bar of its own. */
const cellsWithShadow = (row) => (row?.cells ?? []).filter((c) => c.boxShadow && c.boxShadow !== "none").length;

/** The state that carries the most modifiers on this surface. */
function worstState(states) {
  for (const key of ["row-selected-and-current", "row-selected", "row-current", "row-hover", "row-focus-visible", "row-default"]) {
    const s = states?.[key];
    if (s?.ok) return { key, s };
  }
  return null;
}

const yn = (s) => (!s ? "not driven" : s.ok ? "yes" : "none");

const coverageCols = [
  ["density", (st) => (st.density ? Object.values(st.density).filter((d) => d?.ok).length + "/3" : "not driven")],
  ["hover", (st) => yn(st["row-hover"])],
  ["focus", (st) => yn(st["row-focus-visible"])],
  ["selected", (st) => yn(st["row-selected"])],
  ["sel+hover", (st) => yn(st["row-selected-hovered"])],
  ["current", (st) => yn(st["row-current"])],
  ["both", (st) => yn(st["row-selected-and-current"])],
  ["edge", (st) => yn(st.containerEdge)],
  ["clip", (st) => (st.contentClip?.ok ? `${st.contentClip.clippedCells} clipped` : "not driven")],
  ["toolbar", (st) => yn(st.toolbar)],
  ["panels", (st) => (st.panels ? `${Object.values(st.panels).filter((p) => p?.opened).length}/3` : "not driven")],
  ["empty", (st) => (st.empty?.ok ? (st.empty.rowCount === 0 ? "empty reached" : "filtered, rows remain") : "none")],
  ["error", (st) => yn(st.error)],
  ["loading", (st) => yn(st.loading)],
];

const surfaces = { ...data.surfaces, ...data.portal };
const rows = [];
const defects = [];
const selTreatments = [];
const noCursor = [];
for (const [key, rec] of Object.entries(surfaces)) {
  const st = rec?.states ?? {};
  const base = st["row-default"];
  const w = worstState(st);
  const cellShadows = w ? cellsWithShadow(w.s) : 0;
  const cellShadowValues = w ? [...new Set((w.s.cells ?? []).filter((c) => c.boxShadow !== "none").map((c) => c.boxShadow))] : [];
  const bgValues = w ? [...new Set((w.s.cells ?? []).map((c) => c.backgroundColor))] : [];
  const rowHasPaint = w ? w.s.row?.boxShadow && w.s.row.boxShadow !== "none" : false;
  const rowBg = w ? w.s.row?.backgroundColor : null;

  rows.push({
    key,
    path: rec?.path ?? "",
    rowCount: base?.ok ? base.rowCount : rec?.states?.noRows ? 0 : "?",
    rowClass: base?.ok ? base.rowClass : "",
    rowPoly: base?.ok ? base.cellCount : null,
    worst: w?.key ?? null,
    cellShadows,
    distinctCellShadows: cellShadowValues.length,
    distinctCellBgs: bgValues.length,
    rowHasPaint,
    rowBg,
    cellsCrossing: w?.s.cellsCrossingContainerRight ?? null,
    coverage: Object.fromEntries(coverageCols.map(([n, f]) => [n, f(st)])),
    notes: st["row-selected"]?.why ? [st["row-selected"].why] : [],
  });

  // Defect rule: a row state that paints more than one cell, or a cell that
  // crosses the container, is the class this lane exists to measure.
  if (w && (cellShadows > 1 || (w.s.cellsCrossingContainerRight ?? 0) > 0)) {
    defects.push({
      surface: key,
      path: rec.path,
      state: w.key,
      cellShadows,
      distinctCellShadows: cellShadowValues.length,
      rowHasPaint,
      cellsCrossing: w.s.cellsCrossingContainerRight ?? 0,
      sample: cellShadowValues.slice(0, 3),
      rowBoxShadow: w.s.row?.boxShadow ?? null,
      cells: w.s.cellCount,
    });
  }

  // Selection rules, each read from the measured values.
  const sel = st["row-selected"];
  const selHover = st["row-selected-hovered"];
  if (sel?.ok) {
    const cellsWithBg = (sel.cells ?? []).filter((c) => c.backgroundColor !== "rgba(0, 0, 0, 0)").length;
    const cellsWithRing = (sel.cells ?? []).filter((c) => c.boxShadow !== "none").length;
    const rowPainted = (sel.row?.boxShadow ?? "none") !== "none" || (sel.row?.backgroundColor ?? "rgba(0, 0, 0, 0)") !== "rgba(0, 0, 0, 0)";
    selTreatments.push({
      surface: key,
      path: rec.path,
      mechanism: sel.selectionMechanism ?? null,
      rowClass: sel.rowClass,
      dataSelected: sel.rowAttrs?.dataSelected ?? null,
      rowShadow: sel.row?.boxShadow ?? null,
      rowBg: sel.row?.backgroundColor ?? null,
      cellsPaintedBg: cellsWithBg,
      cellsPaintedRing: cellsWithRing,
      cellCount: sel.cellCount,
      rowPainted,
      stateVisible: sel.stateVisible ?? null,
      hoverReplacesBg:
        selHover?.ok && (selHover.row?.backgroundColor ?? null) !== (sel.row?.backgroundColor ?? null)
          ? { selected: sel.row?.backgroundColor, hovered: selHover.row?.backgroundColor }
          : null,
    });
    if (cellsWithRing > 1) {
      defects.push({
        kind: "row-state-on-cells",
        surface: key,
        path: rec.path,
        detail: `${cellsWithRing} of ${sel.cellCount} cells carry their own box-shadow in the selected state while the row itself carries ${(sel.row?.boxShadow ?? "none") === "none" ? "none" : "one"}`,
      });
    }
    if (cellsWithBg > 1) {
      defects.push({
        kind: "row-fill-on-cells",
        surface: key,
        path: rec.path,
        detail: `${cellsWithBg} of ${sel.cellCount} cells carry their own background in the selected state`,
      });
    }
    if (selHover?.ok && (selHover.row?.backgroundColor ?? null) !== (sel.row?.backgroundColor ?? null)) {
      defects.push({
        kind: "hover-replaces-selected-fill",
        surface: key,
        path: rec.path,
        detail: `selected row background ${sel.row?.backgroundColor}, hovered ${selHover.row?.backgroundColor}`,
      });
    }
  }
  const clip = st.contentClip;
  if (clip?.ok && clip.clippedCells > 0) {
    defects.push({
      kind: "cell-content-clipped",
      surface: key,
      path: rec.path,
      detail: `${clip.clippedCells} cell(s) clip their content; ${clip.clippedWithoutEllipsis} with no ellipsis`,
      cells: clip.cells.filter((c) => c.clipped),
    });
  }

  // Does anything at all render for the cursor row? Compared against the
  // surface's own selected baseline, so a selection that happens to be on the
  // same row is not mistaken for a cursor treatment.
  const paintKey = (s) =>
    JSON.stringify({
      rs: s.row?.boxShadow ?? null,
      rb: s.row?.backgroundColor ?? null,
      cs: (s.cells ?? []).map((c) => `${c.boxShadow}|${c.backgroundColor}|${c.outline}`).join(","),
    });
  const baseline = st["row-selected"]?.ok ? st["row-selected"] : st["row-default"];
  const cur = st["row-current"];
  const d = st["row-default"];
  const alreadyCurrent =
    /row-current|is-current/.test(d?.rowClass ?? "") ||
    d?.rowAttrs?.ariaCurrent === "true" ||
    d?.rowAttrs?.dataCurrent === "true" ||
    (d?.cells ?? []).some((c) => c.boxShadow !== "none");
  noCursor.push({
    surface: key,
    path: rec.path,
    alreadyCurrent: alreadyCurrent ? "yes" : "no",
    defaultCellsPainted: d?.ok ? (d.cells ?? []).filter((c) => c.boxShadow !== "none").length : null,
    defaultCellCount: d?.ok ? d.cellCount : null,
    rendered:
      cur?.ok && baseline?.ok
        ? paintKey(cur) !== paintKey(baseline)
          ? "yes"
          : "nothing changed"
        : cur?.ok
          ? "measured, no baseline"
          : "not driven",
  });
}

if (process.argv.includes("--json")) {
  console.log(JSON.stringify({ surfaces: rows, defects }, null, 2));
  process.exit(0);
}

// --detail <surface>: dump every measured state for one surface, so REVIEW.md
// can quote the computed strings instead of paraphrasing them.
if (process.argv.includes("--detail")) {
  const key = process.argv[process.argv.indexOf("--detail") + 1];
  const rec = surfaces[key];
  if (!rec) {
    console.error(`no surface "${key}"; have: ${Object.keys(surfaces).join(", ")}`);
    process.exit(2);
  }
  console.log(`# ${key}  ${rec.path}`);
  for (const [state, v] of Object.entries(rec.states ?? {})) {
    if (!v || typeof v !== "object") continue;
    if (state === "density") {
      for (const [step, d] of Object.entries(v)) {
        console.log(`\n## density/${step}\n  ${JSON.stringify(d)}`);
      }
      continue;
    }
    if (state === "panels") {
      for (const [p, d] of Object.entries(v)) console.log(`\n## panel/${p}\n  ${JSON.stringify(d)}`);
      continue;
    }
    if (v.ok === false) {
      console.log(`\n## ${state}  NOT DRIVEN: ${v.why}`);
      continue;
    }
    console.log(`\n## ${state}`);
    if (v.row) console.log(`  row class="${v.rowClass}" attrs=${JSON.stringify(v.rowAttrs)} shadow=${v.row.boxShadow} bg=${v.row.backgroundColor} rect=${JSON.stringify(v.row.rect)}`);
    if (v.cells) for (const c of v.cells) console.log(`  cell${c.i} shadow=${c.boxShadow} bg=${c.backgroundColor} bl=${c.borderLeft} right=${c.right}${c.crossesContainerRight ? " PAST-CONTAINER" : ""}`);
    if (v.container) console.log(`  container=${JSON.stringify(v.container)} rowCrosses=${v.rowCrossesContainerRight}`);
    if (v.rowFocusable !== undefined) console.log(`  rowFocusable=${v.rowFocusable} focusables=${JSON.stringify(v.focusablesInRow)}`);
    console.log(`  distinctShadows=${JSON.stringify(v.distinctCellBoxShadows)}`);
  }
  process.exit(0);
}

// --coverage: how many surfaces x states were actually measured.
if (process.argv.includes("--coverage")) {
  const total = rows.length;
  const per = {};
  for (const [name] of coverageCols) {
    per[name] = rows.filter((r) => {
      const v = r.coverage[name];
      return v !== "not driven" && v !== "-" && v !== "0/3" && v !== "0/3";
    }).length;
  }
  console.log(`${total} surfaces driven`);
  console.log("  state                 rendered  concluded");
  for (const [name] of coverageCols) {
    const vals = rows.map((r) => r.coverage[name]);
    const rendered = vals.filter((v) => v === "yes" || /^[1-9]\/3$/.test(v) || /clipped$/.test(v)).length;
    const concluded = vals.filter((v) => v !== "not driven").length;
    console.log(`  ${name.padEnd(19)} ${String(rendered).padStart(4)}/${total}   ${String(concluded).padStart(4)}/${total}`);
  }
  const notDriven = rows.filter((r) => Object.values(r.coverage).includes("not driven")).map((r) => r.path);
  console.log(`\nsurfaces with at least one not-driven state (${notDriven.length}):\n  ${notDriven.join("\n  ")}`);
  process.exit(0);
}

const out = [];
out.push("| Surface | Rows | Density | Hover | Focus | Selected | Sel+hover | Current | Both | Edge | Toolbar | Panels | Empty | Error | Loading |");
out.push("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
for (const r of rows) {
  const c = r.coverage;
  out.push(
    `| \`${r.path}\` | ${r.rowCount} | ${c.density} | ${c.hover} | ${c.focus} | ${c.selected} | ${c.current} | ${c.both} | ${c.edge} | ${c.toolbar} | ${c.panels} | ${c.empty} | ${c.error} | ${c.loading} |`,
  );
}
out.push("");
out.push("## Measured row-state paint per surface");
out.push("");
out.push("| Surface | Worst state driven | Cells painted | Distinct cell shadows | Distinct cell backgrounds | Row itself painted | Cells past the container's right edge |");
out.push("|---|---|---|---|---|---|---|");
for (const r of rows) {
  out.push(
    `| \`${r.path}\` | ${r.worst ?? "—"} | ${r.cellShadows}/${r.rowPoly ?? "?"} | ${r.distinctCellShadows} | ${r.distinctCellBgs} | ${r.rowHasPaint ? "yes" : "no"} | ${r.cellsCrossing ?? "—"} |`,
  );
}
out.push("");
out.push("## Selected-row treatment per surface");
out.push("");
out.push("| Surface | Control used | Row painted | Cells carrying their own background | Cells carrying their own shadow | Hover replaces the selected fill |");
out.push("|---|---|---|---|---|---|");
for (const t of selTreatments) {
  out.push(
    `| \`${t.path}\` | ${t.mechanism ?? "—"} | ${t.rowPainted ? "yes" : "no"} | ${t.cellsPaintedBg}/${t.cellCount} | ${t.cellsPaintedRing}/${t.cellCount} | ${t.hoverReplacesBg ? "yes" : "no"} |`,
  );
}
if (!selTreatments.length) out.push("| — | — | — | — | — | — |");
out.push("");
out.push("## Does the cursor row render anything?");
out.push("");
const activated = noCursor.filter((n) => n.rendered === "yes").length;
const prelit = noCursor.filter((n) => n.alreadyCurrent === "yes").length;
out.push(
  `${activated} of ${noCursor.length} driven surfaces changed the row's paint when the row was made current, measured against that surface's own selected baseline. ${prelit} surface(s) already carry a current-state mark in the default state, so there is no activation delta to see there.`,
);
out.push("");
out.push("| Surface | Paint changes on activation | Already current in the default state | Cells marked in the default state |");
out.push("|---|---|---|---|");
for (const n of noCursor) {
  out.push(
    `| \`${n.path}\` | ${n.rendered} | ${n.alreadyCurrent} | ${n.defaultCellsPainted === null ? "—" : `${n.defaultCellsPainted}/${n.defaultCellCount}`} |`,
  );
}
out.push("");
out.push("## Defects");
out.push("");
if (!defects.length) out.push("None from the rules above.");
for (const d of defects) {
  if (d.kind) {
    out.push(`- \`${d.path}\` ${d.kind.replace(/-/g, " ")}: ${d.detail}.`);
  } else {
    out.push(
      `- \`${d.path}\` in state \`${d.state}\`: ${d.cellShadows} of ${d.cells} cells carry their own box-shadow (${d.distinctCellShadows} distinct value(s)); row itself painted: ${d.rowHasPaint ? "yes" : "no"}; cells past the container's right edge: ${d.cellsCrossing}.`,
    );
  }
}
console.log(out.join("\n"));
