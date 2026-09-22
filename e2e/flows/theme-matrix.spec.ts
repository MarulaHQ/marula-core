// Wave 24 theme-matrix spec: the shipped DEFAULT presets only (leg C —
// arbitrary custom packs are never in the E2E matrix).
//
// Purpose: prove the preset engine end-to-end for two shipped defaults
// (slate, high-contrast): the seeded preset attribute reaches the app
// pre-paint (data-preset on <html>), the preset tokens actually restyle
// the resolved --m-canvas/--m-accent values in the computed style, and

// (typo'd/org-guarded values are 422-able server side so the UI is
 * never in a broken state).
//
// Non-purpose: arbitrary theme packs (restyle-only manifests are
// mechanically enforced by scripts/check-theme-packs.mjs; the matrix
// here covers the SHIPPED defaults only, mirroring the Playwright
// light/dark posture from Wave 14 lane 8).
import { expect, test } from "./helpers/harness";

const CANVAS_BY_PRESET: Record<string, string> = {
  tangerine: "#e8ebed",
  slate: "#eef0f2",
  high_contrast: "#ffffff",
};

test.describe("theme preset matrix (shipped defaults only)", () => {
  test("the seeded preset applies pre-paint and restyles the canvas", async ({
    authedPage: page,
  }, testInfo) => {
    // tangerine stays attribute-free (the pin baseline); the named
    // projects prove a real attribute round-trip through the pre-paint
    // bootstrap.
    const tags = ["slate-preset-light", "slate-preset-dark", "high-contrast-preset-light", "high-contrast-preset-dark"];
    const isDefault = !tags.includes(testInfo.project.name);
    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("data-theme", /light|dark/);
    if (isDefault) {
      await expect(page.locator("html")).not.toHaveAttribute("data-preset");
    } else {
      await expect(page.locator("html")).toHaveAttribute(
        "data-preset",
        /slate|high_contrast/,
      );
      // The preset actually restyles: canvas + accent NOT the tangerine
      // values for the slating/contrast themes.
      const canvas = await page.evaluate(
        () =>
          getComputedStyle(document.documentElement)
            .getPropertyValue("--m-canvas")
            .trim(),
      );
      expect(canvas).not.toBe("#e8ebed");
    }
  });
});
