import { describe, expect, it } from "vitest";
import {
  BUILTIN_PRESETS,
  DEFAULT_PRESET,
  SLATE,
  TANGERINE,
  builtinPresetCss,
  isBuiltinPreset,
  presetCss as renderPresetCss,
  presetById,
  TOKEN_ROLES,
} from "../src/theme-tokens";
import { contrastRatio } from "../src/tokens";

describe("theme preset schema v1", () => {
  it("tangerine mirrors tokens.css verbatim (zero visual change)", () => {
    expect(TANGERINE.light.background).toBe("#e8ebed");
    expect(TANGERINE.dark.background).toBe("#1c2433");
    expect(TANGERINE.light.accent).toBe("#e05d38");
    expect(TANGERINE.dark.accent).toBe(TANGERINE.light.accent);
  });

  it("ships the curated built-in range with distinct ids", () => {
    expect(BUILTIN_PRESETS.map((p) => p.id)).toEqual([
      "tangerine",
      "slate",
      "copper",
      "high_contrast",
    ]);
    for (const preset of BUILTIN_PRESETS) {
      expect(isBuiltinPreset(preset.id)).toBe(true);
      expect(presetById(preset.id)).toBe(preset);
    }
    expect(DEFAULT_PRESET).toBe("tangerine");
  });

  it("every role value across presets is a closed-vocabulary #rrggbb hex", () => {
    for (const preset of BUILTIN_PRESETS) {
      for (const mode of ["light", "dark"] as const) {
        const values = preset[mode];
        for (const [role, value] of Object.entries(values)) {
          expect(TOKEN_ROLES).toContain(role);
          expect(value).toMatch(/^#[0-9a-f]{6}$/);
        }
      }
    }
  });

  it("light and dark cover every closed role for every preset", () => {
    for (const preset of BUILTIN_PRESETS) {
      for (const mode of ["light", "dark"] as const) {
        for (const role of TOKEN_ROLES) {
          expect(preset[mode][role]).toBeDefined();
        }
      }
    }
  });

  it("every preset clears the AA text and 3:1 accent floors both modes", () => {
    for (const preset of BUILTIN_PRESETS) {
      for (const mode of ["light", "dark"] as const) {
        const values = preset[mode]!;
        for (const fg of ["text", "text_secondary", "text_muted"]) {
          for (const bg of ["background", "surface"] as const) {
            expect(contrastRatio(values[fg] as string, values[bg] as string)).toBeGreaterThanOrEqual(4.5);
          }
        }
        expect(contrastRatio(values.on_accent as string, values.accent as string)).toBeGreaterThanOrEqual(3.0);
      }
    }
  });
});

describe("preset css rendering", () => {
  it("scopes rules by data-preset and never touches the tangerine baseline", () => {
    const css = renderPresetCss("slate", SLATE.light, SLATE.dark);
    expect(css).toContain(':root[data-preset="slate"]');
    expect(css).toContain('[data-theme="dark"]');
    expect(css).toContain("--m-canvas");
    expect(builtinPresetCss()).not.toContain('[data-preset="tangerine"]');
  });

  it("emits every closed role as a --m-* token", () => {
    const css = builtinPresetCss();
    expect(css).toContain("--m-canvas");
    expect(css).toContain("--m-status-success-fg");
    expect(css).toContain("--m-chart-5");
  });
});
