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
  number: "MAR-100",
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
  number: "MAR-101",
  title: "Laptop imaging",
  priority: "p2",
  category: "open",
  status_name: "New",
  client_name: null,
  due_at: MIDDAY_EOD,
  sla_resolution_due: null,
};
const LATER = {
  ...OVERDUE,
  id: "00000000-0000-4000-8000-0000000000a3",
  number: "MAR-102",
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
  number: "MAR-103",
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

// The day surface: one booking in the schedule, the rota it rides, no
// travel drafts, and yesterday's gap fed by two entry windows.
const APPOINTMENT_ID = "00000000-0000-4000-8000-0000000000e1";
const DISPATCH_DAY = {
  date: new Date(NOW).toISOString().slice(0, 10),
  unassigned: [],
  technicians: [
    {
      user_id: ME_ID,
      display_name: ME_FIXTURE.display_name,
      on_call: [],
      appointments: [
        {
          id: APPOINTMENT_ID,
          user_id: ME_ID,
          client_id: "00000000-0000-4000-8000-0000000000f1",
          client_name: "Acme Corp",
          title: "Mail server down",
          starts_at: iso(60 * 60_000),
          ends_at: iso(3 * 3600_000),
          status: "scheduled",
          kind: "onsite",
          ticket_id: OVERDUE.id,
          ticket_number: OVERDUE.number,
          travel_minutes: 0,
          travel_distance_meters: 0,
          dispatch_status: null,
        },
      ],
    },
  ],
};
const YESTERDAY_ISO = new Date(NOW - DAY).toISOString().slice(0, 10);
const TIME_ENTRIES = [
  {
    id: "00000000-0000-4000-8000-0000000000e2",
    org_id: ME_FIXTURE.org_id,
    user_id: ME_ID,
    ticket_id: OVERDUE.id,
    started_at: `${YESTERDAY_ISO}T09:00:00Z`,
    ended_at: `${YESTERDAY_ISO}T11:30:00Z`,
    minutes: 150,
    billable: true,
    approval_status: "approved",
    origin: "manual",
  },
];

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
  mock.on("GET", "/api/v1/boards/nav", () => ({ json: [] }));
  mock.on("GET", "/api/v1/tickets/my-work", () => ({ json: myWork }));
  mock.on("GET", "/api/v1/time-gaps/me", () => ({ json: GAP }));
  mock.on("GET", "/api/v1/dispatch/day", () => ({ json: DISPATCH_DAY }));
  mock.on("GET", "/api/v1/dispatch/travel-drafts", () => ({ json: [] }));
  mock.on("GET", "/api/v1/time-entries", () => ({ json: TIME_ENTRIES }));
  mock.on("GET", "/api/v1/time-categories/capture-defaults", () => ({
    json: {
      categories: [
        {
          id: "00000000-0000-4000-8000-0000000000e3",
          org_id: ME_FIXTURE.org_id,
          slug: "remote-support",
          label: "Remote support",
          default_billable: true,
          position: 0,
          created_at: iso(0),
        },
      ],
      work_type_map: [],
    },
  }));
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

    // The pack's day body carries the booking title too; the queue's row is
    // the hover link, so match that one.
    expect(await screen.findByText("Mail server down", { selector: "a > span.hover\\:underline" }, { timeout: 5000 })).toBeInTheDocument();
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

  it("shows the five attention cards with yesterday's gap and the day body", async () => {
    setupApi(APPROVER_ME);
    render(
      <AuthProvider>
        <TodayPage />
      </AuthProvider>,
    );

    // Pack attention cards: labels, figures, one honest line each.
    expect(await screen.findByText("Yours today", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.getByText("At risk")).toBeInTheDocument();
    expect(screen.getByText("Breached")).toBeInTheDocument();
    expect(screen.getByText("Not logged yesterday")).toBeInTheDocument();
    expect(screen.getByText("Changed since you looked")).toBeInTheDocument();
    // Yesterday card: 5h logged of 8h expected, 3h unaccounted.
    expect(
      await screen.findByText((_, el) => el?.textContent === "5h logged of 8h — 3h unaccounted", {}, { timeout: 5000 }),
    ).toBeInTheDocument();
    expect(await screen.findByText("3h not logged", {}, { timeout: 5000 })).toBeInTheDocument();
    // Your day: the booking renders with its pill and the live clock line.
    expect(await screen.findByText("Your day", {}, { timeout: 5000 })).toBeInTheDocument();
    await screen.findByText("Acme Corp", {}, { timeout: 5000 });
    cleanup();
  });

  it("opens the pack's quick-log dialog from the header and the yesterday card", async () => {
    setupApi(APPROVER_ME);
    render(
      <AuthProvider>
        <TodayPage />
      </AuthProvider>,
    );
    await screen.findByText("Not logged yesterday", {}, { timeout: 5000 });
    fireEvent.click(screen.getByRole("button", { name: /Quick log/ }));
    const dialog = await screen.findByRole("dialog", { name: "Quick log" });
    expect(dialog).toBeInTheDocument();
    // The pack's footer: nothing chosen yet, so the primary names the step.
    expect(screen.getByText("Choose a work type")).toBeInTheDocument();
    // The suggested chip is offered, not selected — it never auto-submits.
    expect(screen.getByText("Suggested")).toBeInTheDocument();
    cleanup();
  });

  it("starts a timer from a queue row with an idempotency-keyed envelope POST", async () => {
    const mock = setupApi(APPROVER_ME);
    render(
      <AuthProvider>
        <TodayPage />
      </AuthProvider>,
    );
    await screen.findByText("Mail server down", { selector: "a > span.hover\\:underline" }, { timeout: 5000 });

    fireEvent.click(screen.getByRole("button", { name: "Start timer for MAR-100" }));
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
    await screen.findByText("Mail server down", { selector: "a > span.hover\\:underline" }, { timeout: 5000 });

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
