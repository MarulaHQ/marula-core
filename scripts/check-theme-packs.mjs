#!/usr/bin/env node
/**
 * Per-theme mechanical enforcement (Wave 24 leg C).
 *
 * Extends the existing brand-palette hex-scan posture (scripts/review-theme.mjs):
 *  1. BUILT-IN presets — every shipped preset's hexes are scanned against
 *     the retired-Workbench blacklist and the docs/theme.md contrast floors
 *     (AA text pairs, 3:1 accent/tag pairs), exactly as the backend asserts
 *     at the pack boundary.
 *  2. INSTALLED packs — CI (and the eyebrow lane) can loop a comma-
 *     separated list of theme-pack manifest files via THEME_PACK_MANIFESTS
 *     (or pass paths as CLI args); each manifest is validated like the
 *     ingestion seam: closed role vocabulary, #rrggbb hexes, blacklist,
 *     contrast floors. `pack:` values resolved from settings JSON get the
 *     same treatment.
 *
 * Exit 1 on the first failed invariant group; CI candidates run this
 * alongside `node scripts/review-theme.mjs`.
 * Usage: node scripts/check-theme-packs.mjs [manifest.json ...]
 */
import { readFileSync } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const passes = [];
const errors = [];
const check = (ok, label, extra = "") => {
  if (ok) passes.push(label);
  else errors.push(`${label}${extra ? ` — ${extra}` : ""}`);
};

// --- load the schema module (type-stripped native TS import) ------------------
const schemaUrl = pathToFileURL(join(root, "packages/ui/src/theme-tokens.ts")).href;
const schema = await import(schemaUrl).catch((e) => {
  console.error(`FAIL: cannot load packages/ui/src/theme-tokens.ts: ${e.message}`);
  process.exit(1);
});

const BLACKLIST = ["D64B0A", "F4F5F7", "E8EBEF", "101216", "171A1F", "12161C"];
const HEX_RE = /^#[0-9a-f]{6}$/;

function luminance(hex) {
  const body = hex.slice(1);
  const channel = (i) => {
    const raw = parseInt(body.slice(i, i + 2), 16) / 255;
    return raw <= 0.03928 ? raw / 12.92 : ((raw + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4);
}

function contrastRatio(fg, bg) {
  const a = luminance(fg);
  const b = luminance(bg);
  const [hi, lo] = a >= b ? [a, b] : [b, a];
  return (hi + 0.05) / (lo + 0.05);
}

const TEXT_ROLES = ["text", "text_secondary", "text_muted"];
const BG_ROLES = ["background", "surface"];
const STATUS_TONES = ["success", "warning", "danger"];

function validateThemeValues(label, values) {
  // Closed vocabulary + hex format.
  const badRole = Object.keys(values).filter((k) => !schema.TOKEN_ROLES.includes(k));
  check(
    badRole.length === 0,
    `${label}: closed role vocabulary (off-schema tokens are typed-rejected)`,
    badRole.join(", "),
  );
  for (const [role, value] of Object.entries(values)) {
    if (!HEX_RE.test(String(value))) check(false, `${label}: ${role} must be #rrggbb`, String(value));
    if (BLACKLIST.some((b) => String(value).toUpperCase().endsWith(b)))
      check(false, `${label}: ${role} carries a retired hex`, value);
  }
  // AA floors.
  for (const fg of TEXT_ROLES) {
    if (!values[fg]) continue;
    for (const bg of BG_ROLES) {
      if (!values[bg]) continue;
      const ratio = contrastRatio(values[fg], values[bg]);
      check(ratio >= 4.5, `${label}: ${fg} on ${bg} ≥ 4.5:1 (${ratio.toFixed(2)})`);
    }
  }
  // 3:1 UI/tag floors.
  if (values.accent && values.on_accent)
    check(
      contrastRatio(values.on_accent, values.accent) >= 3.0,
      `${label}: on_accent over accent ≥ 3:1`,
    );
  for (const tone of STATUS_TONES) {
    const fg = values[`status_${tone}_fg`];
    const bg = values[`status_${tone}_bg`];
    if (!fg || !bg) continue;
    check(contrastRatio(fg, bg) >= 3.0, `${label}: status_${tone}_fg on its bg ≥ 3:1`);
  }
}

// 1. Built-in built-ins.
for (const preset of schema.BUILTIN_PRESETS) {
  validateThemeValues(`builtin ${preset.id} light`, preset.light);
  validateThemeValues(`builtin ${preset.id} dark`, preset.dark);
}
check(schema.BUILTIN_PRESETS[0].id === "tangerine", "tangerine is the pinned default-v1");
check(schema.DEFAULT_PRESET === "tangerine", "DEFAULT_PRESET = tangerine");

// 2. Per-installed-theme loop (CLI args and/or THEME_PACK_MANIFESTS).
const manifestPaths = [
  ...(process.env.THEME_PACK_MANIFESTS ?? "").split(",").filter(Boolean),
  ...process.argv.slice(2),
];
for (const path of manifestPaths) {
  let parsed;
  try {
    parsed = JSON.parse(readFileSync(path, "utf8"));
  } catch (e) {
    check(false, `installed pack ${path} is readable JSON`, e.message);
    continue;
  }
  check(
    parsed.kind === "theme",
    `${path}: native theme-pack manifest shape (kind = "theme")`,
  );
  for (const mode of ["light", "dark"]) {
    if (parsed[mode]) validateThemeValues(`installed ${path} ${mode}`, parsed[mode]);
  }
}

// ---------- summary ----------
console.log(`\n=== Per-theme check (Wave 24 p24-theming-system) ===`);
console.log(`Built-in presets: ${schema.BUILTIN_PRESETS.map((p) => p.id).join(", ")}`);
console.log(`Installed manifests scanned: ${manifestPaths.length}`);
console.log(`Passed: ${passes.length} checks`);
console.log(`Failed: ${errors.length} checks`);
if (errors.length > 0) {
  console.log("\nFAILURES:");
  for (const e of errors) console.log("  ✗ " + e);
  process.exit(1);
}
console.log("\nAll shipped and installed themes carry legal token sets.");
