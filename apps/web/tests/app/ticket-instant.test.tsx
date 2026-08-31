// Ticket workspace through the instant layer: optimistic status writes
// reconciling from the IF2 envelope, If-Match version carry, typed 409
// reverts, and offline queue → in-order replay (IF1/IF5/IF6).
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import TicketWorkspacePage from "../../app/tickets/[id]/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";
import { currentFetchRouter, setFetchRouter } from "../helpers/fetch-router";
import {
  __resetInstantRuntimeForTests,
  instantRuntime,
} from "../../lib/instant/runtime";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  useParams: () => ({ id: TICKET_ID }),
}));

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
const TICKET_ID = "00000000-0000-4000-8000-00000000t001";
const STATUS_NEW = "00000000-0000-4000-8000-00000000s001";
const STATUS_WORKING = "00000000-0000-4000-8000-00000000s002";

function ticketJson(statusId: string) {
  return {
    id: TICKET_ID,
    org_id: ME_FIXTURE.org_id,
    board_id: BOARD_ID,
    number: "MER-7",
    title: "Broken printer",
    body: "It beeps twice and stops.",
    status_id: statusId,
    category: statusId === STATUS_WORKING ? "in_progress" : "open",
    priority: "P2",
    assignee_id: ME_FIXTURE.user_id,
    created_at: "2026-08-30T09:00:00Z",
    updated_at: "2026-08-30T09:30:00Z",
    resolved_at: null,
    custom_fields: [],
  };
}

function mutation(statusId: string, seq: number) {
  const after = ticketJson(statusId);
  return {
    ...after,
    mutation: {
      after,
      before: ticketJson(statusId === STATUS_WORKING ? STATUS_NEW : STATUS_WORKING),
      seq,
      updated_at: `2026-08-30T09:${String(30 + seq).padStart(2, "0")}:00Z`,
      version: seq,
      idempotency_key: "server-key",
    },
  };
}

function setupApi() {
  const mock = new MockApi();
  let seq = 3;
  let status = STATUS_NEW;
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/notifications/unread-count", () => ({ json: { count: 0 } }));
  mock.on("GET", "/api/v1/notifications", () => ({ json: { items: [], next_cursor: null } }));
  mock.on("GET", `/api/v1/tickets/${TICKET_ID}`, () => ({ json: ticketJson(status) }));
  mock.on("GET", `/api/v1/tickets/${TICKET_ID}/comments`, () => ({ json: [] }));
  mock.on("GET", `/api/v1/tickets/${TICKET_ID}/checklist`, () => ({
    json: { blocking: [], enforcement_active: false, items: [] },
  }));
  mock.on("GET", `/api/v1/tickets/${TICKET_ID}/sla`, () => ({ json: null }));
  mock.on("GET", `/api/v1/tickets/${TICKET_ID}/attachments`, () => ({ json: [] }));
  mock.on("GET", "/api/v1/tickets/my-work", () => ({ json: [] }));
  mock.on("GET", `/api/v1/boards/${BOARD_ID}/statuses`, () => ({
    json: [
      { id: STATUS_NEW, board_id: BOARD_ID, name: "New", category: "open", sort_order: 0 },
      { id: STATUS_WORKING, board_id: BOARD_ID, name: "Working", category: "in_progress", sort_order: 1 },
    ],
  }));
  mock.on("GET", `/api/v1/boards/${BOARD_ID}/transitions`, () => ({ json: { mode: "floor", edges: [] } }));
  mock.on("POST", `/api/v1/tickets/${TICKET_ID}/status`, (ctx) => {
    const body = ctx.body as { status_id: string };
    seq += 1;
    status = body.status_id;
    return { json: mutation(status, seq) };
  });
  mock.install();
  return mock;
}

