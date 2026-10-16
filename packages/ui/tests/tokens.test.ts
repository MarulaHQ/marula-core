/**
 * Token contract tests — the mechanical enforcement of the kit's token layer.
 *
 * The hex source of truth is the generated `src/tokens-kit.css` (from
 * tokens/tokens.json, and identical to tokens/tokens.css). `src/tokens.css` is
 * the app's compatibility layer: every token with a kit equivalent ALIASES it,
 * `--m-x: var(--kit)`, so no value is ever written twice.
 *
 * Non-purpose: no component behaviour (see tests/components).
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { PALETTE, TOKEN, contrastRatio, STATUS_TONES } from "../src/tokens.js";
import { DENSITY_STEPS, DENSITY_STEP_SPEC, DEFAULT_DENSITY } from "../src/density.js";

const src = join(process.cwd(), "src");
const strip = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, "");
const read = (f: string) => strip(readFileSync(join(src, f), "utf8"));
const tokensCss = read("tokens.css");
const kitCss = read("tokens-kit.css");
const componentsCss = read("components.css");
const themes = JSON.parse(readFileSync(join(process.cwd(), "tokens/tokens.json"), "utf8")) as Record<string, Record<string, string>>;
const TOKEN_MAP = JSON.parse(readFileSync(join(process.cwd(), "tokens/token-map.json"), "utf8")) as Record<string, string>;

const DARK = '[data-theme="dark"]';
const HEX_RE = /#[0-9a-fA-F]{6}\b/g;
const parseVars = (css: string): Map<string, string> => {
  const m = new Map<string, string>();
  for (const x of css.matchAll(/(--[\w-]+):\s*([^;]+);/g)) m.set(x[1]!, x[2]!.trim());
  return m;
};

// --- theme blocks -----------------------------------------------------------------
const DARK_AT = kitCss.indexOf('[data-theme="dark"]');
const LIGHT_AT = kitCss.indexOf('[data-theme="light"]');
const BRAND_AT = kitCss.indexOf(":root{", LIGHT_AT);
const DENSITY_AT = kitCss.indexOf(":root,:root[data-density", BRAND_AT);
const kitNeutral = parseVars(kitCss.slice(0, DARK_AT));
const kitDark = parseVars(kitCss.slice(DARK_AT, LIGHT_AT));
const kitBrand = parseVars(kitCss.slice(BRAND_AT, DENSITY_AT));
const landedRoot = parseVars(tokensCss.slice(0, tokensCss.indexOf(DARK)));
const landedDark = parseVars(tokensCss.slice(tokensCss.indexOf(DARK)));

const mergedLight = new Map([...kitBrand, ...kitNeutral, ...landedRoot]);
const mergedDark = new Map([...kitBrand, ...kitDark, ...landedDark]);

function resolve(map: Map<string, string>, value: string, depth = 0): string {
  const m = value.match(/^var\((--[\w-]+)\)$/);
  if (m && depth < 6) {
    const next = map.get(m[1]!);
    if (next !== undefined) return resolve(map, next, depth + 1);
  }
  return value.trim().toUpperCase();
}

/** Landed --m-* name -> the kit variable it must alias, built from the token map. */
const ALIASES: Array<[string, string]> = [
  ["--m-canvas", "--background"], ["--m-surface", "--card"], ["--m-overlay-surface", "--popover"],
  ["--m-nav", "--mc-nav"], ["--m-sidebar", "--mc-side"], ["--m-surface-2", "--mc-surface-2"],
  ["--m-bar", "--mc-bar"], ["--m-text", "--foreground"], ["--m-text-secondary", "--muted-foreground"],
  ["--m-border", "--border"], ["--m-input", "--input"], ["--m-primary", "--primary"],
  ["--m-on-primary", "--primary-foreground"], ["--m-accent", "--mc-act"], ["--m-on-accent", "--mc-act-fg"],
  ["--m-accent-text", "--mc-gold-text"], ["--m-destructive", "--destructive"],
  ["--m-status-success-fg", "--mc-green-text"], ["--m-status-success-bg", "--mc-green-soft"],
  ["--m-status-warning-fg", "--mc-progress-fg"], ["--m-status-warning-bg", "--mc-progress"],
  ["--m-status-due-fg", "--mc-attn-fg"], ["--m-status-due-bg", "--mc-attn"],
  ["--m-status-neutral-fg", "--mc-note-fg"], ["--m-status-neutral-bg", "--mc-note"],
  ["--m-status-offline-fg", "--mc-off-fg"], ["--m-status-offline-bg", "--mc-off"],
  ["--m-status-danger-fg", "--destructive"], ["--m-client", "--mc-entry-client"],
  ["--m-sys", "--mc-entry-system"], ["--m-time", "--mc-entry-time"], ["--m-time-fg", "--mc-entry-time-fg"],
  ["--m-agent", "--mc-entry-agent"], ["--m-note", "--mc-note"], ["--m-note-fg", "--mc-note-fg"],
  ["--m-asb", "--mc-assist"], ["--m-asf", "--mc-assist-fg"], ["--m-asbtn", "--mc-assist-btn"],
  ["--m-asbtnf", "--mc-assist-btn-fg"], ["--m-asline", "--mc-assist-line"], ["--m-scrim", "--mc-scrim"],
];

