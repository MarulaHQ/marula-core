import { defineConfig } from "vitest/config";

/**
 * Tests run in node, not a DOM: the installer's rules live in `lib/model.ts` as
 * pure functions, and a test that needs a browser to check a floor comparison
 * is testing the browser.
 */
export default defineConfig({
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});
