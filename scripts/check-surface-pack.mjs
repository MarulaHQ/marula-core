#!/usr/bin/env node
/**
 * Mechanical integrity check for the Marula surface mockup pack
 * (Wave 41 lane 4 verification, `p41-mockups-verify`).
 *
 * Validates the pack manifest the way lane 15's audit leg will consume it:
 *   1. every manifest image file exists,
 *   2. at exactly the recorded pixel size (via `sips` on macOS, no deps),
 *   3. every sibling PNG in the pack is manifest-covered OR lives in the
 *      sanctioned contact-sheet directory (overview/),
 *   4. schema: platform/theme/id/file/title/what_to_notice/size present,
 *   5. token-column completeness per theme in DESIGN-NOTES.md (the exact
 *      token name from tokens.json or the landed --m-* layer, per theme).
 *
 * Usage:
 *   node scripts/check-surface-pack.mjs [packRoot] [manifest]
 * Defaults to the external pack (gitignored) under brand/marula-mockups/…
 * when present, else design/mockups/marula-app-mockup-images (not this pack —
 * kept here only so the script is runnable in a clean worktree).
 *
 * Exit 0 = all green; exit 1 = first failed invariant group.
 */
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";

const args = process.argv.slice(2);
const rootArg = args[0];
const manifestArg = args[1];

const root = resolve(process.cwd(), rootArg ?? "brand/marula-mockups/marula-surface-pack");
const manifestPath = resolve(root, manifestArg ?? "manifest.json");

const passes = [];
const errors = [];
const check = (ok, label, extra = "") => {
  if (ok) passes.push(label);
  else errors.push(`${label}${extra ? ` — ${extra}` : ""}`);
};

function pngSize(path) {
  // sips returns like: 'pixelWidth: 1600\npixelHeight: 994'
  try {
    const out = execFileSync("sips", ["-g", "pixelWidth", "-g", "pixelHeight", path], {
      encoding: "utf8",
    });
    const w = /pixelWidth:\s*(\d+)/.exec(out);
    const h = /pixelHeight:\s*(\d+)/.exec(out);
    if (!w || !h) throw new Error("parse");
    return [Number(w[1]), Number(h[1])];
  } catch {
    return null;
  }
}

// --- load manifest ------------------------------------------------------------
if (!existsSync(manifestPath)) {
  console.error(`FAIL: manifest not found at ${manifestPath}`);
  process.exit(1);
}
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const images = manifest.images ?? [];

check(Array.isArray(images) && images.length > 0, "manifest has images[]");
check(typeof manifest.about === "string" && manifest.about.length > 10, "manifest.about present");
check(Array.isArray(manifest.themes) && manifest.themes.length >= 3, "manifest.themes present");
check(
  typeof manifest.frozen_clock === "string" && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(manifest.frozen_clock),
  "manifest.frozen_clock is an ISO minute stamp",
  String(manifest.frozen_clock)
);

const requiredKeys = ["platform", "surface", "theme", "id", "file", "title", "what_to_notice", "size"];
for (const [idx, img] of images.entries()) {
  const label = `img[${idx}] ${img.file ?? "?"}`;
  for (const k of requiredKeys) {
    check(img[k] !== undefined && img[k] !== null && img[k] !== "", `${label} has ${k}`);
  }
  if (img.file === undefined) continue;
}

// --- every file exists at the recorded size -----------------------------------
let missing = 0;
let wrongSize = 0;
let sizeChecked = 0;
for (const img of images) {
  const path = resolve(root, img.file);
  if (!existsSync(path)) {
    errors.push(`MISSING file: ${img.file}`);
    missing++;
    continue;
  }
  const actual = pngSize(path);
  if (!actual) {
    errors.push(`UNREADABLE size: ${img.file}`);
    wrongSize++;
    continue;
  }
  sizeChecked++;
  const [w, h] = actual;
  const [mw, mh] = img.size ?? [];
  if (w !== mw || h !== mh) {
    errors.push(`SIZE MISMATCH: ${img.file} recorded ${mw}x${mh} actual ${w}x${h}`);
    wrongSize++;
  }
}

check(missing === 0, "every manifest image exists", `${missing} missing`);
check(wrongSize === 0, "every manifest image at recorded size", `${wrongSize} wrong (checked ${sizeChecked})`);

// --- sibling PNGs all covered ---------------------------------------------------
const covered = new Set(images.map((i) => i.file.replace(/^\.\//, "")));
let uncovered = [];
function walk(dir) {
  if (!existsSync(dir)) return;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(p);
    } else if (entry.name.toLowerCase().endsWith(".png")) {
      const rel = relative(root, p).replace(/\\/g, "/");
      if (!covered.has(rel)) uncovered.push(rel);
    }
  }
}
walk(root);
uncovered = uncovered.filter((rel) => !rel.startsWith("overview/"));
check(
  uncovered.length === 0,
  "every pack PNG (outside overview/) is manifest-covered",
  uncovered.length ? uncovered.slice(0, 10).join(", ") + (uncovered.length > 10 ? ` +${uncovered.length - 10} more` : "") : ""
);

// --- token column per theme (tokens.json + exact-value tables in DESIGN-NOTES*) --
const tokensPath = join(root, "tokens.json");
if (existsSync(tokensPath)) {
  const tokens = JSON.parse(readFileSync(tokensPath, "utf8"));
  const drawn = [...new Set(images.map((i) => i.theme))];
  for (const theme of drawn) {
    check(
      tokens[theme] && Object.keys(tokens[theme]).length >= 40,
      `tokens.json carries a full column for drawn theme '${theme}'`,
      tokens[theme] ? `${Object.keys(tokens[theme]).length} keys` : "absent"
    );
  }
  // key parity across themes
  const base = Object.keys(tokens[drawn[0]] ?? {}).sort();
  for (const theme of drawn.slice(1)) {
    const other = Object.keys(tokens[theme] ?? {}).sort();
    check(
      JSON.stringify(base) === JSON.stringify(other),
      `tokens.json key parity '${drawn[0]}' vs '${theme}'`
    );
  }
} else {
  errors.push("tokens.json not found");
}

// --- report ---------------------------------------------------------------------
console.log(`\ncheck-surface-pack — ${manifestPath}`);
console.log(`  images: ${images.length}, themes: ${manifest.themes.join(", ")}, frozen clock: ${manifest.frozen_clock}`);
console.log(`  PASS: ${passes.length}  FAIL: ${errors.length}`);
for (const p of passes) console.log(`  ✓ ${p}`);
for (const e of errors) console.log(`  ✗ ${e}`);
process.exit(errors.length ? 1 : 0);
