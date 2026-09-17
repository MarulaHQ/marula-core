// Integration test: CalendarSyncPanel — calendar-sync status/conflict
// surfaces (Wave 14 lane 7 close-out). Covers: connection health render
// (status/direction/last-sync/failure copy), sync-now with outcome
// counters (overlaps surfaced as the conflict signal), the one-time feed
// URL on create/rotate, pause/resume, and connect.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import CalendarSyncPanel from "../../components/calendar-sync-panel";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/calendar",
}));

const ORG = ME_FIXTURE.org_id;
const ME = ME_FIXTURE.user_id;
const INBOUND = "50000000-0000-4000-8000-000000000001";
const OUTBOUND = "50000000-0000-4000-8000-000000000002";
const NOW = "2026-09-14T08:00:00Z";

function conn(over: Record<string, unknown>) {
  return {
    id: INBOUND,
    org_id: ORG,
    user_id: ME,
    name: "Work calendar",
    provider: "ical",
    direction: "inbound",
    status: "active",
    feed_url: "https://cal.example.com/feed.ics",
    feed_path: null,
    last_synced_at: NOW,
    last_error: null,
    created_at: NOW,
    updated_at: NOW,
    ...over,
  };
}

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/calendar-sync", () => ({
    json: [
      conn({}),
      conn({
        id: OUTBOUND,
        name: "Publish feed",
        direction: "outbound",
        feed_url: null,
        feed_path: `/api/v1/calendar-sync/feed/tok123`,
      }),
    ],
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

function mountPanel() {
  return render(
    <AuthProvider>
      <CalendarSyncPanel />
    </AuthProvider>,
  );
}

