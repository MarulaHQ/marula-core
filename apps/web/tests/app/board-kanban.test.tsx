// Kanban interactions: envelope-backed moves, workflow-guarded drops,
// column collapse, filters. jsdom has no HTML5 DnD, so drops are driven
// with a stubbed dataTransfer — the handlers are the unit under test.
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import BoardPage from "../../app/boards/[id]/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";
import {
  __resetInstantRuntimeForTests,
} from "../../lib/instant/runtime";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  useParams: () => ({ id: "00000000-0000-4000-8000-00000000b001" }),
}));

const BOARD_ID = "00000000-0000-4000-8000-00000000b001";
const S_NEW = {
  id: "s-new",
  board_id: BOARD_ID,
  org_id: "o",
  name: "New",
  category: "open",
  sort_order: 0,
  is_entry: true,
};
const S_WORKING = {
  id: "s-working",
  board_id: BOARD_ID,
  org_id: "o",
  name: "Working",
  category: "in_progress",
  sort_order: 1,
  is_entry: false,
};
const S_DONE = {
  id: "s-done",
  board_id: BOARD_ID,
  org_id: "o",
  name: "Done",
  category: "closed",
  sort_order: 2,
  is_entry: false,
};
const TICKET = {
  id: "t-1",
  board_id: BOARD_ID,
  number: "MER-7",
  title: "Fix the flux capacitor",
  status_id: "s-new",
  category: "open",
  priority: "p1",
  created_at: "2026-09-01T09:00:00Z",
  updated_at: "2026-09-01T09:00:00Z",
};

function setupApi(
  statuses: typeof S_NEW[],
  tickets: (typeof TICKET)[],
): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/boards/nav", () => ({ json: [] }));
  mock.on("GET", "/api/v1/board-groups", () => ({ json: [] }));
  mock.on("GET", `/api/v1/boards/${BOARD_ID}/members`, () => ({ status: 403, json: {} }));
  mock.on("GET", "/api/v1/notifications/unread-count", () => ({ json: { count: 0 } }));
  mock.on("GET", "/api/v1/notifications", () => ({ json: { items: [], next_cursor: null } }));
  mock.on("GET", `/api/v1/boards/${BOARD_ID}/statuses`, () => ({ json: statuses }));
  mock.on("GET", `/api/v1/boards/${BOARD_ID}/transitions`, () => ({
    json: { mode: "floor", edges: [] },
  }));
  mock.on("GET", "/api/v1/tickets", () => ({ json: tickets }));
  mock.on("POST", `/api/v1/tickets/${TICKET.id}/status`, () => ({
    json: {
      id: TICKET.id,
      number: TICKET.number,
      status_id: "s-working",
      category: "in_progress",
      mutation: { seq: 2, updated_at: "2026-09-17T00:00:00Z", version: 2 },
    },
  }));
  mock.install();
  return mock;
}

function page() {
  return (
    <AuthProvider>
      <BoardPage />
    </AuthProvider>
  );
}

function dataTransfer(id: string): DataTransfer {
  return {
    setData: vi.fn(),
    getData: () => id,
    effectAllowed: "move",
    dropEffect: "move",
  } as unknown as DataTransfer;
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

describe("/boards/[id] — kanban interactions", () => {
  it("moves a card via the Move menu through the status envelope", async () => {
    const mock = setupApi([S_NEW, S_WORKING, S_DONE], [TICKET]);
    render(page());
    expect(await screen.findByText("Fix the flux capacitor")).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("kanban-move-MER-7"));
    fireEvent.click(screen.getByRole("button", { name: "Working" }));

    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/tickets/${TICKET.id}/status`)).toHaveLength(1);
    });
    const body = mock.lastBody<Record<string, unknown>>(
      "POST",
      `/api/v1/tickets/${TICKET.id}/status`,
    );
    expect(body.status_id).toBe("s-working");
    expect(typeof body.idempotency_key).toBe("string");
    // Optimistic render already moved the card before the wire.
    expect(screen.getByTestId("kanban-count-Working")).toHaveTextContent("1");
    expect(screen.getByTestId("kanban-count-New")).toHaveTextContent("0");
  });

  it("refuses a workflow-invalid drop with a banner and no POST", async () => {
    const mock = setupApi([S_NEW, S_DONE], [{ ...TICKET, status_id: "s-done", category: "closed" }]);
    render(page());
    expect(await screen.findByText("Fix the flux capacitor")).toBeInTheDocument();

    const doneColumn = screen.getByTestId("kanban-column-Done");
    const card = screen.getByTestId("kanban-card-MER-7");
    fireEvent.dragStart(card, { dataTransfer: dataTransfer("t-1") });
    fireEvent.drop(doneColumn, { dataTransfer: dataTransfer("t-1") });

    expect(await screen.findByTestId("board-move-error")).toHaveTextContent(
      "Workflow does not allow Done → Done",
    );
    expect(mock.callsTo("POST", `/api/v1/tickets/${TICKET.id}/status`)).toHaveLength(0);
  });

  it("drops a card into another column and POSTs the status change", async () => {
    const mock = setupApi([S_NEW, S_WORKING, S_DONE], [TICKET]);
    render(page());
    expect(await screen.findByText("Fix the flux capacitor")).toBeInTheDocument();

    const workingColumn = screen.getByTestId("kanban-column-Working");
    const card = screen.getByTestId("kanban-card-MER-7");
    fireEvent.dragStart(card, { dataTransfer: dataTransfer("t-1") });
    fireEvent.drop(workingColumn, { dataTransfer: dataTransfer("t-1") });

    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/tickets/${TICKET.id}/status`)).toHaveLength(1);
    });
    expect(screen.getByTestId("kanban-count-Working")).toHaveTextContent("1");
  });

  it("collapses and expands a column", async () => {
    setupApi([S_NEW, S_WORKING, S_DONE], [TICKET]);
    render(page());
    expect(await screen.findByText("Fix the flux capacitor")).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("kanban-collapse-New"));
    expect(screen.getByRole("button", { name: "Expand New column" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Expand New column" }));
    expect(await screen.findByTestId("kanban-column-New")).toBeInTheDocument();
  });

  it("narrows cards with the search box and priority chips", async () => {
    setupApi([S_NEW, S_WORKING, S_DONE], [TICKET]);
    render(page());
    expect(await screen.findByText("Fix the flux capacitor")).toBeInTheDocument();

    fireEvent.change(screen.getByTestId("board-search"), { target: { value: "zzz" } });
    expect(screen.queryByText("Fix the flux capacitor")).not.toBeInTheDocument();
    fireEvent.change(screen.getByTestId("board-search"), { target: { value: "" } });
    expect(screen.getByText("Fix the flux capacitor")).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("board-filter-p2"));
    expect(screen.queryByText("Fix the flux capacitor")).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("board-filter-p2"));
    fireEvent.click(screen.getByTestId("board-filter-p1"));
    expect(screen.getByText("Fix the flux capacitor")).toBeInTheDocument();
  });
});
