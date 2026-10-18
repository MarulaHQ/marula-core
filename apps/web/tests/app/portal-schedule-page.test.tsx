// Integration test: /portal/schedule — the pack's day-chips-then-times shape
// plus the request semantics. The day is chosen first from chips, then the
// times for that day (portal/sand/states/09-schedule.png); the confirm card
// says REQUESTED, never booked, and the create call carries the typed slot.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import PortalSchedulePage from "../../app/portal/schedule/page";
import { PortalAuthProvider } from "../../lib/portal-api";
import { MockApi } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/portal/schedule",
}));

function seedPortalTokens(): void {
  window.localStorage.setItem(
    "marula-portal-tokens",
    JSON.stringify({
      access_token: "a",
      refresh_token: "r",
      session_id: "s",
      expires_in: 3600,
    }),
  );
}

// Slot dates must be LOCAL calendar dates (Wave 34 lane 6 fix): deriving
// them from the UTC slice of `now + 24h` makes the fixture label yesterday
// as "Today" between local midnight and the UTC/local date boundary,
// failing the day-grouping assertions for a couple of hours every night.
function localDateIn(days: number): string {
  const d = new Date(Date.now() + days * 86_400_000);
  return [
    d.getFullYear(),
    String(d.getMonth() + 1).padStart(2, "0"),
    String(d.getDate()).padStart(2, "0"),
  ].join("-");
}
const DAY1 = localDateIn(1);
const DAY2 = localDateIn(2);

// A slot at `hour` on the reader's clock, on tomorrow's local date. Built from
// local wall-clock, not from `...Z`: the portal groups a slot under the day the
// client reads it on, so a fixture pinned to UTC lands on the day before for
// every client west of Greenwich.
function slots(hour: number): { starts_at: string; ends_at: string } {
  const starts = new Date(`${DAY1}T${String(hour).padStart(2, "0")}:00:00`);
  return {
    starts_at: starts.toISOString(),
    ends_at: new Date(starts.getTime() + 3_600_000).toISOString(),
  };
}

const STAFF_SLOTS = [
  {
    user_id: "u9",
    display_name: "Sam Weaver",
    slots: [slots(13), slots(15)],
  },
];

