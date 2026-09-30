import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CapacityForecastPanel } from "../../app/dispatch/planning/capacity-forecast-panel";
import { WorkNextPanel } from "../../app/dispatch/planning/work-next-panel";

/**
 * Tests for the two panels whose real risk is a LIE rather than a crash.
 *
 * A capacity panel that reports a technician as 0% loaded when nobody declared
 * their availability, or a queue that renumbers rows the dispatcher did not
 * drag, both render perfectly and both mislead. So the assertions here are
 * about what the panel SAYS in the states where the honest answer is "I
 * don't know", not about whether the markup appeared.
 */

const GET = vi.fn();
const POST = vi.fn();
const DELETE = vi.fn();

vi.mock("../../lib/api", () => ({
  api: {
    api: {
      GET: (...a: unknown[]) => GET(...a),
      POST: (...a: unknown[]) => POST(...a),
      DELETE: (...a: unknown[]) => DELETE(...a),
    },
  },
}));

function ok<T>(data: T) {
  return { data, error: undefined, response: {}, request: {} };
}

beforeEach(() => {
  GET.mockReset();
  POST.mockReset();
  DELETE.mockReset();
});

describe("CapacityForecastPanel", () => {
  const view = {
    from: "2026-10-01",
    to: "2026-10-14",
    days: 14,
    entries: [
      {
        user_id: "u1",
        display_name: "Alice Over",
        committed_percent_days: 900,
        allocation_count: 3,
        phase_count: 2,
        available_percent_days: 700,
        utilization: 900 / 700,
        over_allocated: true,
      },
      {
        user_id: "u2",
        display_name: "Bob Undeclared",
        committed_percent_days: 200,
        allocation_count: 1,
        phase_count: 1,
        // No availability declared at all: there is no denominator, so there
        // is no ratio. Rendering 0% or "infinite overload" would both be
        // inventions.
        available_percent_days: 0,
        utilization: null,
        over_allocated: false,
      },
    ],
  };

  it("labels the unit percent-days so a duration is never read as a share", async () => {
    GET.mockResolvedValue(ok(view));
    render(<CapacityForecastPanel />);
    await screen.findByText(/percent-days/i);
    expect(screen.getByText(/percent-days/i)).toBeTruthy();
  });

  it("says 'not declared' rather than a ratio when availability is zero", async () => {
    GET.mockResolvedValue(ok(view));
    render(<CapacityForecastPanel />);
    await screen.findByText("Bob Undeclared");
    // The row must show the honest absence, not a number.
    expect(screen.getAllByText("not declared").length).toBeGreaterThan(0);
    // And Bob is not counted as over-allocated off a null denominator.
    expect(screen.getByText(/Bob Undeclared/)).toBeTruthy();
  });

  it("raises the over-allocation banner naming how many technicians", async () => {
    GET.mockResolvedValue(ok(view));
    render(<CapacityForecastPanel />);
    const banner = await screen.findByText(/1 technician is committed beyond/);
    expect(banner).toBeTruthy();
  });

  it("shows an empty state naming the fix when nothing is committed", async () => {
    GET.mockResolvedValue(ok({ ...view, entries: [], over_allocated: 0 }));
    render(<CapacityForecastPanel />);
    await screen.findByText(/No project commitments in this window/);
    expect(screen.queryByText(/beyond their declared availability/)).toBeNull();
  });
});

describe("WorkNextPanel", () => {
  const queue = {
    user_id: "u1",
    display_name: "Alice",
    entries: [
      {
        id: "e3",
        ticket_id: "t3",
        position: 0,
        ticket_number: "P-3",
        title: "Third",
        priority: "p1",
        status: "Open",
        client_id: null,
        client_name: null,
        created_at: "2026-10-01T00:00:00Z",
        updated_at: "2026-10-01T00:00:00Z",
      },
      {
        id: "e1",
        ticket_id: "t1",
        position: 1,
        ticket_number: "P-1",
        title: "First",
        priority: "p3",
        status: "Open",
        client_id: null,
        client_name: "Acme",
        created_at: "2026-10-01T00:00:00Z",
        updated_at: "2026-10-01T00:00:00Z",
      },
    ],
  };

  it("renders the curated order with the client name beside the ticket", async () => {
    GET.mockResolvedValue(ok(queue));
    render(<WorkNextPanel />);
    await screen.findByText("P-3");
    expect(screen.getByText(/Third/)).toBeTruthy();
    expect(screen.getByText(/Acme/)).toBeTruthy();
  });

  it("states that the order does not change priority", async () => {
    GET.mockResolvedValue(ok(queue));
    render(<WorkNextPanel />);
    await screen.findByText(/never a ticket/);
    expect(screen.getByText(/never a ticket/)).toBeTruthy();
  });

  /**
   * The load-bearing one: a move shifts EVERY row between the old and new
   * rank, so the client must adopt the server's renumbered order rather
   * than patching only the row that was dragged. A client that patched one
   * row would display positions the server has already changed.
   */
  it("adopts the server's whole densified order after a move", async () => {
    const user = userEvent.setup();
    GET.mockResolvedValue(ok(queue));
    // Server renumbers: t1 to the head, t3 down to 1.
    POST.mockResolvedValue(
      ok([
        { ...queue.entries[1], position: 0 },
        { ...queue.entries[0], position: 1 },
      ]),
    );
    render(<WorkNextPanel />);
    await screen.findByText("P-3");

    await user.click(screen.getByLabelText("Move P-1 up"));

    await waitFor(() => {
      const items = screen.getAllByRole("listitem");
      // After the move P-1 is rendered first.
      expect(items[0]?.textContent).toContain("P-1");
      expect(items[1]?.textContent).toContain("P-3");
    });
  });

  it("disables the move-up control on the first row", async () => {
    GET.mockResolvedValue(ok(queue));
    render(<WorkNextPanel />);
    await screen.findByText("P-3");
    expect(screen.getByLabelText("Move P-3 up")).toBeDisabled();
    expect(screen.getByLabelText("Move P-1 down")).toBeDisabled();
  });

  it("shows an empty state pointing at the unassigned board", async () => {
    GET.mockResolvedValue(ok({ ...queue, entries: [] }));
    render(<WorkNextPanel />);
    await screen.findByText(/Nothing in this queue/);
    expect(screen.getByText(/unassigned board/)).toBeTruthy();
  });
});