describe("the kit's generated tokens are the hex source", () => {
  it("tokens-kit.css is the generated tokens/tokens.css", () => {
    expect(kitCss).toBe(strip(readFileSync(join(process.cwd(), "tokens/tokens.css"), "utf8")));
  });

  it("carries the vendor brand tokens the aliases target", () => {
    for (const v of ["--mc-gold", "--mc-cream", "--mc-ink", "--radius"]) {
      expect(kitBrand.has(v), v).toBe(true);
    }
  });

  it("holds to the 60 of 60 AA claim in CONTRAST.md", () => {
    const rows = readFileSync(join(process.cwd(), "tokens/CONTRAST.md"), "utf8").match(/of 60 pairs pass AA/);
    expect(rows).not.toBeNull();
  });
});

describe("every shared --m-* token ALIASES its kit token", () => {
  it.each(ALIASES)("%s is var(%s)", (landed, kitVar) => {
    expect(landedRoot.get(landed)).toBe(`var(${kitVar})`);
  });

  it("every alias target is defined in the kit tokens", () => {
    for (const [, kitVar] of ALIASES) {
      expect(kitNeutral.has(kitVar) || kitDark.has(kitVar) || kitBrand.has(kitVar), kitVar).toBe(true);
    }
  });
});

describe("resolved values equal the kit tokens.json", () => {
  it.each(Object.keys(TOKEN_MAP).filter((k) => themes.neutral![k] !== undefined))(
    "neutral: --m-* resolving kit %s matches tokens.json",
    (key) => {
      const landed = ALIASES.find(([, v]) => v === TOKEN_MAP[key]);
      if (!landed) return; // token has no app alias (unused by the app)
      expect(resolve(mergedLight, landedRoot.get(landed[0])!)).toBe(themes.neutral![key]!.toUpperCase());
    },
  );

  it("dark primary flips to gold and the assistant surface inverts", () => {
    expect(resolve(mergedDark, landedDark.get("--m-primary")!)).toBe(themes.dark!.pr!.toUpperCase());
    expect(resolve(mergedDark, landedDark.get("--m-asb")!)).toBe(themes.dark!.asb!.toUpperCase());
    expect(resolve(mergedLight, landedRoot.get("--m-asb")!)).toBe(themes.neutral!.asb!.toUpperCase());
  });
});

describe("contrast floors the app still owns", () => {
  it("keeps gold off light text entirely", () => {
    expect(contrastRatio(TOKEN.gold, "#FBF7EC")).toBeLessThan(3.0);
    expect(contrastRatio(TOKEN.goldText, "#FFFFFF")).toBeGreaterThanOrEqual(4.5);
  });

  it("the flipping primary clears 4.5 both ways", () => {
    expect(contrastRatio(TOKEN.gold, TOKEN.ink)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(TOKEN.ink, TOKEN.gold)).toBeGreaterThanOrEqual(4.5);
  });

  it("keeps the assistant divider decorative at the pack's own value", () => {
    expect(contrastRatio(TOKEN.textSecondary, TOKEN.ink)).toBeGreaterThanOrEqual(1.5);
    expect(contrastRatio(TOKEN.textSecondary, TOKEN.ink)).toBeLessThan(3.0);
  });

  it("holds the mockup status pairs at AA in every theme", () => {
    const pairs: Array<[string, string]> = [
      ["st-new-b", "st-new-f"],
      ["st-wait-b", "st-wait-f"],
      ["st-sched-b", "st-sched-f"],
      ["st-res-b", "st-res-f"],
      ["st-br-b", "st-br-f"],
    ];
    for (const theme of ["neutral", "dark", "light"]) {
      for (const [bgKey, fgKey] of pairs) {
        expect(
          contrastRatio(themes[theme]![fgKey]!, themes[theme]![bgKey]!),
          `${theme} ${fgKey} on ${bgKey}`,
        ).toBeGreaterThanOrEqual(4.5);
      }
    }
    expect(STATUS_TONES.length).toBeGreaterThan(0);
  });
});

