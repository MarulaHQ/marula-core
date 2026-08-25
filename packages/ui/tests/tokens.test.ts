/**
 * Token contract tests — the mechanical enforcement of docs/theme.md.
 * Purpose: prove tokens.css is the single hex source, mirrors light/dark,
 * and meets WCAG floors for the pairs the spec pins.
 * Non-purpose: no component behavior testing here (see tests/components).
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { PALETTE, contrastRatio, STATUS_TONES } from "../src/tokens.js";

const src = join(process.cwd(), "src");
const tokensCss = readFileSync(join(src, "tokens.css"), "utf8");
const componentsCss = readFileSync(join(src, "components.css"), "utf8");

const DARK_SPLIT = '[data-theme="dark"]';

/** Comments may mention variable names; parse only real declarations. */
function stripComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, "");
}

const strippedCss = stripComments(tokensCss);
const rootBlock = strippedCss.slice(0, strippedCss.indexOf(DARK_SPLIT));
const darkBlock = strippedCss.slice(strippedCss.indexOf(DARK_SPLIT));

const HEX_RE = /#[0-9a-fA-F]{6}\b/g;

function declaredHexes(css: string): string[] {
  return [...css.matchAll(HEX_RE)].map((m) => m[0].toUpperCase());
}

/** Parse every custom-property declaration in a CSS block into a map. */
function parseVars(css: string): Map<string, string> {
  const vars = new Map<string, string>();
  for (const m of css.matchAll(/(--[\w-]+):\s*([^;]+);/g)) {
    vars.set(m[1]!, m[2]!.trim());
  }
  return vars;
}

/**
 * Resolve `var()` chains (up to 4 hops) against primitive definitions.
 * color-mix()/transparent values are returned as-is (excluded from WCAG math).
 */
function makeResolver(blocks: Array<Map<string, string>>) {
  return function resolve(value: string, depth = 0): string {
    const m = value.match(/^var\((--[\w-]+)\)$/);
    if (!m || depth > 3) {
      return value;
    }
    for (const block of blocks) {
      const next = block.get(m[1]!);
      if (next !== undefined) {
        return resolve(next, depth + 1);
      }
    }
    return value;
  };
}

const resolveLight = makeResolver([parseVars(rootBlock)]);
const resolveDark = makeResolver([parseVars(rootBlock), parseVars(darkBlock)]);

describe("token source of truth", () => {
  it("declares every hex inside the theme.md palette", () => {
    const hexes = new Set(declaredHexes(tokensCss));
    expect(hexes.size).toBe(PALETTE.length);
    for (const hex of hexes) {
      expect(PALETTE).toContain(hex);
    }
  });

  it("keeps components.css free of any raw hex", () => {
    expect(declaredHexes(componentsCss)).toEqual([]);
  });

  it("keeps tokens.ts PALETTE in sync with tokens.css", () => {
    const cssHexes = new Set(declaredHexes(tokensCss));
    for (const hex of PALETTE) {
      expect(cssHexes, `${hex} missing from tokens.css`).toContain(hex.toUpperCase());
    }
  });
});

describe("dark mode completeness", () => {
  const REQUIRED_OVERRIDES = [
    "--m-canvas",
    "--m-surface",
    "--m-overlay-surface",
    "--m-text",
    "--m-text-secondary",
    "--m-text-muted",
    "--m-border",
    "--m-border-strong",
    "--m-accent",
    "--m-on-accent",
    "--m-focus-ring",
    ...STATUS_TONES.flatMap((t) => [`--m-status-${t}-fg`, `--m-status-${t}-bg`]),
    "--m-scrim",
    "--m-shadow",
    "--m-shadow-strong",
  ];

  it("overrides every core semantic variable", () => {
    const declared = new Set([...darkBlock.matchAll(/(--[\w-]+):/g)].map((m) => m[1]!));
    for (const v of REQUIRED_OVERRIDES) {
      expect(declared.has(v), v).toBe(true);
    }
  });

  it("uses the lightened ember as the dark accent", () => {
    expect(parseVars(darkBlock).get("--m-accent")).toBe("var(--m-ember-dark)");
  });

  it("resolves dark canvas/surface/text to the spec anchors", () => {
    const vars = parseVars(darkBlock);
    expect(resolveDark(vars.get("--m-canvas")!).toUpperCase()).toBe("#101216");
    expect(resolveDark(vars.get("--m-surface")!).toUpperCase()).toBe("#171A1F");
    expect(resolveDark(vars.get("--m-text")!).toUpperCase()).toBe("#E8EBEF");
  });
});

