// E2E API helpers — thin fetch wrappers over the canonical /api/v1 surface.
// The harness talks to the live demo-seeded core-api directly for scratch-data
// staging (approvals queue, calendar connections) so the UI tests exercise the
// final close-out surfaces against REAL data, never a moving mock.
//
// `scripts/e2e-stack.sh` writes E2E_API_URL with a trailing slash; every
// helper concatenates `/api/v1…`, and a doubled `//api` path 404s on the
// router. Normalize once here so the documented invocation (source the stack
// env, run Playwright) works with no hand-editing (Wave 32 lane 6).
export const API_URL = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
export const WEB_URL = process.env.E2E_WEB_URL ?? "http://localhost:3000";
/**
 * The profile the stack was built with (`release` unless the boot said
 * otherwise). One thing depends on it: the outbound-URL guard admits loopback
 * only when the build is debug AND `MARULA_DEV_TOOLS=1`, so a flow that serves
 * its own loopback feed can only run on a debug stack. Stating that beats a
 * silent `fixme` or a red gate.
 */
export const API_PROFILE = process.env.E2E_API_PROFILE ?? "release";

export const DEMO_OWNER_EMAIL = "owner@demo.local";
export const DEMO_PASSWORD = "demo-password-123";

export interface TokenPair {
  access_token: string;
  refresh_token: string;
  session_id: string;
  expires_in: number;
}

/**
 * Demo-owner login for fixtures. Retries briefly through the durable
 * auth-gate lockout the "wrong password" e2e flow seeds (Wave 19 hardening:
 * 30s backoff on one bad attempt) instead of cascading instant 401s through
 * every later test. 401 bad-credential responses regenerate nothing server
 * side; only credentials that were EVER correct clear the gate again.
 */
export async function apiLogin(
  email = DEMO_OWNER_EMAIL,
  password = DEMO_PASSWORD,
): Promise<TokenPair> {
  let lastError = "";
  for (let attempt = 0; attempt < 20; attempt++) {
    const res = await fetch(`${API_URL}/api/v1/auth/login`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ email, password }),
    });
    if (res.ok) return (await res.json()) as TokenPair;
    lastError = await res.text();
    if (res.status !== 401) throw new Error(`login failed (${res.status}): ${lastError}`);
    await new Promise((resolve) => setTimeout(resolve, 3_000));
  }
  throw new Error(`login failed after gate retries: ${lastError}`);
}

export function authedFetch(token: string) {
  return async <T>(
    path: string,
    init: { method?: string; body?: unknown } = {},
  ): Promise<T> => {
    const res = await fetch(`${API_URL}/api/v1${path}`, {
      method: init.method ?? "GET",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${token}`,
      },
      body: init.body === undefined ? undefined : JSON.stringify(init.body),
    });
    if (!res.ok) {
      throw new Error(`${init.method ?? "GET"} ${path} failed (${res.status}): ${await res.text()}`);
    }
    if (res.status === 204) return undefined as T;
    const text = await res.text();
    if (text === "") return undefined as T;
    return JSON.parse(text) as T;
  };
}

export interface DemoTicket {
  id: string;
  number: string;
  title: string;
  board_id: string;
  status_id: string;
  category?: string;
}

/** The service-desk board is demo_uuid(60) — deterministic across reseeds. */
export const DEMO_SERVICE_DESK_BOARD_ID = "6d657269-6469-616e-0000-00000000003c";

export async function firstDemoTicket(token: string): Promise<DemoTicket> {
  const get = authedFetch(token);
  const tickets = await get<DemoTicket[]>(
    `/tickets?limit=100&board_id=${DEMO_SERVICE_DESK_BOARD_ID}`,
  );
  const open = tickets.find((t) => t.category !== "closed");
  if (!open) throw new Error("no open demo ticket on the service desk board");
  return open;
}

export async function waitForApi(timeoutMs = 60_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let lastErr = "";
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`${API_URL}/api/v1/health`);
      if (res.ok) return;
      lastErr = `healthz ${res.status}`;
    } catch (e) {
      lastErr = String(e);
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  throw new Error(`core-api not healthy at ${API_URL} after ${timeoutMs}ms (${lastErr})`);
}
