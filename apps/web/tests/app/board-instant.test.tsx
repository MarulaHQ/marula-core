// Board page through the instant layer: hydrate-once (IF4), deltas over
// the org-wide SSE stream (IF5, Wave 13 lane 9), no loading flash on
// known data.
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

  it("applies an org-stream ticket event as a per-ticket delta", async () => {
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

    // Deliver one SSE frame as the stream would (org-wide scope).
    instantRuntime().stream.deliver(
      JSON.stringify({
        type: "ticket.created",
        org_id: ME_FIXTURE.org_id,
        entity: "ticket",
        entity_id: created.id,
        seq: 2,
        at: "2026-08-30T10:00:00Z",
      }),
      null,
    );

    await waitFor(() => {
      expect(screen.getByText("Delta ticket")).toBeInTheDocument();
    });
    // Delta, not a re-snapshot: the collection endpoint was NOT re-hit.
    expect(mock.callsTo("GET", "/api/v1/tickets")).toHaveLength(1);
    expect(mock.callsTo("GET", `/api/v1/tickets/${created.id}`)).toHaveLength(1);
  });

  it("applies a routable status_changed event as a per-ticket delta", async () => {
    // Core event payloads carry board_id/assignee_id (gap-1 closure), so a
    // status change fans out to the board channel and patches one row.
    const mock = setupApi([TICKET_A]);
    const moved = { ...TICKET_A, category: "in_progress" };
    mock.on("GET", `/api/v1/tickets/${TICKET_A.id}`, () => ({ json: moved }));

    render(page());
    await screen.findByText("First ticket", {}, { timeout: 5000 });

    instantRuntime().stream.deliver(
      JSON.stringify({
        type: "ticket.status_changed",
        org_id: ME_FIXTURE.org_id,
        entity: "ticket",
        entity_id: TICKET_A.id,
        seq: 3,
        at: "2026-08-30T10:02:00Z",
      }),
      null,
    );

    await waitFor(() => {
      expect(screen.getByText("in_progress")).toBeInTheDocument();
    });
    // Delta, not a re-snapshot.
    expect(mock.callsTo("GET", "/api/v1/tickets")).toHaveLength(1);
    expect(mock.callsTo("GET", `/api/v1/tickets/${TICKET_A.id}`)).toHaveLength(1);
  });

  it("keeps rendering from cache when irrelevant events arrive", async () => {
    const mock = setupApi([TICKET_A]);
    render(page());
    await screen.findByText("First ticket", {}, { timeout: 5000 });

    instantRuntime().stream.deliver(
      JSON.stringify({
        type: "notification.created",
        org_id: ME_FIXTURE.org_id,
        entity: "notification",
        entity_id: "n1",
        at: "2026-08-30T10:01:00Z",
      }),
      null,
    );
    await new Promise((r) => setTimeout(r, 20));
    expect(mock.callsTo("GET", "/api/v1/tickets")).toHaveLength(1);
    expect(screen.getByText("First ticket")).toBeInTheDocument();
  });
});
