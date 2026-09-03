// Integration test: /provenance — filter application, keyset paging
// via next_cursor, and before/after rendering.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import ProvenancePage from "../../app/provenance/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/provenance",
}));

function event(overrides: Record<string, unknown> = {}) {
  return {
    write_id: "80000000-0000-4000-8000-000000000001",
    entity: "ticket",
    entity_id: "81000000-0000-4000-8000-000000000001",
    seq: 12,
    op: "update",
    actor_kind: "human",
    actor_id: ME_FIXTURE.user_id,
    field: "status",
    before: "open",
    after: "waiting",
    undo: false,
    occurred_at: "2026-09-02T10:00:00Z",
    idempotency_key: null,
    ruleset_hash: null,
    session_id: null,
    skill: null,
    ...overrides,
  };
}

let rows: Record<string, unknown>[];

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/provenance", ({ query }) => {
    let out = rows;
    if (query.actor_kind !== undefined && query.actor_kind !== "") {
      out = out.filter((r) => r.actor_kind === query.actor_kind);
    }
    if (query.cursor !== undefined && query.cursor !== "") {
      out = out.filter((r) => Number(r.seq) > 12);
    }
    if (query.entity !== undefined && query.entity !== "") {
      out = out.filter((r) => r.entity === query.entity);
    }
    return {
      json: {
        events: out,
        next_cursor: out.length > 0 && query.cursor === undefined ? "cur-1" : null,
      },
    };
  });
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  rows = [
    event(),
    event({
      write_id: "80000000-0000-4000-8000-000000000002",
      seq: 14,
      actor_kind: "agent",
      field: "priority",
      before: "p3",
      after: "p2",
      skill: "triage",
      session_id: "82000000-0000-4000-8000-000000000001",
    }),
  ];
});

describe("/provenance integration", () => {
  it("renders field-level before/after rows with actor attribution", async () => {
    setupApi();
    render(
      <AuthProvider>
        <ProvenancePage />
      </AuthProvider>,
    );
    await screen.findByText("status");
    expect(screen.getByText("open")).toBeInTheDocument();
    expect(screen.getByText("waiting")).toBeInTheDocument();
    expect(screen.getByText("priority")).toBeInTheDocument();
    cleanup();
  });

  it("pages with the keyset cursor", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ProvenancePage />
      </AuthProvider>,
    );
    await screen.findByText("status");
    await user.click(screen.getByRole("button", { name: "Next page" }));
    await waitFor(() => {
      const calls = mock.callsTo("GET", "/api/v1/provenance");
      expect(calls[calls.length - 1]?.query.cursor).toBe("cur-1");
    });
    cleanup();
  });

  it("shows the typed empty state when filters match nothing", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ProvenancePage />
      </AuthProvider>,
    );
    await screen.findByText("status");
    await user.type(screen.getByLabelText("Entity"), "invoice");
    await user.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => {
      const calls = mock.callsTo("GET", "/api/v1/provenance");
      expect(calls[calls.length - 1]?.query.entity).toBe("invoice");
    });
    await screen.findByText(/No provenance rows match/);
    cleanup();
  });
});
