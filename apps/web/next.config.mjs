// Next.js configuration for the Marula web app.
//
// Purpose: transpile the in-repo TypeScript source packages (@marula-core/sdk,
// @marula-core/layout and @marula-core/ui ship TS entry points from `src`) and
// emit a standalone server bundle for the deploy/Dockerfile.web runtime image.
// The kit is transpiled for a reason worth keeping written down: its `exports`
// used to point at `dist`, which is per-checkout build output, so a checkout
// whose kit had not been rebuilt compiled against a stale bundle — a merged kit
// export threw "formatWallClock is not a function" at runtime (2026-10-19).
//
// Performance budget (p52-frontend-perf-budget): the bundle analyzer is wired
// here behind `ANALYZE=1` so a heavy route can be opened up interactively, and
// the ceiling that actually fails a job lives in
// `scripts/check-web-bundle-budget.mjs` against `perf/bundle-budget.json`.
// Neither changes the default build: with `ANALYZE` unset this config is the
// same object it was before, and the budget gate only reads the output.
//
// Non-purpose: no custom webpack, no runtime config — defaults otherwise.
import path from "node:path";
import bundleAnalyzer from "@next/bundle-analyzer";

/** @type {import('next').NextConfig} */
const nextConfig = {
  transpilePackages: ["@marula-core/layout", "@marula-core/sdk", "@marula-core/ui"],
  output: "standalone",
  // Pin the output-tracing root to apps/web so the standalone bundle path
  // is deterministic (apps/web/.next/standalone/server.js) even inside
  // git worktrees, where Next may otherwise infer the wrong repo root.
  outputFileTracingRoot: path.join(import.meta.dirname),
};

// `ANALYZE=1 pnpm --filter @marula-core/web perf:analyze` writes the treemap to
// .next/analyze/*.html. Off by default: it wraps the webpack config and would
// otherwise cost every ordinary build.
const withBundleAnalyzer = bundleAnalyzer({ enabled: process.env.ANALYZE === "1" });

export default withBundleAnalyzer(nextConfig);
