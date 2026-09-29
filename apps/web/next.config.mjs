// Next.js configuration for the Marula web app.
//
// Purpose: transpile the in-repo TypeScript source packages (@marula-core/sdk,
// @marula-core/layout ship TS entry points) and emit a standalone server bundle
// for the deploy/Dockerfile.web runtime image.
// Non-purpose: no custom webpack, no runtime config — defaults otherwise.
import path from "node:path";

/** @type {import('next').NextConfig} */
const nextConfig = {
  transpilePackages: ["@marula-core/layout", "@marula-core/sdk"],
  output: "standalone",
  // Pin the output-tracing root to apps/web so the standalone bundle path
  // is deterministic (apps/web/.next/standalone/server.js) even inside
  // git worktrees, where Next may otherwise infer the wrong repo root.
  outputFileTracingRoot: path.join(import.meta.dirname),
};

export default nextConfig;
