// Integration test: /calendar — week/month/agenda presentations, entry
// differentiation (appointment / on call / synced busy / time off /
// ticket deadline), overlap flagging consistent with the sync panel, and
// the reschedule path riding POST /appointments/{id}/move (Wave 17 lane 4).
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import CalendarPage from "../../app/calendar/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/calendar",
}));

const ME_ID = ME_FIXTURE.user_id;
const APPT_ID = "00000000-0000-4000-8000-0000000000a1";
const ONCALL_ID = "00000000-0000-4000-8000-0000000000a2";
const SYNCED_ID = "00000000-0000-4000-8000-0000000000e1";
const TICKET_ID = "00000000-0000-4000-8000-0000000000t9";

const TODAY = new Date();
const at = (day: Date, h: number, m: number) =>
  new Date(day.getFullYear(), day.getMonth(), day.getDate(), h, m).toISOString();

function calendarView() {
  return {
    from: at(TODAY, 0, 0),
    to: at(new Date(TODAY.getFullYear(), TODAY.getMonth(), TODAY.getDate() + 7), 0, 0),
    timezone: "UTC",
    user_id: ME_ID,
    availability_windows: [],
    items: [
      {
        id: APPT_ID,
        item_type: "appointment",
        starts_at: at(TODAY, 9, 0),
        ends_at: at(TODAY, 10, 0),
        title: "Site visit",
        kind: "on_site",
        status: null,
        ticket_id: TICKET_ID,
        user_id: ME_ID,
      },
      {
        id: ONCALL_ID,
        item_type: "on_call",
        starts_at: at(TODAY, 14, 0),
        ends_at: at(TODAY, 15, 0),
        title: null,
        category: "Primary",
        kind: null,
        status: null,
        ticket_id: null,
        user_id: ME_ID,
      },
    ],
    exclusions: [
      {
        id: SYNCED_ID,
        org_id: ME_FIXTURE.org_id,
        user_id: ME_ID,
        starts_at: at(TODAY, 9, 30),
        ends_at: at(TODAY, 10, 30),
        reason: "Calendar sync: Vendor maintenance window",
        created_at: "",
      },
      {
        id: "00000000-0000-4000-8000-0000000000e2",
        org_id: ME_FIXTURE.org_id,
        user_id: ME_ID,
        starts_at: at(new Date(TODAY.getFullYear(), TODAY.getMonth(), TODAY.getDate() + 2), 9, 0),
        ends_at: at(new Date(TODAY.getFullYear(), TODAY.getMonth(), TODAY.getDate() + 2), 17, 0),
        reason: "Time off",
        created_at: "",
      },
    ],
  };
}

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/calendar/week", () => ({ json: calendarView() }));
  mock.on("GET", "/api/v1/calendar/team", () => ({ json: calendarView() }));
  mock.on("GET", "/api/v1/calendar/deadlines", () => ({
    json: {
      from: "",
      to: "",
      deadlines: [
        {
          ticket_id: TICKET_ID,
          board_id: "00000000-0000-4000-8000-0000000000b1",
          number: "MAR-42",
          title: "Mail server rejecting inbound",
          due_at: at(TODAY, 12, 0),
          category: "open",
          assignee_id: null,
          client_id: null,
        },
      ],
    },
  }));
  mock.on("POST", `/api/v1/appointments/${APPT_ID}/move`, () => ({
    json: { id: APPT_ID, title: "Site visit" },
  }));
  return mock.install();
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

function mount() {
  return render(
    <AuthProvider>
      <CalendarPage />
    </AuthProvider>,
  );
}

describe("CalendarPage", () => {
  it("renders the week grid with differentiated entries and the overlap badge", async () => {
    setupApi();
    mount();
    const appt = await screen.findByTestId(`week-block-${APPT_ID}`);
    expect(appt).toHaveTextContent("Site visit");
    expect(appt).toHaveAttribute("aria-label", expect.stringContaining("overlaps"));
    expect(screen.getByTestId(`week-block-${ONCALL_ID}`)).toHaveTextContent("On call · Primary");
    expect(screen.getByTestId(`week-block-${SYNCED_ID}`)).toHaveTextContent("Synced · Vendor maintenance window");
    expect(
      screen.getByTestId(`week-block-deadline:${TICKET_ID}`),
    ).toHaveTextContent("MAR-42");
    expect(screen.getByTestId(`day-overlaps-${isoDay(TODAY)}`)).toHaveTextContent("1 overlap");
    cleanup();
  });

  it("the agenda lists every entry class and marks the conflicting row", async () => {
    setupApi();
    mount();
    await screen.findByTestId(`week-block-${APPT_ID}`);
    fireEvent.click(screen.getByRole("button", { name: "Agenda" }));
    const list = await screen.findByTestId("agenda-list");
    expect(list).toHaveTextContent("Site visit");
    expect(list).toHaveTextContent("MAR-42 Mail server rejecting inbound");
    expect(screen.getByTestId(`agenda-entry-${SYNCED_ID}`)).toHaveTextContent("Synced busy");
    const apptRow = screen.getByTestId(`agenda-entry-${APPT_ID}`);
    expect(apptRow).toHaveTextContent("overlaps");
    cleanup();
  });

  it("the month grid covers the month and opens a week on cell click", async () => {
    setupApi();
    mount();
    await screen.findByTestId(`week-block-${APPT_ID}`);
    fireEvent.click(screen.getByRole("button", { name: "Month" }));
    const grid = await screen.findByTestId("month-grid");
    expect(grid).toBeInTheDocument();
    fireEvent.click(screen.getByText("Site visit"));
    await waitFor(() => {
      expect(screen.getByTestId("week-grid")).toBeInTheDocument();
    });
    cleanup();
  });

  it("reschedules through the details dialog via the move endpoint", async () => {
    const mock = setupApi();
    mount();
    fireEvent.click(await screen.findByTestId(`week-block-${APPT_ID}`));
    fireEvent.click(await screen.findByRole("button", { name: "Reschedule" }));
    fireEvent.click(screen.getByRole("button", { name: "Move" }));
    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/appointments/${APPT_ID}/move`)).toHaveLength(1);
    });
    const body = mock.lastBody<{ starts_at: string; ends_at: string }>(
      "POST",
      `/api/v1/appointments/${APPT_ID}/move`,
    );
    expect(new Date(body.starts_at).getTime()).toBeLessThan(new Date(body.ends_at).getTime());
    cleanup();
  });

  it("a conflicting move surfaces the atomic-rejection copy and keeps the entry", async () => {
    const mock = setupApi();
    mock.on("POST", `/api/v1/appointments/${APPT_ID}/move`, () => ({
      status: 409,
      json: { error: { message: "target slot already booked" } },
    }));
    mount();
    fireEvent.click(await screen.findByTestId(`week-block-${APPT_ID}`));
    fireEvent.click(await screen.findByRole("button", { name: "Reschedule" }));
    fireEvent.click(screen.getByRole("button", { name: "Move" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "That slot is already booked — the move was rejected and the appointment is unchanged.",
    );
    cleanup();
  });
});

function isoDay(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}