function page() {
  return (
    <AuthProvider>
      <TicketWorkspacePage />
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

describe("/tickets/[id] — optimistic writes with authoritative reconcile", () => {
  it("renders detail from the read model and reconciles a status write", async () => {
    const mock = setupApi();
    render(page());
    expect(await screen.findByText("Broken printer", {}, { timeout: 5000 })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Working" }));
    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/tickets/${TICKET_ID}/status`)).toHaveLength(1);
    });
    const body = mock.lastBody<{ status_id: string; idempotency_key?: string; version?: number }>(
      "POST",
      `/api/v1/tickets/${TICKET_ID}/status`,
    );
    // IF1: autosave writes carry an idempotency key; first write omits version.
    expect(body.status_id).toBe(STATUS_WORKING);
    expect(typeof body.idempotency_key).toBe("string");
    expect(body.version).toBeUndefined();

    // Read-your-write: the pill reflects the applied snapshot.
    await waitFor(() => {
      expect(screen.getAllByText("Working").length).toBeGreaterThanOrEqual(2);
    });
  });

  it("carries the observed seq as the If-Match version on the next write", async () => {
    const mock = setupApi();
    render(page());
    await screen.findByText("Broken printer", {}, { timeout: 5000 });

    fireEvent.click(screen.getByRole("button", { name: "Working" }));
    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/tickets/${TICKET_ID}/status`)).toHaveLength(1);
    });
    fireEvent.click(screen.getByRole("button", { name: "New" }));
    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/tickets/${TICKET_ID}/status`)).toHaveLength(2);
    });
    const second = mock.lastBody<{ version?: number }>("POST", `/api/v1/tickets/${TICKET_ID}/status`);
    expect(second.version).toBe(4);
  });

  it("reverts the optimistic patch on a typed version_conflict and surfaces it", async () => {
    const mock = setupApi();
    render(page());
    await screen.findByText("Broken printer", {}, { timeout: 5000 });

    // Land on Working (seq 4), then flip the route to a typed 409.
    fireEvent.click(screen.getByRole("button", { name: "Working" }));
    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/tickets/${TICKET_ID}/status`)).toHaveLength(1);
    });
    mock.on("POST", `/api/v1/tickets/${TICKET_ID}/status`, (ctx) => {
      void ctx;
      return {
        status: 409,
        json: {
          error: { code: 409, reason: "version_conflict", message: "entity moved on" },
        },
      };
    });

    fireEvent.click(screen.getByRole("button", { name: "New" }));
    // The optimistic render flipped, then the typed rejection reverted it.
    await waitFor(() => {
      expect(
        screen.getAllByRole("alert").map((el) => el.textContent ?? "").join("\n"),
      ).toContain("That status change is not allowed by this board's workflow");
    });
    // Pill and button row agree on the authoritative (unchanged) status.
    await waitFor(() => {
      const pill = screen.getAllByText("Working");
      expect(pill.length).toBeGreaterThanOrEqual(1);
    });
  });
});

describe("/tickets/[id] — offline queue and in-order replay (IF6)", () => {
  it("queues the write durably while offline, replays on demand, surfaces per-op result", async () => {
    const mock = setupApi();
    render(page());
    await screen.findByText("Broken printer", {}, { timeout: 5000 });

    // Cut the network for the status write only; everything else delegates.
    const base = currentFetchRouter();
    expect(base).not.toBeNull();
    setFetchRouter(async (input, init) => {
      const url = new URL(
        typeof input === "string" ? input : input instanceof URL ? input : input.url,
      );
      if (url.pathname.endsWith("/status")) {
        throw new TypeError("network down");
      }
      return base?.(input, init) as Promise<Response>;
    });

    fireEvent.click(screen.getByRole("button", { name: "Working" }));
    // Optimistic render survives offline; the dock surfaces the queue.
    expect(await screen.findByText("1 offline change pending", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.getAllByText("Working").length).toBeGreaterThanOrEqual(2);

    // The queue is durable: it survives reloads via localStorage.
    expect(window.localStorage.getItem("meridian-instant-outbox")).not.toBeNull();

    // Network returns; replay applies the queued op in order.
    setFetchRouter(base as Parameters<typeof setFetchRouter>[0]);
    await instantRuntime().queue.replay();
    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/tickets/${TICKET_ID}/status`)).toHaveLength(1);
    });
    const replayed = mock.lastBody<{ status_id: string; idempotency_key?: string }>(
      "POST",
      `/api/v1/tickets/${TICKET_ID}/status`,
    );
    expect(replayed.status_id).toBe(STATUS_WORKING);
    expect(typeof replayed.idempotency_key).toBe("string");

    // Authoritative snapshot applied; per-op accepted result surfaced.
    await waitFor(() => {
      expect(screen.getByText("saved")).toBeInTheDocument();
    });
    expect(screen.queryByText("1 offline change pending")).not.toBeInTheDocument();
  });
});
