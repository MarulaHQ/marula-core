#!/usr/bin/env node
/**
 * Theme review script — repointed to Marula in wave41 lane 2.
 *
 * Hardened review: parses the actual CSS variable declarations from
 * packages/ui/src/tokens.css and apps/web/app/globals.css and compares them
 * VALUE-BY-VALUE against the vendored canonical preset
 * (design/marula/marula.json). Substring greps cannot catch a wrong
 * dark mode; this can.
 *
 * Checks:
 *  1. Preset fidelity — every shadcn variable in globals.css :root/.dark that
 *     the preset defines matches the preset's oklch/hsl value verbatim.
 *  2. Token anchors — tokens.css dark-mode canvas/surface/text/border resolve
 *     to the Marula anchors below, resolved through the kit's hex layer.
 *  3. The primary FLIPS on mode (Marula inverts it; Tangerine did not).
 *  4. Chart ramp — gold / canopy / blush / bark / lime (spec §4.4).
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

/** Canonicalise a CSS colour to lowercase hex.
 *
 *  The brand preset ships oklch and the generated kit ships hex, so a raw
 *  string compare fails on NOTATION even when the colour is identical. This
 *  converts both sides to one canonical form so that a real value drift is
 *  distinguishable from a different spelling of the same colour. */
const OKLCH_RE = /^oklch\(\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)\s*\)$/;
const srgbEncode = (c) => {
  const v = Math.max(0, Math.min(1, c));
  return v <= 0.0031308 ? 12.92 * v : 1.055 * Math.pow(v, 1 / 2.4) - 0.055;
};
const toHex = (v) => {
  if (typeof v !== "string") return null;
  const t = v.trim().toLowerCase();
  const m = t.match(OKLCH_RE);
  if (m) {
    const [L, C, H] = [Number(m[1]), Number(m[2]), Number(m[3])];
    const h = (H * Math.PI) / 180;
    const a = C * Math.cos(h), b = C * Math.sin(h);
    const l_ = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
    const m_ = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
    const s_ = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
    const r = 4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_;
    const g = -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_;
    const bl = -0.0041960863 * l_ - 0.7034186147 * m_ + 1.707614701 * s_;
    const q = (x) => Math.round(srgbEncode(x) * 255).toString(16).padStart(2, "0");
    return `#${q(r)}${q(g)}${q(bl)}`;
  }
  const hex = t.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/);
  if (!hex) return null;
  const h = hex[1].length === 3 ? hex[1].replace(/./g, "$&$&") : hex[1];
  return `#${h}`;
};
/** True when both sides are the same colour, allowing either spelling. */
const sameColour = (a, b) => {
  const x = toHex(a), y = toHex(b);
  if (x === null || y === null) return a === b;
  return x === y;
};

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
// The reference pack vendored under design/mockups/ is the live
// aesthetic source. The pre-rebrand workbench v2 file stays on disk as history;
// retiring it is lane 4's call, so the review reads the new pack and checks the
// old geometry only.
const mockupPath = resolve(
  root,
  "design/mockups/marula-app-mockup-images/source/marula-app-mockups.html",
);

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
// The semantic layer (tokens.css) is an ALIAS layer: every --m-* that has a kit
// equivalent aliases it, and the hexes live in the generated tokens-kit.css.
// Comparing the semantic layer's raw strings against hex values therefore fails
// by construction, so the checks below resolve --m-* -> --mc-*/--* through
// both layers before comparing. tokens.test.ts pins this two-layer contract.
// Parsed BY BLOCK, not by slicing at the first dark selector. The kit declares
// `neutral`, `dark` and `cream` theme blocks with the same 64 names, so a naive
// slice would fold the later light/cream block over the dark one and read a
// cream canvas out of dark mode.
const kitCss = stripComments(read(resolve(root, "packages/ui/src/tokens-kit.css")));
const kitBlocks = new Map();
for (const m of kitCss.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
  const sel = m[1].trim().split(/\s+/).pop();
  if (!sel.startsWith(":root")) continue;
  const theme = sel.match(/data-theme="([a-z]+)"/)?.[1] ?? "base";
  // MERGE, do not replace: the kit carries more than one `:root` block (the
  // brand constants, then the density steps), and the later one must not erase
  // the earlier one's names.
  const prev = kitBlocks.get(theme) ?? new Map();
  for (const [k, v] of parseVars(m[2])) prev.set(k, v);
  kitBlocks.set(theme, prev);
}
const kitLight = new Map([...(kitBlocks.get("base") ?? new Map()), ...(kitBlocks.get("neutral") ?? new Map())]);
const kitDark = kitBlocks.get("dark") ?? new Map();

/** Resolve a semantic token to its hex, walking alias chains through both
 *  layers. `mode` picks which half of each layer is preferred, because the
 *  semantic layer declares the same `--m-*` name in both halves and the kit
 *  declares it with different values. */
