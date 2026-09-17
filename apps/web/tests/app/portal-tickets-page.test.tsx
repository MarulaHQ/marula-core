// Integration test: /portal/tickets — the client portal request list.
// Covers the honest stat strip from real rows, the client-side search +
// status filter, and the priority/status pill row shape (Wave 17 lane 8).
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import PortalTicketsPage from "../../app/portal/tickets/page";
import { PortalAuthProvider } from "../../lib/portal-api";
import { MockApi } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/portal/tickets",
}));

function seedPortalTokens(): void {
  window.localStorage.setItem(
    "meridian-portal-tokens",
    JSON.stringify({
      access_token: "a",
      refresh_token: "r",
      session_id: "s",
      expires_in: 3600,
    }),
  );
}

const TICKETS: Array<Record<string, unknown>> = [
  {
    id: "t1",
    number: "MER-100",
    title: "Mail server down",
    status_id: "st1",
    category: "in_progress",
    priority: "p1",
    board_id: "b1",
    created_at: new Date(Date.now() - 3 * 86_400_000).toISOString(),
    updated_at: new Date(Date.now() - 3600_000).toISOString(),
  },
  {
    id: "t2",
    number: "MER-101",
    title: "Laptop imaging",
    status_id: "st2",
    category: "waiting",
    priority: "p2",
    board_id: "b1",
    created_at: new Date(Date.now() - 2 * 86_400_000).toISOString(),
    updated_at: new Date(Date.now() - 120_000).toISOString(),
  },
  {
    id: "t3",
    number: "MER-102",
    title: "Docs cleanup",
    status_id: "st3",
    category: "closed",
    priority: "p4",
    board_id: "b1",
    created_at: new Date(Date.now() - 30 * 86_400_000).toISOString(),
    updated_at: new Date(Date.now() - 20 * 86_400_000).toISOString(),
  },
  {
    id: "t4",
    number: "MER-103",
    title: "Printer offline",
    status_id: "st4",
    category: "open",
    priority: "p3",
    board_id: "b1",
    created_at: new Date(Date.now() - 86_400_000).toISOString(),
    updated_at: new Date(Date.now() - 300_000).toISOString(),
  },
];

function setupApi(): MockApi {
  const now = new Date().toISOString();
  const mock = new MockApi();
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
  mock.on("GET", "/api/v1/portal/tickets", () => ({ json: TICKETS }));
  void now;
  mock.on("GET", "/api/v1/portal/invoices", () => ({ json: [] }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedPortalTokens();
});

describe("/portal/tickets", () => {
  it("lists rows with number, title, priority and status pills", async () => {
    const mock = setupApi();
    render(
      <PortalAuthProvider>
        <PortalTicketsPage />
      </PortalAuthProvider>,
    );
    expect(await screen.findByText("Mail server down")).toBeInTheDocument();
    expect(screen.getByText("MER-100")).toBeInTheDocument();
    expect(screen.getByText("Laptop imaging")).toBeInTheDocument();
    expect(screen.getAllByText("Waiting").length).toBeGreaterThan(0);
    expect(screen.getAllByText("P4")).toHaveLength(1);
    expect(mock.calls.some((c) => c.method === "GET" && c.path === "/api/v1/portal/tickets")).toBe(true);
    cleanup();
  });

  it("shows the honest stat strip (active + waiting counts)", async () => {
    setupApi();
    render(
      <PortalAuthProvider>
        <PortalTicketsPage />
      </PortalAuthProvider>,
    );
    const strip = await screen.findByTestId("tickets-stat-strip");
    expect(strip.textContent).toContain("2");
    expect(strip.textContent).toContain("active");
    expect(strip.textContent).toContain("1");
    expect(strip.textContent).toContain("waiting on you");
    cleanup();
  });

  it("filters by status chip and searches client-side without refetching", async () => {
    const mock = setupApi();
    render(
      <PortalAuthProvider>
        <PortalTicketsPage />
      </PortalAuthProvider>,
    );
    await screen.findByText("Mail server down");
    const before = mock.callsTo("GET", "/api/v1/portal/tickets").length;

    fireEvent.click(screen.getByRole("button", { name: "Waiting" }));
    await waitFor(() => expect(screen.queryByText("Mail server down")).toBeNull());
    expect(screen.getByText("Laptop imaging")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "All" }));
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "docs" } });
    expect(screen.queryByText("Mail server down")).toBeNull();
    expect(screen.getByText("Docs cleanup")).toBeInTheDocument();

    expect(mock.callsTo("GET", "/api/v1/portal/tickets").length).toBe(before);
    cleanup();
  });

  it("shows the empty state when no requests exist", async () => {
    const mock = setupApi();
    mock.on("GET", "/api/v1/portal/tickets", () => ({ json: [] }));
    render(
      <PortalAuthProvider>
        <PortalTicketsPage />
      </PortalAuthProvider>,
    );
    expect(
      await screen.findByText(/No requests yet/),
    ).toBeInTheDocument();
    cleanup();
  });
});
