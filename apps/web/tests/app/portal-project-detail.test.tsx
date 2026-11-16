// Integration test: /portal/projects/[id] — the paused card. The portal read
// carries `hold_reason` only when the PM wrote one for the client, so the card
// must say the reason when it exists and fall back to the pause itself when it
// does not.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import PortalProjectPage from "../../app/portal/projects/[id]/page";
import { PortalAuthProvider } from "../../lib/portal-api";
import { MockApi } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/portal/projects/p1",
  useParams: () => ({ id: "p1" }),
}));

function seedPortalTokens(): void {
  window.localStorage.setItem(
    "marula-portal-tokens",
    JSON.stringify({
      access_token: "a",
      refresh_token: "r",
      session_id: "s",
      expires_in: 3600,
    }),
  );
}

function project(holdReason: string | null): Record<string, unknown> {
  return {
    id: "p1",
    name: "Network refresh",
    status: "on_hold",
    status_text: "On hold",
    hold_reason: holdReason,
    due_date: null,
    steps: [],
    updates: [],
    change_orders: [],
    plan_shared: true,
  };
}

function setupApi(detail: Record<string, unknown>): MockApi {
  const mock = new MockApi();
  mock.on("POST", "/api/v1/portal/refresh", () => ({
    json: { access_token: "a", expires_in: 3600 },
  }));
  mock.on("GET", "/api/v1/portal/me", () => ({
    json: {
      id: "u1",
      org_id: "o1",
      client_id: "c1",
      email: "ops@acme.com",
      display_name: "Ops User",
      role: "contact",
    },
  }));
  mock.on("GET", "/api/v1/portal/projects/p1", () => ({ json: detail }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedPortalTokens();
});

describe("/portal/projects/[id] hold card", () => {
  it("shows the reason the PM wrote for the client", async () => {
    setupApi(project("Waiting on the new switchgear delivery"));
    render(
      <PortalAuthProvider>
        <PortalProjectPage />
      </PortalAuthProvider>,
    );
    const card = await screen.findByTestId("portal-hold");
    expect(card).toHaveTextContent("Work is paused");
    expect(card).toHaveTextContent("Waiting on the new switchgear delivery");
    cleanup();
  });

  it("falls back to saying work is paused when no reason was written", async () => {
    setupApi(project(null));
    render(
      <PortalAuthProvider>
        <PortalProjectPage />
      </PortalAuthProvider>,
    );
    const card = await screen.findByTestId("portal-hold");
    expect(card).toHaveTextContent("Work is paused.");
    cleanup();
  });
});