const REQUESTS = [
  {
    id: "r1",
    user_id: "u9",
    title: "Quarterly review",
    starts_at: `${DAY2}T14:00:00.000Z`,
    ends_at: `${DAY2}T15:00:00.000Z`,
    kind: "remote",
    notes: null,
    status: "pending",
    decision_note: null,
    created_at: new Date().toISOString(),
  },
];

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/portal/me", () => ({
    json: {
      id: "u1",
      org_id: "o1",
      client_id: "c1",
      email: "ops@acme.com",
      display_name: "Ops User",
      role: "contact",
    },
  }));
  mock.on("GET", "/api/v1/portal/schedule/slots", () => ({ json: STAFF_SLOTS }));
  mock.on("GET", "/api/v1/portal/appointment-requests", () => ({ json: REQUESTS }));
  mock.on("POST", "/api/v1/portal/appointment-requests", () => ({
    json: { id: "r2", status: "pending" },
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedPortalTokens();
});

describe("/portal/schedule", () => {
  it("shows day chips first, then that day's times, and lists the pending request", async () => {
    const mock = setupApi();
    render(
      <PortalAuthProvider>
        <PortalSchedulePage />
      </PortalAuthProvider>,
    );
    // Day chips carry the day; no times are offered until a day is picked.
    expect(await screen.findByTestId(`portal-day-${DAY1}`)).toBeInTheDocument();
    expect(screen.getByTestId(`portal-day-${DAY1}`).textContent).toBe("Tomorrow");
    expect(screen.queryByLabelText("What time?")).toBeNull();

    // The pending request is listed with the honest pill, not as booked.
    expect(screen.getByText("Quarterly review")).toBeInTheDocument();
    expect(screen.getAllByText("Waiting to be confirmed").length).toBeGreaterThanOrEqual(1);
    expect(mock.calls.some((c) => c.path === "/api/v1/portal/schedule/slots")).toBe(true);
    cleanup();
  });

  it("carries the picked slot into the typed create call and says REQUESTED", async () => {
    const mock = setupApi();
    render(
      <PortalAuthProvider>
        <PortalSchedulePage />
      </PortalAuthProvider>,
    );
    fireEvent.click(await screen.findByTestId(`portal-day-${DAY1}`));
    const timeGroup = screen.getByLabelText("What time?");
    expect(timeGroup).toBeInTheDocument();

    // Times are the only thing on the time buttons — the day was already
    // picked, so the button is not a sentence.
    const times = Array.from(
      timeGroup.querySelectorAll('[data-testid^="portal-slot-"]'),
    );
    expect(times).toHaveLength(2);
    const local = (iso: string) =>
      new Date(iso).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false });
    expect(times.map((t) => t.textContent)).toEqual([
      local(slots(13).starts_at),
      local(slots(15).starts_at),
    ]);
    fireEvent.click(times[0] as HTMLElement);

    // The primary is disabled until a slot is picked; the reason select and
    // the optional description ride along.
    const primary = screen.getByTestId("portal-schedule-request");
    expect(primary).toBeEnabled();
    fireEvent.change(screen.getByLabelText("What is it about?"), {
      target: { value: "A new starter or a leaver" },
    });
    fireEvent.click(primary);
    await waitFor(() => {
      expect(screen.getByTestId("portal-schedule-requested").textContent).toContain(
        "Requested. We'll confirm within the hour.",
      );
    });
    const body = mock.lastBody<Record<string, unknown>>("POST", "/api/v1/portal/appointment-requests");
    expect(body.user_id).toBe("u9");
    expect(body.title).toBe("A new starter or a leaver");
    expect(body.starts_at).toBe(slots(13).starts_at);
    expect(body.kind).toBe("remote");
    expect((body.idempotency_key as string).startsWith("portal-req-")).toBe(true);
    cleanup();
  });

  it("names WHO a shared clock hour is with, so the client never books 'whoever'", async () => {
    const mock = new MockApi();
    mock.on("GET", "/api/v1/portal/me", () => ({
      json: {
        id: "u1",
        org_id: "o1",
        client_id: "c1",
        email: "ops@acme.com",
        display_name: "Ops User",
        role: "contact",
      },
    }));
    // Two people hold the SAME clock hour on the day in hand (the seed's
    // Diego and Tessa both open the same slot) — the buttons must say who each
    // is with, or the client would be booking "whoever" (capture review,
    // batch 9).
    mock.on("GET", "/api/v1/portal/schedule/slots", () => ({
      json: [
        { user_id: "u9", display_name: "Diego Dispatcher", slots: [slots(13)] },
        { user_id: "u10", display_name: "Tessa Technician", slots: [slots(13), slots(15)] },
      ],
    }));
    mock.on("GET", "/api/v1/portal/appointment-requests", () => ({ json: [] }));
    mock.install();
    render(
      <PortalAuthProvider>
        <PortalSchedulePage />
      </PortalAuthProvider>,
    );
    fireEvent.click(await screen.findByTestId(`portal-day-${DAY1}`));
    const timeGroup = screen.getByLabelText("What time?");
    const timeButtons = Array.from(
      timeGroup.querySelectorAll('[data-testid^="portal-slot-"]'),
    );
    expect(timeButtons).toHaveLength(3);
    // The two buttons sharing an hour carry first names; the unique hour does
    // not (the pack's day is one person, so it draws bare times).
    expect(timeButtons[0]?.textContent).toContain("Diego");
    expect(timeButtons[1]?.textContent).toContain("Tessa");
    expect(timeButtons[2]?.textContent).not.toContain("Tessa");
    // Unique testids, because a start time alone is not a person.
    const ids = timeButtons.map((s) => s.getAttribute("data-testid"));
    expect(new Set(ids).size).toBe(3);
    cleanup();
  });
});
