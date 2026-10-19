#!/usr/bin/env node
/**
 * p47-row-effect-check — mechanical detector for the "row treatment authored as
 * a cell treatment" defect class.
 *
 * The class: a stylesheet names a ROW state (selected, current, hover, active,
 * expanded …) but paints it on the row's CELLS (`td` / `th`) instead of on the
 * row (`tr`). Every visual device then repeats once per cell:
 *
 *   - an inset ring (`inset 0 0 0 2px`) draws a box around every cell, so the
 *     row reads as a grid of boxes with seams rather than one ring;
 *   - a background fill tints every cell, so the row's fill has to abut cell
 *     borders and the container edge;
 *   - an inset edge bar (`inset 3px 0 0`) sits on the left edge of EVERY cell,
 *     so a row gets N bars instead of one.
 *
 * The proving case is `apps/web/components/money.css:228-238` (the approvals
 * queue). This detector MUST report it. If it does not, the detector is wrong.
 *
 * Purpose: find the class, mechanically, over every stylesheet the app ships.
 * Non-purpose: it does not decide whether a hit is a defect. It reports the
 * selector, the file:line span, the subject, the row-state token and the
 * signals; a human or the review doc rules on it. A hit whose selector subject
 * is the `tr` is the correct authoring and is reported separately, as the
 * counter-example the audit compares against.
 *
 * Usage:
 *   node scripts/p47-row-effect-check.mjs                 # human summary
 *   node scripts/p47-row-effect-check.mjs --json out.json # + machine output
 *   node scripts/p47-row-effect-check.mjs --strict        # exit 1 on any cell hit
 */

