// Integration test: board move from the ticket status bar (Wave 12 lane 7).
// The affordance lives in the status menu with a details-tab fallback; the
// mutation rides the instant-first envelope (optimistic board/status swap,
// authoritative reconcile), the `reset_sla` toggle rides the body, and a
// typed `board_move_status_conflict` renders the target statuses inline as a
// pick-list that retries with an explicit `status_id`.
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
// Wave 15 lane 7: /tickets/[id] is now Workbench v2 — these instant-layer
// semantics (pods, tabs, close flow) live on the preserved classic
// workspace, so the suite pins that component.
import { TicketWorkspaceClassic } from "../../components/ticket-workspace/classic-workspace";
import { MockApi, seedAuthTokens } from "../helpers/mock-api";
import { __resetInstantRuntimeForTests } from "../../lib/instant/runtime";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  useParams: () => ({ id: TICKET_ID }),
}));


const TICKET_ID = "00000000-0000-4000-8000-00000000t001";
const BOARD_A = "00000000-0000-4000-8000-00000000b00a";
const BOARD_B = "00000000-0000-4000-8000-00000000b00b";
const BOARD_C = "00000000-0000-4000-8000-00000000b00c";
const STATUS_A_NEW = "00000000-0000-4000-8000-00000000s00a";
const STATUS_B_TRIAGE = "00000000-0000-4000-8000-00000000s00b";
const STATUS_B_DONE = "00000000-0000-4000-8000-00000000s00c";
const STATUS_C_DONE = "00000000-0000-4000-8000-00000000s00d";
const NOW = "2026-08-30T09:00:00Z";

const ME_EDIT = {
  user_id: "00000000-0000-4000-8000-000000000001",
  org_id: "00000000-0000-4000-8000-000000000002",
  email: "tech@marula.test",
  display_name: "Tech",
  timezone: "UTC",
  permissions: ["tickets:edit"],
};

function ticketJson(boardId: string, statusId: string, category: string) {
  return {
    id: TICKET_ID,
    org_id: ME_EDIT.org_id,
    board_id: boardId,
    number: "MER-7",
    title: "Broken printer",
    body: "It beeps twice and stops.",
    status_id: statusId,
    category,
    priority: "P2",
    assignee_id: ME_EDIT.user_id,
    created_at: NOW,
    updated_at: NOW,
    resolved_at: null,
    custom_fields: [],
  };
}

function moveResponse(moved: Record<string, unknown>, seq: number, idempotencyKey: string | null) {
  return {
    ...moved,
    mutation: {
      after: moved,
      before: ticketJson(BOARD_A, STATUS_A_NEW, "open"),
      seq,
      updated_at: NOW,
      version: seq,
      idempotency_key: idempotencyKey,
    },
  };
}

function setupApi(): { mock: MockApi; state: { ticket: Record<string, unknown> } } {
  const state = { ticket: ticketJson(BOARD_A, STATUS_A_NEW, "open") };
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_EDIT }));
  mock.on("GET", "/api/v1/notifications/unread-count", () => ({ json: { count: 0 } }));
  mock.on("GET", "/api/v1/notifications", () => ({ json: { items: [], next_cursor: null } }));
  mock.on("GET", `/api/v1/tickets/${TICKET_ID}`, () => ({ json: state.ticket }));
  mock.on("GET", `/api/v1/tickets/${TICKET_ID}/comments`, () => ({ json: [] }));
  mock.on("GET", `/api/v1/tickets/${TICKET_ID}/checklist`, () => ({
    json: { blocking: [], enforcement_active: false, items: [] },
  }));
  mock.on("GET", `/api/v1/tickets/${TICKET_ID}/sla`, () => ({ json: null }));
  mock.on("GET", `/api/v1/tickets/${TICKET_ID}/attachments`, () => ({ json: [] }));
  mock.on("GET", "/api/v1/tickets/my-work", () => ({ json: [] }));
  mock.on("GET", "/api/v1/boards", () => ({
    json: [
      { id: BOARD_A, org_id: ME_EDIT.org_id, name: "Service Desk", is_default: true, description: null, require_checklist_close: false, created_at: NOW },
      { id: BOARD_B, org_id: ME_EDIT.org_id, name: "Field Ops", is_default: false, description: null, require_checklist_close: false, created_at: NOW },
      { id: BOARD_C, org_id: ME_EDIT.org_id, name: "Archive", is_default: false, description: null, require_checklist_close: false, created_at: NOW },
    ],
  }));
  mock.on("GET", `/api/v1/boards/${BOARD_A}/statuses`, () => ({
    json: [{ id: STATUS_A_NEW, board_id: BOARD_A, name: "New", category: "open", sort_order: 0 }],
  }));
  mock.on("GET", `/api/v1/boards/${BOARD_A}/transitions`, () => ({ json: { mode: "floor", edges: [] } }));
  mock.on("GET", `/api/v1/boards/${BOARD_B}/statuses`, () => ({
    json: [
      { id: STATUS_B_TRIAGE, board_id: BOARD_B, name: "Triage", category: "open", sort_order: 0 },
      { id: STATUS_B_DONE, board_id: BOARD_B, name: "Done", category: "closed", sort_order: 1 },
    ],
  }));
  mock.on("GET", `/api/v1/boards/${BOARD_C}/statuses`, () => ({
    json: [{ id: STATUS_C_DONE, board_id: BOARD_C, name: "Done", category: "closed", sort_order: 0 }],
  }));
  mock.install();
  return { mock, state };
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  __resetInstantRuntimeForTests();
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

