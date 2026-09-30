/**
 * Token contract tests — the mechanical enforcement of docs/theme.md.
 * Purpose: prove tokens.css is the single hex source, mirrors light/dark,
 * and meets WCAG floors for the pairs the spec pins.
 * Non-purpose: no component behavior testing here (see tests/components).
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { PALETTE, TOKEN, contrastRatio, STATUS_TONES } from "../src/tokens.js";

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
      // blocks are ordered [base, override]; the LAST match wins, so a dark
      // mix naming --m-surface resolves the dark surface. Reading forward made
      // every dark color-mix resolve against the light canvas.
      for (let i = blocks.length - 1; i >= 0; i -= 1) {
        const next = blocks[i]!.get(v[1]!);
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
      for (let i = blocks.length - 1; i >= 0; i -= 1) {
        const next = blocks[i]!.get(m[1]!);
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
    // strippedCss, not tokensCss: the file header names the retired hexes on
    // purpose, as a tombstone. A comment is not a declaration.
    const hexes = new Set(declaredHexes(strippedCss));
    expect(hexes.size).toBe(PALETTE.length);
    for (const hex of hexes) {
      expect(PALETTE).toContain(hex);
    }
  });

  it("keeps components.css free of any raw hex", () => {
    expect(declaredHexes(componentsCss)).toEqual([]);
  });

  it("keeps tokens.ts PALETTE in sync with tokens.css", () => {
    const cssHexes = new Set(declaredHexes(strippedCss));
    for (const hex of PALETTE) {
      expect(cssHexes, `${hex} missing from tokens.css`).toContain(hex.toUpperCase());
    }
  });

  it("keeps the named TOKEN record identical to PALETTE", () => {
    // Nothing may index into PALETTE by position: its order is the hex-scan
    // contract, and a reorder would silently repoint a chart series. TOKEN is
    // the addressable view, and this pins the two together.
    // Set identity in both directions. Aliases are legitimate and useful
    // (`ink` and `text` are the same value under two names); what must never
    // happen is a colour in one record that the other does not carry.
    const named = new Set(Object.values(TOKEN).map((v) => v.toUpperCase()));
    const allowed = new Set(PALETTE.map((v) => v.toUpperCase()));
    for (const hex of named) {
      expect([...allowed], `${hex} in TOKEN but not in PALETTE`).toContain(hex);
    }
    for (const hex of allowed) {
      expect([...named], `${hex} in PALETTE but not in TOKEN`).toContain(hex);
    }
  });

  it("carries no retired Tangerine or Workbench hex", () => {
    // Tangerine is retired and Workbench went before it. Both ramps are
    // blacklisted so a later mechanical pass cannot reintroduce a colour that
    // has no authority in the brand spec.
    const retired = [
      "#E05D38", // Tangerine accent
      "#E8EBED", // Tangerine light canvas
      "#1C2433", // Tangerine dark canvas
      "#2A3040", // Tangerine dark surface
      "#3D4354", // Tangerine dark border
      "#86A7C8", // Tangerine chart-1
      "#D64B0A", // Ember, retired by spec §4.6
      "#FF8A4C", // Ember, retired by spec §4.6
    ];
    const live = new Set([...declaredHexes(strippedCss), ...PALETTE.map((h) => h.toUpperCase())]);
    for (const hex of retired) {
      expect(live, `${hex} is retired and must not appear`).not.toContain(hex);
    }
  });
});

describe("dark mode completeness", () => {
  const REQUIRED_OVERRIDES = [
    "--m-canvas",
    "--m-surface",
    "--m-overlay-surface",
    "--m-nav",
    "--m-sidebar",
    "--m-surface-2",
    "--m-text",
    "--m-text-secondary",
    "--m-text-muted",
    "--m-border",
    "--m-input",
    "--m-disabled-bg",
    "--m-disabled-fg",
    "--m-primary",
    "--m-on-primary",
    "--m-accent",
    "--m-on-accent",
    "--m-accent-text",
    "--m-focus-ring",
    "--m-asb",
    "--m-asf",
    "--m-asbtn",
    "--m-asbtnf",
    "--m-asline",
    "--m-client",
    "--m-note",
    "--m-note-fg",
    "--m-sys",
    "--m-time",
    "--m-time-fg",
    "--m-agent",
    "--m-on-success",
    "--m-destructive",
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

  it("resolves dark canvas/surface/text to the spec anchors", () => {
    const vars = parseVars(darkBlock);
    expect(resolveDark(vars.get("--m-canvas")!).toUpperCase()).toBe("#2B2A33");
    expect(resolveDark(vars.get("--m-surface")!).toUpperCase()).toBe("#34333D");
    expect(resolveDark(vars.get("--m-text")!).toUpperCase()).toBe("#F6EFD9");
  });

  it("keeps three distinct chrome tones, not one", () => {
    // The icon rail, the context column and the cards are three SEPARATE
    // tokens, which is the fix: the old system had one "chrome" tone and the
    // layout lost its depth. The rendered pack then collapses one pair per mode
    // (nav==surface in light, nav==sidebar in dark), so the provable claim is
    // that all three are declared and not all three are the same value.
    for (const mode of ["light", "dark"] as const) {
      const block = mode === "light" ? rootBlock : darkBlock;
      const tones = ["--m-nav", "--m-sidebar", "--m-surface"].map((v) =>
        parseVars(block).get(v),
      );
      for (const [i, v] of tones.entries()) {
        expect(v, `${mode} ${["--m-nav", "--m-sidebar", "--m-surface"][i]}`).toBeDefined();
      }
      expect(new Set(tones).size, `${mode} chrome collapsed to one tone`).toBeGreaterThan(1);
    }
  });
});

describe("the timer bar does not follow the mode flip", () => {
  it("is declared once, in :root, and never overridden in the dark block", () => {
    // DESIGN-NOTES §2: the timer bar is ink in EVERY theme. It lives in :root
    // precisely so the mechanical dark-mode pass skips it. Moving it into
    // [data-theme="dark"] would paint it cream.
    expect(parseVars(rootBlock).get("--m-bar")?.toUpperCase()).toBe("#2B2A33");
    expect(parseVars(darkBlock).has("--m-bar")).toBe(false);
  });

  it("puts gold on the bar, never deep amber", () => {
    // #E8B83A on #2B2A33 is 7.7:1. #9A5B0F is 2.6:1 and would fail outright.
    expect(contrastRatio(TOKEN.gold, TOKEN.ink)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(TOKEN.goldText, TOKEN.ink)).toBeLessThan(4.5);
  });
});

describe("the flipping primary", () => {
  it("is ink with gold text on light and gold with ink text on dark", () => {
    const light = parseVars(rootBlock);
    const dark = parseVars(darkBlock);
    expect(light.get("--m-primary")?.toUpperCase()).toBe("#2B2A33");
    expect(light.get("--m-on-primary")?.toUpperCase()).toBe("#E8B83A");
    expect(dark.get("--m-primary")?.toUpperCase()).toBe("#E8B83A");
    expect(dark.get("--m-on-primary")?.toUpperCase()).toBe("#2B2A33");
  });

  it("clears 4.5:1 for the button's label in BOTH modes", () => {
    // Tangerine shipped white-on-orange at 3.6:1, a documented spec tradeoff.
    // Marula has no such tradeoff: both sides clear AA comfortably.
    expect(contrastRatio(TOKEN.gold, TOKEN.ink)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(TOKEN.ink, TOKEN.gold)).toBeGreaterThanOrEqual(4.5);
  });
});

describe("the accent redesign (wave41 D)", () => {
  it("uses a gold-soft TINT and a deep-amber TEXT on light, never gold", () => {
    const light = parseVars(rootBlock);
    expect(light.get("--m-accent")?.toUpperCase()).toBe("#F7E3A8");
    expect(light.get("--m-accent-text")?.toUpperCase()).toBe("#9A5B0F");
    // Gold as TEXT or as a FILL on the light canvas fails; only the tint and
    // the deep amber pass.
    expect(contrastRatio(TOKEN.gold, TOKEN.canvas)).toBeLessThan(3.0);
    expect(contrastRatio(TOKEN.goldText, TOKEN.canvas)).toBeGreaterThanOrEqual(4.5);
  });

  it("derives the hover steps from the tint, not from a hue mix", () => {
    // The old formula was color-mix(accent, ink), which in light mode is a
    // mix of ink with ink and produces no visible step at all. Each step now
    // names the tint explicitly.
    for (const [mode, block] of [
      ["light", rootBlock],
      ["dark", darkBlock],
    ] as const) {
      for (const step of ["--m-accent-soft", "--m-accent-muted", "--m-accent-hover", "--m-accent-active"]) {
        const value = parseVars(block).get(step) ?? "";
        expect(value, `${mode} ${step} must derive from the tint`).toMatch(
          /color-mix\(in srgb, var\(--m-accent\)/,
        );
      }
    }
  });

  it("recedes on hover in dark so the gold label keeps its contrast", () => {
    // Dark hovers go DARKER. Lightening the fill toward cream drops gold from
    // 6.5:1 to 3.9:1 and fails.
    const hover = parseVars(darkBlock).get("--m-accent-hover")!;
    expect(hover).toContain("var(--m-canvas)");
    const resolved = toHex(resolveDark(hover));
    expect(contrastRatio(TOKEN.gold, resolved)).toBeGreaterThanOrEqual(4.5);
  });
});

describe("input borders are distinct from decorative borders", () => {
  it("holds --input at 3:1 against its own background and --border below it", () => {
    // design.md §4.2 splits them: form-control borders need 3:1, decorative
    // dividers need nothing. The old system used one token for both.
    expect(contrastRatio(TOKEN.input, TOKEN.canvas)).toBeGreaterThanOrEqual(3.0);
    expect(contrastRatio(TOKEN.input, TOKEN.surface)).toBeGreaterThanOrEqual(3.0);
    expect(contrastRatio(TOKEN.inputDark, TOKEN.surfaceDark)).toBeGreaterThanOrEqual(3.0);
    expect(contrastRatio(TOKEN.border, TOKEN.canvas)).toBeLessThan(3.0);
  });
});

describe("data meters", () => {
  it("holds the fill at the 3:1 non-text floor in both modes", () => {
    // Found by the vision review: the logged-vs-expected bar was filled with
    // the accent tint, which is 1.04:1 on the light canvas and 1.19:1 in dark.
    // A 2px indicator that carries a number has to clear 3:1, so the meter
    // carries its own pair, and the pair flips on mode for the same reason the
    // primary does.
    const light = parseVars(rootBlock);
    const dark = parseVars(darkBlock);
    expect(light.get("--m-meter-fill")?.toUpperCase()).toBe("#9A5B0F");
    expect(light.get("--m-meter-track")?.toUpperCase()).toBe("#EFEDE6");
    expect(dark.get("--m-meter-fill")?.toUpperCase()).toBe("#E8B83A");
    expect(dark.get("--m-meter-track")?.toUpperCase()).toBe("#2B2A33");
    expect(contrastRatio("#9A5B0F", "#EFEDE6")).toBeGreaterThanOrEqual(3.0);
    expect(contrastRatio("#E8B83A", "#2B2A33")).toBeGreaterThanOrEqual(3.0);
  });

  it("is not the accent tint wearing a different name", () => {
    // Pin the reason the meter exists, so a future pass that collapses it back
    // into --m-accent-muted fails here rather than in someone's eyes.
    expect(contrastRatio("#F7E3A8", "#EFEDE6")).toBeLessThan(3.0);
    expect(contrastRatio("#3A3944", "#2B2A33")).toBeLessThan(3.0);
  });
});

describe("disabled is a muted fill, not an opacity", () => {
  it("gives disabled text its own token pair at the 3:1 UI floor", () => {
    expect(contrastRatio(TOKEN.disabledFg, TOKEN.surface2)).toBeGreaterThanOrEqual(3.0);
    expect(contrastRatio(TOKEN.disabledFgDark, TOKEN.surface2Dark)).toBeGreaterThanOrEqual(3.0);
  });
});

describe("the assistant inversion", () => {
  it("is ink on cream in light and cream on ink in dark", () => {
    const light = parseVars(rootBlock);
    const dark = parseVars(darkBlock);
    expect(light.get("--m-asb")?.toUpperCase()).toBe("#2B2A33");
    expect(light.get("--m-asf")?.toUpperCase()).toBe("#F6EFD9");
    expect(dark.get("--m-asb")?.toUpperCase()).toBe("#F6EFD9");
    expect(dark.get("--m-asf")?.toUpperCase()).toBe("#2B2A33");
  });

  it("carries all five internals, and its button clears AA both ways", () => {
    for (const mode of [rootBlock, darkBlock]) {
      for (const v of ["--m-asb", "--m-asf", "--m-asbtn", "--m-asbtnf", "--m-asline"]) {
        expect(parseVars(mode).get(v), v).toBeDefined();
      }
    }
    expect(contrastRatio(TOKEN.ink, TOKEN.gold)).toBeGreaterThanOrEqual(4.5);
  });

  it("keeps the divider decorative, at the pack's own rendered value", () => {
    // #5F584F on the ink card is 2.02:1. WCAG 1.4.11 covers dividers that
    // identify a control or its state, not a rule inside a card, so this is a
    // deliberate pass rather than a gap. Pinned so nobody "fixes" it.
    expect(contrastRatio(TOKEN.textSecondary, TOKEN.ink)).toBeLessThan(3.0);
    expect(contrastRatio(TOKEN.textSecondary, TOKEN.ink)).toBeGreaterThanOrEqual(1.5);
  });
});

describe("the five timeline entry tints", () => {
  it("declares all five in both modes", () => {
    for (const mode of [rootBlock, darkBlock]) {
      for (const v of ["--m-client", "--m-note", "--m-sys", "--m-time", "--m-agent"]) {
        expect(parseVars(mode).get(v), v).toBeDefined();
      }
    }
  });

  it("keeps entry text readable on every tint", () => {
    for (const [fg, bg] of [
      [TOKEN.text, TOKEN.client],
      [TOKEN.text, TOKEN.note],
      [TOKEN.text, TOKEN.sys],
      [TOKEN.text, TOKEN.time],
      [TOKEN.text, TOKEN.agent],
      [TOKEN.noteFg, TOKEN.note],
      [TOKEN.timeFg, TOKEN.time],
    ] as const) {
      expect(contrastRatio(fg, bg), `${fg} on ${bg}`).toBeGreaterThanOrEqual(4.5);
    }
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
    it(`meets AA (4.5) for primary/secondary/muted text on canvas+surface (${mode.label})`, () => {
      const vars = mode.label === "light" ? parseVars(rootBlock) : parseVars(darkBlock);
      const text = [
        vars.get("--m-text")!,
        vars.get("--m-text-secondary")!,
        vars.get("--m-text-muted")!,
      ];
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

    it(`keeps the on-accent label readable across the whole tint ramp (${mode.label})`, () => {
      // The tint has four strengths. A label that passes on the base tint and
      // fails on its own hover step is the bug this catches.
      const vars = mode.label === "light" ? parseVars(rootBlock) : parseVars(darkBlock);
      const onAccent = toHex(mode.resolve(vars.get("--m-on-accent")!));
      for (const step of ["--m-accent-soft", "--m-accent-muted", "--m-accent", "--m-accent-hover", "--m-accent-active"]) {
        const bg = toHex(mode.resolve(vars.get(step)!));
        expect(contrastRatio(onAccent, bg), `${mode.label} ${step}`).toBeGreaterThanOrEqual(4.5);
      }
    });
  }

  it("meets non-text contrast (3.0) for the focus ring on both canvases", () => {
    expect(contrastRatio(TOKEN.ring, TOKEN.canvas)).toBeGreaterThanOrEqual(3.0);
    expect(contrastRatio(TOKEN.gold, TOKEN.ink)).toBeGreaterThanOrEqual(3.0);
  });

  it("keeps gold off light text entirely", () => {
    // design.md §4.2: gold is never text on a light background. The spec puts
    // it at 1.7:1 on cream; on the Neutral canvas it is no better.
    expect(contrastRatio(TOKEN.gold, "#FBF7EC")).toBeLessThan(3.0);
    expect(contrastRatio(TOKEN.goldText, "#FFFFFF")).toBeGreaterThanOrEqual(4.5);
  });

  it.each([...STATUS_TONES])("%s tag pair meets full AA 4.5", (tone) => {
    const light = parseVars(rootBlock);
    const dark = parseVars(darkBlock);
    expect(
      contrastRatio(
        resolveLight(light.get(`--m-status-${tone}-fg`)!),
        resolveLight(light.get(`--m-status-${tone}-bg`)!),
      ),
      `light ${tone}`,
    ).toBeGreaterThanOrEqual(4.5);
    expect(
      contrastRatio(
        resolveLight(dark.get(`--m-status-${tone}-fg`)!),
        resolveLight(dark.get(`--m-status-${tone}-bg`)!),
      ),
      `dark ${tone}`,
    ).toBeGreaterThanOrEqual(4.5);
  });

  it("the due-today pair clears AA on the blush family", () => {
    // design.md §8 gives due-today its own pair rather than reusing warning;
    // it is the pair most likely to be collapsed into amber by a later pass.
    expect(contrastRatio(TOKEN.dueFg, TOKEN.dueBg)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(TOKEN.dueFgDark, TOKEN.dueBgDark)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(TOKEN.destructive, TOKEN.surface)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(TOKEN.dangerFgDark, TOKEN.surfaceDark)).toBeGreaterThanOrEqual(4.5);
  });

  it("uses deep canopy for small text on a canopy fill", () => {
    // design.md §4.2: cream on a canopy fill is 3.99:1 and fails, so small text
    // on green uses deep canopy at 5.39:1. Both figures are the spec's.
    expect(contrastRatio(TOKEN.cream, TOKEN.canopy)).toBeLessThan(4.5);
    expect(contrastRatio(TOKEN.cream, TOKEN.canopyDeep)).toBeGreaterThanOrEqual(4.5);
  });
});

describe("density steps (wave41 D7)", () => {
  it("defaults to comfortable and offers compact and dense", () => {
    for (const step of ["comfortable", "compact", "dense"]) {
      expect(tokensCss, step).toContain(`[data-density="${step}"]`);
    }
  });

  it("pins each step to its spec triple", () => {
    const block = (step: string): Map<string, string> => {
      const at = tokensCss.indexOf(`[data-density="${step}"]`);
      const chunk = tokensCss.slice(at, tokensCss.indexOf("}", at));
      return parseVars(chunk);
    };
    expect(block("comfortable").get("--m-density-control")).toBe("44px");
    expect(block("comfortable").get("--m-density-row")).toBe("48px");
    expect(block("comfortable").get("--m-card-padding")).toBe("24px");
    expect(block("compact").get("--m-density-control")).toBe("36px");
    expect(block("compact").get("--m-density-row")).toBe("40px");
    expect(block("compact").get("--m-card-padding")).toBe("16px");
    // dense reproduces the repo's pre-Marula values so the fleet, stock and
    // billing tables built at 28px rows do not regress.
    expect(block("dense").get("--m-density-control")).toBe("32px");
    expect(block("dense").get("--m-density-row")).toBe("32px");
    expect(block("dense").get("--m-card-padding")).toBe("12px");
  });
});

