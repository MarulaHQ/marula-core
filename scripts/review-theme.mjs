#!/usr/bin/env node
/**
 * Theme review script — p15-theme-tangerine.
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
const presetPath = resolve(root, "design/tangerine/tangerine.json");
const tokensCssPath = resolve(root, "packages/ui/src/tokens.css");
const globalsPath = resolve(root, "apps/web/app/globals.css");
const componentsPath = resolve(root, "packages/ui/src/components.css");
const tokensTsPath = resolve(root, "packages/ui/src/tokens.ts");
const mockupPath = resolve(root, "design/mockups/meridian-ticket-workbench-html-mockup.html");

check(existsSync(presetPath), "vendored preset exists (design/tangerine/tangerine.json)");
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
check(Object.keys(preset.cssVars.light).length - PRESET_BRIDGED.size - MODE_INDEPENDENT.size === presetChecked,
  `globals :root matches preset verbatim (${presetChecked} non-bridged vars)`);

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
check(Object.keys(preset.cssVars.dark).length - PRESET_BRIDGED.size - MODE_INDEPENDENT.size === presetDarkChecked,
  `globals dark block matches preset verbatim (${presetDarkChecked} non-bridged vars)`);

// bridged variables must reference the token layer, not redeclare hexes
for (const name of PRESET_BRIDGED) {
  const v = gRoot.get(name);
  check(v !== undefined && (v.startsWith("var(--m-") || v === "0.75rem"),
    `globals :root ${name} bridged to token layer`, v ?? "missing");
}

// ---------- 2. token dark anchors (preset converted) ----------
const tRoot = parseVars(tokensCss.slice(0, tokensCss.indexOf('[data-theme="dark"]')));
const tDark = parseVars(tokensCss.slice(tokensCss.indexOf('[data-theme="dark"]')));

const ANCHORS = {
  "--m-canvas-light": "#e8ebed",
  "--m-canvas-dark": "#1c2433",
  "--m-surface-dark": "#2a3040",
  "--m-overlay-dark": "#262b38",
  "--m-border-dark": "#3d4354",
  "--m-text-dark": "#e5e5e5",
  "--m-ember": "#e05d38",
  "--m-ember-dark": "#e05d38",
  "--m-ink": "#333333",
  "--m-graphite": "#4b5563",
  "--m-slate": "#6b7280",
  "--m-fog": "#dcdfe2",
  "--m-destructive": "#ef4444",
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
check((resolveDark(tDark.get("--m-canvas")) ?? "").toLowerCase() === "#1c2433",
  "dark --m-canvas resolves to #1C2433");
check((resolveDark(tDark.get("--m-surface")) ?? "").toLowerCase() === "#2a3040",
  "dark --m-surface resolves to #2A3040");
check((resolveDark(tDark.get("--m-text")) ?? "").toLowerCase() === "#e5e5e5",
  "dark --m-text resolves to #E5E5E5");
check((resolveDark(tDark.get("--m-border")) ?? "").toLowerCase() === "#3d4354",
  "dark --m-border resolves to #3D4354");

// ---------- 3. dark accent = light accent ----------
check(tDark.get("--m-accent") === "var(--m-ember-dark)",
  "dark --m-accent references --m-ember-dark (preset: primary identical both modes)");

// ---------- 4. chart ramp ----------
const CHARTS_LIGHT = {
  "--m-chart-1": "#86a7c8", "--m-chart-2": "#eea591", "--m-chart-3": "#5a7ca6",
  "--m-chart-4": "#466494", "--m-chart-5": "#334c82",
};
for (const [name, hex] of Object.entries(CHARTS_LIGHT)) {
  check((tRoot.get(name) ?? "").toLowerCase() === hex, `chart ramp ${name} = ${hex}`);
}
check((tDark.get("--m-chart-2") ?? "").toLowerCase() === "#e6a08f",
  "dark chart-2 = #E6A08F (preset dark variant)");
for (const name of ["--m-chart-1", "--m-chart-3", "--m-chart-4", "--m-chart-5"]) {
  check(tDark.get(name) === undefined, `dark block inherits ${name} (preset keeps light value)`);
}

// ---------- 5. retired Workbench hexes (CI blacklist) ----------
const BLACKLIST = ["D64B0A", "F4F5F7", "E8EBEF", "101216", "171A1F", "12161C"];
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
for (const file of scanned) {
  const content = read(file);
  for (const hex of BLACKLIST) {
    if (new RegExp(`#${hex}\\b`, "i").test(content)) offenders.push(`${file}: #${hex}`);
  }
}
check(offenders.length === 0, "no retired Workbench hexes in scanned UI files", offenders.join("; "));

// ---------- 6. craft rules ----------
check(componentsCss.includes("border-radius: 999px"), "components.css pill tags (999px)");
check(/font-family:\s*var\(--m-font-heading/.test(componentsCss),
  "components.css headings reference --m-font-heading (Source Serif 4)");
check(/--m-radius-control:\s*0\.75rem/.test(tokensCss), "tokens.css radius 0.75rem");
check(/--m-duration:\s*120ms/.test(tokensCss), "motion law token 120ms present");
check(componentsCss.includes("prefers-reduced-motion"), "prefers-reduced-motion honored");

const layoutTsx = read(resolve(root, "apps/web/app/layout.tsx"));
check(layoutTsx.includes("Source_Serif_4") && layoutTsx.includes('"--font-heading"'),
  "next/font serves Source Serif 4 as --font-heading");
check(layoutTsx.includes("JetBrains_Mono") && layoutTsx.includes('"--font-mono"'),
  "next/font serves JetBrains Mono as --font-mono");
check(layoutTsx.includes("Inter") && layoutTsx.includes('"--font-sans"'),
  "next/font serves Inter as --font-sans");

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
check(mockup.includes("999px"), "mockup pill tags (999px)");
check(mockup.includes('"Source Serif 4"'), "mockup Source Serif 4 headings");
check(/#e05d38/i.test(mockup), "mockup tangerine primary");
check(/--radius:\s*0\.75rem/i.test(mockup), "mockup radius 0.75rem");
check(/#e8ebed/i.test(mockup), "mockup warm-neutral canvas");
check(/Tangerine shadcn preset design rules/i.test(mockup), "mockup design-rules comment block");

// ---------- summary ----------
console.log(`\n=== Meridian Theme Review — value-level (p15-theme-tangerine) ===`);
console.log(`Preset:  design/tangerine/tangerine.json (${Object.keys(preset.cssVars.light).length} light / ${Object.keys(preset.cssVars.dark).length} dark vars)`);
console.log(`Passed:  ${passes} checks`);
console.log(`Failed:  ${errors.length} checks`);
if (errors.length > 0) {
  console.log("\nFAILURES:");
  for (const e of errors) console.log("  ✗ " + e);
  process.exit(1);
}
console.log("\nAll checks passed — generated layers match the canonical Tangerine preset.");
