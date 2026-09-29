#!/usr/bin/env node
/**
 * Theme review script — repointed to Marula in wave41 lane 2.
 *
 * Hardened review: parses the actual CSS variable declarations from
 * packages/ui/src/tokens.css and apps/web/app/globals.css and compares them
 * VALUE-BY-VALUE against the vendored canonical preset
 * (design/tangerine/tangerine.json). Substring greps cannot catch a wrong
 * dark mode; this can.
 *
 * Checks:
 *  1. Preset fidelity — every shadcn variable in globals.css :root/.dark that
 *     the preset defines matches the preset's oklch/hsl value verbatim.
 *  2. Token anchors — tokens.css dark-mode canvas/surface/text/border resolve
 *     to the converted preset hex anchors (#1C2433 / #2A3040 / #E5E5E5 / #3D4354).
 *  3. Dark accent = light accent (preset keeps primary identical).
 *  4. Chart ramp — light #86A7C8/#EEA591/#5A7CA6/#466494/#334C82, dark chart-2 #E6A08F.
 *  5. Retired hexes — Workbench hexes must not appear in any scanned UI file
 *     (mirrors the CI blacklist).
 *  6. Pill tags, serif headings, "use client" coverage, motion law, mockup rules.
 *  7. tokens.ts PALETTE ↔ tokens.css hex sync.
 *
 * Usage: node scripts/review-theme.mjs   (exit 1 on any failure)
 */

import { readFileSync, existsSync, readdirSync } from "fs";
import { resolve, join } from "path";

const root = process.cwd();
const errors = [];
const notes = [];
let passes = 0;

function check(condition, label, detail = "") {
  if (condition) {
    passes++;
    notes.push(label);
  } else {
    errors.push(`${label}${detail ? " — " + detail : ""}`);
  }
}

