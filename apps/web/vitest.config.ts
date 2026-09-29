// Vitest runner configuration for @marula-core/web.
//
// Purpose: one config serves both test families — headless unit suites of
// pure lib modules (dispatch drag-state reducer, admin form reducers) and
// jsdom page-level integration tests that mock global fetch against
// SDK-typed handlers.
// Non-purpose: no browser E2E here — that stays in the Playwright suite;
// no backend contract testing — that lives in Rust integration tests.
import { defineConfig } from "vitest/config";
import path from "node:path";

export default defineConfig({
  esbuild: { jsx: "automatic" },
  resolve: {
    // The lib layer uses "@/..." imports (tsconfig paths); tests that pull
    // lib modules through the registry (Wave 29 commands) need the alias.
    alias: { "@": path.resolve(__dirname) },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./tests/setup.ts"],
    include: [
      "lib/**/*.test.?(c|m)[jt]s?(x)",
      "tests/**/*.test.?(c|m)[jt]s?(x)",
    ],
  },
});
