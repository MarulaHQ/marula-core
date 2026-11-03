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
// Non-purpose: no custom webpack, no runtime config — defaults otherwise,
// EXCEPT the response isolation policy below, which is a security floor and
// not a preference.
import path from "node:path";
import bundleAnalyzer from "@next/bundle-analyzer";

/**
 * Response isolation policy (p56-isolation-headers; audit record
 * `apps/web/response-headers-missing-coop-frame-isolation`).
 *
 * THE PROBLEM IT CLOSES. Without `Cross-Origin-Opener-Policy` a page on
 * another origin that opens the app keeps a live `WindowProxy` to it: it can
 * count the app's frames and drive its navigation, and a scriptable
 * authenticated window is a lever even where the session cookie is
 * `SameSite=Lax`. Without a framing directive the document is frameable at
 * all, which puts the same window inside someone else's page.
 *
 * WHY `same-origin` AND NOT `same-origin-allow-popups`. The app's multi-window
 * affordance (`lib/popout/popout.ts`) opens popouts of ITS OWN origin, and
 * `same-origin` keeps a same-origin popup in the opener's browsing context
 * group — the opener relationship survives, which is exactly what
 * `same-origin-allow-popups` would be needed for only if popouts went
 * cross-origin. They do not: `window.open` is called with a route path, and
 * the popouts coordinate over a `BroadcastChannel` named `marula-windows`
 * (`lib/popout/window-bus.ts`), which is origin-scoped and unaffected by
 * COOP. OIDC sign-in is a top-level redirect (`/api/v1/auth/oidc/{p}/login`),
 * never a popup handshake, so no `window.opener` contract crosses an origin.
 *
 * WHY CORP `same-origin` COSTS NOTHING HERE. CORP governs who may load THIS
 * app's responses as a subresource. Every asset the app renders is either
 * same-origin (`/_next/*`, `/public`) or a `data:` URI built in the browser;
 * there is no embeddable widget, no CDN host, and the client portal is a
 * top-level page, not a frame. Bytes the app LOADS from elsewhere are covered
 * by the other side's policy, not this one — the presigned object-store
 * download (`lib/api.tsx` `saveFromPresignedUrl`) is a CORS-mode `fetch`
 * against the store, and the store's own CORS rule is what admits it.
 *
 * WHY COEP IS ABSENT (and must stay absent without a deliberate decision).
 * COEP `require-corp` would demand `Cross-Origin-Resource-Policy` (or CORS) on
 * every cross-origin subresource. The KB renders registered oEmbed embeds —
 * `youtube-nocookie.com` iframes (`lib/editor/extensions/index.ts`) and
 * allowlisted remote images — none of which send COEP, and a framed document
 * under a COEP parent must itself opt in. Turning COEP on would break article
 * embeds, pasted images and the desktop shell's Tauri IPC before it bought
 * anything this app does not already get from COOP + CORP. See the lane's
 * REVIEW for the full list and for the measured proof.
 *
 * WHY BOTH `X-Frame-Options` AND `frame-ancestors`. XFO is the header every
 * browser that can reach this app understands; `frame-ancestors` is the one
 * the current spec governs, and it wins where both are present. They are
 * emitted together and asserted together (`tests/app/security-headers.test.ts`)
 * so a later edit cannot leave them disagreeing — `DENY` with `'self'` would
 * read as a policy and behave as the looser of the two on older engines.
 */
const SECURITY_HEADERS = [
  { key: "Cross-Origin-Opener-Policy", value: "same-origin" },
  { key: "Cross-Origin-Resource-Policy", value: "same-origin" },
  { key: "X-Frame-Options", value: "DENY" },
  { key: "Content-Security-Policy", value: "frame-ancestors 'none'" },
];

/** @type {import('next').NextConfig} */
const nextConfig = {
  transpilePackages: ["@marula-core/layout", "@marula-core/sdk", "@marula-core/ui"],
  output: "standalone",
  // Pin the output-tracing root to apps/web so the standalone bundle path
  // is deterministic (apps/web/.next/standalone/server.js) even inside
  // git worktrees, where Next may otherwise infer the wrong repo root.
  outputFileTracingRoot: path.join(import.meta.dirname),
  // Turbopack (the default builder from 16) took this pin as its resolution
  // root too, and that breaks the build: pnpm's node_modules is a symlink farm
  // rooted at the repo root, so `next` resolves outside a root of apps/web and
  // the compile dies with "Could not find the Next.js package
  // (next/package.json)". Setting `turbopack.root` to the monorepo does not
  // change the root it reports, so the builder is chosen instead: package.json
  // passes `--webpack` (see apps/web/package.json) and this pin keeps the
  // standalone layout the web image expects (apps/web/.next/standalone/server.js
  // — a monorepo tracing root nests it as standalone/apps/web/server.js and
  // breaks deploy/Dockerfile.web's COPY paths). Moving to Turbopack is a deploy
  // change, not a flag change.
  // Every route, documents and static assets alike. A narrower `source` would
  // leave the shell's own HTML — the document the opener handle and the frame
  // point at — outside the policy, which is the only response that matters
  // for this finding.
  async headers() {
    return [{ source: "/:path*", headers: SECURITY_HEADERS }];
  },
};

// `ANALYZE=1 pnpm --filter @marula-core/web perf:analyze` writes the treemap to
// .next/analyze/*.html. Off by default: it wraps the webpack config and would
// otherwise cost every ordinary build.
const withBundleAnalyzer = bundleAnalyzer({ enabled: process.env.ANALYZE === "1" });

export default withBundleAnalyzer(nextConfig);
