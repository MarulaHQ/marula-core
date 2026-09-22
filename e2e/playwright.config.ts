// Playwright E2E — Wave 14 lane 8 `p14-e2e-ci`.
//
// 10 core flows in BOTH themes: the `light` and `dark` projects run the same
// specs; every spec reads the theme through the harness fixture, and theme-
// specific assertions (dark parity) live in the flows that need them.
//
// Authenticated harness against the seeded demo env (meridian seed-demo):
// tokens are seeded into localStorage per page — the same keys the app's
// AuthProvider reads — so specs exercise the real app bootstrap, not a mock.
//
// Local run (compose stack up + demo seeded):
//   pnpm exec playwright test --config=e2e/playwright.config.ts
import { defineConfig, devices } from "@playwright/test";
import { MAX_RETRIES } from "./helpers/budget";

const API_URL = process.env.E2E_API_URL ?? "http://localhost:8080";
const WEB_PORT = process.env.E2E_WEB_PORT ?? "3000";
const WEB_URL = process.env.E2E_WEB_URL ?? `http://localhost:${WEB_PORT}`;
const CI = Boolean(process.env.CI);

export default defineConfig({
  testDir: "./flows",
  outputDir: "../test-results",
  testMatch: "**/*.spec.ts",
  globalSetup: "./global-setup.ts",
  // NOTE: the flake gate is NOT globalTeardown — Playwright flushes the
  // JSON report after teardown, so a teardown reader always sees nothing.
  // It runs as the post-test step (`node e2e/flake-gate.ts`) in CI and in
  // the `pnpm e2e` script.
  reporter: [
    ["list"],
    // Reporter paths resolve against THIS config file's dir (e2e/) — so
    // this lands at <repo>/e2e/test-results/report.json, which the
    // flake gate (cwd = repo root) reads as e2e/test-results/report.json.
    ["json", { outputFile: "test-results/report.json" }],
  ],
  // Flake budget: CI retries within the budget cap; the global teardown
  // fails the job if the number of retried tests exceeds the budget.
  retries: CI ? MAX_RETRIES : 0,
  // One worker: the flows share the seeded demo org's rows (same tickets,
  // same approvals queue). Parallel workers race on those rows and flake;
  // a deterministic gate beats a fast flaky one. Runtime stays ~3 min.
  workers: 1,
  timeout: 120_000,
  expect: { timeout: 15_000 },
  fullyParallel: true,
  forbidOnly: CI,
  use: {
    baseURL: WEB_URL,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    { name: "light", use: { ...devices["Desktop Chrome"] } },
    { name: "dark", use: { ...devices["Desktop Chrome"] } },
    // Wave 24 (leg C): the SHIPPED default presets ride the SAME flow
    // set as light/dark; arbitrary custom packs stay out of the matrix
    // (restyle-only manifests are enforced mechanically per-theme by
    // scripts/check-theme-packs.mjs instead). testIgnore keeps their
    // runtime scoped to the auth + settings + theme-matrix flows.
    {
      name: "slate-preset-light",
      use: { ...devices["Desktop Chrome"] },
      testMatch: [
        "flows/01-auth.spec.ts",
        "flows/10-settings.spec.ts",
        "flows/theme-matrix.spec.ts",
      ],
    },
    {
      name: "slate-preset-dark",
      use: { ...devices["Desktop Chrome"] },
      testMatch: [
        "flows/01-auth.spec.ts",
        "flows/10-settings.spec.ts",
        "flows/theme-matrix.spec.ts",
      ],
    },
    {
      name: "high-contrast-preset-light",
      use: { ...devices["Desktop Chrome"] },
      testMatch: [
        "flows/01-auth.spec.ts",
        "flows/10-settings.spec.ts",
        "flows/theme-matrix.spec.ts",
      ],
    },
    {
      name: "high-contrast-preset-dark",
      use: { ...devices["Desktop Chrome"] },
      testMatch: [
        "flows/01-auth.spec.ts",
        "flows/10-settings.spec.ts",
        "flows/theme-matrix.spec.ts",
      ],
    },
  ],
  // Production build served standalone (apps/web uses output:"standalone";
  // `next start` does NOT serve it). Build first with NEXT_PUBLIC_API_URL
  // set, then copy .next/static + public into the standalone root (Next does
  // not copy them itself). `pnpm e2e:build-web` does all three steps.
  webServer: {
    command: `sh -c 'cd "${process.cwd()}/apps/web" && HOSTNAME=127.0.0.1 PORT=${WEB_PORT} node .next/standalone/server.js'`,
    url: WEB_URL,
    reuseExistingServer: !CI,
    timeout: 120_000,
    env: { NEXT_PUBLIC_API_URL: API_URL },
  },
  metadata: { apiUrl: API_URL, webUrl: WEB_URL },
});
