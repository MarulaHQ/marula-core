// Flow 1 — Auth: the real login form resolves the demo org, issues the
// token pair, and lands on /today. Runs identically in light + dark.
//
// KNOWN FINDING (recorded honestly, do not paper over): on production
// builds the pre-paint theme bootstrap sets `data-theme` on <html>, but
// React hydration recovery (minified error #418) wipes the attribute on
// authed pages — dark mode silently re-renders light after hydration.
// The assertions below therefore pin the STABLE behaviors: the bootstrap
// applies the theme pre-paint (visible on the low-hydration login page)
// and the stored preference persists. The hydration wipe is an app fix
// for the web lane, tracked in the lane pass record.
import { expect, test } from "../helpers/harness";

test("login page bootstraps the theme pre-paint and signs the demo owner in", async ({ page }) => {
  await page.goto("/login");
  // The bootstrap script ran before first paint (attribute on <html>).
  // The DOM deliberately carries the KIT's names: the app's `light` is
  // written as `neutral` so the kit's cream block cannot win
  // (packages/ui/src/theme.ts toDomTheme). The app's own vocabulary stays
  // light/dark; only the attribute is translated.
  await expect(page.locator("html")).toHaveAttribute("data-theme", /neutral|dark/, {
    timeout: 15_000,
  });

  await page.fill('input[type="email"]', "owner@demo.local");
  await page.fill('input[type="password"]', "demo-password-123");
  await page.click('button[type="submit"]');
  await page.waitForURL(/\/today/, { timeout: 30_000 });
  // The `/today` h1 is the time-of-day greeting ("Good afternoon, Fiona"),
  // which no stable assertion can name, so the page's own section heading
  // is the anchor instead.
  await expect(page.getByRole("heading", { name: "Your day" })).toBeVisible({ timeout: 30_000 });
});

test("wrong password renders friendly error copy, not raw JSON", async ({ page }) => {
  await page.goto("/login");
  await page.fill('input[type="email"]', "owner@demo.local");
  await page.fill('input[type="password"]', "not-the-password");
  await page.click('button[type="submit"]');
  await expect(page.locator('[role="alert"]').first()).toBeVisible({ timeout: 20_000 });
  const alert = await page.locator('[role="alert"]').first().innerText();
  expect(alert.toLowerCase()).not.toContain("{");
  expect(alert.toLowerCase()).not.toContain('"error"');
});

test("seeded theme preference persists for the authed session", async ({ authedPage }) => {
  await authedPage.goto("/today");
  // Same anchor as the sign-in test: the h1 is a greeting.
  await expect(authedPage.getByRole("heading", { name: "Your day" })).toBeVisible({ timeout: 30_000 });
  const stored = await authedPage.evaluate(() => localStorage.getItem("marula-theme"));
  expect(["light", "dark"]).toContain(stored);
});
