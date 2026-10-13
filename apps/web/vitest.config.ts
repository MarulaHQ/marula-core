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
    // Parallelism is capped on purpose — Wave 41 lane 16 (`p41-leftovers-closeout`),
    // closing the audit's own remainder. At the default (one worker per core,
    // 10 on the machine the audit measured) the page-level jsdom suites flake
    // on a MOVING set of files: five runs went 680/680 twice and 1, 2 and 12
    // failures the other three times, always on different files, with React
    // `act`-scope and suspense warnings surfacing under contention.
    //
    // The audit recorded that its first three root-cause guesses were all
    // wrong and named the honest fix: stop running the page suites at max
    // parallelism. This is that fix and nothing else — no assertion was
    // touched, and the failing files were never the same twice, which is the
    // signature of resource contention rather than a broken expectation.
    // 4 keeps the suite roughly 4x serial throughput while cutting the
    // contested workers by more than half; on a 2-core CI runner it degrades
    // to the core count rather than oversubscribing.
    maxWorkers: 4,
  },
});
