// Integration test: /today — the daily landing work queue. Covers the
// overdue/due-today grouping, the attention strip, the approver-only
// approvals count, and the instant-first timer start (idempotency-keyed
// envelope POST) against SDK-typed mock routes.
import { describe, expect, it, beforeEach, afterEach, afterAll, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import TodayPage from "../../app/today/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/today",
}));

const ME_ID = ME_FIXTURE.user_id;

// The "due today" fixture sits NOW + 2h; freezing the clock to module-load

/** jsdom has no usable WebSocket; the timer hook must degrade quietly. */
class FakeWebSocket {
  close(): void {}
  send(): void {}
}
vi.stubGlobal("WebSocket", FakeWebSocket as unknown as typeof WebSocket);
afterAll(() => {
  vi.unstubAllGlobals();
});

// The "due today" fixture sits NOW + 2h; freezing the clock to module-load
// time keeps +2h inside the same local day no matter when the suite runs.
const NOW = Date.now();
const DAY = 86_400_000;
const iso = (offsetMs: number) => new Date(NOW + offsetMs).toISOString();
// A due time that stays inside the running suite's local day no matter
// what weekday/time the suite is executed at (half the way to midnight).
const END_OF_DAY = (() => {
  const e = new Date(NOW);
  e.setHours(23, 59, 59, 999);
  return e.getTime();
})();
const MIDDAY_EOD = new Date(NOW + Math.floor((END_OF_DAY - NOW) / 2)).toISOString();
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
});
afterEach(() => {
  vi.useRealTimers();
});

const OVERDUE = {
  id: "00000000-0000-4000-8000-0000000000a1",
  board_id: "00000000-0000-4000-8000-0000000000b1",
  board_name: "Service Desk",
  number: "MER-100",
  title: "Mail server down",
  priority: "p1",
  category: "in_progress",
  status_name: "In progress",
  client_name: "Acme Corp",
  due_at: null,
  sla_resolution_due: iso(-2 * 3600_000),
  updated_at: iso(-3600_000),
};
const DUE_TODAY = {
  ...OVERDUE,
  id: "00000000-0000-4000-8000-0000000000a2",
  number: "MER-101",
  title: "Laptop imaging",
  priority: "p2",
  category: "open",
  status_name: "New",
  client_name: null,
  due_at: new Date(MIDDAY_EOD).toISOString(),
  sla_resolution_due: null,
};
const LATER = {
  ...OVERDUE,
  id: "00000000-0000-4000-8000-0000000000a3",
  number: "MER-102",
  title: "Quarterly patch cycle",
  priority: "p3",
  category: "open",
  status_name: "New",
  client_name: null,
  due_at: iso(3 * DAY),
  sla_resolution_due: null,
};
const NO_DUE = {
  ...OVERDUE,
  id: "00000000-0000-4000-8000-0000000000a4",
  number: "MER-103",
  title: "Docs cleanup",
  priority: "p3",
  category: "open",
  status_name: "New",
  client_name: null,
  due_at: null,
  sla_resolution_due: null,
};

const MY_WORK = [LATER, DUE_TODAY, OVERDUE, NO_DUE];

const GAP = {
  user_id: ME_ID,
  timezone: "UTC",
  expected_minutes: 480,
  days: [
    {
      day: new Date(NOW - DAY).toISOString().slice(0, 10),
      activity: true,
      expected_minutes: 480,
      logged_minutes: 300,
      has_gap: true,
      unaccounted_minutes: 180,
      nudge_sent_at: null,
    },
  ],
};

const QUEUE_ITEM = (entryId: string) => ({
  decided_at: null,
  decided_by: null,
  entry: {
    id: entryId,
    org_id: ME_FIXTURE.org_id,
    ticket_id: OVERDUE.id,
    user_id: ME_ID,
    started_at: iso(-4 * 3600_000),
    ended_at: iso(-3 * 3600_000),
    minutes: 60,
    billable: true,
    notes: null,
    submitted_at: iso(-2 * 3600_000),
    submitted_by: ME_ID,
  },
  locked: false,
  submitted_at: iso(-2 * 3600_000),
  submitted_by: ME_ID,
});

const TIMER_SESSION = {
  id: "00000000-0000-4000-8000-0000000000c1",
  org_id: ME_FIXTURE.org_id,
  user_id: ME_ID,
  ticket_id: OVERDUE.id,
  started_at: iso(-600_000),
  stopped_at: null,
  segments: [{ started_at: iso(-600_000), ended_at: null }],
};

const APPROVER_ME = {
  ...ME_FIXTURE,
  permissions: ["special:approve_time"],
};
const MEMBER_ME = {
  ...ME_FIXTURE,
  permissions: ["tickets:view"],
};

