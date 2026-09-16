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
 * Resolve `var()` chains (up to 4 hops) against primitive definitions, then
 * EVALUATE color-mix(in srgb, A p%, B) into a concrete rgb() string so WCAG
 * math can run on derived tones.
 */
function makeResolver(blocks: Array<Map<string, string>>) {
  function resolveMix(value: string): string {
    // color-mix(in srgb, X p%, Y) — X/Y may be hex or var(); p may be omitted.
    const m = value.match(
      /^color-mix\(in srgb,\s*(var\(--[\w-]+\)|#[0-9a-fA-F]{6})(?:\s+([\d.]+)%)?,\s*(var\(--[\w-]+\)|#[0-9a-fA-F]{6}|transparent)\)$/,
    );
    if (!m) return value;
    const toRgb = (token: string): [number, number, number] | null => {
      const hex = token.match(/^#([0-9a-fA-F]{6})$/);
      if (hex) {
        // hex[1] excludes the '#' — offsets are 0,2,4
        return [0, 2, 4].map((i) => parseInt(hex[1]!.slice(i, i + 2), 16)) as [
          number,
          number,
          number,
        ];
      }
      const v = token.match(/^var\((--[\w-]+)\)$/);
      if (!v) return null;
      for (const block of blocks) {
        const next = block.get(v[1]!);
        if (next !== undefined) return toRgb(resolve(next));
      }
      return null;
    };
    const a = toRgb(m[1]!);
    if (!a) return value;
    const p = m[2] !== undefined ? parseFloat(m[2]) / 100 : 0.5;
    if (m[3] === "transparent") {
      // Alpha over white isn't meaningful for contrast math; keep symbolic.
      return value;
    }
    const b = toRgb(m[3]!);
    if (!b) return value;
    const out = a.map((v, i) => Math.round(v * p + b[i]! * (1 - p)));
    return `rgb(${out[0]}, ${out[1]}, ${out[2]})`;
  }
  function resolve(value: string, depth = 0): string {
    const m = value.match(/^var\((--[\w-]+)\)$/);
    if (m && depth <= 3) {
      for (const block of blocks) {
        const next = block.get(m[1]!);
        if (next !== undefined) return resolve(next, depth + 1);
      }
    }
    return resolveMix(value);
  }
  return resolve;
}

const resolveLight = makeResolver([parseVars(rootBlock)]);
const resolveDark = makeResolver([parseVars(rootBlock), parseVars(darkBlock)]);

/** Convert a resolved color (hex or rgb()) into #RRGGBB for contrast math. */
function toHex(c: string): string {
  if (c.startsWith("#")) return c.toUpperCase();
  const n = (c.match(/[\d.]+/g) ?? []).map(Number);
  return (
    "#" +
    [n[0] ?? 0, n[1] ?? 0, n[2] ?? 0]
      .map((v) => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, "0"))
      .join("")
      .toUpperCase()
  );
}

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
    expect(resolveDark(vars.get("--m-canvas")!).toUpperCase()).toBe("#1C2433");
    expect(resolveDark(vars.get("--m-surface")!).toUpperCase()).toBe("#2A3040");
    expect(resolveDark(vars.get("--m-text")!).toUpperCase()).toBe("#E5E5E5");
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
          const ratio = contrastRatio(toHex(mode.resolve(fg)), toHex(bg));
          expect(ratio, `${fg} on ${bg}`).toBeGreaterThanOrEqual(4.5);
        }
      }
    });

    it(`meets AA (4.5) for muted text on canvas+surface+accent-soft (${mode.label})`, () => {
      // Regression guard: the first Tangerine pass carried the Workbench
      // color-mix muted formula into dark mode, producing a mid-grey on the
      // blue-slate surfaces (~1.4–3.5:1 — illegible). Muted text must pass
      // AA everywhere it appears: canvases, surfaces, and the tangerine
      // accent-soft hover fills used behind nav chevrons.
      const vars = mode.label === "light" ? parseVars(rootBlock) : parseVars(darkBlock);
      const muted = toHex(mode.resolve(vars.get("--m-text-muted")!));
      const surface = toHex(mode.resolve(vars.get("--m-surface")!));
      const canvas = toHex(mode.resolve(vars.get("--m-canvas")!));
      // accent-soft: color-mix(accent N%, transparent) composited over surface
      const softRaw = mode.resolve(vars.get("--m-accent-soft")!);
      const accent = toHex(mode.resolve(vars.get("--m-accent")!));
      const pct = softRaw.match(/([\d.]+)%,\s*transparent/);
      const a = pct ? parseFloat(pct[1]!) / 100 : 0.1;
      const composite = [1, 3, 5]
        .map((i) => parseInt(accent.slice(i, i + 2), 16))
        .map((av, i) => {
          const sv = parseInt(surface.slice(1 + i * 2, 3 + i * 2), 16);
          return Math.round(av * a + sv * (1 - a));
        });
      const softBg =
        "#" + composite.map((v) => v.toString(16).padStart(2, "0")).join("").toUpperCase();
      for (const bg of [canvas, surface, softBg]) {
        const ratio = contrastRatio(muted, bg);
        expect(ratio, `muted ${muted} on ${bg}`).toBeGreaterThanOrEqual(4.5);
      }
    });
  }

  it("meets non-text contrast (3.0) for focus ring on both canvases", () => {
    expect(contrastRatio("#E05D38", "#E8EBED")).toBeGreaterThanOrEqual(3.0);
    expect(contrastRatio("#E05D38", "#1C2433")).toBeGreaterThanOrEqual(3.0);
  });

  it("primary button pairing meets the spec-pinned floor", () => {
    // theme.md §6 pins solid Tangerine + white text in both modes (≈3.63:1):
    // the preset's own pairing — above the 3:1 large/UI floor, below
    // normal-text 4.5 — documented spec tradeoff (docs/theme.md §3.1).
    expect(contrastRatio("#FFFFFF", "#E05D38")).toBeGreaterThanOrEqual(3.5);
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
    ["#4B5563", "#E5E7EB", "light neutral"],
    ["#4ADE80", "#14532D", "dark success"],
    ["#FBBF24", "#78350F", "dark warning"],
    ["#A3A3A3", "#2A303E", "dark neutral"],
  ])("%s on %s (%s) meets full AA 4.5", (fg, bg, label) => {
    expect(contrastRatio(fg, bg), label).toBeGreaterThanOrEqual(4.5);
  });
});
