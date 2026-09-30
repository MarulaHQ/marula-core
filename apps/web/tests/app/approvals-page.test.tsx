// Integration test: /approvals — time-entry approval queues + lock states
// (Wave 14 lane 7 close-out). Covers: the approver queue render, bulk
// approve with read-your-write removal, reject gated on the typed reason
// vocabulary, per-item bulk outcomes flagged on the row (locked entries
// stay + say why), the append-only trail view, non-approver posture (own
// submissions + reopen), and friendly typed-403 copy (never raw JSON).
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import ApprovalsPage from "../../app/approvals/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/approvals",
}));

const ORG = ME_FIXTURE.org_id;
const ME = ME_FIXTURE.user_id;
const T1 = "30000000-0000-4000-8000-000000000001";
const T2 = "30000000-0000-4000-8000-000000000002";
const E1 = "40000000-0000-4000-8000-000000000001";
const E2 = "40000000-0000-4000-8000-000000000002";
const E3 = "40000000-0000-4000-8000-000000000003";
const NOW = "2026-09-14T09:00:00Z";
const DECIDE = "/api/v1/time-entries/approvals/decide";

function entry(over: Record<string, unknown>) {
  return {
    id: E1,
    org_id: ORG,
    ticket_id: T1,
    user_id: ME,
    minutes: 90,
    notes: "Fixed the printer",
    started_at: NOW,
    ended_at: NOW,
    billable: true,
    approval_status: "submitted",
    approval_reason_code: null,
    timer_session_id: null,
    created_at: NOW,
    updated_at: NOW,
    ...over,
  };
}

function queueItem(over: Record<string, unknown>) {
  return {
    submitted_by: ME,
    submitted_at: NOW,
    decided_by: null,
    decided_at: null,
    locked: false,
    ...over,
  };
}

function setupApi(permissions: string[] = ["special:approve_time"]): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: { ...ME_FIXTURE, permissions } }));
  mock.on("GET", "/api/v1/time-entries/approvals/queue", (ctx) => ({
    json:
      ctx.query.status === "rejected"
        ? [
            queueItem({
              entry: entry({
                id: E3,
                minutes: 60,
                approval_status: "rejected",
                approval_reason_code: "missing_notes",
              }),
            }),
          ]
        : [
            queueItem({ entry: entry({ id: E1 }) }),
            queueItem({ entry: entry({ id: E2, minutes: 30, ticket_id: T2 }) }),
          ],
  }));
  mock.install();
  return mock;
}