describe("the app layer still keeps its own rules", () => {
  it("declares no retired Tangerine or Workbench hex", () => {
    const live = new Set([...tokensCss.matchAll(HEX_RE)].map((m) => m[0].toUpperCase()));
    for (const hex of ["#E05D38", "#E8EBED", "#1C2433", "#2A3040", "#3D4354", "#86A7C8", "#D64B0A", "#FF8A4C"]) {
      expect(live, `${hex} is retired`).not.toContain(hex);
    }
  });

  it("keeps components.css free of any raw hex", () => {
    expect([...componentsCss.matchAll(HEX_RE)].map((m) => m[0])).toEqual([]);
  });

  it("PALETTE still names the brand primitives", () => {
    expect(PALETTE.length).toBeGreaterThan(0);
    expect(PALETTE.map((h) => h.toUpperCase())).toContain(TOKEN.ink.toUpperCase());
  });
});

describe("density steps (wave41 D7)", () => {
  const block = (step: string): Map<string, string> => {
    const at = tokensCss.indexOf(`[data-density="${step}"]`);
    return parseVars(tokensCss.slice(at, tokensCss.indexOf("}", at)));
  };

  it("defaults to comfortable and offers compact and dense", () => {
    expect(DEFAULT_DENSITY).toBe("comfortable");
    expect(DENSITY_STEPS).toEqual(["comfortable", "compact", "dense"]);
  });

  it("pins each step to its spec triple", () => {
    for (const step of DENSITY_STEPS) {
      const spec = DENSITY_STEP_SPEC[step];
      const vars = block(step);
      expect(vars.get("--m-density-control")).toBe(`${spec.control}px`);
      expect(vars.get("--m-density-row")).toBe(`${spec.row}px`);
      expect(vars.get("--m-card-padding")).toBe(`${spec.card}px`);
    }
  });

  /**
   * The template's ladder is wider than the two values D7 landed with
   * (DESIGN-NOTES-tickets.md §4): text, cell inset, header height, avatar and
   * pill all move with the step. Until they did, `comfortable` rendered a table
   * whose text was a point tight and whose header never changed height.
   */
  it("carries the template's full ladder at every step", () => {
    const ladder: Record<string, Record<string, number>> = {
      comfortable: { fs: 14, px: 12, hdr: 44, av: 26, pill: 24 },
      compact: { fs: 14, px: 12, hdr: 40, av: 24, pill: 22 },
      dense: { fs: 13, px: 10, hdr: 32, av: 20, pill: 20 },
    };
    for (const step of DENSITY_STEPS) {
      const vars = block(step);
      for (const [key, value] of Object.entries(ladder[step] ?? {})) {
        expect(vars.get(`--m-density-${key}`)).toBe(`${value}px`);
      }
    }
  });

  it("never lets a step drop text below the 12px floor", () => {
    for (const step of DENSITY_STEPS) {
      const vars = block(step);
      const fs = Number.parseFloat(vars.get("--m-density-fs") ?? "0");
      expect(fs).toBeGreaterThanOrEqual(12);
    }
  });

  /** The app tokens and the kit tokens are two ladders for one setting; drift
   * between them renders half the table at one step and half at another. */
  it("agrees with the kit's own --mc-* ladder at every step", () => {
    const kit = (step: string): Map<string, string> => {
      const at = kitCss.indexOf(`data-density="${step}"`);
      // `parseVars` needs a terminator on every declaration; the kit's generated
      // blocks drop the last one, so close the slice before parsing it.
      return parseVars(`${kitCss.slice(at, kitCss.indexOf("}", at))};`);
    };
    const pairs: ReadonlyArray<[string, string]> = [
      ["--m-density-control", "--mc-ctl"],
      ["--m-density-row", "--mc-row"],
      ["--m-density-fs", "--mc-fs"],
      ["--m-density-px", "--mc-px"],
      ["--m-density-hdr", "--mc-hdr"],
      ["--m-density-av", "--mc-av"],
      ["--m-density-pill", "--mc-pill"],
    ];
    for (const step of DENSITY_STEPS) {
      const appVars = block(step);
      const kitVars = kit(step);
      for (const [appKey, kitKey] of pairs) {
        expect(appVars.get(appKey)).toBe(kitVars.get(kitKey));
      }
    }
  });

  it("a control in components.css is token-driven", () => {
    expect(componentsCss).toContain("height: var(--m-density-control)");
    expect(componentsCss).toContain("height: var(--m-density-row)");
    // The table's own type and inset read the step too.
    expect(componentsCss).toContain("font-size: var(--m-density-fs)");
    expect(componentsCss).toContain("height: var(--m-density-hdr)");
    expect(componentsCss).toContain("padding: 0 var(--m-density-px)");
  });
});