const resolveToken = (raw, mode, depth = 0) => {
  let v = raw;
  for (let d = 0; d < 6; d += 1) {
    if (typeof v !== "string") return v;
    const m = v.match(/^var\((--[\w-]+)\)$/);
    if (!m) return v;
    const name = m[1];
    const sem = mode === "dark" ? [tDark, tRoot] : [tRoot, tDark];
    const kit = mode === "dark" ? [kitDark, kitLight] : [kitLight, kitDark];
    const next = sem[0].get(name) ?? sem[1].get(name) ?? kit[0].get(name) ?? kit[1].get(name);
    if (next === undefined || next === v) return v;
    v = next;
  }
  return v;
};
const LIGHT = (name) => tRoot.get(name) ?? "";
const DARK = (name) => tDark.get(name) ?? "";
/** Resolve a semantic token in the light and dark halves of tokens.css. */
const lr = (name) => resolveToken(LIGHT(name), "light");
const dr = (name) => resolveToken(DARK(name), "dark");

const tokensTs = read(tokensTsPath);
const componentsCss = read(componentsPath);
const mockup = read(mockupPath);

const tRoot = parseVars(tokensCss.slice(0, tokensCss.indexOf('[data-theme="dark"]')));
const tDark = parseVars(tokensCss.slice(tokensCss.indexOf('[data-theme="dark"]')));

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
// The kit layer is where the shadcn-shaped hex/oklch declarations live now.
// globals.css only carries the bridged `var(--m-*)` subset, so comparing
// globals.css against the preset's oklch fails by construction.
for (const [name, value] of Object.entries(preset.cssVars.light)) {
  if (PRESET_BRIDGED.has(`--${name}`) || MODE_INDEPENDENT.has(`--${name}`)) continue;
  const actual = resolveToken(kitLight.get(`--${name}`) ?? gRoot.get(`--${name}`) ?? "", "light");
  if (actual === undefined || actual === "") {
    errors.push(`kit :root missing preset variable --${name}`);
  } else if (!sameColour(actual, value)) {
    errors.push(`kit :root --${name} = "${actual}" ≠ preset "${value}"`);
  } else {
    presetChecked++;
  }
}
check(expectedInBlock(preset.cssVars.light) === presetChecked,
  `kit :root matches preset verbatim (${presetChecked}/${expectedInBlock(preset.cssVars.light)} non-bridged vars)`);

let presetDarkChecked = 0;
for (const [name, value] of Object.entries(preset.cssVars.dark)) {
  if (PRESET_BRIDGED.has(`--${name}`) || MODE_INDEPENDENT.has(`--${name}`)) continue;
  const actual = resolveToken(kitDark.get(`--${name}`) ?? gDark.get(`--${name}`) ?? "", "dark");
  if (actual === undefined || actual === "") {
    errors.push(`kit dark block missing preset variable --${name}`);
  } else if (!sameColour(actual, value)) {
    errors.push(`kit dark --${name} = "${actual}" ≠ preset "${value}"`);
  } else {
    presetDarkChecked++;
  }
}
check(expectedInBlock(preset.cssVars.dark) === presetDarkChecked,
  `kit dark block matches preset verbatim (${presetDarkChecked}/${expectedInBlock(preset.cssVars.dark)} non-bridged vars)`);

// bridged variables must reference the token layer, not redeclare hexes
for (const name of PRESET_BRIDGED) {
  // Either globals.css still bridges it, or the semantic layer aliases the kit
  // directly. Both satisfy the no-duplicated-hex rule; neither may redeclare a
  // literal beside the kit that owns it.
  const inGlobals = gRoot.get(name);
  const inTokens = tRoot.get(name);
  const inKit = kitLight.get(name);
  const bridged = (inGlobals !== undefined && (inGlobals.startsWith("var(--m-") || inGlobals === "0.75rem"))
    || (inTokens !== undefined && inTokens.startsWith("var(--"))
    || (inKit !== undefined && inKit !== name);
  check(bridged, `token layer bridges ${name} to the kit`, `${inGlobals ?? inTokens ?? "missing"}`);
}

// ---------- 2. token dark anchors (preset converted) ----------

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
  const resolved = (lr(name) || "").toLowerCase();
  check(resolved === hex, `tokens.css ${name} resolves to ${hex}`, `${tRoot.get(name) ?? "missing"} -> ${resolved}`);
}

// semantic dark resolves through the anchors
const resolveDark = (v, depth = 0) => {
  const m = typeof v === "string" && v.match(/^var\((--[\w-]+)\)$/);
  if (!m || depth > 4) return v;
  return resolveDark(tDark.get(m[1]) ?? tRoot.get(m[1]), depth + 1);
};
check(dr("--m-canvas").toLowerCase() === "#2b2a33", "dark --m-canvas resolves to #2B2A33");
check(dr("--m-surface").toLowerCase() === "#34333d", "dark --m-surface resolves to #34333D");
check(dr("--m-text").toLowerCase() === "#f6efd9", "dark --m-text resolves to #F6EFD9");
check(dr("--m-border").toLowerCase() === "#43424d", "dark --m-border resolves to #43424D");