describe("WCAG contrast", () => {
  interface Mode {
    label: string;
    resolve: (v: string) => string;
  }

  const modes: Mode[] = [
    { label: "light", resolve: resolveLight },
    { label: "dark", resolve: resolveDark },
  ];

  for (const mode of modes) {
    it(`meets AA (4.5) for primary/secondary text on canvas+surface (${mode.label})`, () => {
      const vars = mode.label === "light" ? parseVars(rootBlock) : parseVars(darkBlock);
      const text = [vars.get("--m-text")!, vars.get("--m-text-secondary")!];
      const surfaces = [
        mode.resolve(vars.get("--m-canvas")!),
        mode.resolve(vars.get("--m-surface")!),
      ];
      for (const fg of text) {
        for (const bg of surfaces) {
          const ratio = contrastRatio(mode.resolve(fg), bg);
          expect(ratio, `${fg} on ${bg}`).toBeGreaterThanOrEqual(4.5);
        }
      }
    });
  }

  it("meets non-text contrast (3.0) for focus ring on both canvases", () => {
    expect(contrastRatio("#D64B0A", "#F4F5F7")).toBeGreaterThanOrEqual(3.0);
    expect(contrastRatio("#FF8A4C", "#101216")).toBeGreaterThanOrEqual(3.0);
  });

  it("primary button pairing meets the spec-pinned floor", () => {
    // theme.md §6 pins solid Ember + white text in light mode (~4.32:1):
    // above the 3:1 large/UI floor, below normal-text 4.5 — spec tradeoff.
    expect(contrastRatio("#FFFFFF", "#D64B0A")).toBeGreaterThanOrEqual(4.2);
    // Dark mode uses ink text on the lightened ember and passes full AA.
    expect(contrastRatio("#12161C", "#FF8A4C")).toBeGreaterThanOrEqual(4.5);
  });

  it.each([...STATUS_TONES])("%s tag pair meets the UI floor (3.0)", (tone) => {
    const vars = parseVars(rootBlock);
    const fgLight = resolveLight(vars.get(`--m-status-${tone}-fg`)!);
    const bgLight = resolveLight(vars.get(`--m-status-${tone}-bg`)!);
    const fgDark = resolveLight(parseVars(darkBlock).get(`--m-status-${tone}-fg`)!);
    const bgDark = resolveLight(parseVars(darkBlock).get(`--m-status-${tone}-bg`)!);
    // Dark danger (#F87171 on #7F1D1D ≈ 3.62) is spec-pinned; floor is UI-level 3.0.
    expect(contrastRatio(fgLight, bgLight), `light ${tone}`).toBeGreaterThanOrEqual(3.0);
    expect(contrastRatio(fgDark, bgDark), `dark ${tone}`).toBeGreaterThanOrEqual(3.0);
  });

  it.each([
    ["#15803D", "#DCFCE7", "light success"],
    ["#B45309", "#FEF3C7", "light warning"],
    ["#B91C1C", "#FEE2E2", "light danger"],
    ["#55606E", "#E5E7EB", "light neutral"],
    ["#4ADE80", "#14532D", "dark success"],
    ["#FBBF24", "#78350F", "dark warning"],
    ["#9AA4B2", "#262B33", "dark neutral"],
  ])("%s on %s (%s) meets full AA 4.5", (fg, bg, label) => {
    expect(contrastRatio(fg, bg), label).toBeGreaterThanOrEqual(4.5);
  });
});
