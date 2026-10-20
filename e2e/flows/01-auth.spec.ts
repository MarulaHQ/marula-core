// Flow 1 — Auth: the real login form resolves the demo org, issues the
// token pair, and lands on /today. Runs identically in light + dark.
//
// THEME PAINT ORDER (Wave 47 lane A3, `p47-prepaint-theme-flash`): the
// pre-paint bootstrap in the root layout's <head> resolves the stored
// preference (else the system's) and writes the KIT's own attribute values
// before first paint, so neither mode ever flashes. The layout also mounts
// ThemeBootstrap, whose layout effect re-applies the same resolved attribute
// if React's hydration recovery (minified error #418) re-creates <html> on an
// authed route — which used to leave the page on the tokens.css baseline
// until the next write. The flash is closed in both directions; the
// assertions below pin the settled behaviour. The underlying hydration
// mismatch that triggers the recovery on authed pages is NOT root-caused
// here — it is recorded in design/qa-p47-prepaint-theme-flash/LANDING.md as
// open work for the web lane.
import { expect, test } from "../helpers/harness";
import { apiLogin } from "../helpers/api";

test("login page bootstraps the theme pre-paint and signs the demo owner in", async ({ page }, testInfo) => {
  // The login route is the first screen a prospect ever sees, so the test
  // seeds the preference each theme project stands for and asserts the KIT's
  // settled literal for that direction — `dark` for a dark preference,
  // `neutral` for light (the app's `light` is written as `neutral` so the
  // kit's cream block cannot win, packages/ui/src/theme.ts toDomTheme). The
  // pre-paint bootstrap reads this key; the recovery leg holds it through
  // hydration.
  await page.addInitScript((theme) => {
    window.localStorage.setItem("marula-theme", theme);
  }, testInfo.project.name.includes("dark") ? "dark" : "light");
  await page.goto("/login");
  await expect(page.locator("html")).toHaveAttribute(
    "data-theme",
    testInfo.project.name.includes("dark") ? "dark" : "neutral",
    { timeout: 15_000 },
  );

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

  // Put the lockout back that this test deliberately armed (Wave 47 lane
  // A26). A wrong password writes a durable `password_login` counter for
  // THIS egress IP, and the gate denies every password login inside that
  // window — so for 30s every later flow's fixture gets no session and
  // dies on a sign-in page it never navigated to. A successful login is
  // the product's own reset (gate_clear drops both the IP and the email
  // key), and it proves the credential path recovered, so no test-only
  // backdoor and no fixed sleep is needed here.
  await apiLogin();
});

test("seeded theme preference persists for the authed session", async ({ authedPage }) => {
  await authedPage.goto("/today");
  // Same anchor as the sign-in test: the h1 is a greeting.
  await expect(authedPage.getByRole("heading", { name: "Your day" })).toBeVisible({ timeout: 30_000 });
  const stored = await authedPage.evaluate(() => localStorage.getItem("marula-theme"));
  expect(["light", "dark"]).toContain(stored);
});
