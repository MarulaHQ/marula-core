/// <summary>
/// Vitest config: jsdom for component tests; token tests run in node env via per-file pragma.
/// </summary>
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./tests/setup.ts"],
    include: ["tests/**/*.test.?(c|m)[jt]s?(x)"],
  },
});
