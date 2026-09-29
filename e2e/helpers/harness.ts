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

export const TOKENS_KEY = "marula-tokens";
export const TOKENS_EXP_KEY = "marula-tokens-exp";
export const THEME_KEY = "marula-theme";
/** Wave 24: the persisted preset attribute (localStorage key the app's
 * pre-paint bootstrap reads; empty/absent = Tangerine baseline). */
export const THEME_PRESET_KEY = "marula-theme-preset";

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
    // The base projects are named exactly `light` / `dark`; the preset
    // projects suffix them (`slate-preset-dark`). Both shapes must resolve
    // the theme, or the `dark` project silently runs light.
    const projectName = testInfo.project.name;
    const theme = (
      projectName === "dark" || projectName.endsWith("-dark") ? "dark" : "light"
    ) as Theme;
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
    // Order-independence (p37 doctrine): the demo stack resets per-user
    // tours rows at boot, so the FIRST data-flow spec to visit a surface
    // gets the modal onboarding tour over its content and every later one
    // doesn't — run-order flake. Default the account to tours-off here;
    // 13-contextual-help re-arms (disabled=false, completed="") when it
    // asserts on tour behavior. (p29-accessibility-deep: found while the
    // tour modal blocked flow 02's board.)
    await ctx.request.put(`${API_URL}/api/v1/settings/user`, {
      data: { key: "tours.disabled", value: true },
      headers: { authorization: `Bearer ${ownerTokens.access_token}` },
    });
    await seedAuth(page, ownerTokens, theme, preset);
    await use(page);
    await ctx.close();
  },
});

/** Log the demo owner in through the context's cookie jar (the Set-Cookie
 * session cookie from the login response lands in the browser's cookie
 * store, which the app's cookie-driven refresh rotation reads). Retries
 * through the durable 30s wrong-password gate the auth flow seeds, same
 * policy as apiLogin. */
export async function loginContext(ctx: BrowserContext): Promise<TokenPair> {
  let lastStatus = 0;
  let lastError = "";
  for (let attempt = 0; attempt < 20; attempt++) {
    const res = await ctx.request.post(`${API_URL}/api/v1/auth/login`, {
      data: { email: DEMO_OWNER_EMAIL, password: DEMO_PASSWORD },
    });
    if (res.ok()) return (await res.json()) as TokenPair;
    lastError = await res.text();
    if (res.status() !== 401) throw new Error(`context login failed (${res.status()})`);
    await new Promise((resolve) => setTimeout(resolve, 3_000));
  }
  throw new Error(`context login failed after gate retries: ${lastError}`);
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
