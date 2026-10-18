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
import { currentFetchRouter, setFetchRouter } from "../helpers/fetch-router";

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
/** A second tech: an approver's queue is other people's time, and nobody signs
 *  off their own. */
const OTHER = "50000000-0000-4000-8000-000000000002";

function entry(over: Record<string, unknown>) {
  return {
    id: E1,
    org_id: ORG,
    ticket_id: T1,
    user_id: OTHER,
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
    submitted_by: OTHER,
    submitted_at: NOW,
    decided_by: null,
    decided_at: null,
    locked: false,
    ...over,
  };
}

function setupApi(
  permissions: string[] = ["special:approve_time"],
  timezone = "UTC",
): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: { ...ME_FIXTURE, permissions, timezone } }));
  mock.on("GET", "/api/v1/time-entries/approvals/queue", (ctx) => ({
    json:
      ctx.query.status === "rejected"
        ? [
            // The viewer's own rejected submission: only they may reopen it.
            queueItem({
              submitted_by: ME,
              entry: entry({
                id: E3,
                user_id: ME,
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
    expect((await screen.findAllByText("1:30")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("0:30").length).toBeGreaterThan(0);
    expect(screen.getAllByText(/ticket 30000000/).length).toBe(2);
    expect(screen.getByTestId("queue-bar")).toBeInTheDocument();
    cleanup();
  });

  it("draws the queue in the approver's own zone, not the machine's", async () => {
    // 09:00Z is 18:00 in Tokyo. The account's zone decides the reading; the
    // machine's zone is whatever the suite happens to run in.
    setupApi(["special:approve_time"], "Asia/Tokyo");
    mount(<ApprovalsPage />);
    await screen.findAllByText("1:30");
    expect(screen.getAllByText("18:00").length).toBeGreaterThan(0);
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
    await screen.findAllByText("1:30");
    fireEvent.click(screen.getByLabelText(`Select entry on ticket ${T1}`));
    fireEvent.click(screen.getByTestId("bulk-approve"));

    await waitFor(() => {
      expect(mock.callsTo("POST", DECIDE)).toHaveLength(1);
    });
    const body = mock.lastBody<{ entry_ids: string[]; decision: string }>("POST", DECIDE);
    expect(body.decision).toBe("approve");
    expect(body.entry_ids).toEqual([E1]);
    await waitFor(() => {
      expect(screen.queryByText("1h 30m")).not.toBeInTheDocument();
    });
    expect(screen.getAllByText("0:30").length).toBeGreaterThan(0);
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
    await screen.findAllByText("1:30");
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
    await screen.findAllByText("1:30");
    fireEvent.click(screen.getByLabelText("Select all"));
    fireEvent.click(screen.getByTestId("bulk-approve"));

    await waitFor(() => {
      expect(screen.queryByText("1h 30m")).not.toBeInTheDocument();
    });
    expect(screen.getByText("Already invoiced — locked, nothing to decide")).toBeInTheDocument();
    expect(screen.getAllByText("0:30").length).toBeGreaterThan(0);
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
    await screen.findAllByText("1:30");
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
    await screen.findAllByText("1:30");
    expect(screen.queryByTestId("queue-bar")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("radio", { name: /Rejected/ }));
    const reopenBtn = await screen.findByTestId(`reopen-${E3}`);
    expect(screen.getByText("Rejected — Description is too vague")).toBeInTheDocument();
    fireEvent.click(reopenBtn);

    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/time-entries/${E3}/reopen`)).toHaveLength(1);
    });
    await waitFor(() => {
      expect(screen.queryByTestId(`reopen-${E3}`)).not.toBeInTheDocument();
    });
    cleanup();
  });

  it("keeps the viewer's own time out of the pass, and says why in the pane", async () => {
    const mock = setupApi();
    mock.on("GET", "/api/v1/time-entries/approvals/queue", () => ({
      json: [
        queueItem({
          entry: entry({
            id: E1,
            user_id: ME,
            notes: "Replaced the failed drive and confirmed the array rebuilt.",
          }),
        }),
        queueItem({
          entry: entry({
            id: E2,
            minutes: 30,
            ticket_id: T2,
            notes: "Reset the switch and confirmed the link came back up.",
          }),
        }),
      ],
    }));
    mount(<ApprovalsPage />);
    await screen.findAllByText("1:30");
    // The viewer's own row cannot be selected, and the pass skips it.
    expect(screen.getByLabelText(`Select entry on ticket ${T1}`)).toBeDisabled();
    fireEvent.click(screen.getByTestId("approvals-select-easy"));
    const bar = screen.getByTestId("bulk-bar");
    expect(screen.getByTestId("bulk-count")).toHaveTextContent("1 selected");
    expect(screen.getByText(/Another approver signs it off/)).toBeInTheDocument();
    cleanup();
  });

  // Wave 47 lane A15 (`p47-drift-recheck`). The self-approval rule above is
  // enforced from the ROW's own fields, and one of those fields is the
  // viewer's id — which `/api/v1/me` supplies asynchronously. Loading the
  // queue first renders every row with `meId` still "", so a row is
  // actionable for as long as that read is in flight. The queue now waits for
  // the identity, which is the only way the rule is true from first paint.
  it("holds the queue until the caller's identity is known", async () => {
    const mock = setupApi();
    // Hold `/api/v1/me` open and watch what renders while it is unanswered.
    const inner = currentFetchRouter();
    let releaseMe: () => void = () => undefined;
    const meGate = new Promise<void>((resolve) => {
      releaseMe = resolve;
    });
    setFetchRouter(async (input, init) => {
      const url =
        typeof input === "string" ? new URL(input) : input instanceof URL ? input : new URL(input.url);
      if (url.pathname === "/api/v1/me") await meGate;
      if (inner === null) throw new Error("MockApi router not installed");
      return inner(input, init);
    });

    mount(<ApprovalsPage />);
    // Settle the effects and their microtasks; the identity read is still open.
    await new Promise((resolve) => setTimeout(resolve, 25));
    // No queue was asked for, so no row exists — and in particular no row of
    // the viewer's own time rendered actionable.
    expect(mock.callsTo("GET", "/api/v1/time-entries/approvals/queue")).toHaveLength(0);
    expect(screen.queryByTestId(`approval-row-${E1}`)).not.toBeInTheDocument();

    releaseMe();
    await screen.findAllByText("1:30");
    // One load, not a pre-identity load plus a second after it.
    expect(mock.callsTo("GET", "/api/v1/time-entries/approvals/queue")).toHaveLength(1);
    cleanup();
  });

  it("renders the viewer's own row controls disabled, so the rule holds at the row too", async () => {
    const mock = setupApi();
    mock.on("GET", "/api/v1/time-entries/approvals/queue", () => ({
      json: [queueItem({ entry: entry({ id: E1, user_id: ME }) })],
    }));
    mount(<ApprovalsPage />);
    await screen.findAllByText("1:30");
    const row = screen.getByTestId(`approval-row-${E1}`);
    // Per-item controls, not just the bulk checkbox: the disabled Approve the
    // drift sweep read as a product bug is this rule, and it is the point.
    expect(within(row).getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(within(row).getByRole("button", { name: "Reject…" })).toBeDisabled();
    cleanup();
  });

  it("surfaces typed 403 copy, never raw JSON", async () => {
    const mock = setupApi();
    mock.on("POST", DECIDE, () => ({
      status: 403,
      json: { error: { reason: "forbidden", message: "missing special:approve_time" } },
    }));
    mount(<ApprovalsPage />);
    await screen.findAllByText("1:30");
    fireEvent.click(screen.getByLabelText(`Select entry on ticket ${T1}`));
    fireEvent.click(screen.getByTestId("bulk-approve"));
    expect(
      await screen.findByText("You don't have permission for this action — ask an owner or admin."),
    ).toBeInTheDocument();
    cleanup();
  });
});