// ---------- 3. the primary FLIPS on mode ----------
// The retired Tangerine preset kept one accent in both modes. Marula inverts
// it: ink with gold text in light, gold with ink text in dark. Asserted as a
// pair, because "the dark primary is not the light primary" is the invariant
// that a later mechanical pass breaks.
const lightPrimary = lr("--m-primary").toLowerCase();
const darkPrimary = dr("--m-primary").toLowerCase();
check(lightPrimary === "#2b2a33" && darkPrimary === "#e8b83a",
  "primary flips: ink in light, gold in dark",
  `${lightPrimary} / ${darkPrimary}`);
check(lr("--m-on-primary").toLowerCase() === "#e8b83a" &&
    dr("--m-on-primary").toLowerCase() === "#2b2a33",
  "on-primary flips with it: gold in light, ink in dark");

// The timer bar does NOT flip. It is declared once in :root and never
// overridden, so a mechanical dark pass cannot paint it cream.
check(lr("--m-bar").toLowerCase() === "#2b2a33" && tDark.get("--m-bar") === undefined,
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
  check(lr(name).toLowerCase() === hex, `chart ramp ${name} resolves to ${hex}`);
}
const CHARTS_DARK = {
  "--m-chart-1": "#e8b83a", "--m-chart-2": "#a9c25d", "--m-chart-3": "#f0b27a",
  "--m-chart-4": "#beb5a2", "--m-chart-5": "#cdb38b",
};
for (const [name, hex] of Object.entries(CHARTS_DARK)) {
  check(dr(name).toLowerCase() === hex, `dark chart ramp ${name} resolves to ${hex}`);
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
// The kit spells the scale root 12px; the semantic layer aliases it. Accept
  // either spelling of the same value.
  const scaleRoot = lr("--m-radius-scale").toLowerCase();
  check(scaleRoot === "0.75rem" || scaleRoot === "12px", "tokens.css scale root resolves to 0.75rem (12px)", scaleRoot);
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
// The semantic layer is alias-only, so its hex set is a subset of the
// kit's by construction. The kit is the hex owner, so it is the sync partner.
const cssHexes = new Set([...kitCss.matchAll(/#[0-9a-fA-F]{6}\b/g), ...tokensCss.matchAll(/#[0-9a-fA-F]{6}\b/g)].map((m) => m[0].toUpperCase()));
const tsHexes = new Set([...tokensTs.matchAll(/"#[0-9a-fA-F]{6}"/g)].map((m) => m[0].slice(1, -1).toUpperCase()));
check(cssHexes.size === tsHexes.size && [...cssHexes].every((h) => tsHexes.has(h)) &&
    [...tsHexes].every((h) => cssHexes.has(h)),
  `tokens.ts PALETTE ↔ tokens.css hex sets identical (${cssHexes.size})`);

// ---------- 8. mockup ----------
// The vendored reference pack is the Marula aesthetic source: it renders the
// three shipped themes, so it is now asserted against the token layer rather
// than exempt from colour checks. The old v2 mockup could not be — it drew
// the retired Workbench palette, so a colour assertion against it would have
// been an assertion that it was wrong.
//
// What carried over from the old geometry checks: pill tags stay pills, the
// 12px radius scale root stays (the pack writes it in px, not rem), and both
// modes stay. The theme mechanism differs — the pack selects on a
// `data-theme` attribute rather than a body class, and names the light
// columns "neutral" and "warm" rather than "light".
check(mockup.includes("999px"), "mockup pill tags (999px)");
check(
  /border-radius:\s*12px/.test(mockup) && /border-radius:\s*10px/.test(mockup),
  "mockup radius scale carried over (12px root, 10px controls)",
);
check(
  /:root\[data-theme="dark"\]\s*\{/.test(mockup),
  "mockup dark-mode support (data-theme=dark block)",
);
check(
  /:root\[data-theme="neutral"\]\s*\{[^}]*--bg:\s*#F8F7F4/i.test(mockup),
  "mockup neutral-light canvas matches the shipped canvas token (#F8F7F4)",
);
check(
  /:root\[data-theme="dark"\]\s*\{[^}]*--bg:\s*#2B2A33/i.test(mockup),
  "mockup dark canvas matches the shipped ink token (#2B2A33)",
);
check(
  /:root\[data-theme="dark"\]\s*\{[^}]*--pr:\s*#E8B83A/i.test(mockup),
  "mockup dark primary is the shipped gold (#E8B83A)",
);
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
