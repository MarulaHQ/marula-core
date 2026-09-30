// Integration test: /audit — the append-only ledger as a data surface.
// The pack is explicit: audit and provenance take the view switch, column
// chooser, filter bar and export control, and NO selection bulk bar.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import AuditPage from "../../app/audit/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/audit",
}));

function event(overrides: Record<string, unknown> = {}) {
  return {
    id: "90000000-0000-4000-8000-000000000001",
    action: "auth.login.succeeded",
    actor: { kind: "human", id: ME_FIXTURE.user_id },
    entity_type: "user",
    entity_id: "81000000-0000-4000-8000-000000000001",
    ip: "203.0.113.7",
    detail: { method: "password" },
    occurred_at: "2026-09-29T11:41:00Z",
    ...overrides,
  };
}

function setupApi(rows: Record<string, unknown>[] = [event(), event({
  id: "90000000-0000-4000-8000-000000000002",
  action: "ticket.status.changed",
  actor: { kind: "agent", id: "82000000-0000-4000-8000-000000000001" },
  entity_type: "ticket",
  entity_id: "81000000-0000-4000-8000-000000000009",
  occurred_at: "2026-09-29T11:35:00Z",
})]): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/audit-events", () => ({
    json: { events: rows, next_cursor: null },
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/audit integration", () => {
  it("carries the data-surface controls and no selection bulk bar", async () => {
    setupApi();
    render(
      <AuthProvider>
        <AuditPage />
      </AuthProvider>,
    );
    await screen.findByText("auth.login.succeeded");
    expect(screen.getByRole("tablist", { name: "Audit views" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /add filter/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Columns" })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /export audit ledger/i }).length).toBeGreaterThan(0);
    expect(screen.getByRole("radiogroup", { name: "Density" })).toBeInTheDocument();
    // The bulk bar is wrong here: an append-only ledger has no bulk action.
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    expect(screen.queryByText(/selected/)).not.toBeInTheDocument();
    cleanup();
  });

  it("expands a row in place to its structured detail", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <AuditPage />
      </AuthProvider>,
    );
    await screen.findByText("auth.login.succeeded");
    await user.click(screen.getByTestId("audit-row-90000000-0000-4000-8000-000000000001"));
    expect(await screen.findByText(/"method": "password"/)).toBeInTheDocument();
    cleanup();
  });

  it("keeps the empty ledger quiet, with no fabricated rows", async () => {
    setupApi([]);
    render(
      <AuthProvider>
        <AuditPage />
      </AuthProvider>,
    );
    expect(await screen.findByText("Nothing in the ledger yet")).toBeInTheDocument();
    cleanup();
  });

  it("surfaces a load failure in the blush family, not red", async () => {
    const mock = setupApi([]);
    mock.on("GET", "/api/v1/audit-events", () => ({
      status: 503,
      json: { error: { message: "the ledger is briefly unavailable" } },
    }));
    render(
      <AuthProvider>
        <AuditPage />
      </AuthProvider>,
    );
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveStyle({
      background: "var(--m-status-due-bg)",
      color: "var(--m-status-due-fg)",
    });
    await waitFor(() => {
      expect(mock.callsTo("GET", "/api/v1/audit-events").length).toBeGreaterThan(0);
    });
    cleanup();
  });
});