async function openMoveDialog() {
  fireEvent.click(screen.getByTestId("ticket-status-change"));
  fireEvent.click(screen.getByTestId("ticket-move-board"));
  return screen.findByRole("heading", { name: "Move to another board" });
}

describe("ticket board move", () => {
  it("remaps through the semantic category and rides reset_sla on the envelope", async () => {
    const { mock, state } = setupApi();
    mock.on("POST", `/api/v1/tickets/${TICKET_ID}/move-board`, (ctx) => {
      const body = ctx.body as {
        target_board_id: string;
        status_id: string | null;
        reset_sla: boolean;
        idempotency_key?: string;
      };
      const moved = ticketJson(
        body.target_board_id,
        body.status_id ?? STATUS_B_TRIAGE,
        "open",
      );
      state.ticket = moved;
      return { json: moveResponse(moved, 5, body.idempotency_key ?? null) };
    });

    render(
      <AuthProvider>
        <TicketWorkspaceClassic />
      </AuthProvider>,
    );
    expect(await screen.findByText("Broken printer", {}, { timeout: 5000 })).toBeInTheDocument();

    await openMoveDialog();
    // Boards load when the dialog opens; wait before selecting a value.
    await screen.findByRole("option", { name: /Field Ops/ });
    fireEvent.change(screen.getByTestId("move-board-target"), { target: { value: BOARD_B } });
    // Wait for the target status set before submitting (auto remap preview).
    await waitFor(() => {
      expect(mock.callsTo("GET", `/api/v1/boards/${BOARD_B}/statuses`)).toHaveLength(1);
    });
    expect(await screen.findByRole("option", { name: "Triage (Open)" })).toBeInTheDocument();

    const resetToggle = screen.getByRole("checkbox", { name: /Reset SLA clocks/i });
    expect(resetToggle).not.toBeChecked();
    fireEvent.click(resetToggle);

    fireEvent.click(screen.getByTestId("confirm-move-board"));
    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/tickets/${TICKET_ID}/move-board`)).toHaveLength(1);
    });
    const body = mock.lastBody<{
      target_board_id: string;
      status_id: string | null;
      reset_sla: boolean;
      idempotency_key?: string;
    }>("POST", `/api/v1/tickets/${TICKET_ID}/move-board`);
    expect(body.target_board_id).toBe(BOARD_B);
    expect(body.status_id).toBeNull();
    expect(body.reset_sla).toBe(true);
    expect(typeof body.idempotency_key).toBe("string");

    // Authoritative reconcile lands the target board's status in the bar.
    await waitFor(() => {
      expect(screen.getByTestId("ticket-status-text").textContent).toBe("Triage");
    });
  });

  it("renders a board_move_status_conflict inline and retries with an explicit status_id", async () => {
    const { mock, state } = setupApi();
    let calls = 0;
    mock.on("POST", `/api/v1/tickets/${TICKET_ID}/move-board`, (ctx) => {
      calls += 1;
      const body = ctx.body as { status_id: string | null; idempotency_key?: string };
      if (calls === 1) {
        return {
          status: 409,
          json: {
            error: {
              code: 409,
              reason: "board_move_status_conflict",
              message: "no status on the target board matches the current semantic category",
              target_board_id: BOARD_C,
              from_status_id: STATUS_A_NEW,
              from_category: "open",
              target_statuses: [{ id: STATUS_C_DONE, name: "Done", category: "closed" }],
            },
          },
        };
      }
      const moved = ticketJson(BOARD_C, body.status_id ?? STATUS_C_DONE, "closed");
      state.ticket = moved;
      return { json: moveResponse(moved, 6, body.idempotency_key ?? null) };
    });

    render(
      <AuthProvider>
        <TicketWorkspaceClassic />
      </AuthProvider>,
    );
    expect(await screen.findByText("Broken printer", {}, { timeout: 5000 })).toBeInTheDocument();

    await openMoveDialog();
    await screen.findByRole("option", { name: /Archive/ });
    fireEvent.change(screen.getByTestId("move-board-target"), { target: { value: BOARD_C } });
    await waitFor(() => {
      expect(mock.callsTo("GET", `/api/v1/boards/${BOARD_C}/statuses`)).toHaveLength(1);
    });
    fireEvent.click(screen.getByTestId("confirm-move-board"));

    // Typed conflict rendered inline with a real pick-list, never a dead end.
    expect(await screen.findByTestId("board-move-conflict")).toBeInTheDocument();
    // The rejection reverts the optimistic board/status swap: the ticket's own
    // status must resolve again (never "Unknown status" under the target set).
    expect(screen.getByTestId("ticket-status-text").textContent).toBe("New");
    fireEvent.click(screen.getByTestId("board-move-conflict-closed"));
    fireEvent.click(screen.getByTestId("confirm-move-board"));

    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/tickets/${TICKET_ID}/move-board`)).toHaveLength(2);
    });
    const first = mock.callsTo("POST", `/api/v1/tickets/${TICKET_ID}/move-board`)[0];
    expect((first?.body as { status_id: string | null }).status_id).toBeNull();
    const retry = mock.lastBody<{ status_id: string | null }>(
      "POST",
      `/api/v1/tickets/${TICKET_ID}/move-board`,
    );
    expect(retry.status_id).toBe(STATUS_C_DONE);
  });

  it("offers the move from the details tab as a fallback", async () => {
    setupApi();
    render(
      <AuthProvider>
        <TicketWorkspaceClassic />
      </AuthProvider>,
    );
    expect(await screen.findByText("Broken printer", {}, { timeout: 5000 })).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("detail-tab-details"));
    fireEvent.click(screen.getByTestId("details-move-board"));
    expect(await screen.findByRole("heading", { name: "Move to another board" })).toBeInTheDocument();
  });
});
