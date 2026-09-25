// Fetch wrapper: one transparent token refresh on a 401.
//
// Purpose: the SDK client attaches the in-memory access token on every
// typed call but never retries. Native sessions outlive an access-token TTL,
// so the app injects this wrapper into MeridianClient: clone the request
// up-front, replay it once with a freshly rotated token when the server
// answers 401. Refresh itself is single-flighted by MeridianClient.
//
// Non-purpose: no retry loops (one attempt), no queueing, no logging.

const AUTH_PATHS = ["/api/v1/auth/refresh", "/api/v1/auth/login", "/api/v1/auth/mfa/verify"];

export function fetchWithAuthRefresh(
  baseFetch: typeof globalThis.fetch,
  freshToken: () => Promise<string | null>,
): typeof globalThis.fetch {
  return async (input, init) => {
    const request = input instanceof Request ? input : null;
    const clone = request?.clone() ?? null;
    const response = await baseFetch(input as Parameters<typeof baseFetch>[0], init);
    if (response.status !== 401 || clone === null) return response;
    const url = clone.url;
    if (AUTH_PATHS.some((path) => url.includes(path))) return response;
    const token = await freshToken();
    if (token === null) return response;
    const retry = clone.clone();
    retry.headers.set("authorization", `Bearer ${token}`);
    return baseFetch(retry);
  };
}