describe("CalendarSyncPanel", () => {
  it("renders connection health: direction, status, last sync, and failure copy", async () => {
    const mock = setupApi();
    mock.on("GET", "/api/v1/calendar-sync", () => ({
      json: [
        conn({ status: "error", last_error: "the external calendar is unreachable", last_synced_at: null }),
        conn({
          id: OUTBOUND,
          name: "Publish feed",
          direction: "outbound",
          feed_url: null,
          feed_path: "/api/v1/calendar-sync/feed/tok123",
        }),
      ],
    }));
    mountPanel();
    expect(await screen.findByText("Work calendar")).toBeInTheDocument();
    expect(screen.getByText("Error")).toBeInTheDocument();
    expect(screen.getByText("the external calendar is unreachable")).toBeInTheDocument();
    expect(screen.getByText(/Outbound — publishes your bookings/)).toBeInTheDocument();
    expect(screen.getByText(/last sync never/)).toBeInTheDocument();
    cleanup();
  });

  it("sync-now posts to the typed endpoint and renders outcome counters with overlaps flagged", async () => {
    const mock = setupApi();
    mock.on("POST", `/api/v1/calendar-sync/${INBOUND}/sync`, () => ({
      json: {
        connection_id: INBOUND,
        outcome: { imported: 3, updated: 1, released: 0, overlaps: 2, duplicates: 0 },
      },
    }));
    mountPanel();
    fireEvent.click(await screen.findByTestId(`sync-now-${INBOUND}`));
    const outcome = await screen.findByTestId(`sync-outcome-${INBOUND}`);
    expect(outcome).toHaveTextContent("imported 3");
    expect(outcome).toHaveTextContent("2 external events overlapped existing bookings");
    await waitFor(() => {
      expect(mock.callsTo("GET", "/api/v1/calendar-sync").length).toBeGreaterThanOrEqual(2);
    });
    cleanup();
  });

  it("rotate-token shows the raw feed URL exactly once with a copy control", async () => {
    const mock = setupApi();
    mock.on("POST", `/api/v1/calendar-sync/${OUTBOUND}/rotate-token`, () => ({
      json: { connection: OUTBOUND, feed_token: "rot789" },
    }));
    mountPanel();
    fireEvent.click(await screen.findByText("Rotate feed token"));
    const block = await screen.findByTestId("one-time-feed-url");
    expect(block).toHaveTextContent("shown once");
    expect(block).toHaveTextContent("calendar-sync/feed/rot789");
    cleanup();
  });

  it("pause disables the connection through PATCH (read-your-write row update)", async () => {
    const mock = setupApi();
    mock.on("PATCH", `/api/v1/calendar-sync/${INBOUND}`, () => ({
      json: conn({ status: "disabled" }),
    }));
    mountPanel();
    const row = await screen.findByTestId(`sync-connection-${INBOUND}`);
    fireEvent.click(within(row).getByText("Pause"));
    await waitFor(() => {
      expect(mock.callsTo("PATCH", `/api/v1/calendar-sync/${INBOUND}`)).toHaveLength(1);
    });
    const body = mock.lastBody<{ status: string }>("PATCH", `/api/v1/calendar-sync/${INBOUND}`);
    expect(body.status).toBe("disabled");
    expect(await screen.findByText("Paused")).toBeInTheDocument();
    cleanup();
  });

  it("connect creates a connection and surfaces the one-time feed token", async () => {
    const mock = setupApi();
    mock.on("POST", "/api/v1/calendar-sync", (ctx) => {
      const body = ctx.body as { name: string; direction: string; feed_url?: string };
      expect(body.name).toBe("Side calendar");
      expect(body.direction).toBe("inbound");
      expect(body.feed_url).toBe("https://x.example.com/f.ics");
      return { json: { connection: "50000000-0000-4000-8000-000000000003", feed_token: "newtok" } };
    });
    mountPanel();
    fireEvent.click(await screen.findByTestId("connect-calendar"));
    fireEvent.change(screen.getByPlaceholderText("Work calendar"), { target: { value: "Side calendar" } });
    fireEvent.change(screen.getByPlaceholderText("https://calendar.example.com/feed.ics"), {
      target: { value: "https://x.example.com/f.ics" },
    });
    fireEvent.click(screen.getByTestId("connect-save"));
    expect(await screen.findByTestId("one-time-feed-url")).toHaveTextContent("feed/newtok");
    cleanup();
  });

  it("renders the empty state when no connections exist", async () => {
    const mock = setupApi();
    mock.on("GET", "/api/v1/calendar-sync", () => ({ json: [] }));
    mountPanel();
    expect(await screen.findByText(/No external calendars connected yet/)).toBeInTheDocument();
    cleanup();
  });

  it("the inbound mirror lists imported events and marks overlaps against rendered entries", async () => {
    const mock = setupApi();
    mock.on("GET", `/api/v1/calendar-sync/${INBOUND}/events`, () => ({
      json: {
        connection_id: INBOUND,
        events: [
          {
            id: "60000000-0000-4000-8000-000000000001",
            connection_id: INBOUND,
            external_uid: "uid-1",
            user_id: ME,
            org_id: ORG,
            starts_at: "2026-09-14T09:30:00Z",
            ends_at: "2026-09-14T10:30:00Z",
            title: "Vendor maintenance",
            sequence: 1,
            first_seen_at: NOW,
            last_seen_at: NOW,
            exclusion_id: null,
          },
          {
            id: "60000000-0000-4000-8000-000000000002",
            connection_id: INBOUND,
            external_uid: "uid-2",
            user_id: ME,
            org_id: ORG,
            starts_at: "2026-09-14T15:00:00Z",
            ends_at: "2026-09-14T16:00:00Z",
            title: "Quiet hour",
            sequence: 1,
            first_seen_at: NOW,
            last_seen_at: NOW,
            exclusion_id: null,
          },
        ],
      },
    }));
    render(
      <AuthProvider>
        <CalendarSyncPanel
          entries={[
            {
              kind: "appointment",
              id: "appt-1",
              title: "Site visit",
              startsAt: "2026-09-14T09:00:00Z",
              endsAt: "2026-09-14T10:00:00Z",
              ticketId: null,
              itemKind: null,
              status: null,
              userId: null,
              number: null,
            },
          ]}
        />
      </AuthProvider>,
    );
    fireEvent.click(await screen.findByTestId(`mirror-toggle-${INBOUND}`));
    const list = await screen.findByTestId(`mirror-list-${INBOUND}`);
    expect(within(list).getByText("Vendor maintenance")).toBeInTheDocument();
    expect(within(list).getByText("overlaps your Site visit")).toBeInTheDocument();
    expect(within(list).getByText("Quiet hour")).toBeInTheDocument();
    expect(within(list).queryByText("overlaps your Site visit")).not.toBeNull();
    expect(within(list).getAllByText(/overlaps your/)).toHaveLength(1);
    cleanup();
  });
});
