// Unit tests: chart token bridge — token-set provenance and theme parity.
import { describe, expect, it } from "vitest";
import { PALETTE } from "@marula-core/ui";
import { chartTokensFor } from "../../lib/chart-tokens";

describe("chart tokens", () => {
  it("source every value from the Marula palette (no off-token hex)", () => {
    for (const theme of ["light", "dark"] as const) {
      const tokens = chartTokensFor(theme);
      for (const value of Object.values(tokens)) {
        expect(PALETTE).toContain(value);
      }
    }
  });

  it("give light and dark parity through the standard theme switch", () => {
    const light = chartTokensFor("light");
    const dark = chartTokensFor("dark");
    // The chart ramp leads with the ring gold in both modes. On light it is the
    // darker ring value, because gold as a 2px stroke on a cream canvas is
    // invisible; on dark it is the gold itself, because gold on ink is 7.7:1.
    // One leading hue, two grounds, so a chart reads the same either way.
    expect(light.primary).toBe("#A8641A");
    expect(dark.primary).toBe("#E8B83A");
    expect(light.tooltipBg).toBe("#FFFFFF");
    expect(dark.tooltipBg).toBe("#34333D");
    // Series stay distinguishable in both modes.
    expect(light.secondary).not.toBe(light.primary);
    expect(dark.secondary).not.toBe(dark.primary);
  });

  it("carries no retired identity hex", () => {
    for (const theme of ["light", "dark"] as const) {
      for (const value of Object.values(chartTokensFor(theme))) {
        for (const retired of ["#E05D38", "#86A7C8", "#1C2433", "#2A3040", "#D64B0A"]) {
          expect(value.toUpperCase(), `${retired} is retired`).not.toBe(retired);
        }
      }
    }
  });
});