function mount(ui: React.ReactElement) {
  return render(<AuthProvider>{ui}</AuthProvider>);
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/approvals", () => {
  it("renders the approver queue with duration, ticket link, and bulk bar", async () => {
    setupApi();
    mount(<ApprovalsPage />);
    expect(await screen.findByText("1h 30m")).toBeInTheDocument();
    expect(screen.getByText("30m")).toBeInTheDocument();
    expect(screen.getAllByText(/ticket 30000000/).length).toBe(2);
    expect(screen.getByTestId("bulk-bar")).toBeInTheDocument();
    cleanup();
  });

  it("bulk-approves selected entries and removes applied rows (read-your-write)", async () => {
    const mock = setupApi();
    mock.on("POST", DECIDE, (ctx) => {
      const ids = (ctx.body as { entry_ids: string[] }).entry_ids;
      return {
        json: {
          results: ids.map((id) => ({
            entry_id: id,
            outcome: "applied",
            entry: entry({ id, approval_status: "approved" }),
          })),
        },
      };
    });
    mount(<ApprovalsPage />);
    await screen.findByText("1h 30m");
    fireEvent.click(screen.getByLabelText(`Select entry on ticket ${T1}`));
    fireEvent.click(screen.getByText("Approve selected"));

    await waitFor(() => {
      expect(mock.callsTo("POST", DECIDE)).toHaveLength(1);
    });
    const body = mock.lastBody<{ entry_ids: string[]; decision: string }>("POST", DECIDE);
    expect(body.decision).toBe("approve");
    expect(body.entry_ids).toEqual([E1]);
    await waitFor(() => {
      expect(screen.queryByText("1h 30m")).not.toBeInTheDocument();
    });
    expect(screen.getByText("30m")).toBeInTheDocument();
    cleanup();
  });

  it("reject requires a typed reason before the decision posts", async () => {
    const mock = setupApi();
    mock.on("POST", DECIDE, (ctx) => {
      const ids = (ctx.body as { entry_ids: string[] }).entry_ids;
      return {
        json: {
          results: ids.map((id) => ({
            entry_id: id,
            outcome: "applied",
            entry: entry({ id, approval_status: "rejected", approval_reason_code: "missing_notes" }),
          })),
        },
      };
    });
    mount(<ApprovalsPage />);
    await screen.findByText("1h 30m");
    fireEvent.click(screen.getByLabelText(`Select entry on ticket ${T1}`));
    fireEvent.click(within(screen.getByTestId("bulk-bar")).getByText("Reject…"));
    fireEvent.click(screen.getByTestId("reject-send"));

    expect(await screen.findByText(/Pick a rejection reason/)).toBeInTheDocument();
    expect(mock.callsTo("POST", DECIDE)).toHaveLength(0);

    fireEvent.change(screen.getByTestId("reject-reason"), { target: { value: "missing_notes" } });
    fireEvent.click(screen.getByTestId("reject-send"));
    await waitFor(() => {
      expect(mock.callsTo("POST", DECIDE)).toHaveLength(1);
    });
    const body = mock.lastBody<{ decision: string; reason_code?: string; note?: string }>("POST", DECIDE);
    expect(body.decision).toBe("reject");
    expect(body.reason_code).toBe("missing_notes");
    cleanup();
  });

  it("flags refused rows with their exact outcome — locked stays with a reason, applied leaves", async () => {
    const mock = setupApi();
    mock.on("POST", DECIDE, () => ({
      json: {
        results: [
          { entry_id: E1, outcome: "applied", entry: entry({ id: E1, approval_status: "approved" }) },
          { entry_id: E2, outcome: "locked", entry: null },
        ],
      },
    }));
    mount(<ApprovalsPage />);
    await screen.findByText("1h 30m");
    fireEvent.click(screen.getByLabelText("Select all"));
    fireEvent.click(screen.getByText("Approve selected"));

    await waitFor(() => {
      expect(screen.queryByText("1h 30m")).not.toBeInTheDocument();
    });
    expect(screen.getByText("Already invoiced — locked, nothing to decide")).toBeInTheDocument();
    expect(screen.getByText("30m")).toBeInTheDocument();
    cleanup();
  });

  it("renders the append-only decision trail per entry", async () => {
    const mock = setupApi();
    mock.on("GET", `/api/v1/time-entries/${E1}/approvals`, () => ({
      json: [
        {
          id: "60000000-0000-4000-8000-000000000001",
          org_id: ORG,
          time_entry_id: E1,
          seq: 1,
          action: "submitted",
          actor_id: ME,
          created_at: NOW,
          reason_code: null,
          note: null,
        },
        {
          id: "60000000-0000-4000-8000-000000000002",
          org_id: ORG,
          time_entry_id: E1,
          seq: 2,
          action: "rejected",
          actor_id: ME,
          created_at: NOW,
          reason_code: "wrong_ticket",
          note: "Work belongs to T-2",
        },
      ],
    }));
    mount(<ApprovalsPage />);
    await screen.findByText("1h 30m");
    fireEvent.click(screen.getByTestId(`history-${E1}`));
    // waitFor, not findByTestId: the trail element mounts immediately with its
    // empty state, and only fills once the approvals fetch resolves. Waiting for
    // the element therefore raced the fetch, and the assertion read
    // "No recorded decisions yet." roughly one run in four.
    await waitFor(() => {
      expect(screen.getByTestId(`trail-${E1}`)).toHaveTextContent("Rejected");
    });
    expect(screen.getByTestId(`trail-${E1}`)).toHaveTextContent("Wrong ticket");
    expect(mock.callsTo("GET", `/api/v1/time-entries/${E1}/approvals`)).toHaveLength(1);
    cleanup();
  });

  it("non-approvers see their own submissions and can reopen rejected entries", async () => {
    const mock = setupApi([]);
    mock.on("POST", `/api/v1/time-entries/${E3}/reopen`, () => ({
      json: entry({ id: E3, approval_status: "draft" }),
    }));
    mount(<ApprovalsPage />);
    await screen.findByText("1h 30m");
    expect(screen.queryByTestId("bulk-bar")).not.toBeInTheDocument();

    fireEvent.click(screen.getByText("Rejected"));
    const reopenBtn = await screen.findByTestId(`reopen-${E3}`);
    expect(screen.getByText("Rejected — Missing notes")).toBeInTheDocument();
    fireEvent.click(reopenBtn);

    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/time-entries/${E3}/reopen`)).toHaveLength(1);
    });
    await waitFor(() => {
      expect(screen.queryByTestId(`reopen-${E3}`)).not.toBeInTheDocument();
    });
    cleanup();
  });

  it("surfaces typed 403 copy, never raw JSON", async () => {
    const mock = setupApi();
    mock.on("POST", DECIDE, () => ({
      status: 403,
      json: { error: { reason: "forbidden", message: "missing special:approve_time" } },
    }));
    mount(<ApprovalsPage />);
    await screen.findByText("1h 30m");
    fireEvent.click(screen.getByLabelText(`Select entry on ticket ${T1}`));
    fireEvent.click(screen.getByText("Approve selected"));
    expect(
      await screen.findByText("You don't have permission for this action — ask an owner or admin."),
    ).toBeInTheDocument();
    cleanup();
  });
});
