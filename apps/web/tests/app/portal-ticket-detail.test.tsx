// Integration test: /portal/tickets/[id] — detail thread + reply composer.
// Covers the exact status bar (color+text), optimistic reply rendering with
// typed failure preservation, and the attachments list over the pinned
// portal endpoints (Wave 17 lane 8).
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import PortalTicketPage from "../../app/portal/tickets/[id]/page";
import { PortalAuthProvider } from "../../lib/portal-api";
import { MockApi } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/portal/tickets/t1",
  useParams: () => ({ id: "t1" }),
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

const TICKET = {
  id: "t1",
  number: "MER-100",
  title: "Mail server down",
  status_id: "st1",
  category: "in_progress",
  priority: "p1",
  board_id: "b1",
  due_at: new Date(Date.now() + 86_400_000).toISOString(),
  created_at: new Date(Date.now() - 86_400_000).toISOString(),
  updated_at: new Date(Date.now() - 3600_000).toISOString(),
  resolved_at: null,
};

const COMMENTS = [
  { id: "c1", kind: "client_visible", body: "Opened: mail is down for the office.", created_at: new Date(Date.now() - 3600_000).toISOString() },
  { id: "c2", kind: "in_house_update", body: "Vendor escalated to the provider.", created_at: new Date().toISOString() },
];

const ATTACHMENTS = [
  {
    id: "att1",
    filename: "smtp-log.txt",
    content_type: "text/plain",
    size_bytes: 2048,
    sha256: "aa",
    created_at: TICKET.updated_at,
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
      role: "it_manager",
    },
  }));
  mock.on("GET", "/api/v1/portal/tickets/t1", () => ({
    json: { ticket: TICKET, comments: COMMENTS, can_view_in_house_updates: true },
  }));
  mock.on("GET", "/api/v1/portal/tickets/t1/attachments", () => ({ json: ATTACHMENTS }));
  mock.on("POST", "/api/v1/portal/tickets/t1/reply", (ctx) => ({
    json: {
      id: "c-reply",
      kind: "client_visible",
      body: (ctx.body as { body: string }).body,
      created_at: new Date().toISOString(),
    },
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedPortalTokens();
});

describe("/portal/tickets/[id]", () => {
  it("renders the exact status bar, priority chip, and thread", async () => {
    const mock = setupApi();
    render(
      <PortalAuthProvider>
        <PortalTicketPage />
      </PortalAuthProvider>,
    );
    const bar = await screen.findByTestId("ticket-status-bar");
    expect(bar.textContent).toContain("In progress");
    expect(bar.textContent).toContain("desk is working on it");

    // Role-based glance: portal it_manager sees the in-house update, labeled.
    expect(await screen.findByText("Vendor escalated to the provider.")).toBeInTheDocument();
    expect(screen.getByText("Update from the team")).toBeInTheDocument();
    expect(screen.getByText("P1")).toBeInTheDocument();
    expect(mock.calls.some((c) => c.path === "/api/v1/portal/tickets/t1/attachments")).toBe(true);
    cleanup();
  });

  it("renders the reply optimistically and reconciles from reload", async () => {
    const mock = setupApi();
    render(
      <PortalAuthProvider>
        <PortalTicketPage />
      </PortalAuthProvider>,
    );
    const box = await screen.findByLabelText("Reply");
    fireEvent.change(box, { target: { value: "Here are more details." } });
    fireEvent.click(screen.getByRole("button", { name: "Send reply" }));

    // Optimistic: pending reply appears before the API answers.
    expect(screen.getByTestId("portal-pending-reply").textContent).toContain("Here are more details.");

    await waitFor(() => {
      expect(screen.queryByTestId("portal-pending-reply")).toBeNull();
    });
    const body = mock.lastBody<{ body: string }>("POST", "/api/v1/portal/tickets/t1/reply");
    expect(body.body).toBe("Here are more details.");
    cleanup();
  });

  it("preserves draft text and shows a typed banner on rejection", async () => {
    const mock = setupApi();
    mock.on("POST", "/api/v1/portal/tickets/t1/reply", () => ({
      status: 409,
      json: { error: { code: 409, message: "conflict: ticket is closed" } },
    }));
    render(
      <PortalAuthProvider>
        <PortalTicketPage />
      </PortalAuthProvider>,
    );
    const box = await screen.findByLabelText("Reply");
    fireEvent.change(box, { target: { value: "Please reopen" } });
    fireEvent.click(screen.getByRole("button", { name: "Send reply" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("delivered");
    // Draft preserved for re-send.
    expect((box as HTMLTextAreaElement).value).toBe("Please reopen");
    expect(screen.queryByTestId("portal-pending-reply")).toBeNull();
    cleanup();
  });

  it("lists attachments with sizes and download controls", async () => {
    setupApi();
    render(
      <PortalAuthProvider>
        <PortalTicketPage />
      </PortalAuthProvider>,
    );
    const row = await screen.findByTestId("portal-attachment-0");
    expect(row.textContent).toContain("smtp-log.txt");
    expect(row.textContent).toContain("2 KB");
    expect(screen.getByRole("button", { name: "Download" })).toBeInTheDocument();
    cleanup();
  });
});
