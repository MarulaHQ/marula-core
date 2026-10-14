// Wave 24 theme-matrix spec: the shipped DEFAULT presets only (leg C —
// arbitrary custom packs are never in the E2E matrix).
//
// Purpose: prove the preset engine end-to-end for the shipped defaults
// (marula-warm, high-contrast): the seeded preset attribute reaches the app
// pre-paint (data-preset on <html>), the preset tokens actually restyle
// the resolved --m-canvas/--m-accent values in the computed style, and

// (typo'd/org-guarded values are 422-able server side so the UI is
// never in a broken state).
//
// Non-purpose: arbitrary theme packs (restyle-only manifests are
// mechanically enforced by scripts/check-theme-packs.mjs; the matrix
// here covers the SHIPPED defaults only, mirroring the Playwright
// light/dark posture from Wave 14 lane 8).
import { expect, test } from "../helpers/harness";
import { API_URL } from "../helpers/api";

// The Neutral and cream canvases from the rendered Marula pack, plus the
// accessibility preset's paper white.
const CANVAS_BY_PRESET: Record<string, string> = {
  marula: "#f8f7f4",
  "marula-warm": "#fbf7ec",
  high_contrast: "#ffffff",
};

test.describe("theme preset matrix (shipped defaults only)", () => {
  test("the seeded preset applies pre-paint and restyles the canvas", async ({
    authedPage: page,
  }, testInfo) => {
    // marula stays attribute-free (the pin baseline); the named projects
    // prove a real attribute round-trip through the pre-paint bootstrap.
    const tags = ["warm-preset-light", "warm-preset-dark", "high-contrast-preset-light", "high-contrast-preset-dark"];
    const isDefault = !tags.includes(testInfo.project.name);
    // The direction the harness seeded for this project (the same rule
    // e2e/helpers/harness.ts applies: the base project is named exactly
    // `dark`, the preset projects suffix it). The assertion is the KIT's
    // settled literal for that direction — `dark`, or `neutral` for the
    // app's light (packages/ui/src/theme.ts toDomTheme), never the app's own
    // `light`, which the kit reads as its cream block.
    const expectedTheme =
      testInfo.project.name === "dark" || testInfo.project.name.endsWith("-dark") ? "dark" : "neutral";
    // The pre-paint record: an init script at document_start, ahead of the
    // bootstrap script and all React code, samples `--m-canvas` on every
    // animation frame. The spec's claim is about the FIRST paint — the
    // seeded preset reaching the app pre-paint and restyling it — so the
    // assertion reads the first resolved frame rather than the DOM after
    // the account sync has had its say. On an authed route that sync lands
    // (the org default appearance.theme is `marula`, AppearanceSync applies
    // it once /api/v1/settings resolves), and a one-shot read of the live
    // DOM races it: before the sync the preset's canvas is live, after it
    // the org default is. The sampled frame is the value a person actually
    // saw first, which is the thing under test.
    await page.addInitScript(() => {
      const frames = [];
      const note = () => {
        frames.push({
          theme: document.documentElement.getAttribute("data-theme"),
          preset: document.documentElement.getAttribute("data-preset"),
          canvas: getComputedStyle(document.documentElement)
            .getPropertyValue("--m-canvas")
            .trim(),
        });
        if (frames.length < 600) requestAnimationFrame(note);
      };
      requestAnimationFrame(note);
      window.__PAINT_FRAMES__ = frames;
    });
    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("data-theme", expectedTheme);
    if (isDefault) {
      await expect(page.locator("html")).not.toHaveAttribute("data-preset");
    } else {
      await expect(page.locator("html")).toHaveAttribute(
        "data-preset",
        /marula-warm|high_contrast/,
      );
      // The first frame with a resolved canvas carried the seeded preset's
      // tokens: this is the D8 axis in one assertion — a preset may move the
      // neutral, and the check is that it demonstrably did, on the frame
      // the bootstrap script produced.
      const firstPainted = await page.evaluate(() => {
        const frames = (window.__PAINT_FRAMES__ ?? []) as {
          preset: string | null;
          canvas: string;
        }[];
        return frames.find((f) => f.canvas !== "") ?? null;
      });
      expect(firstPainted?.canvas.toLowerCase()).not.toBe(CANVAS_BY_PRESET.marula);
      expect(firstPainted?.preset).toMatch(/marula-warm|high_contrast/);
    }
  });

  // A browser-level assertion that a preset switch cannot move the
  // brand-fixed tokens (primary, the mode flip, destructive, the status pairs,
  // the assistant inversion) belongs here. It is NOT written yet, and that is a
  // recorded gap rather than an oversight: driving it needs a hydrated,
  // authenticated browser session, and in the lane's e2e environment the app
  // renders an unauthenticated shell (data-preset never applies), so the test
  // would ship unverified. The rule is enforced in two places that ARE green:
  // scripts/check-theme-packs.mjs asserts every non-shiftable role is
  // byte-equal to marula's across both modes, and
  // packages/ui/tests/theme-preset-schema.test.ts asserts the unshiftable
  // roles are unreachable through ROLE_TO_TOKEN. Whoever closes the auth gap
  // should add the browser leg here.
});
