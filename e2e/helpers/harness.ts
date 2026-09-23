// Authenticated session fixture (the "authed harness").
//
// Wave 19 lane 5 auth: the app keeps the TokenPair memory-only and rotates
// via the httpOnly session cookie, so authentication is established by
// logging the demo owner in through the browser context's request jar (the
// Set-Cookie from /api/v1/auth/login lands in the page cookie store).
// Theme keys are still seeded into localStorage (the pre-paint bootstrap
// reads those).
//
// THEME MATRIX: the Playwright project name is the theme ("light" / "dark");
// every test runs once per theme against the same seeded demo env.
import { test as base, expect, type BrowserContext, type Page } from "@playwright/test";
import { API_URL, DEMO_OWNER_EMAIL, DEMO_PASSWORD, apiLogin, type TokenPair } from "./api";

export const TOKENS_KEY = "meridian-tokens";
export const TOKENS_EXP_KEY = "meridian-tokens-exp";
export const THEME_KEY = "meridian-theme";
/** Wave 24: the persisted preset attribute (localStorage key the app's
 * pre-paint bootstrap reads; empty/absent = Tangerine baseline). */
export const THEME_PRESET_KEY = "meridian-theme-preset";

export type Theme = "light" | "dark";
/** A shipped default preset id (playwright only covers shipped defaults
 * — arbitrary custom packs are not part of the E2E matrix). */
export type ThemePresetSeed = "tangerine" | "slate" | "copper" | "high_contrast";

type Fixtures = {
  /** Page authenticated as the demo org owner, themed by the active project. */
  authedPage: Page;
  /** Owner TokenPair (for API-side scratch-data staging inside tests). */
  ownerTokens: TokenPair;
};

export const test = base.extend<Fixtures>({
  // eslint-disable-next-line no-empty-pattern
  ownerTokens: async ({}, use) => {
    await use(await apiLogin());
  },

  authedPage: async ({ browser, ownerTokens }, use, testInfo) => {
    const theme = (testInfo.project.name.endsWith("-dark") ? "dark" : "light") as Theme;
    const preset = testInfo.project.name.startsWith("slate-preset")
      ? "slate"
      : testInfo.project.name.startsWith("high-contrast-preset")
        ? "high_contrast"
        : null;
    const ctx = await browser.newContext({
      viewport: { width: 1440, height: 900 },
    });
    const page = await ctx.newPage();
    // Wave 19 lane 5 auth: the app keeps tokens memory-only and rotates via
    // the httpOnly session cookie, so the browser context must OWN the
    // session — log in through the context's request jar (shares the cookie
    // store with pages) instead of seeding localStorage.
    await loginContext(ctx);
    await seedAuth(page, ownerTokens, theme, preset);
    await use(page);
    await ctx.close();
  },
});

/** Log the demo owner in through the context's cookie jar (the Set-Cookie
 * session cookie from the login response lands in the browser's cookie
 * store, which the app's cookie-driven refresh rotation reads). */
export async function loginContext(ctx: BrowserContext): Promise<TokenPair> {
  const res = await ctx.request.post(`${API_URL}/api/v1/auth/login`, {
    data: { email: DEMO_OWNER_EMAIL, password: DEMO_PASSWORD },
  });
  if (!res.ok()) throw new Error(`context login failed (${res.status()})`);
  return (await res.json()) as TokenPair;
}

/** Seed the token pair + expiry hint + theme preset into a page's
 * localStorage (the project determines both). */
export async function seedAuth(
  page: Page,
  tokens: TokenPair,
  theme?: Theme,
  preset?: string | null,
) {
  await page.addInitScript(
    ([tokensKey, expKey, themeKey, presetKey, raw, exp, theme, preset]) => {
      window.localStorage.setItem(tokensKey, raw);
      window.localStorage.setItem(expKey, exp);
      if (theme !== null) window.localStorage.setItem(themeKey, theme);
      if (preset !== null) window.localStorage.setItem(presetKey, preset);
    },
    [
      TOKENS_KEY,
      TOKENS_EXP_KEY,
      THEME_KEY,
      THEME_PRESET_KEY,
      JSON.stringify(tokens),
      String(Date.now() + tokens.expires_in * 1000 - 30_000),
      theme ?? null,
      preset ?? null,
    ] as unknown as [string, string, string, string, string, string, string, string],
  );
}

export { expect };
