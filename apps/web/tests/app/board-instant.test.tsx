// Board page through the instant layer: hydrate-once (IF4), deltas over
// the board channel (IF5), no loading flash on known data.
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import BoardPage from "../../app/boards/[id]/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";
import {
  __resetInstantRuntimeForTests,
  instantRuntime,
} from "../../lib/instant/runtime";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  useParams: () => ({ id: "00000000-0000-4000-8000-00000000b001" }),
}));

// Never-connecting socket stand-in: page tests drive events through the
// runtime's test seam instead of a live wire.
class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  url: string;
  sent: string[] = [];
  constructor(url: string) {
    this.url = url;
    FakeWebSocket.instances.push(this);
  }
  send(data: string) {
    this.sent.push(data);
  }
  close() {}
  addEventListener() {}
  removeEventListener() {}
  onopen: unknown = null;
  onclose: unknown = null;
  onmessage: unknown = null;
}

const BOARD_ID = "00000000-0000-4000-8000-00000000b001";
const TICKET_A = {
  id: "00000000-0000-4000-8000-00000000t001",
  board_id: BOARD_ID,
  number: "MER-1",
  title: "First ticket",
  status_id: "00000000-0000-4000-8000-00000000s001",
  category: "open",
  priority: "P3",
  created_at: "2026-08-30T09:00:00Z",
  updated_at: "2026-08-30T09:00:00Z",
};
const STATUS_NEW = {
  id: "00000000-0000-4000-8000-00000000s001",
  board_id: BOARD_ID,
  name: "New",
  category: "open",
  sort_order: 0,
};

function setupApi(tickets: unknown[]): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/boards/nav", () => ({ json: [] }));
  mock.on("GET", "/api/v1/board-groups", () => ({ json: [] }));
  mock.on("GET", `/api/v1/boards/${BOARD_ID}/members`, () => ({
    status: 403,
    json: { error: { code: 403, reason: "forbidden", message: "no board role" } },
  }));
  mock.on("GET", "/api/v1/notifications/unread-count", () => ({ json: { count: 0 } }));
  mock.on("GET", "/api/v1/notifications", () => ({ json: { items: [], next_cursor: null } }));
  mock.on("GET", `/api/v1/boards/${BOARD_ID}/statuses`, () => ({ json: [STATUS_NEW] }));
  mock.on("GET", "/api/v1/tickets", () => ({ json: tickets }));
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

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  vi.stubGlobal("WebSocket", FakeWebSocket);
  FakeWebSocket.instances = [];
  __resetInstantRuntimeForTests();
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("/boards/[id] — instant read model", () => {
  it("hydrates once and serves a remount from cache with no refetch, no flash", async () => {
    const mock = setupApi([TICKET_A]);
    render(page());
    expect(await screen.findByText("First ticket", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(mock.callsTo("GET", "/api/v1/tickets")).toHaveLength(1);

    // "Navigate away and back": same runtime, same cache. Auth re-resolves
    // on the new mount, but the moment data renders it comes from the
    // cache — the ticket snapshot endpoint is never re-hit.
    cleanup();
    render(page());
    expect(await screen.findByText("First ticket", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(mock.callsTo("GET", "/api/v1/tickets")).toHaveLength(1);
    expect(mock.callsTo("GET", `/api/v1/boards/${BOARD_ID}/statuses`)).toHaveLength(1);
  });

  it("applies a board-channel ticket event as a per-ticket delta", async () => {
    const mock = setupApi([TICKET_A]);
    const created = {
      ...TICKET_A,
      id: "00000000-0000-4000-8000-00000000t002",
      number: "MER-2",
      title: "Delta ticket",
    };
    mock.on("GET", `/api/v1/tickets/${created.id}`, (ctx) => {
      // Delta fetches target the event's entity only.
      expect(ctx.query).toEqual({});
      return { json: created };
    });

    render(page());
    await screen.findByText("First ticket", {}, { timeout: 5000 });

    // Deliver one wire frame as the hub would.
    instantRuntime().socket.deliver(
      JSON.stringify({
        type: "event",
        channel: `board:${BOARD_ID}`,
        event: {
          id: "e1",
          type: "ticket.created",
          entity: "ticket",
          entity_id: created.id,
          occurred_at: "2026-08-30T10:00:00Z",
          payload: { ticket_id: created.id, board_id: BOARD_ID },
        },
      }),
    );

    await waitFor(() => {
      expect(screen.getByText("Delta ticket")).toBeInTheDocument();
    });
    // Delta, not a re-snapshot: the collection endpoint was NOT re-hit.
    expect(mock.callsTo("GET", "/api/v1/tickets")).toHaveLength(1);
    expect(mock.callsTo("GET", `/api/v1/tickets/${created.id}`)).toHaveLength(1);
  });

  it("keeps rendering from cache when irrelevant events arrive", async () => {
    const mock = setupApi([TICKET_A]);
    render(page());
    await screen.findByText("First ticket", {}, { timeout: 5000 });

    instantRuntime().socket.deliver(
      JSON.stringify({
        type: "event",
        channel: `board:${BOARD_ID}`,
        event: {
          id: "e2",
          type: "notification.created",
          entity: "notification",
          entity_id: "n1",
          occurred_at: "2026-08-30T10:01:00Z",
          payload: { user_id: ME_FIXTURE.user_id },
        },
      }),
    );
    await new Promise((r) => setTimeout(r, 20));
    expect(mock.callsTo("GET", "/api/v1/tickets")).toHaveLength(1);
    expect(screen.getByText("First ticket")).toBeInTheDocument();
  });
});
