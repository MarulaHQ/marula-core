// Shared mock-fetch harness for page-level integration tests.
//
// Purpose: installs a route-table-based global.fetch stub so real SDK
// clients (openapi-fetch typing intact) issue requests through it.
// Records every call (method/path/body) for payload assertions, returns
// fabrications shaped like fetch Responses.
//
// Non-purpose: no HTTP listening and no Node interceptors (msw etc.);
// everything here is dependency-light and synchronous.
import { setFetchRouter } from "./fetch-router";
import { saveTokens } from "../../lib/api";

interface RecordedCall {
  method: string;
  path: string;
  query: Record<string, string>;
  body: unknown;
}

type RouteHandler = (
  ctx: Pick<RecordedCall, "query" | "body">,
) => { status?: number; json: unknown };

export class MockApi {
  private readonly routes = new Map<string, RouteHandler>();
  readonly calls: RecordedCall[] = [];

  /** Register a JSON responder for METHOD PATH ('?' ignored). */
  on(method: string, path: string, handler: RouteHandler): this {
    this.routes.set(`${method.toUpperCase()} ${path}`, handler);
    return this;
  }

  callsTo(method: string, path: string): RecordedCall[] {
    return this.calls.filter((c) => c.method === method.toUpperCase() && c.path === path);
  }

  lastBody<T>(method: string, path: string): T {
    const relevant = this.callsTo(method, path);
    const call = relevant[relevant.length - 1];
    if (call === undefined) throw new Error(`no ${method} ${path} recorded`);
    return call.body as T;
  }

  /**
   * Route global fetch through this table. The underlying stub is installed
   * once in tests/setup.ts; here we only swap the active handler.
   */
  install(): MockApi {
    const self = this;
    setFetchRouter(async (input, init) => {
      // openapi-fetch passes either (url, init) or a constructed Request
      // (mutation verbs) — normalize both.
      const isRequest = typeof input !== "string" && !(input instanceof URL);
      const url =
        typeof input === "string"
          ? new URL(input)
          : input instanceof URL
            ? input
            : new URL(input.url);
      const method = (
        isRequest && typeof input !== "string" && !(input instanceof URL)
          ? input.method
          : (init?.method ?? "GET")
      ).toUpperCase();
      let rawBody: string | undefined =
        typeof init?.body === "string"
          ? init.body
          : isRequest && typeof input !== "string" && !(input instanceof URL)
            ? await input.clone().text()
            : undefined;
      if (rawBody !== undefined && rawBody.length === 0) rawBody = undefined;
      const parsedBody =
        rawBody !== undefined ? (JSON.parse(rawBody) as unknown) : undefined;
      const query: Record<string, string> = {};
      url.searchParams.forEach((v, k) => {
        query[k] = v;
      });
      self.calls.push({ method, path: url.pathname, query, body: parsedBody });

      const handler = self.routes.get(`${method} ${url.pathname}`);
      if (handler === undefined) {
        return makeJsonResponse({ error: { message: `unmatched ${method} ${url.pathname}` } }, 500);
      }
      try {
        const out = handler({ query, body: parsedBody });
        return makeJsonResponse(out.json, out.status ?? 200);
      } catch (e) {
        return makeJsonResponse(
          { error: { message: e instanceof Error ? e.message : String(e) } },
          400,
        );
      }
    });
    return this;
  }
}

/** Minimal fetch-Response stand-in openapi-fetch relies on (ok/status/json/text). */
function makeJsonResponse(json: unknown, status: number): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: new Headers({ "content-type": "application/json" }),
    json: async () => json,
    text: async () => JSON.stringify(json),
  } as unknown as Response;
}

/** Seeds stored auth so AuthProvider restores instantly (no refresh call). */
/**
 * Wave 19 lane 5: tokens persist MEMORY-ONLY in lib/api (httpOnly cookie
 * owns the refresh secret now) — tests seed through saveTokens (the same
 * public surface pages use), not the localStorage family, which saveTokens
 * clears on first write.
 */
export function seedAuthTokens(): void {
  saveTokens({
    access_token: "test-access",
    refresh_token: "test-refresh",
    session_id: "sess",
    expires_in: 900,
  });
}

/** Standard /api/v1/me answer matching MeResponse (owner-permissioned). */
export const ME_FIXTURE = {
  user_id: "00000000-0000-4000-8000-000000000001",
  org_id: "00000000-0000-4000-8000-000000000002",
  email: "owner@marula.test",
  display_name: "Owner",
  timezone: "UTC",
  permissions: ["special:manage_boards"],
};
