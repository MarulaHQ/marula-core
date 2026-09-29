import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";
import {
  BUILTIN_PRESETS,
  DEFAULT_PRESET,
  defaultPresetFor,
  PLATFORM_PRESET_DEFAULT,
  FIXED_ROLES,
  MARULA,
  MARULA_WARM,
  SHIFTABLE_AXIS,
  UNSHIFTABLE_ROLES,
  assertPresetIsLegal,
  builtinPresetCss,
  isBuiltinPreset,
  presetCss as renderPresetCss,
  presetById,
  presetViolation,
  shiftableValues,
  TOKEN_ROLES,
  type Platform,
} from "../src/theme-tokens";
import { contrastRatio } from "../src/tokens";

describe("theme preset schema v1", () => {
  it("marula mirrors tokens.css verbatim (zero visual change)", () => {
    // The Neutral column of the rendered pack, lowercased to match the source.
    expect(MARULA.light.background).toBe("#f8f7f4");
    expect(MARULA.dark.background).toBe("#2b2a33");
    expect(MARULA.light.accent).toBe("#f7e3a8");
    // The accent is the gold-soft TINT in both modes. Gold itself is 1.7:1 on
    // this canvas and fails, which is the whole reason the accent split exists.
    expect(MARULA.light.accent).not.toBe("#e8b83a");
    expect(MARULA.dark.accent).toBe("#3a3944");
  });

  it("marula-warm moves the neutral axis and nothing else", () => {
    // The two light themes differ on canvas, border, muted text and the neutral
    // status pair. Everything the brand owns is byte-identical (wave41 D8).
    for (const role of SHIFTABLE_AXIS) {
      if (role === "background" || role === "border" || role === "text_muted") continue;
      expect(MARULA_WARM.light[role], role).toBe(MARULA.light[role]);
    }
    for (const role of UNSHIFTABLE_ROLES) {
      expect(MARULA_WARM.light[role], role).toBe(MARULA.light[role]);
      expect(MARULA_WARM.dark[role], role).toBe(MARULA.dark[role]);
    }
    expect(MARULA_WARM.light.background).toBe("#fbf7ec");
    expect(MARULA_WARM.dark).toBe(MARULA.dark);
  });

  it("ships the curated built-in range with distinct ids", () => {
    expect(BUILTIN_PRESETS.map((p) => p.id)).toEqual([
      "marula",
      "marula-warm",
      "high_contrast",
    ]);
    for (const preset of BUILTIN_PRESETS) {
      expect(isBuiltinPreset(preset.id)).toBe(true);
      expect(presetById(preset.id)).toBe(preset);
    }
    expect(DEFAULT_PRESET).toBe("marula");
  });

  it("picks the platform default from one place (wave41 D2)", () => {
    // Neutral for web and desktop, cream for mobile. Both stay selectable; this
    // is only what a user gets before an org or per-user override.
    expect(defaultPresetFor("web")).toBe("marula");
    expect(defaultPresetFor("desktop")).toBe("marula");
    expect(defaultPresetFor("mobile")).toBe("marula-warm");
    for (const platform of Object.keys(PLATFORM_PRESET_DEFAULT)) {
      expect(isBuiltinPreset(defaultPresetFor(platform as Platform)), platform).toBe(true);
    }
    // The mobile default must actually be the cream column, not the Neutral one.
    expect(MARULA_WARM.light.background).not.toBe(MARULA.light.background);
  });

  it("pins the native app to the same mobile default", () => {
    // The native app repeats the id because it cannot import this module.
    // Read the file rather than trusting a comment.
    const native = readFileSync(
      join(process.cwd(), "../../native/src/ui/tokens.ts"),
      "utf8",
    );
    const declared = native.match(/PLATFORM_PRESET_ID\s*=\s*"([\w-]+)"/)?.[1];
    expect(declared, "native/src/ui/tokens.ts must declare PLATFORM_PRESET_ID").toBe(
      defaultPresetFor("mobile"),
    );
  });

  it("retired presets are gone from the built-in range", () => {
    // Tangerine is the retired identity; slate and copper are dropped per D8.
    for (const retired of ["tangerine", "slate", "copper"]) {
      expect(isBuiltinPreset(retired), retired).toBe(false);
      expect(presetById(retired), retired).toBeUndefined();
    }
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

describe("the fixed-axis rule (wave41 D8, brand spec §4.5)", () => {
  it("a preset may shift the neutrals, the chrome and the accent tint", () => {
    expect(SHIFTABLE_AXIS).toContain("background");
    expect(SHIFTABLE_AXIS).toContain("border");
    expect(SHIFTABLE_AXIS).toContain("accent");
  });

  it("a preset may not repaint the brand semantics", () => {
    for (const role of ["primary", "assistant", "timeline_note", "gold_text", "input"]) {
      expect(FIXED_ROLES, role).toContain(role);
    }
    // destructive and the status pairs are reachable in the closed vocabulary
    // (a full pack round-trips through it) but are NOT on the shiftable axis.
    expect(UNSHIFTABLE_ROLES, "destructive").toContain("destructive");
    // destructive and every status pair are reachable in the closed vocabulary
    // (a full pack round-trips through it) but are NOT on the shiftable axis.
    expect(UNSHIFTABLE_ROLES).toContain("destructive");
    expect(UNSHIFTABLE_ROLES).toContain("status_success_fg");
  });

  it("rejects a preset that reaches a fixed role", () => {
    const bad = { primary: "#ff0000" };
    expect(presetViolation("rogue", bad)).toMatch(/brand-fixed role/);
    expect(() => assertPresetIsLegal("rogue", bad)).toThrow(/brand-fixed role/);
  });

  it("rejects a preset that reaches an unshiftable vocabulary role", () => {
    const bad = { status_danger_fg: "#ff0000", destructive: "#ff0000" };
    expect(presetViolation("rogue", bad)).toMatch(/outside the shiftable axis/);
  });

  it("drops an unshiftable role from the emitted CSS even if it is named", () => {
    const css = renderPresetCss("rogue", { ...MARULA.light, destructive: "#ff0000" }, MARULA.dark);
    expect(css).not.toContain("--m-destructive");
    expect(css).toContain("--m-canvas");
  });

  it("accepts the shipped presets unchanged", () => {
    // A built-in carries the whole resolved vocabulary for round-tripping; it
    // is not a set of DECLARED overrides. What it declares is the shift.
    for (const preset of BUILTIN_PRESETS) {
      expect(presetViolation(preset.id, shiftableValues(preset.light))).toBeNull();
      expect(presetViolation(preset.id, shiftableValues(preset.dark))).toBeNull();
    }
  });
});

describe("preset css rendering", () => {
  it("scopes rules by data-preset and never touches the marula baseline", () => {
    const css = renderPresetCss("marula-warm", MARULA_WARM.light, MARULA_WARM.dark);
    expect(css).toContain(':root[data-preset="marula-warm"]');
    expect(css).toContain('[data-theme="dark"]');
    expect(css).toContain("--m-canvas");
    expect(builtinPresetCss()).not.toContain('[data-preset="marula"]');
  });

  it("emits the shiftable axis as --m-* tokens", () => {
    const css = builtinPresetCss();
    expect(css).toContain("--m-canvas");
    expect(css).toContain("--m-accent");
    expect(css).toContain("--m-chart-5");
  });
});
