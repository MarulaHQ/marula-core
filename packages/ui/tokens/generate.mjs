#!/usr/bin/env node
/**
 * Marula token generator (Wave 41 lane 20, p41-ui-kit-sweep).
 *
 * `tokens.json` is the only hand-edited token file. This script turns it into
 *   tokens.css   the shadcn variable contract plus --mc-*
 *   tokens.ts    the same values for React Native
 *   CONTRAST.md  the WCAG pair table
 * theme.css is a static Tailwind 4 mapping and ships unchanged.
 *
 * Run: node tokens/generate.mjs        (writes the three outputs)
 *      node tokens/generate.mjs --check (fails if an output is stale)
 *
 * The kit at internal/design/marula-handoff-kit/tokens/ is the reference these outputs
 * were reconciled against; this generator reproduces it.
 */
import { readFileSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const check = process.argv.includes("--check");

const themes = JSON.parse(readFileSync(join(here, "tokens.json"), "utf8"));
const map = JSON.parse(readFileSync(join(here, "token-map.json"), "utf8"));

const THEME_ORDER = ["neutral", "dark", "light"];
const camel = (s) => s.replace(/-([a-z])/g, (_, c) => c.toUpperCase());

/** shadcn vars derived from the pack's own columns, so no value is written twice. */
function derived(t) {
  return [
    ["--card-foreground", t.fg],
    ["--popover", t.surf],
    ["--popover-foreground", t.fg],
    ["--secondary", t.muted],
    ["--secondary-foreground", t.fg],
    ["--accent", t.muted],
    ["--accent-foreground", t.fg],
    ["--ring", t.pr],
  ];
}

function selector(name) {
  if (name === "neutral") return ':root,\n:root[data-theme="neutral"]';
  if (name === "light") return ':root[data-theme="light"],\n:root[data-theme="sand"],\n:root[data-theme="cream"]';
  return `:root[data-theme="${name}"]`;
}

const HEADER = `/* Marula tokens. Generated from tokens.json: do not edit by hand.
 *
 * Contract: the shadcn/ui variable names (--background, --primary, ...) carry our values, so every shadcn
 * component themes itself. Everything shadcn has no slot for is a --mc-* variable.
 *
 * Two mappings that surprise people:
 *   --accent        is our HOVER SURFACE (our "muted"), because shadcn uses accent for hover. It is not gold.
 *   --mc-act        is our GOLD TINT ("where you act"): selected chips, pressed filters, the current step.
 * Themes: neutral (staff app default), dark, light (the warm cream theme: mobile light, the client portal's "sand",
 * and the marketing site's "cream"). Switch with data-theme on any ancestor.
 */`;

function tokensCss() {
  const blocks = THEME_ORDER.map((name) => {
    const t = themes[name];
    const scheme = name === "dark" ? "dark" : "light";
    const rows = [];
    // shadcn-mapped keys first, in token-map order; then the --mc-* keys.
    for (const [key, css] of Object.entries(map)) {
      if (!css.startsWith("--mc-")) rows.push([css, t[key]]);
    }
    for (const [key, css] of Object.entries(map)) {
      if (css.startsWith("--mc-")) rows.push([css, t[key]]);
    }
    for (const [css, value] of derived(t)) rows.push([css, value]);
    return `${selector(name)}{\n  color-scheme:${scheme};\n${rows.map(([k, v]) => `  ${k}:${v};`).join("\n")}\n}`;
  }).join("\n");

  const brand = `/* Brand colours that never change with the theme (logo, avatars, the timer clock, fixed accents) */
:root{
  --mc-gold:#E8B83A; --mc-blush:#D9822B; --mc-cream:#F6EFD9; --mc-canopy:#5E7F3A; --mc-lime:#A9C25D;
  --mc-sand:#CDB38B; --mc-bark:#7C746A; --mc-ink:#2B2A33; --mc-avatar:#4A6A2A; --mc-avatar-fg:#F6EFD9;
  --mc-radius-ctl:12px; --mc-radius-field:10px; --mc-radius-card:16px; --mc-radius-modal:20px; --radius:12px;
}
/* Density: set data-density on the app root. Every table, toolbar and control reads these. */
:root,:root[data-density="comfortable"]{--mc-ctl:44px;--mc-row:48px;--mc-fs:14px;--mc-px:12px;--mc-hdr:44px;--mc-av:26px;--mc-pill:24px}
:root[data-density="compact"]{--mc-ctl:36px;--mc-row:40px;--mc-fs:14px;--mc-px:12px;--mc-hdr:40px;--mc-av:24px;--mc-pill:22px}
:root[data-density="dense"]{--mc-ctl:32px;--mc-row:32px;--mc-fs:13px;--mc-px:10px;--mc-hdr:32px;--mc-av:20px;--mc-pill:20px}`;

  return `${HEADER}\n${blocks}\n\n${brand}\n`;
}

function tokensTs() {
  const themeObj = THEME_ORDER.map((name) => {
    const rows = Object.entries(themes[name]).map(([k, v]) => `    ${camel(k)}: ${JSON.stringify(v)},`);
    return `  ${name}: {\n${rows.join("\n")}\n  },`;
  }).join("\n");

  const cssVar = Object.entries(map).map(([k, v]) => `  ${JSON.stringify(camel(k))}: ${JSON.stringify(v)}`).join(",\n");

  return `// Generated from tokens.json. Use in React Native, where CSS variables are not available.
// Names are the original short names, camelCased (surf, pr, prf, st-br-f -> stBrF); cssVar maps each to its CSS variable.
export const themes = {
${themeObj}
} as const
export type ThemeName = keyof typeof themes
export type Theme = (typeof themes)[ThemeName]
export const cssVar = {
${cssVar}
} as const
export const brand = { gold:"#E8B83A", blush:"#D9822B", cream:"#F6EFD9", canopy:"#5E7F3A", lime:"#A9C25D", sand:"#CDB38B", bark:"#7C746A", ink:"#2B2A33", avatar:"#4A6A2A", avatarFg:"#F6EFD9" } as const
export const density = {
  comfortable:{ctl:44,row:48,fs:14,px:12,hdr:44,av:26,pill:24},
  compact:{ctl:36,row:40,fs:14,px:12,hdr:40,av:24,pill:22},
  dense:{ctl:32,row:32,fs:13,px:10,hdr:32,av:20,pill:20},
} as const
export const radius = { ctl:12, field:10, card:16, modal:20, pill:999 } as const
/** Minimum sizes: text never below 12, tap targets 44 (52 for primary actions on mobile). */
export const min = { text:12, tap:44, tapPrimary:52 } as const
`;
}

/** sRGB relative luminance, WCAG 2.x. */
function luminance(hex) {
  const m = hex.replace("#", "");
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(m.slice(i, i + 2), 16) / 255);
  const f = (c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}
function ratio(a, b) {
  const [l1, l2] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
}

const PAIRS = [
  ["fg", "bg", "Body text on the page"],
  ["fg", "surf", "Text on a card"],
  ["sub", "bg", "Secondary text on the page"],
  ["sub", "surf", "Secondary text on a card"],
  ["prf", "pr", "Primary button"],
  ["accf", "acc", "Gold-tint chip and pressed filter"],
  ["duef", "dueb", "Attention (blush) pill and strip"],
  ["gtext", "gsoft", "Green pill"],
  ["st-new-f", "st-new-b", "Status: New"],
  ["st-wait-f", "st-wait-b", "Status: Waiting"],
  ["st-sched-f", "st-sched-b", "Status: Scheduled"],
  ["st-res-f", "st-res-b", "Status: Resolved"],
  ["st-br-f", "st-br-b", "Status: Breached"],
  ["redf", "red", "Filled red (destructive, P1)"],
  ["notef", "note", "Internal note"],
  ["asf", "asb", "Assistant card text"],
  ["asbtnf", "asbtn", "Assistant card button"],
  ["goldtext", "bg", "Gold text on the page"],
  ["timef", "time", "Time entry"],
  ["progf", "progb", "In progress pill"],
];

function contrastMd() {
  const rows = PAIRS.map(([fgKey, bgKey, use]) => {
    const cols = THEME_ORDER.map((name) => ratio(themes[name][fgKey], themes[name][bgKey]).toFixed(2));
    const key = `\`${fgKey}\` on \`${bgKey}\``;
    return `| ${key} | ${use} | ${cols[0]} | ${cols[1]} | ${cols[2]} |`;
  });
  let pass = 0;
  for (const [fgKey, bgKey] of PAIRS) {
    for (const name of THEME_ORDER) {
      if (ratio(themes[name][fgKey], themes[name][bgKey]) >= 4.5) pass++;
    }
  }
  const total = PAIRS.length * THEME_ORDER.length;
  return `# Contrast report

WCAG 2.x ratios for the pairs the design actually uses. AA needs 4.5 for text. Generated from \`tokens.json\`.

| Pair | Use | Neutral | Dark | Light |
|---|---|---|---|---|
${rows.join("\n")}

${pass} of ${total} pairs pass AA.
`;
}

const outputs = {
  "tokens.css": tokensCss(),
  "tokens.ts": tokensTs(),
  "CONTRAST.md": contrastMd(),
};

let stale = 0;
for (const [file, content] of Object.entries(outputs)) {
  const path = join(here, file);
  if (check) {
    let current = "";
    try {
      current = readFileSync(path, "utf8");
    } catch {
      /* missing counts as stale */
    }
    if (current !== content) {
      console.error(`STALE: tokens/${file}`);
      stale++;
    }
  } else {
    writeFileSync(path, content);
    console.log(`wrote tokens/${file}`);
  }
}

// packages/ui-native/src/generated.ts is the same token data for React
// Native, where CSS variables do not exist. Native and web therefore read the
// same values from the same file.
const nativePath = join(here, "..", "..", "ui-native", "src", "generated.ts");
if (check) {
  let current = "";
  try {
    current = readFileSync(nativePath, "utf8");
  } catch {
    /* missing counts as stale */
  }
  if (current !== outputs["tokens.ts"]) {
    console.error("STALE: packages/ui-native/src/generated.ts");
    stale++;
  }
} else {
  writeFileSync(nativePath, outputs["tokens.ts"]);
  console.log("wrote packages/ui-native/src/generated.ts");
}

// src/tokens-kit.css is the same generated token block, shipped inside the
// package so `@marula-core/ui/tokens.css` can import it. The app's `--m-*`
// layer aliases these names rather than restating their values.
const kitCssPath = join(here, "..", "src", "tokens-kit.css");
if (check) {
  let current = "";
  try {
    current = readFileSync(kitCssPath, "utf8");
  } catch {
    /* missing counts as stale */
  }
  if (current !== outputs["tokens.css"]) {
    console.error("STALE: src/tokens-kit.css");
    stale++;
  }
} else {
  writeFileSync(kitCssPath, outputs["tokens.css"]);
  console.log("wrote src/tokens-kit.css");
}

if (check) {
  if (stale) {
    console.error(`\nFAIL: ${stale} generated token file(s) are stale. Run: pnpm --filter @marula-core/ui tokens`);
    process.exit(1);
  }
  console.log("OK: generated token files are current.");
}
