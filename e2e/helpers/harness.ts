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
import { API_URL, DEMO_OWNER_EMAIL, DEMO_PASSWORD, WEB_URL, apiLogin, type TokenPair } from "./api";
import { resetSurfacePrefs } from "./prefs";

export const TOKENS_KEY = "marula-tokens";
export const TOKENS_EXP_KEY = "marula-tokens-exp";
export const THEME_KEY = "marula-theme";
/** Wave 24: the persisted preset attribute (localStorage key the app's
 * pre-paint bootstrap reads; empty/absent = the tokens.css baseline). */
export const THEME_PRESET_KEY = "marula-theme-preset";

export type Theme = "light" | "dark";
/** A shipped default preset id (playwright only covers shipped defaults
 * — arbitrary custom packs are not part of the E2E matrix). */
// Repointed in wave41 lane 2. `tangerine` is renamed to `marula` (migration
// 0115) and `slate`/`copper` are DROPPED: they were second-brand neutrals with
// no authority in the brand spec, and the axis they wanted to move is now
// `marula` vs `marula-warm`.
export type ThemePresetSeed = "marula" | "marula-warm" | "high_contrast";

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

  authedPage: async ({ browser }, use, testInfo) => {
    // The base projects are named exactly `light` / `dark`; the preset
    // projects suffix them (`warm-preset-dark`). Both shapes must resolve
    // the theme, or the `dark` project silently runs light.
    const projectName = testInfo.project.name;
    const theme = (
      projectName === "dark" || projectName.endsWith("-dark") ? "dark" : "light"
    ) as Theme;
    const preset = testInfo.project.name.startsWith("warm-preset")
      ? "marula-warm"
      : testInfo.project.name.startsWith("high-contrast-preset")
        ? "high_contrast"
        : null;
    const ctx = await browser.newContext({
      // Relative page.goto() needs the base the config uses; a context made by
      // hand does not inherit it from `use`.
      baseURL: WEB_URL,
      viewport: { width: 1440, height: 900 },
    });
    const page = await ctx.newPage();
    // Wave 19 lane 5 auth: the app keeps tokens memory-only and rotates via
    // the httpOnly session cookie, so the browser context must OWN the
    // session — log in through the context's request jar (shares the cookie
    // store with pages) instead of seeding localStorage.
    //
    // ONE session, not two (Wave 62 lane `p62-gate-truth`). This used to log
    // in twice — `apiLogin()` for `ownerTokens`, then `loginContext(ctx)` —
    // and seed localStorage from the FIRST pair while the cookie jar held the
    // SECOND. The app then presented a token the server had already rotated,
    // which arms the escalating refresh gate and turns unrelated specs red.
    // The pair this fixture seeds is now the pair the cookie jar owns, and
    // `ownerTokens` is requested only by the tests that stage data through the
    // API (it is a separate, non-rotating bearer session).
    const tokens = await loginContext(ctx);
    // Order-independence (p37 doctrine): the demo stack resets per-user
    // tours rows at boot, so the FIRST data-flow spec to visit a surface
    // gets the modal onboarding tour over its content and every later one
    // doesn't — run-order flake. Default the account to tours-off here;
    // 13-contextual-help re-arms (disabled=false, completed="") when it
    // asserts on tour behavior. (p29-accessibility-deep: found while the
    // tour modal blocked flow 02's board.)
    await ctx.request.put(`${API_URL}/api/v1/settings/user`, {
      data: { key: "tours.disabled", value: true },
      headers: { authorization: `Bearer ${tokens.access_token}` },
    });
    // Saved surface preferences are per-user state, not spec state: a case that
    // ends on a filtered tab changes what the next case sees (e2e/README.md
    // rule 4). Reset per case, before the page reads them.
    await resetSurfacePrefs(tokens.access_token);
    await seedAuth(page, tokens, theme, preset);
    await use(page);
    await ctx.close();
  },
});

/**
 * A failed test that ended on the sign-in page is almost always the durable
 * auth gate, not a broken surface: an earlier flow's deliberate refusal shut
 * every password login for the window, and the app cannot rotate, so it
 * bounces to `/login`. The capture then shows a page the test never navigated
 * to, which reads as "the surface broke". Annotate it so the report says what
 * happened (Wave 47 lane A26's row, closed here as `p62-gate-truth`).
 */
test.afterEach(async ({ page }, testInfo) => {
  if (testInfo.status === testInfo.expectedStatus) return;
  if (!page.url().includes("/login")) return;
  testInfo.annotations.push({
    type: "auth-gate-window",
    description:
      "the test ended on /login: the durable auth gate denies logins for a window after a deliberate refusal (e2e/README.md rule 1)",
  });
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
 * localStorage (the project determines both). */export async function seedAuth(
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
