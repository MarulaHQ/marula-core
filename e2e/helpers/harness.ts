// Authenticated session fixture (the "authed harness").
//
// The TokenPair is seeded into localStorage for every new page (the same
// keys the app's AuthProvider reads), so tests skip the login round-trip.
// The expiry hint is seeded the way the app's own refresh path writes it —
// without it a full navigation triggers a proactive refresh race (see
// scripts/lane7-verify.mjs).
//
// THEME MATRIX: the Playwright project name is the theme ("light" / "dark");
// every test runs once per theme against the same seeded demo env.
import { test as base, expect, type Page } from "@playwright/test";
import { apiLogin, type TokenPair } from "./api";

export const TOKENS_KEY = "meridian-tokens";
export const TOKENS_EXP_KEY = "meridian-tokens-exp";
export const THEME_KEY = "meridian-theme";

export type Theme = "light" | "dark";

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
    const theme = (testInfo.project.name === "dark" ? "dark" : "light") as Theme;
    const ctx = await browser.newContext({
      viewport: { width: 1440, height: 900 },
    });
    const page = await ctx.newPage();
    await seedAuth(page, ownerTokens, theme);
    await use(page);
    await ctx.close();
  },
});

/** Seed the token pair + expiry hint + theme into a page's localStorage. */
export async function seedAuth(page: Page, tokens: TokenPair, theme?: Theme) {
  await page.addInitScript(
    ([tokensKey, expKey, themeKey, raw, exp, theme]) => {
      window.localStorage.setItem(tokensKey, raw);
      window.localStorage.setItem(expKey, exp);
      if (theme !== null) window.localStorage.setItem(themeKey, theme);
    },
    [
      TOKENS_KEY,
      TOKENS_EXP_KEY,
      THEME_KEY,
      JSON.stringify(tokens),
      String(Date.now() + tokens.expires_in * 1000 - 30_000),
      theme ?? null,
    ] as unknown as [string, string, string, string, string, string],
  );
}

export { expect };
