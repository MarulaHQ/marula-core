#!/usr/bin/env node
/**
 * Time-display check (Wave 50 lane T3, p50-tz-surface-rollout).
 *
 * `toLocale*` formats in the machine's zone and locale. Every reading a surface
 * draws has to name its zone instead: `formatInTimeZone` / `<Time>` / `<DualTime>`
 * with the reader's account zone or a record's own, `formatWallClock` for the
 * reader's own clock, `formatDayKey` for a bare day, `formatNumber` for a count.
 *
 * This script is the sweep's done-condition, so it cannot be argued with: no
 * `toLocale*` call survives in the app or the kit outside tests. Comments and
 * docstrings are stripped before scanning — a comment that names the call it
 * replaced is documentation, not a display call.
 *
 * Scope: apps/web and packages/ui/src. Test files legitimately assert strings
 * and build fixtures, so `tests/` and `*.test.*` are out of scope.
 *
 * Read-only, no browser, no build needed — it runs on the light path.
 * Usage: node scripts/check-time-display.mjs
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname, resolve, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ROOTS = ["apps/web", "packages/ui/src"];
const SKIP_DIRS = new Set(["node_modules", ".next", "dist", "tests", "__snapshots__"]);
const SKIP_FILE = /\.test\.[cm]?[jt]sx?$/;

/** Everything a browser would ignore is not a display call. */
function stripComments(source) {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/(^|[^:"'`\\])\/\/[^\n]*/g, "$1");
}

function walk(dir, out = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (SKIP_DIRS.has(entry.name)) continue;
      walk(join(dir, entry.name), out);
    } else if (/\.tsx?$/.test(entry.name) && !SKIP_FILE.test(entry.name)) {
      out.push(join(dir, entry.name));
    }
  }
  return out;
}

const CALL = /\btoLocale[A-Za-z]*\s*\(/g;
const hits = [];
let scanned = 0;

for (const rel of ROOTS) {
  const abs = join(root, rel);
  let files;
  try {
    files = statSync(abs).isDirectory() ? walk(abs) : [abs];
  } catch {
    continue;
  }
  for (const file of files) {
    scanned += 1;
    const lines = stripComments(readFileSync(file, "utf8")).split("\n");
    lines.forEach((line, index) => {
      CALL.lastIndex = 0;
      if (CALL.test(line)) {
        hits.push(`${relative(root, file)}:${index + 1}: ${line.trim()}`);
      }
    });
  }
}

if (hits.length > 0) {
  console.error(`time-display: ${hits.length} toLocale* call(s) outside the kit:`);
  for (const hit of hits) console.error(`  ${hit}`);
  console.error(
    "\nName the zone: formatInTimeZone / <Time> / <DualTime> for a record, formatWallClock for the reader's own clock, formatDayKey for a bare day, formatNumber for a count.",
  );
  process.exit(1);
}

console.log(`time-display: ${scanned} files scanned, no toLocale* calls.`);
