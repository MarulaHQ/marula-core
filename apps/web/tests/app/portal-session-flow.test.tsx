// Portal session flow — Wave 47 lane A1 (`p47-portal-session-hardening`).
//
// Pins the two halfs of the signed-in portal defect BUILD-PROMPT follow-up 11
// recorded (docs/BUILD-PROMPT-VERDICTS.md:270, evidence
// design/qa-p34-spec-verdict-sweep/REVIEW.md batch 5):
//
//   (A) an in-page login is OBSERVED by PortalAuthProvider — a mount-time
//       /portal/me restore probe that settles AFTER the credentials land must
//       never stomp the fresh session back to anonymous (which is what
//       useRequirePortalAuth then bounces to /portal/login). The guard is
//       never weakened: with genuinely no session the bounce still fires.
//   (B) the client half of the double-submit contract: the cookie-driven
//       rotation echoes the readable `marula_csrf` value on x-marula-csrf, so
//       a full page load re-auths through the cookie jar (the server half is
//       pinned by portal_flow's cookie round-trip test).
//
// The harness is a local deferred-capable fetch router (tests/helpers/
// fetch-router's registry) rather than MockApi: the mount probe's response
// has to be held open across a form submit, which a synchronous route table
// cannot express.
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import PortalLoginPage from "../../app/portal/login/page";
import PortalTicketsPage from "../../app/portal/tickets/page";
import { PortalAuthProvider, clearPortalTokens, portalApi } from "../../lib/portal-api";
import { setFetchRouter } from "../helpers/fetch-router";

const nav = vi.hoisted(() => ({ replaceCalls: [] as string[] }));

vi.mock("next/navigation", () => ({
  useRouter: () => ({
    replace: (to: string) => nav.replaceCalls.push(to),
    push: (to: string) => nav.replaceCalls.push(to),
    prefetch: vi.fn(),
  }),
  usePathname: () => "/portal",
  useSearchParams: () => new URLSearchParams(),
}));

/** The portal profile every auth response in this file answers with. */
const PROFILE = {
  id: "u1",
  org_id: "o1",
  client_id: "c1",
  email: "ops@acme.com",
  display_name: "Ops User",
  role: "contact",
};

const TOKENS = {
  access_token: "access-token",
  refresh_token: "refresh-token",
  session_id: "session-1",
  expires_in: 900,
  portal_user: PROFILE,
};

const TICKETS = [
  {
    id: "t1",
    number: "MAR-100",
    title: "Mail server down",
    status_id: "st1",
    category: "in_progress",
    priority: "p1",
    board_id: "b1",
    created_at: new Date(Date.now() - 3 * 86_400_000).toISOString(),
    updated_at: new Date(Date.now() - 3600_000).toISOString(),
  },
];

interface Reply {
  status: number;
  json: unknown;
}

interface Route {
  method: string;
  path: string;
  respond: (
    body: unknown,
    headers: Record<string, string>,
  ) => Reply | Promise<Reply>;
}

let routes: Route[] = [];

function jsonResponse(status: number, json: unknown): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: new Headers({ "content-type": "application/json" }),
    json: async () => json,
    text: async () => JSON.stringify(json),
  } as unknown as Response;
}

function installRoutes(): void {
  setFetchRouter(async (input, init) => {
    const url =
      typeof input === "string"
        ? new URL(input)
        : input instanceof URL
          ? input
          : new URL((input as Request).url);
    const method = (init?.method ?? "GET").toUpperCase();
    const route = routes.find((r) => r.method === method && r.path === url.pathname);
    if (route === undefined) {
      return jsonResponse(500, {
        error: { message: `unmatched ${method} ${url.pathname}` },
      });
    }
    const body =
      typeof init?.body === "string" && init.body.length > 0
        ? (JSON.parse(init.body) as unknown)
        : undefined;
    const headers = (init?.headers ?? {}) as Record<string, string>;
    const out = await route.respond(body, headers);
    return jsonResponse(out.status, out.json);
  });
}