import { readFileSync, writeFileSync } from "node:fs";
import { readdirSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";

const ROOT = resolve(new URL("..", import.meta.url).pathname);
const SCAN_DIRS = ["apps/web", "packages/ui"];
const SKIP = new Set(["node_modules", ".next", "dist", "target", ".turbo", "coverage", ".trail"]);

/* ------------------------------------------------------------------ *
 * Selector knowledge
 * ------------------------------------------------------------------ */

/** A row-state token: the selector is talking about a row's state. */
const ROW_STATE = [
  { name: "row-selected", re: /(^|[^a-z])row-selected\b/ },
  { name: "row-current", re: /(^|[^a-z])row-current\b/ },
  { name: "row-hover", re: /(^|[^a-z])row-hover\b/ },
  { name: "row-focus", re: /(^|[^a-z])row-focus\b/ },
  { name: "is-selected", re: /(^|[^a-z])is-selected\b/ },
  { name: "is-current", re: /(^|[^a-z])is-current\b/ },
  { name: "is-active", re: /(^|[^a-z])is-active\b/ },
  { name: "is-expanded", re: /(^|[^a-z])is-expanded\b/ },
  { name: "is-drag-source", re: /(^|[^a-z])is-drag-source\b/ },
  { name: "data-selected", re: /\[data-selected/ },
  { name: "data-current", re: /\[data-current/ },
  { name: "data-state", re: /\[data-state/ },
  { name: "aria-selected", re: /\[aria-selected/ },
  { name: "aria-current", re: /\[aria-current/ },
  { name: "aria-expanded", re: /\[aria-expanded/ },
  { name: "pseudo-selected", re: /:(?:checked|selected)\b/ },
  { name: "tr-attr", re: /\btr\[/ },
];

/** Declaration properties that paint the cell rather than the row. */
const PAINT = /(^|;|\s)(box-shadow|outline|border|border-\w+|background|background-color|background-image)\s*:/;

/* ------------------------------------------------------------------ *
 * CSS parsing — enough to find rule blocks and their file:line spans.
 * ------------------------------------------------------------------ */

function stripComments(css) {
  return css.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " "));
}

/** Split a declaration block's selector list on top-level commas. */
function splitSelectors(sel) {
  const out = [];
  let depth = 0;
  let buf = "";
  for (const ch of sel) {
    if (ch === "(" || ch === "[") depth++;
    else if (ch === ")" || ch === "]") depth--;
    if (ch === "," && depth === 0) {
      out.push(buf.trim());
      buf = "";
    } else buf += ch;
  }
  if (buf.trim()) out.push(buf.trim());
  return out;
}

/** Split a selector into its subject (last compound) and its ancestry. */
function splitSubject(selector) {
  // Break on combinators at paren/bracket depth 0.
  const parts = [];
  let depth = 0;
  let buf = "";
  for (let i = 0; i < selector.length; i++) {
    const ch = selector[i];
    if (ch === "(" || ch === "[") depth++;
    else if (ch === ")" || ch === "]") depth--;
    if (depth === 0 && (ch === ">" || ch === "+" || ch === "~")) {
      parts.push(buf.trim());
      buf = "";
      continue;
    }
    if (depth === 0 && /\s/.test(ch)) {
      // whitespace combinator, but not inside a functional pseudo like :not(a b)
      if (buf.trim()) parts.push(buf.trim());
      buf = "";
      continue;
    }
    buf += ch;
  }
  if (buf.trim()) parts.push(buf.trim());
  const subject = parts.length ? parts[parts.length - 1] : selector;
  const ancestry = parts.slice(0, -1).join(" ");
  return { subject, ancestry };
}

/** Walk a stylesheet into { selector, decls, line } blocks (recursing into at-rules). */
function walk(css, sourceName, out, baseLine = 1) {
  const src = stripComments(css);
  let i = 0;
  let line = baseLine;
  const advance = (s) => {
    for (const c of s) if (c === "\n") line++;
  };
  let prelude = "";
  while (i < src.length) {
    const ch = src[i];
    if (ch === "{") {
      const startLine = line;
      // find matching close brace
      let depth = 1;
      let j = i + 1;
      while (j < src.length && depth > 0) {
        if (src[j] === "{") depth++;
        else if (src[j] === "}") depth--;
        j++;
      }
      const body = src.slice(i + 1, j - 1);
      const sel = prelude.trim();
      if (sel.startsWith("@")) {
        // at-rule: recurse for nested rules (@media/@supports/@layer)
        if (/^@(media|supports|layer|container|scope)/.test(sel)) {
          const innerStartLine = startLine + (body.slice(0, body.indexOf("\n")).split("\n").length - 1 || 0);
          walk(body, sourceName, out, startLine);
        }
      } else {
        out.push({ selector: sel, decls: body, file: sourceName, line: startLine });
      }
      advance(src.slice(i, j));
      i = j;
      prelude = "";
      continue;
    }
    if (ch === "}") {
      i++;
      continue;
    }
    prelude += ch;
    if (ch === "\n") line++;
    i++;
  }
}

/* ------------------------------------------------------------------ *
 * Signals
 * ------------------------------------------------------------------ */

function classify(decls) {
  const d = decls.replace(/\s+/g, " ");
  const signals = [];
  if (/inset\s+0\s+0\s+0\b/.test(d)) signals.push("ring:inset");
  if (/box-shadow[^;]*\b0\s+0\s+0\s+\d/.test(d) && !/inset/.test(d.match(/box-shadow[^;]*/)?.[0] ?? ""))
    signals.push("ring:outset");
  if (/(^|;|\s)outline\s*:/.test(d)) signals.push("outline");
  if (/inset\s+-?\d+(?:\.\d+)?px\s+0\s+0/.test(d)) signals.push("edge-bar:left");
  if (/inset\s+0\s+-?\d+(?:\.\d+)?px\s+0/.test(d)) signals.push("edge-bar:top");
  if (/(^|;|\s)background(-color|-image)?\s*:/.test(d)) signals.push("fill:background");
  const borders = [...d.matchAll(/(?:^|;|\s)(border(?:-top|-right|-bottom|-left)?(?:-color|-width)?)\s*:/g)];
  if (borders.length) signals.push("border");
  return signals;
}

function matchRowState(text) {
  const hits = [];
  for (const { name, re } of ROW_STATE) if (re.test(text)) hits.push(name);
  return hits;
}

/* ------------------------------------------------------------------ *
 * Walk the tree
 * ------------------------------------------------------------------ */

function* cssFiles(dir) {
  let entries;
  try {
    entries = readdirSync(dir);
  } catch {
    return;
  }
  for (const name of entries) {
    if (SKIP.has(name)) continue;
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) yield* cssFiles(p);
    else if (name.endsWith(".css")) yield p;
  }
}

const blocks = [];
for (const d of SCAN_DIRS) {
  for (const f of cssFiles(join(ROOT, d))) {
    const rel = relative(ROOT, f);
    walk(readFileSync(f, "utf8"), rel, blocks);
  }
}

const cellHits = [];
const rowHits = [];

for (const b of blocks) {
  if (!PAINT.test(b.decls)) continue;
  const lineCount = b.decls.split("\n").length;
  const span = `${b.file}:${b.line}-${b.line + lineCount - 1}`;
  for (const sel of splitSelectors(b.selector)) {
    const { subject, ancestry } = splitSubject(sel);
    const subjectIsCell = /\b(td|th)\b/.test(subject);
    const subjectIsRow = /^tr\b/.test(subject.trim());
    const signals = classify(b.decls);
    if (!signals.length) continue;
    const rec = {
      file: b.file,
      line: b.line,
      span,
      selector: sel,
      subject,
      ancestry,
      rowState: [],
      signals,
      decls: b.decls.replace(/\s+/g, " ").trim(),
    };
    if (subjectIsCell) {
      // A row state must be named on the row, in the ancestry, for this to be
      // the defect: the cell is painted because an ancestor row is in a state.
      rec.rowState = matchRowState(ancestry);
      if (rec.rowState.length) cellHits.push(rec);
    } else if (subjectIsRow) {
      // The counter-example: the same state authored on the row itself.
      rec.rowState = matchRowState(sel);
      if (rec.rowState.length) rowHits.push(rec);
    }
  }
}

/* ------------------------------------------------------------------ *
 * Report
 * ------------------------------------------------------------------ */

const target = "apps/web/components/money.css";
const found = cellHits.some((h) => h.file === target && h.line >= 220 && h.line <= 245);

if (process.argv.includes("--json")) {
  const idx = process.argv.indexOf("--json");
  const outPath = process.argv[idx + 1] && !process.argv[idx + 1].startsWith("--") ? process.argv[idx + 1] : null;
  const payload = {
    generatedAt: new Date().toISOString(),
    scanned: { dirs: SCAN_DIRS, rules: blocks.length },
    provingCase: { file: `${target}:228-238`, found },
    cellTreatmentUnderRowState: cellHits,
    rowTreatmentCounterExamples: rowHits,
  };
  if (outPath) {
    writeFileSync(resolve(outPath), JSON.stringify(payload, null, 2) + "\n");
    console.log(`wrote ${outPath}`);
  } else {
    console.log(JSON.stringify(payload, null, 2));
  }
}

console.log("");
console.log("p47-row-effect-check — row state painted on cells");
console.log("=================================================");
console.log(`stylesheets scanned: ${blocks.length} rule blocks across ${SCAN_DIRS.join(", ")}`);
console.log("");
if (!cellHits.length) {
  console.log("no cell-treatment row-state rules found.");
} else {
  for (const h of cellHits) {
    console.log(`  ${h.span}`);
    console.log(`    selector : ${h.selector}`);
    console.log(`    subject  : <${h.subject}>   row state on: ${h.rowState.join(", ")}`);
    console.log(`    signals  : ${h.signals.join(", ")}`);
    console.log(`    decls    : ${h.decls}`);
    console.log("");
  }
}
console.log(`cell-treatment hits : ${cellHits.length}`);
console.log(`row-treatment hits  : ${rowHits.length} (the correct authoring — counter-examples)`);
for (const h of rowHits) console.log(`    ✓ ${h.span}  ${h.selector}  [${h.signals.join(", ")}]`);
console.log("");
const proving = found
  ? "present: YES"
  : cellHits.length > 0
    ? "present: NO — detector is wrong"
    : "present: NO (fixed by p51-row-treatment; zero cell hits is the correct result)";
console.log(`proving case ${target}:228-238 ${proving}`);
console.log("");

if (process.argv.includes("--strict") && cellHits.length && !found) {
  console.error("detector failed its own proving case — aborting");
  process.exit(2);
}
process.exit(process.argv.includes("--strict") && cellHits.length ? 1 : 0);