function setupApi(me: typeof ME_FIXTURE, myWork: unknown[] = MY_WORK): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: me }));
  mock.on("GET", "/api/v1/tickets/my-work", () => ({ json: myWork }));
  mock.on("GET", "/api/v1/time-gaps/me", () => ({ json: GAP }));
  mock.on("GET", "/api/v1/time-entries/approvals/queue", () => ({
    json: [QUEUE_ITEM("00000000-0000-4000-8000-0000000000d1"), QUEUE_ITEM("00000000-0000-4000-8000-0000000000d2")],
  }));
  mock.on("GET", "/api/v1/timers/working-set", () => ({ json: [] }));
  mock.on("POST", "/api/v1/timers/start", () => ({ json: TIMER_SESSION }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/today integration", () => {
  it("groups the queue into overdue / due today / later / no due date", async () => {
    setupApi(APPROVER_ME);
    render(
      <AuthProvider>
        <TodayPage />
      </AuthProvider>,
    );

    expect(await screen.findByText("Mail server down", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Overdue" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Due today" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Later" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "No due date" })).toBeInTheDocument();
    // Countdown column: overdue rows read as overdue, future rows as spans.
    expect(screen.getByText("overdue 2h")).toBeInTheDocument();
    expect(screen.getByText("Laptop imaging")).toBeInTheDocument();
    expect(screen.getByText("Quarterly patch cycle")).toBeInTheDocument();
    expect(screen.getByText("Docs cleanup")).toBeInTheDocument();
    cleanup();
  });

  it("shows the attention strip, missing-time audit, and idle focus line", async () => {
    setupApi(APPROVER_ME);
    render(
      <AuthProvider>
        <TodayPage />
      </AuthProvider>,
    );

    expect(await screen.findByText("Approvals waiting", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.getByText("Overdue", { selector: "span" })).toBeInTheDocument();
    expect(screen.getByText("Time to log")).toBeInTheDocument();
    // Missing-time strip: yesterday 5h logged of 8h expected, 3h unaccounted.
    expect(
      screen.getByText((_, el) =>
        el?.textContent === "Yesterday 5h logged of 8h expected — 3h unaccounted",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("No active timer — start one from a row below; the work bar tracks it.")).toBeInTheDocument();
    cleanup();
  });

  it("opens the quick-log dialog from the missing-time strip", async () => {
    setupApi(APPROVER_ME);
    render(
      <AuthProvider>
        <TodayPage />
      </AuthProvider>,
    );
    await screen.findByText("Time to log", {}, { timeout: 5000 });
    fireEvent.click(screen.getByRole("button", { name: "Log time" }));
    expect(await screen.findByRole("dialog", { name: "Log missing time" })).toBeInTheDocument();
    cleanup();
  });

  it("starts a timer from a queue row with an idempotency-keyed envelope POST", async () => {
    const mock = setupApi(APPROVER_ME);
    render(
      <AuthProvider>
        <TodayPage />
      </AuthProvider>,
    );
    await screen.findByText("Mail server down", {}, { timeout: 5000 });

    fireEvent.click(screen.getByRole("button", { name: "Start timer for MER-100" }));
    await waitFor(() => {
      expect(mock.callsTo("POST", "/api/v1/timers/start")).toHaveLength(1);
    });
    const body = mock.lastBody<{ idempotency_key: string; ticket_id: string }>(
      "POST",
      "/api/v1/timers/start",
    );
    expect(body.ticket_id).toBe(OVERDUE.id);
    expect(body.idempotency_key).toBeTruthy();
    cleanup();
  });

  it("hides the approvals strip from non-approvers and never fetches the queue", async () => {
    const mock = setupApi(MEMBER_ME);
    render(
      <AuthProvider>
        <TodayPage />
      </AuthProvider>,
    );
    await screen.findByText("Mail server down", {}, { timeout: 5000 });

    expect(screen.queryByText("Approvals waiting")).not.toBeInTheDocument();
    expect(mock.callsTo("GET", "/api/v1/time-entries/approvals/queue")).toHaveLength(0);
    cleanup();
  });

  it("shows the clean empty state when nothing is assigned", async () => {
    setupApi(APPROVER_ME, []);
    render(
      <AuthProvider>
        <TodayPage />
      </AuthProvider>,
    );
    expect(
      await screen.findByText(
        "Nothing assigned to you yet. Tickets appear here highest-priority first.",
        {},
        { timeout: 5000 },
      ),
    ).toBeInTheDocument();
    cleanup();
  });
});
