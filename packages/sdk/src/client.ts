// @meridian/sdk client — thin auth-aware wrapper around the generated
// OpenAPI types. All route types derive from packages/sdk/openapi.json;
// nothing here is hand-declared per endpoint.
//
// Versioning stance: every emitted request targets the CANONICAL /api/v1
// surface documented by the spec. The server also serves unversioned
// legacy aliases at runtime (upgrade-compatibility-policy.md §2/§3), but
// they are undocumented compatibility seams — this client never relies on
// them.
//
// Non-purpose: no business logic, no React bindings, no retry/queueing —
// callers own policies; this owns transport + token lifecycle only.
import createClient from "openapi-fetch";
import type { paths } from "./generated/schema.js";

export type MeridianPaths = paths;

/** A typed path key of the canonical /api/v1 contract. */
export type ApiV1Path = keyof MeridianPaths & `/api/v1/${string}`;

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

/** Returned when any second factor is enrolled for the account. */
export interface MfaRequired {
  mfa_required: true;
  /** Short-lived bridge token — pass to /api/v1/auth/mfa/verify or the
   * WebAuthn begin endpoints. Grants NO API access by itself. */
  mfa_token: string;
  /** Second factors able to satisfy this login. */
  methods: string[];
}

/** Login either yields tokens directly or an MFA challenge bridge. */
export type LoginResult = TokenPair | MfaRequired;

/**
 * Runtime-typed view of the applied/prior entity snapshot inside a
 * {@link MutationMeta}-like envelope. The OpenAPI schema types snapshots as
 * opaque objects (they are entity-shaped at runtime, varying per endpoint),
 * so the SDK owns this honest accessor instead of every consumer casting.
 */
export interface EntitySnapshot {
  id: string;
  updated_at?: string;
  [field: string]: unknown;
}

/** The additive `mutation` object every gated mutation response carries. */
export interface MutationMetaView {
  after: unknown;
  before: unknown;
  seq: number;
  updated_at: string;
  version: number;
  idempotency_key?: string | null;
}

/**
 * Narrows the applied/prior snapshots of an IF2 mutation envelope.
 * Returns null for a missing snapshot (`before` is null on creates and on
 * records that predate the write trail) or one without an `id`.
 */
export function snapshotOf(meta: MutationMetaView): {
  after: EntitySnapshot | null;
  before: EntitySnapshot | null;
} {
  const asSnapshot = (value: unknown): EntitySnapshot | null => {
    if (value === null || typeof value !== "object") return null;
    const record = value as { id?: unknown };
    return typeof record.id === "string"
      ? (value as EntitySnapshot)
      : null;
  };
  return { after: asSnapshot(meta.after), before: asSnapshot(meta.before) };
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
    // Bind: bare `fetch` loses its receiver and browsers throw Illegal invocation.
    this.fetchFn = options.fetch ?? globalThis.fetch.bind(globalThis);
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

  /** POST /api/v1/auth/register — bootstraps an org + its first Owner. */
  async register(body: RegisterRequest): Promise<TokenPair> {
    return this.authCall("/api/v1/auth/register", body);
  }

  /** POST /api/v1/auth/login. */
  async login(body: LoginRequest): Promise<LoginResult> {
    const result = await this.authCall("/api/v1/auth/login", body);
    // MFA-gated logins return a bridge token instead of a token pair.
    if ("mfa_required" in result && result.mfa_required) {
      return result;
    }
    this.setTokens(result);
    return result;
  }

  /** POST /api/v1/auth/mfa/verify — completes an MFA-gated login. */
  async mfaVerify(body: { mfa_token: string; code: string }): Promise<TokenPair> {
    return this.authCall("/api/v1/auth/mfa/verify", body);
  }

  /**
   * POST /api/v1/auth/refresh — rotates the stored refresh token in place.
   * Wave 19 lane 5: when no secret is held in memory (fresh page load) the
   * httpOnly session cookie drives the rotation (credentials included);
   * the server falls back to the cookie pair and answers the same shape.
   */
  async refresh(): Promise<TokenPair> {
    if (this.refreshToken && this.sessionId) {
      return this.authCall("/api/v1/auth/refresh", {
        refresh_token: this.refreshToken,
        session_id: this.sessionId,
      });
    }
    // Cookie-only bootstrap (no ambient body credential) — requires the
    // double-submit csrf cookie value.
    return this.authCall("/api/v1/auth/refresh", {});
  }

  /** POST /api/v1/auth/logout — revokes the stored refresh session. */
  async logout(): Promise<void> {
    const had = this.refreshToken !== null || this.sessionId !== null;
    await this.authCall("/api/v1/auth/logout", had
      ? {
          refresh_token: this.refreshToken ?? "",
          session_id: this.sessionId,
        }
      : {});
    this.accessToken = null;
    this.refreshToken = null;
    this.sessionId = null;
  }

  private async authCall(
    path:
      | "/api/v1/auth/register"
      | "/api/v1/auth/login"
      | "/api/v1/auth/mfa/verify"
      | "/api/v1/auth/refresh"
      | "/api/v1/auth/logout",
    body: unknown,
  ): Promise<TokenPair> {
    const headers: Record<string, string> = {
      "content-type": "application/json",
    };
    // Cookie-session CSRF double-submit: browsers echo the non-httpOnly
    // csrf cookie the API set on the last auth response. Bearer- and
    // cookie-less calls are unaffected (server-side no-op then).
    if (typeof document !== "undefined") {
      const csrf = document.cookie
        .split(";")
        .map((c) => c.trim())
        .find((c) => c.startsWith("meridian_csrf="));
      if (csrf) headers["x-meridian-csrf"] = csrf.slice("meridian_csrf=".length);
    }
    const response = await this.fetchFn(`${this.baseUrl}${path}`, {
      method: "POST",
      headers,
      credentials: "include",
      body: JSON.stringify(body),
    });
    if (response.status === 204 || path === "/api/v1/auth/logout") {
      if (!response.ok && path !== "/api/v1/auth/logout") {
        throw await meridianError(path, response);
      }
      return { access_token: "", refresh_token: "", session_id: "", expires_in: 0 };
    }
    if (!response.ok) {
      throw await meridianError(path, response);
    }
    const tokens = (await response.json()) as TokenPair;
    this.setTokens(tokens);
    return tokens;
  }
}

/**
 * Typed auth-call failure carrying the IF3 envelope: `reason` is the
 * machine code clients branch on, `body` the whole envelope (conflict
 * extras like the `org_slug_taken` suggestion ride along), and `message`
 * the server's fixed human copy — never raw JSON or framework text.
 */
export class MeridianApiError extends Error {
  readonly path: string;
  readonly status: number;
  readonly reason?: string;
  readonly body?: unknown;

  constructor(path: string, status: number, message: string, body?: unknown) {
    super(message);
    this.name = "MeridianApiError";
    this.path = path;
    this.status = status;
    this.reason = reasonOf(body);
    this.body = body;
  }
}

function reasonOf(body: unknown): string | undefined {
  const reason = (body as { error?: { reason?: unknown } } | null | undefined)?.error?.reason;
  return typeof reason === "string" ? reason : undefined;
}

async function meridianError(path: string, response: Response): Promise<MeridianApiError> {
  const body = (await response.json().catch(() => null)) as
    | { error?: { message?: string } }
    | null;
  const message = body?.error?.message ?? response.statusText;
  return new MeridianApiError(path, response.status, message, body);
}