/** jsdom's cookie jar, emptied between tests. */
function clearCookies(): void {
  for (const pair of document.cookie.split(";")) {
    const name = (pair.split("=")[0] ?? "").trim();
    if (name !== "") {
      document.cookie = `${name}=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/`;
    }
  }
}

beforeEach(() => {
  clearCookies();
  clearPortalTokens();
  nav.replaceCalls.length = 0;
  installRoutes();
});

afterEach(() => {
  setFetchRouter(null);
});

/** Signs in through the real form and waits for the provider to publish. */
async function submitPasswordLogin(): Promise<void> {
  // The card opens on the link half; the password form is one click away.
  fireEvent.click(await screen.findByTestId("portal-use-password"));
  const email = document.querySelector("#portal-email") as HTMLInputElement;
  const password = document.querySelector("#portal-password") as HTMLInputElement;
  fireEvent.change(email, { target: { value: "ops@acme.com" } });
  fireEvent.change(password, { target: { value: "correct-horse-battery" } });
  fireEvent.click(screen.getByTestId("portal-password-submit"));
  await waitFor(() =>
    expect(nav.replaceCalls).toContain("/portal/tickets"),
  );
}

describe("portal session flow (Wave 47 lane A1)", () => {
  it("(A) keeps a fresh login when the mount-time restore probe settles afterwards", async () => {
    // The /portal/me probe fired at provider mount is still in flight when
    // the credentials land — a cold first load or a slow API. Its eventual
    // 401 must not overwrite the session the provider just published.
    let releaseMe: (reply: Reply) => void = () => {};
    const meGate = new Promise<Reply>((resolve) => {
      releaseMe = resolve;
    });
    routes = [
      { method: "GET", path: "/api/v1/portal/me", respond: async () => await meGate },
      {
        method: "POST",
        path: "/api/v1/portal/login",
        respond: () => ({ status: 200, json: TOKENS }),
      },
      {
        method: "POST",
        path: "/api/v1/portal/refresh",
        respond: () => ({ status: 401, json: { error: { message: "unauthorized" } } }),
      },
      { method: "GET", path: "/api/v1/portal/tickets", respond: () => ({ status: 200, json: TICKETS }) },
      { method: "GET", path: "/api/v1/portal/invoices", respond: () => ({ status: 200, json: [] }) },
    ];

    render(
      <PortalAuthProvider>
        <PortalLoginPage />
        <PortalTicketsPage />
      </PortalAuthProvider>,
    );

    // The form is usable while the restore probe is still outstanding.
    await screen.findByTestId("portal-use-password");
    await submitPasswordLogin();

    // The guarded page renders for the freshly signed-in session.
    expect(await screen.findByText("Mail server down")).toBeInTheDocument();

    // NOW the mount probe fails — exactly the wave-34 bounce trigger.
    await act(async () => {
      releaseMe({ status: 401, json: { error: { message: "unauthorized" } } });
      for (let i = 0; i < 8; i++) await Promise.resolve();
      await new Promise((r) => setTimeout(r, 0));
    });

    // The provider observed the login: no bounce to the gate, content stays.
    expect(nav.replaceCalls).not.toContain("/portal/login");
    expect(screen.getByText("Mail server down")).toBeInTheDocument();
    cleanup();
  });

  it("(A) still bounces to /portal/login when there is genuinely no session (guard not weakened)", async () => {
    routes = [
      {
        method: "GET",
        path: "/api/v1/portal/me",
        respond: () => ({ status: 401, json: { error: { message: "unauthorized" } } }),
      },
      {
        method: "POST",
        path: "/api/v1/portal/refresh",
        respond: () => ({ status: 401, json: { error: { message: "unauthorized" } } }),
      },
    ];

    render(
      <PortalAuthProvider>
        <PortalTicketsPage />
      </PortalAuthProvider>,
    );

    await waitFor(() => expect(nav.replaceCalls).toContain("/portal/login"));
    expect(screen.queryByText("Mail server down")).toBeNull();
    cleanup();
  });

  it("(B) restores a full page load through the cookie rotation and echoes the double-submit header", async () => {
    // The browser's full-page-load path: no in-memory tokens, the httpOnly
    // jar carries the session, and the JS-readable csrf value rides the
    // rotation request as x-marula-csrf.
    document.cookie = "marula_csrf=csrf-secret-value; path=/";
    const seenHeaders: Record<string, string>[] = [];
    let meCalls = 0;
    routes = [
      {
        method: "GET",
        path: "/api/v1/portal/me",
        respond: (_body, headers) => {
          meCalls += 1;
          seenHeaders.push(headers);
          return meCalls === 1
            ? { status: 401, json: { error: { message: "unauthorized" } } }
            : { status: 200, json: PROFILE };
        },
      },
      {
        method: "POST",
        path: "/api/v1/portal/refresh",
        respond: (_body, headers) => {
          seenHeaders.push(headers);
          return { status: 200, json: TOKENS };
        },
      },
      { method: "GET", path: "/api/v1/portal/tickets", respond: () => ({ status: 200, json: TICKETS }) },
      { method: "GET", path: "/api/v1/portal/invoices", respond: () => ({ status: 200, json: [] }) },
    ];

    render(
      <PortalAuthProvider>
        <PortalTicketsPage />
      </PortalAuthProvider>,
    );

    // The restore rode 401 -> rotation -> retry and landed signed-in.
    expect(await screen.findByText("Mail server down")).toBeInTheDocument();
    expect(nav.replaceCalls).not.toContain("/portal/login");

    const rotation = seenHeaders.find((h) => "x-marula-csrf" in h);
    expect(rotation?.["x-marula-csrf"]).toBe("csrf-secret-value");
    // The retried probe carries the fresh bearer.
    const retried = seenHeaders[seenHeaders.length - 1];
    expect(retried?.authorization).toBe("Bearer access-token");
    cleanup();
  });

  it("(A) shares one rotation between the restore probe and the page's first fetch", async () => {
    // A full page load fires several bearer-less calls at once — the
    // provider's /portal/me probe plus this page's first data fetch. The
    // server retires the cookie-only refresh pair on first use, so a second
    // concurrent rotation draws a replay 401. Before the single-flight fix
    // one of the two callers lost that race: either the provider flipped to
    // anonymous (and the guard bounced the session) or this page's first
    // fetch rejected. Both must land on one shared rotation.
    let rotations = 0;
    let meCalls = 0;
    routes = [
      {
        method: "GET",
        path: "/api/v1/portal/me",
        respond: () => {
          meCalls += 1;
          return meCalls === 1
            ? { status: 401, json: { error: { message: "unauthorized" } } }
            : { status: 200, json: PROFILE };
        },
      },
      {
        method: "POST",
        path: "/api/v1/portal/refresh",
        respond: () => {
          rotations += 1;
          // The single-use pair: a second rotation of the same pair replays.
          return rotations === 1
            ? { status: 200, json: TOKENS }
            : { status: 401, json: { error: { message: "unauthorized" } } };
        },
      },
      {
        method: "GET",
        path: "/api/v1/portal/tickets",
        respond: (_body, headers) =>
          "authorization" in headers
            ? { status: 200, json: TICKETS }
            : { status: 401, json: { error: { message: "unauthorized" } } },
      },
      { method: "GET", path: "/api/v1/portal/invoices", respond: () => ({ status: 200, json: [] }) },
    ];

    const [profile, tickets] = await Promise.all([
      portalApi.me(),
      portalApi.listTickets(),
    ]);

    expect(profile.email).toBe("ops@acme.com");
    expect(tickets).toHaveLength(1);
    expect(rotations).toBe(1);
    cleanup();
  });
});
