// The web app's response isolation policy, asserted on the map Next.js will
// actually serve (p56-isolation-headers; audit record
// `apps/web/response-headers-missing-coop-frame-isolation`).
//
// WHAT THIS SUITE IS FOR. The audit's finding was an absence: no header seam
// existed, so "no policy" was the shipped behaviour and nothing could fail
// when it stayed that way. The config now publishes a policy, and this suite
// reads it back through the same entry point Next.js uses — the config's own
// `headers()` — so a value changed in `next.config.mjs`, or a route pattern
// narrowed until the app shell falls outside it, fails here rather than in a
// browser nobody looked at.
//
// WHAT IT DELIBERATELY DOES NOT DO: boot a server. The headers reaching a real
// response through the standalone build are asserted by
// `e2e/flows/response-headers.spec.ts`; this suite is the cheap half that runs
// on every `pnpm test`.
import { describe, expect, it } from "vitest";

type HeaderEntry = { key: string; value: string };
type RouteHeaders = { source: string; headers: HeaderEntry[] };
type WebConfig = { headers?: () => Promise<RouteHeaders[]> };

/**
 * The specifier is a variable on purpose. `next.config.mjs` is JavaScript and
 * `apps/web/tsconfig.json` sets `allowJs: false`, so a literal import needs a
 * hand-maintained `.d.mts` beside a file Next.js owns — one more place for the
 * policy to go stale. A non-literal specifier is typed `any` at compile time
 * and still resolves through Vitest's loader at run time.
 */
async function loadWebConfig(): Promise<WebConfig> {
  const specifier = "../../next.config.mjs";
  const loaded = (await import(specifier)) as { default: WebConfig };
  return loaded.default;
}

/** `headers()` is the only source of truth; read it the way the server does. */
async function servedHeaders(): Promise<{
  routes: RouteHeaders[];
  map: Map<string, string>;
}> {
  const config = await loadWebConfig();
  expect(config.headers, "next.config.mjs must export a headers() function").toBeTypeOf(
    "function",
  );
  const routes = await config.headers!();
  expect(routes.length, "exactly one route entry carries the policy").toBe(1);
  const route = routes[0]!;
  return {
    routes,
    // Lower-cased keys: header names are case-insensitive, and a test that
    // cares about casing would fail on a correct change.
    map: new Map(route.headers.map(({ key, value }) => [key.toLowerCase(), value])),
  };
}

/** The `frame-ancestors` source list from the CSP value, or null when the
 *  directive is absent. */
function frameAncestors(csp: string | undefined): string[] | null {
  if (csp === undefined) return null;
  for (const directive of csp.split(";")) {
    const [name, ...sources] = directive.trim().split(/\s+/);
    if (name?.toLowerCase() === "frame-ancestors") return sources;
  }
  return null;
}

describe("response isolation policy", () => {
  it("covers every route, so the app shell itself is inside the policy", async () => {
    const { routes } = await servedHeaders();
    // `/:path*` matches `/` as well as nested paths. A narrower pattern (or a
    // second entry that omits the document) leaves the one response this
    // finding is about — the HTML an opener handle or a frame points at —
    // unheadered while the suite stays green.
    expect(routes[0]!.source).toBe("/:path*");
  });

  it("isolates the opener relationship: COOP same-origin", async () => {
    const { map } = await servedHeaders();
    // NOT `same-origin-allow-popups`: the app's popouts are its own origin
    // (lib/popout/popout.ts), so `same-origin` keeps them working while
    // severing a cross-origin opener — which is the finding.
    expect(map.get("cross-origin-opener-policy")).toBe("same-origin");
  });

  it("isolates subresource reuse: CORP same-origin", async () => {
    const { map } = await servedHeaders();
    expect(map.get("cross-origin-resource-policy")).toBe("same-origin");
  });

  it("refuses to be framed: X-Frame-Options DENY", async () => {
    const { map } = await servedHeaders();
    expect(map.get("x-frame-options")).toBe("DENY");
  });

  it("refuses to be framed under the current spec: frame-ancestors 'none'", async () => {
    const { map } = await servedHeaders();
    expect(frameAncestors(map.get("content-security-policy"))).toEqual(["'none'"]);
  });

  it("keeps XFO and frame-ancestors agreeing", async () => {
    const { map } = await servedHeaders();
    const xfo = map.get("x-frame-options");
    const ancestors = frameAncestors(map.get("content-security-policy"));
    // Both headers are sent, and modern engines honour frame-ancestors. A pair
    // that disagrees is the failure worth catching: `DENY` next to
    // `frame-ancestors 'self'` reads as a refusal and behaves as permission,
    // and the looser of the two wins on the engine that reads it.
    expect(ancestors, "frame-ancestors must be present").not.toBeNull();
    const denies = xfo === "DENY";
    const allowsOnlyNone = ancestors!.length === 1 && ancestors![0] === "'none'";
    expect(
      allowsOnlyNone,
      `X-Frame-Options is ${String(xfo)}; frame-ancestors is ${ancestors!.join(" ")}`,
    ).toBe(denies);
  });

  it("leaves COEP off", async () => {
    const { map } = await servedHeaders();
    // Off means not sent, not `unsafe-none`. COEP require-corp would demand
    // CORP or CORS on every cross-origin subresource, and the KB renders
    // registered oEmbed embeds (youtube-nocookie.com) and allowlisted remote
    // images that send neither — see the lane REVIEW for the measured list.
    expect(map.has("cross-origin-embedder-policy")).toBe(false);
  });

  it("publishes exactly this set, so a new header is a reviewed change", async () => {
    const { map } = await servedHeaders();
    expect([...map.keys()].sort()).toEqual([
      "content-security-policy",
      "cross-origin-opener-policy",
      "cross-origin-resource-policy",
      "x-frame-options",
    ]);
  });
});
