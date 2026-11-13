// Integration test: TicketUndo — IF7 undo on the ticket workspace:
// stack loading, Ctrl/Cmd+Z whole-record revert in `open` mode, and the
// per-field revert affordance (whole-record rejected) in `reverts_only`.
import { describe, expect, it, beforeEach } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TicketUndo } from "../../components/ticket-undo";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

const TICKET_ID = "90000000-0000-4000-8000-000000000001";
const W1 = "91000000-0000-4000-8000-000000000001";
const W2 = "91000000-0000-4000-8000-000000000002";

let mode: "open" | "reverts_only";
let revertCalls: { body: Record<string, unknown> }[] = [];
let onChanged = 0;

function stack() {
  return {
    entity: "ticket",
    entity_id: TICKET_ID,
    mode,
    current_version: 14,
    steps: [
      {
        write_id: W2,
        seq: 14,
        op: "update",
        actor_id: "92000000-0000-4000-8000-000000000001",
        actor_kind: "human",
        before: { priority: "p3" },
        after: { priority: "p2" },
        restorable: true,
        created_at: "2026-09-02T10:00:00Z",
        idempotency_key: null,
      },
      {
        write_id: W1,
        seq: 13,
        op: "create",
        actor_id: "92000000-0000-4000-8000-000000000001",
        actor_kind: "human",
        before: null,
        after: { title: "New ticket" },
        restorable: false,
        created_at: "2026-09-02T09:00:00Z",
        idempotency_key: null,
      },
    ],
    fields: [
      {
        write_id: W2,
        seq: 14,
        op: "update",
        field: "priority",
        prior: "p3",
        new_value: "p2",
        changed: true,
        actor_id: "92000000-0000-4000-8000-000000000001",
        actor_kind: "human",
        created_at: "2026-09-02T10:00:00Z",
      },
    ],
  };
}

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", `/api/v1/tickets/${TICKET_ID}/undo-stack`, () => ({ json: stack() }));
  mock.on("POST", `/api/v1/tickets/${TICKET_ID}/revert`, ({ body }) => {
    const b = body as Record<string, unknown>;
    revertCalls.push({ body: b });
    if (mode === "reverts_only" && (b.fields === null || b.fields === undefined)) {
      return { status: 422, json: { error: { code: "undo_reverts_only", message: "closed record" } } };
    }
    return {
      json: {
        id: TICKET_ID,
        mutation: { after: null, before: null, seq: 15, updated_at: "2026-09-02T11:00:00Z", version: 15 },
        revert:
          b.fields === null || b.fields === undefined
            ? { reverted_seq: 14, fields: [], whole_record: true }
            : { reverted_seq: 14, fields: b.fields as string[], whole_record: false },
      },
    };
  });
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  mode = "open";
  revertCalls = [];
  onChanged = 0;
});

function renderPanel(): void {
  render(<TicketUndo ticketId={TICKET_ID} onChanged={() => (onChanged += 1)} />);
}

describe("TicketUndo (IF7)", () => {
  it("shows the newest restorable write and steps it back via Ctrl+Z", async () => {
    setupApi();
    renderPanel();
    await screen.findByText(/Undo: update by you/);
    expect(screen.getByRole("button", { name: /Undo: update/ })).toHaveAttribute("title", "Ctrl/Cmd+Z");
    // `fireEvent` dispatches synchronously, so a keypress fired before React has
    // flushed the PASSIVE effect that binds the window listener is dropped
    // silently: no listener, no revert, and the assertion below sees zero calls
    // with the panel fully rendered. `findByText` resolves on the DOM mutation,
    // and under a loaded runner the effect can land after it. Flush first, then
    // press. In a browser this cannot bite a person — passive effects flush
    // before paint, so no keystroke arrives in between — it is a harness race.
    await act(async () => {});
    fireEvent.keyDown(window, { key: "z", ctrlKey: true });
    // Five seconds for the same reason the `onChanged` wait below uses five:
    // a CI runner running 133 jsdom environments starves the timer that
    // completes a mock POST far more than a laptop does.
    await waitFor(() => expect(revertCalls.length).toBe(1), { timeout: 5000 });
    expect(revertCalls[0]?.body.seq).toBe(14);
    expect(revertCalls[0]?.body.idempotency_key).toBeTruthy();
    // onChanged fires after the handler's refresh() settles — a tick after
    // the revert POST the waitFor above observes — so await it. The default
    // one-second budget is tight for a POST plus a refresh under load; this
    // case flaked about one run in ten on the component before T3 touched it,
    // so the budget is five seconds rather than a red gate.
    await waitFor(() => expect(onChanged).toBe(1), { timeout: 5000 });
    await screen.findByText(/Stepped back write #14/);
    cleanup();
  });

  it("keeps native input undo — the binding ignores keydown inside inputs", async () => {
    setupApi();
    renderPanel();
    await screen.findByText(/Undo: update by you/);
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    // Same flush as the case above — and here it matters for the opposite
    // reason: this asserts a NEGATIVE, so a listener that had not attached yet
    // would make the test pass without the input guard ever running.
    await act(async () => {});
    fireEvent.keyDown(input, { key: "z", ctrlKey: true, bubbles: true });
    await new Promise((r) => setTimeout(r, 50));
    expect(revertCalls.length).toBe(0);
    input.remove();
    cleanup();
  });

  it("in reverts_only mode offers per-field reverts, never whole-record undo", async () => {
    setupApi();
    const user = userEvent.setup();
    mode = "reverts_only";
    renderPanel();
    await screen.findByText(/closed · reverts only/);
    expect(screen.queryByText(/Undo: update/)).not.toBeInTheDocument();
    await user.click(screen.getByLabelText("priority"));
    await user.click(screen.getByRole("button", { name: "Revert selected fields" }));
    await waitFor(() => expect(revertCalls.length).toBe(1));
    expect(revertCalls[0]?.body.fields).toEqual(["priority"]);
    await screen.findByText(/Restored priority/);
    cleanup();
  });

  it("expands the write history with restorability badges", async () => {
    setupApi();
    const user = userEvent.setup();
    renderPanel();
    await screen.findByText(/Undo: update by you/);
    await user.click(screen.getByRole("button", { name: /Show history \(2\)/ }));
    expect(screen.getByText("not value-restorable")).toBeInTheDocument();
    expect(screen.getByText("#13")).toBeInTheDocument();
    cleanup();
  });
});
