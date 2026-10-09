// Integration test: /portal/tickets — the client portal request list.
// Pins the p41 pack redesign: card rows split into Open and Recently resolved,
// the plain-worded status pill with its mark, the gold-highlighted waiting row
// (the one state that needs her), and the pack's empty state. The list has no
// filters, no search, no priority and no stat strip — a client list of a
// handful of rows does not need a data toolbar (DESIGN-NOTES-portal.md §2).
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
    "marula-portal-tokens",
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
    number: "MAR-100",
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
    number: "MAR-101",
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
    number: "MAR-102",
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
    number: "MAR-103",
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
  mock.on("GET", "/api/v1/portal/invoices", () => ({ json: [] }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedPortalTokens();
});

describe("/portal/tickets", () => {
  it("lists rows with title, number, plain-worded status and no priority", async () => {
    const mock = setupApi();
    render(
      <PortalAuthProvider>
        <PortalTicketsPage />
      </PortalAuthProvider>,
    );
    expect(await screen.findByText("Mail server down")).toBeInTheDocument();
    // The number lives on the row's meta line, never alone.
    expect(screen.getByText(/MAR-100/)).toBeInTheDocument();
    expect(screen.getByText("Laptop imaging")).toBeInTheDocument();
    // The client's words, not our statuses (lib/portal-copy.ts owns them).
    expect(screen.getAllByText("We're on it").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Waiting for you").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Resolved").length).toBeGreaterThan(0);
    // PRIORITY NEVER APPEARS on the client's list, even though the API sends it.
    expect(screen.queryByText(/P[1-4]/)).toBeNull();
    expect(mock.calls.some((c) => c.method === "GET" && c.path === "/api/v1/portal/tickets")).toBe(true);
    cleanup();
  });

  it("splits the list into Open and Recently resolved, and only the waiting row is gold", async () => {
    setupApi();
    render(
      <PortalAuthProvider>
        <PortalTicketsPage />
      </PortalAuthProvider>,
    );
    const open = await screen.findByTestId("portal-tickets-open");
    expect(open.textContent).toContain("Mail server down");
    expect(open.textContent).toContain("Laptop imaging");
    expect(open.textContent).toContain("Printer offline");
    const resolved = screen.getByTestId("portal-tickets-resolved");
    expect(resolved.textContent).toContain("Docs cleanup");

    // "Waiting for you" is THE ONLY highlighted row: gold tint, gold edge bar.
    const waitingRow = screen.getByTestId("portal-ticket-MAR-101");
    expect(waitingRow.className).toContain("portal-row--waiting");
    for (const id of ["portal-ticket-MAR-100", "portal-ticket-MAR-103"]) {
      expect(screen.getByTestId(id).className).not.toContain("portal-row--waiting");
    }
    cleanup();
  });

  it("offers the CSAT invitation on resolved rows and nothing on open ones", async () => {
    const mock = setupApi();
    render(
      <PortalAuthProvider>
        <PortalTicketsPage />
      </PortalAuthProvider>,
    );
    await screen.findByText("Docs cleanup");
    expect(screen.queryByTestId("portal-csat-open-MAR-100")).toBeNull();
    fireEvent.click(screen.getByTestId("portal-csat-open-MAR-102"));
    expect(await screen.findByRole("dialog", { name: "How did we do?" })).toBeInTheDocument();
    // The dialog hands off to the CSAT surface; no request fires from here.
    expect(
      mock.calls.some((c) => c.path.startsWith("/api/v1/portal/csat")),
    ).toBe(false);
    cleanup();
  });

  it("shows the pack's empty state when no requests exist", async () => {
    const mock = setupApi();
    mock.on("GET", "/api/v1/portal/tickets", () => ({ json: [] }));
    render(
      <PortalAuthProvider>
        <PortalTicketsPage />
      </PortalAuthProvider>,
    );
    expect(
      await screen.findByText(/Nothing open right now/),
    ).toBeInTheDocument();
    // An empty page is one sentence and one action (the pack's state 11).
    expect(screen.getByText(/Contact your service desk/)).toBeInTheDocument();
    cleanup();
  });
});
