// Vitest runner configuration for @meridian/web.
//
// Purpose: jsdom environment for page-level integration tests (mock
// global fetch → SDK-typed handlers) plus plain-node-friendly unit tests
// of lib/admin pure modules (same config serves both).
// Non-purpose: no browser E2E here — that stays in the Playwright suite.
import { defineConfig } from "vitest/config";

export default defineConfig({
  esbuild: { jsx: "automatic" },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./tests/setup.ts"],
    include: ["tests/**/*.test.?(c|m)[jt]s?(x)"],
  },
});
