/// <summary>
/// Vitest config: node environment — the dispatch reducer is pure logic
/// with no DOM dependency, so headless node keeps it fast.
/// </summary>
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    globals: true,
    include: ["lib/**/*.test.?(c|m)[jt]s?(x)"],
  },
});