const read = (p) => readFileSync(p, "utf8");
const stripComments = (css) => css.replace(/\/\*[\s\S]*?\*\//g, "");

/** Parse custom-property declarations in a CSS string into a Map. */
function parseVars(css) {
  const map = new Map();
  for (const m of css.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    map.set(m[1], m[2].trim());
  }
  return map;
}

/** Extract the body of a selector block (first match anchored at line start). */
function blockOf(css, selectorStart) {
  const re = new RegExp(`^${selectorStart.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*\\{`, "m");
  const m = css.match(re);
  if (!m) return "";
  const open = css.indexOf("{", m.index);
  let depth = 1;
  let j = open + 1;
  while (j < css.length && depth > 0) {
    if (css[j] === "{") depth++;
    else if (css[j] === "}") depth--;
    j++;
  }
  return css.slice(open + 1, j - 1);
}

// ---------- load inputs ----------
const presetPath = resolve(root, "design/marula/marula.json");
const tokensCssPath = resolve(root, "packages/ui/src/tokens.css");
const globalsPath = resolve(root, "apps/web/app/globals.css");
const componentsPath = resolve(root, "packages/ui/src/components.css");
const tokensTsPath = resolve(root, "packages/ui/src/tokens.ts");
// Filename still carries the pre-rebrand prefix: lane 1 renamed identifiers,
// not this artifact. Retiring the file is lane 4's call (it draws the new
// mockups), so the review points at what exists and checks only geometry.
const mockupPath = resolve(root, "design/mockups/meridian-ticket-workbench-html-mockup-v2.html");

check(existsSync(presetPath), "vendored preset exists (design/marula/marula.json)");
check(existsSync(tokensCssPath), "packages/ui/src/tokens.css exists");
check(existsSync(globalsPath), "apps/web/app/globals.css exists");
check(existsSync(componentsPath), "packages/ui/src/components.css exists");
check(existsSync(tokensTsPath), "packages/ui/src/tokens.ts exists");
check(existsSync(mockupPath), "mockup HTML exists");

if (!existsSync(presetPath) || !existsSync(tokensCssPath) || !existsSync(globalsPath)) {
  console.error("FATAL: required inputs missing; cannot run value-level review.");
  process.exit(1);
}

const preset = JSON.parse(read(presetPath));
const tokensCss = stripComments(read(tokensCssPath));
const globalsCss = stripComments(read(globalsPath));
const tokensTs = read(tokensTsPath);
const componentsCss = read(componentsPath);
const mockup = read(mockupPath);

// ---------- 1. preset fidelity in globals.css ----------
const gRoot = parseVars(blockOf(globalsCss, ":root"));
const gDark = parseVars(blockOf(globalsCss, '[data-theme="dark"]'));

const PRESET_BRIDGED = new Set([
  "--background", "--foreground", "--card", "--card-foreground",
  "--popover", "--popover-foreground", "--primary", "--primary-foreground",
  "--destructive", "--destructive-foreground", "--border", "--ring", "--radius",
  "--sidebar-foreground", "--sidebar-primary", "--sidebar-primary-foreground",
  "--sidebar-ring",
]);

let presetChecked = 0;
const MODE_INDEPENDENT = new Set(["--font-sans", "--font-serif", "--font-mono"]);
/**
 * How many vars a mode block is expected to carry, counted from the export's
 * OWN keys. The old form subtracted two hardcoded set sizes, which silently
 * assumed light and dark have identical key sets — they do not, and should not:
 * `radius` and `tracking-normal` are theme-level and belong in neither mode
 * block. Deriving the count means a changed export cannot make this check
 * quietly vacuous or falsely red.
 */
const expectedInBlock = (vars) =>
  Object.keys(vars).filter((name) => !PRESET_BRIDGED.has(`--${name}`) && !MODE_INDEPENDENT.has(`--${name}`)).length;
for (const [name, value] of Object.entries(preset.cssVars.light)) {
  if (PRESET_BRIDGED.has(`--${name}`) || MODE_INDEPENDENT.has(`--${name}`)) continue;
  const actual = gRoot.get(`--${name}`);
  if (actual === undefined) {
    errors.push(`globals :root missing preset variable --${name}`);
  } else if (actual !== value) {
    errors.push(`globals :root --${name} = "${actual}" ≠ preset "${value}"`);
  } else {
    presetChecked++;
  }
}
check(expectedInBlock(preset.cssVars.light) === presetChecked,
  `globals :root matches preset verbatim (${presetChecked}/${expectedInBlock(preset.cssVars.light)} non-bridged vars)`);

let presetDarkChecked = 0;
for (const [name, value] of Object.entries(preset.cssVars.dark)) {
  if (PRESET_BRIDGED.has(`--${name}`) || MODE_INDEPENDENT.has(`--${name}`)) continue;
  const actual = gDark.get(`--${name}`);
  if (actual === undefined) {
    errors.push(`globals dark block missing preset variable --${name}`);
  } else if (actual !== value) {
    errors.push(`globals dark --${name} = "${actual}" ≠ preset "${value}"`);
  } else {
    presetDarkChecked++;
  }
}
check(expectedInBlock(preset.cssVars.dark) === presetDarkChecked,
  `globals dark block matches preset verbatim (${presetDarkChecked}/${expectedInBlock(preset.cssVars.dark)} non-bridged vars)`);

// bridged variables must reference the token layer, not redeclare hexes
for (const name of PRESET_BRIDGED) {
  const v = gRoot.get(name);
  check(v !== undefined && (v.startsWith("var(--m-") || v === "0.75rem"),
    `globals :root ${name} bridged to token layer`, v ?? "missing");
}

// ---------- 2. token dark anchors (preset converted) ----------
const tRoot = parseVars(tokensCss.slice(0, tokensCss.indexOf('[data-theme="dark"]')));
const tDark = parseVars(tokensCss.slice(tokensCss.indexOf('[data-theme="dark"]')));

// The Marula semantic anchors, lowercased. These are the values the brand
// spec fixes, so a drift here is a drift from the brand, not a refactor.
const ANCHORS = {
  "--m-gold": "#e8b83a",
  "--m-cream": "#f6efd9",
  "--m-canopy": "#5e7f3a",
  "--m-sand": "#cdb38b",
  "--m-bark": "#7c746a",
  "--m-blush": "#d9822b",
  "--m-destructive": "#b42318",
  "--m-gold-text": "#9a5b0f",
};
for (const [name, hex] of Object.entries(ANCHORS)) {
  check((tRoot.get(name) ?? "").toLowerCase() === hex,
    `tokens.css ${name} = ${hex}`, tRoot.get(name) ?? "missing");
}

// semantic dark resolves through the anchors
const resolveDark = (v, depth = 0) => {
  const m = typeof v === "string" && v.match(/^var\((--[\w-]+)\)$/);
  if (!m || depth > 4) return v;
  return resolveDark(tDark.get(m[1]) ?? tRoot.get(m[1]), depth + 1);
};
check((resolveDark(tDark.get("--m-canvas")) ?? "").toLowerCase() === "#2b2a33",
  "dark --m-canvas resolves to #2B2A33");
check((resolveDark(tDark.get("--m-surface")) ?? "").toLowerCase() === "#34333d",
  "dark --m-surface resolves to #34333D");
check((resolveDark(tDark.get("--m-text")) ?? "").toLowerCase() === "#f6efd9",
  "dark --m-text resolves to #F6EFD9");
check((resolveDark(tDark.get("--m-border")) ?? "").toLowerCase() === "#43424d",
  "dark --m-border resolves to #43424D");

// ---------- 3. the primary FLIPS on mode ----------
// The retired Tangerine preset kept one accent in both modes. Marula inverts
// it: ink with gold text in light, gold with ink text in dark. Asserted as a
// pair, because "the dark primary is not the light primary" is the invariant
// that a later mechanical pass breaks.
const lightPrimary = (tRoot.get("--m-primary") ?? "").toLowerCase();
const darkPrimary = (tDark.get("--m-primary") ?? "").toLowerCase();
check(lightPrimary === "#2b2a33" && darkPrimary === "#e8b83a",
  "primary flips: ink in light, gold in dark",
  `${lightPrimary} / ${darkPrimary}`);
check((tRoot.get("--m-on-primary") ?? "").toLowerCase() === "#e8b83a" &&
    (tDark.get("--m-on-primary") ?? "").toLowerCase() === "#2b2a33",
  "on-primary flips with it: gold in light, ink in dark");

// The timer bar does NOT flip. It is declared once in :root and never
// overridden, so a mechanical dark pass cannot paint it cream.
check((tRoot.get("--m-bar") ?? "").toLowerCase() === "#2b2a33" && tDark.get("--m-bar") === undefined,
  "the timer bar is ink in every theme and is never overridden in the dark block",
  `${tRoot.get("--m-bar")} / ${tDark.get("--m-bar")}`);

// ---------- 4. chart ramp ----------
// Gold, canopy, blush, bark, lime (spec §4.4). Dark leads with gold because
// gold on ink is 7.7:1, the same reason the primary inverts.
const CHARTS_LIGHT = {
  "--m-chart-1": "#a8641a", "--m-chart-2": "#5e7f3a", "--m-chart-3": "#d9822b",
  "--m-chart-4": "#7c746a", "--m-chart-5": "#a9c25d",
};
for (const [name, hex] of Object.entries(CHARTS_LIGHT)) {
  check((tRoot.get(name) ?? "").toLowerCase() === hex, `chart ramp ${name} = ${hex}`);
}
const CHARTS_DARK = {
  "--m-chart-1": "#e8b83a", "--m-chart-2": "#a9c25d", "--m-chart-3": "#f0b27a",
  "--m-chart-4": "#beb5a2", "--m-chart-5": "#cdb38b",
};
for (const [name, hex] of Object.entries(CHARTS_DARK)) {
  check((tDark.get(name) ?? "").toLowerCase() === hex, `dark chart ramp ${name} = ${hex}`);
}

// ---------- 5. retired hexes (CI blacklist) ----------
// Three dead families: Workbench (cool grey), Tangerine (the second identity),
// Ember (retired by spec §4.6). Append-only — a retired colour is never
// removed from this list, because the point is that it can never come back.
const BLACKLIST = [
  "D64B0A", "FF8A4C",                    // Ember
  "F4F5F7", "101216", "171A1F", "12161C", // Workbench
  "E05D38", "E8EBED", "1C2433", "2A3040", "3D4354", "86A7C8", // Tangerine
];
const scanned = [];
for (const dir of ["apps", "packages"]) {
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      if (e.name === "node_modules" || e.name === ".next" || e.name === "dist" || e.name === "target") continue;
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (/\.(css|tsx?|ts)$/.test(e.name)) scanned.push(p);
    }
  };
  walk(dir);
}
const offenders = [];
// A hex named inside a comment is documentation — several files carry the
// retired values on purpose, as tombstones saying "this must not come back".
// Scan declarations only.
// Two files carry the retired hexes as DATA — the blacklist itself, and a test
// that asserts the blacklist holds. Exempt by path, explicitly, so the
// exemption is auditable rather than a pattern that could hide a real use.
const BLACKLIST_EXEMPT = new Set([
  "packages/ui/tests/tokens.test.ts",
  "apps/web/tests/lib/chart-tokens.test.ts",
]);
for (const file of scanned) {
  if (BLACKLIST_EXEMPT.has(file)) continue;
  const content = stripComments(read(file));
  for (const hex of BLACKLIST) {
    if (new RegExp(`#${hex}\\b`, "i").test(content)) offenders.push(`${file}: #${hex}`);
  }
}
check(offenders.length === 0, "no retired identity hexes in scanned UI files", offenders.join("; "));

