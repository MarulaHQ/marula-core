// Next.js configuration for the Marula web app.
//
// Purpose: transpile the in-repo TypeScript source packages (@marula-core/sdk,
// @marula-core/layout and @marula-core/ui ship TS entry points from `src`) and
// emit a standalone server bundle for the deploy/Dockerfile.web runtime image.
// The kit is transpiled for a reason worth keeping written down: its `exports`
// used to point at `dist`, which is per-checkout build output, so a checkout
// whose kit had not been rebuilt compiled against a stale bundle — a merged kit
// export threw "formatWallClock is not a function" at runtime (2026-10-19).
// Non-purpose: no custom webpack, no runtime config — defaults otherwise.
import path from "node:path";

/** @type {import('next').NextConfig} */
const nextConfig = {
  transpilePackages: ["@marula-core/layout", "@marula-core/sdk", "@marula-core/ui"],
  output: "standalone",
  // Pin the output-tracing root to apps/web so the standalone bundle path
  // is deterministic (apps/web/.next/standalone/server.js) even inside
  // git worktrees, where Next may otherwise infer the wrong repo root.
  outputFileTracingRoot: path.join(import.meta.dirname),
};

export default nextConfig;
