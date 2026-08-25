// @meridian/sdk client — thin auth-aware wrapper around the generated
// OpenAPI types. All route types derive from packages/sdk/openapi.json;
// nothing here is hand-declared per endpoint.
//
// Non-purpose: no business logic, no React bindings, no retry/queueing —
// callers own policies; this owns transport + token lifecycle only.
import createClient from "openapi-fetch";
import type { paths } from "./generated/schema.js";

export type MeridianPaths = paths;

export interface TokenPair {
  access_token: string;
  refresh_token: string;
  session_id: string;
  expires_in: number;
}

export interface MeridianClientOptions {
  /** Base URL of core-api, e.g. http://localhost:8080 */
  baseUrl?: string;
  /** Injected fetch for tests / edge runtimes. Defaults to globalThis.fetch. */
  fetch?: typeof globalThis.fetch;
}

export interface RegisterRequest {
  org_name: string;
  org_slug: string;
  ticket_prefix?: string;
  email: string;
  password: string;
  display_name: string;
  timezone?: string;
}

export interface LoginRequest {
  email: string;
  password: string;
}

/**
 * Typed Meridian API client. Generated types come from `paths`; the wrapper
 * adds bearer-token management and the auth endpoints' convenience methods.
 */
export class MeridianClient {
  readonly api: ReturnType<typeof createClient<MeridianPaths>>;
  private readonly baseUrl: string;
  private readonly fetchFn: typeof globalThis.fetch;
  private accessToken: string | null = null;
  private refreshToken: string | null = null;
  private sessionId: string | null = null;

  constructor(options: MeridianClientOptions = {}) {
    this.baseUrl = (options.baseUrl ?? "").replace(/\/$/, "");
    this.fetchFn = options.fetch ?? globalThis.fetch;
    const self = this;
    this.api = createClient<MeridianPaths>({ baseUrl: this.baseUrl, fetch: this.fetchFn });
    // Attach the bearer token on every typed request when present.
    this.api.use({
      async onRequest({ request }) {
        if (self.accessToken) {
          request.headers.set("Authorization", `Bearer ${self.accessToken}`);
        }
        return request;
      },
    });
  }

  /** Sets tokens from a TokenPair response (register/login/refresh). */
  setTokens(tokens: TokenPair): void {
    this.accessToken = tokens.access_token;
    this.refreshToken = tokens.refresh_token;
    this.sessionId = tokens.session_id;
  }

  get tokens(): { accessToken: string | null; sessionId: string | null } {
    return { accessToken: this.accessToken, sessionId: this.sessionId };
  }

  /** POST /api/auth/register — bootstraps an org + its first Owner. */
  async register(body: RegisterRequest): Promise<TokenPair> {
    return this.authCall("/api/auth/register", body);
  }

  /** POST /api/auth/login. */
  async login(body: LoginRequest): Promise<TokenPair> {
    return this.authCall("/api/auth/login", body);
  }

  /** POST /api/auth/refresh — rotates the stored refresh token in place. */
  async refresh(): Promise<TokenPair> {
    if (!this.refreshToken || !this.sessionId) {
      throw new Error("no refresh token held — login first");
    }
    return this.authCall("/api/auth/refresh", {
      refresh_token: this.refreshToken,
      session_id: this.sessionId,
    });
  }

  /** POST /api/auth/logout — revokes the stored refresh session. */
  async logout(): Promise<void> {
    if (!this.refreshToken || !this.sessionId) return;
    await this.authCall("/api/auth/logout", {
      refresh_token: this.refreshToken,
      session_id: this.sessionId,
    });
    this.accessToken = null;
    this.refreshToken = null;
    this.sessionId = null;
  }

  private async authCall(
    path: "/api/auth/register" | "/api/auth/login" | "/api/auth/refresh" | "/api/auth/logout",
    body: unknown,
  ): Promise<TokenPair> {
    const response = await this.fetchFn(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (response.status === 204 || path === "/api/auth/logout") {
      if (!response.ok && path !== "/api/auth/logout") {
        throw new Error(`${path} failed (${response.status}): ${await safeError(response)}`);
      }
      return { access_token: "", refresh_token: "", session_id: "", expires_in: 0 };
    }
    if (!response.ok) {
      throw new Error(`${path} failed (${response.status}): ${await safeError(response)}`);
    }
    const tokens = (await response.json()) as TokenPair;
    this.setTokens(tokens);
    return tokens;
  }
}

async function safeError(response: Response): Promise<string> {
  try {
    const body = (await response.json()) as { error?: { message?: string } };
    return body.error?.message ?? response.statusText;
  } catch {
    return response.statusText;
  }
}