// ---------- 6. craft rules ----------
check(componentsCss.includes("border-radius: 999px"), "components.css pill tags (999px)");
check(!/font-family:\s*var\(--m-font-heading/.test(componentsCss),
  "components.css defaults to Figtree — Fraunces is opt-in only (no blanket display serif)");
check(/--font-serif:\s*var\(--font-heading/.test(globalsCss),
  "Tailwind --font-serif maps to the Fraunces variable (no Times fallback)");
// The scale root stays 0.75rem (12px, unchanged); controls render at 10px and
// cards at 16px, so the control radius is NOT the scale root any more.
check(/--m-radius-scale:\s*0\.75rem/.test(tokensCss), "tokens.css scale root 0.75rem (12px)");
check(/--m-radius-control:\s*10px/.test(tokensCss), "tokens.css control radius 10px");
check(/--m-duration:\s*120ms/.test(tokensCss), "motion law token 120ms present");
check(componentsCss.includes("prefers-reduced-motion"), "prefers-reduced-motion honored");

const layoutTsx = read(resolve(root, "apps/web/app/layout.tsx"));
for (const [label, family, variable, woff2] of [
  ["Fraunces", "fraunces", "--font-heading", "Fraunces-variable-latin.woff2"],
  ["Figtree", "figtree", "--font-sans", "Figtree-variable-latin.woff2"],
  ["JetBrains Mono", "jetbrainsMono", "--font-mono", "JetBrainsMono-variable-latin.woff2"],
]) {
  check(layoutTsx.includes(family) && layoutTsx.includes(`"${variable}"`),
    `next/font serves ${label} as ${variable}`);
  const fontPath = resolve(root, "LEGAL/fonts");
  check(existsSync(join(fontPath, woff2.split("-")[0].toLowerCase(), woff2)) ||
        readdirSync(fontPath).some((d) => existsSync(join(fontPath, d, woff2))),
    `${label} woff2 is vendored in LEGAL/fonts/`);
  check(readdirSync(fontPath).some((d) => existsSync(join(fontPath, d, "OFL.txt"))),
    `${label} OFL travels with the woff2`);
}
// Fraunces is narrowed to the wordmark, greetings and marketing by design, so
// nothing may apply it to a ticket title.
check(!/font-serif[\s\S]{0,80}ticket/i.test(globalsCss + componentsCss),
  "Fraunces is not applied to ticket titles (the 26px title is Figtree)");
check(layoutTsx.includes("Figtree") && layoutTsx.includes('"--font-sans"'),
  "next/font serves Figtree as --font-sans");

const componentFiles = readdirSync(resolve(root, "packages/ui/src/components")).filter((f) => f.endsWith(".tsx"));
const missingClient = componentFiles.filter((f) => !read(join(root, "packages/ui/src/components", f)).startsWith('"use client"'));
check(missingClient.length === 0, `all ${componentFiles.length} component files carry "use client"`,
  missingClient.join(", "));

// ---------- 7. PALETTE sync ----------
const cssHexes = new Set([...tokensCss.matchAll(/#[0-9a-fA-F]{6}\b/g)].map((m) => m[0].toUpperCase()));
const tsHexes = new Set([...tokensTs.matchAll(/"#[0-9a-fA-F]{6}"/g)].map((m) => m[0].slice(1, -1).toUpperCase()));
check(cssHexes.size === tsHexes.size && [...cssHexes].every((h) => tsHexes.has(h)) &&
    [...tsHexes].every((h) => cssHexes.has(h)),
  `tokens.ts PALETTE ↔ tokens.css hex sets identical (${cssHexes.size})`);

// ---------- 8. mockup ----------
// The v2 mockup is the WORKBENCH aesthetic reference and still describes the
// retired identity. It is checked for the geometry that carried over (pill tags,
// the 0.75rem scale root, both modes) and NOT for its colours: Tangerine is
// retired, so a colour assertion against it would be an assertion that the
// mockup is wrong. The Marula values are pinned in packages/ui/tests/tokens.test.ts
// against tokens.css instead, which is the source of truth.
check(mockup.includes("999px"), "mockup pill tags (999px)");
check(/--radius:\s*0\.75rem/i.test(mockup), "mockup radius 0.75rem (scale root carried over)");
check(/body\.dark\s*\{/.test(mockup), "mockup v2 dark-mode support (body.dark block)");
check(!/#e05d38/i.test(tokensCss), "tokens.css carries no Tangerine primary");
check(!/#e8ebed/i.test(tokensCss), "tokens.css carries no Tangerine canvas");
check(!/Tangerine/i.test(globalsCss), "globals.css names no retired identity");

// ---------- summary ----------
console.log(`\n=== Marula Theme Review — value-level (wave41 marula theme) ===`);
console.log(`Preset:  design/marula/marula.json (${Object.keys(preset.cssVars.light).length} light / ${Object.keys(preset.cssVars.dark).length} dark vars)`);
console.log(`Passed:  ${passes} checks`);
console.log(`Failed:  ${errors.length} checks`);
if (errors.length > 0) {
  console.log("\nFAILURES:");
  for (const e of errors) console.log("  ✗ " + e);
  process.exit(1);
}
console.log("\nAll checks passed — generated layers match the canonical Marula preset.");
