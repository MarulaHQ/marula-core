// Unit tests: chart token bridge — token-set provenance and theme parity.
import { describe, expect, it } from "vitest";
import { PALETTE } from "@meridian/ui";
import { chartTokensFor } from "../../lib/chart-tokens";

describe("chart tokens", () => {
  it("source every value from the Workbench palette (no off-token hex)", () => {
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
    expect(light.primary).toBe("#D64B0A"); // ember (light accent)
    expect(dark.primary).toBe("#FF8A4C"); // ember-dark (dark accent)
    expect(light.tooltipBg).toBe("#FFFFFF");
    expect(dark.tooltipBg).toBe("#171A1F");
    // Series stay distinguishable in both modes.
    expect(light.secondary).not.toBe(light.primary);
    expect(dark.secondary).not.toBe(dark.primary);
  });
});
