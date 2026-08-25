// Next.js configuration for the Meridian web app.
//
// Purpose: transpile the in-repo TypeScript source packages (@meridian/sdk,
// @meridian/layout ship TS entry points) and emit a standalone server bundle
// for the deploy/Dockerfile.web runtime image.
// Non-purpose: no custom webpack, no runtime config — defaults otherwise.
/** @type {import('next').NextConfig} */
const nextConfig = {
  transpilePackages: ["@meridian/layout", "@meridian/sdk"],
  output: "standalone",
};

export default nextConfig;
