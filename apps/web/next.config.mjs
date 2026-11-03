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
  // `next build` type-checks the project, and the web image builds from a
  // context that carries no tests (deliberately: .dockerignore keeps scripts/
  // and fixtures/ out). The app's tests import helper modules from those trees,
  // so with the default tsconfig the image build fails on module resolution in
  // files it never runs. The image build therefore checks app source only;
  // `pnpm typecheck` still checks the tests against the real tsconfig.
  typescript: { tsconfigPath: "./tsconfig.build.json" },
  transpilePackages: ["@marula-core/layout", "@marula-core/sdk", "@marula-core/ui"],
  output: "standalone",
  // The tracing root is the MONOREPO, not apps/web: Turbopack (the default
  // builder from 16) takes this value as its module-resolution root, and pnpm's
  // store lives at the repo root, so a tracing root of apps/web makes even
  // `next` unresolvable. The standalone tree is therefore monorepo-shaped —
  // `.next/standalone/apps/web/server.js`, with the store beside it — and every
  // consumer names that path explicitly.
  outputFileTracingRoot: path.join(import.meta.dirname, "../.."),
  // Turbopack (the default builder from 16) takes this pin as its module-
  // resolution root, so it has to be the monorepo root: pnpm's store lives
  // there and resolving `next` through a symlink that escapes apps/web fails
  // with "Could not find the Next.js package". The standalone tree therefore
  // nests as `.next/standalone/apps/web/server.js`, and every consumer of that
  // path names it explicitly — deploy/Dockerfile.web, scripts/e2e-build-web.sh,
  // scripts/build-desktop.sh and desktop/src/main.rs.
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
