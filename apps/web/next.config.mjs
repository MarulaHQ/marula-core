// Next.js configuration for the Meridian web app.
//
// Purpose: transpile the in-repo TypeScript source packages (@meridian/sdk,
// @meridian/layout ship TS entry points) and nothing else.
// Non-purpose: no custom webpack, no runtime config — defaults only.
/** @type {import('next').NextConfig} */
const nextConfig = {
  transpilePackages: ["@meridian/layout", "@meridian/sdk"],
};

export default nextConfig;
