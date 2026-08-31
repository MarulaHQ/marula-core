// Global fetch router for page-level integration tests.
//
// Purpose: vitest setup stubs globalThis.fetch ONCE — before @meridian/sdk
// is imported, so openapi-fetch's default capture lands on this wrapper —
// and delegtes every call to whichever MockApi route-table is currently
// installed. Tests register their fixtures through MockApi#install().
//
// Non-purpose: no HTTP server, no msw dependency; unmatched routes fail
// loudly rather than leaking to the real network.
export type RoutedFetchHandler = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>;

interface Registry {
  current: RoutedFetchHandler | null;
}

const REGISTRY_KEY = "__meridian_test_fetch_registry__";

function registry(): Registry {
  const g = globalThis as typeof globalThis & Record<string, unknown>;
  if (g[REGISTRY_KEY] === undefined) {
    g[REGISTRY_KEY] = { current: null } satisfies Registry;
  }
  return g[REGISTRY_KEY] as Registry;
}

/** Installs (or clears with null) the active route table handler. */
export function setFetchRouter(handler: RoutedFetchHandler | null): void {
  registry().current = handler;
}

/** Reads the active handler (test composition: wrap-then-delegate). */
export function currentFetchRouter(): RoutedFetchHandler | null {
  return registry().current;
}

async function unrouted(): Promise<Response> {
  return {
    ok: false,
    status: 599,
    headers: new Headers({ "content-type": "application/json" }),
    json: async () => ({ error: { message: "no MockApi router installed" } }),
  } as unknown as Response;
}

/** The stable fetch used by SDK clients under test. */
export async function routedFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const handler = registry().current;
  if (handler === null) return unrouted();
  return handler(input, init);
}
