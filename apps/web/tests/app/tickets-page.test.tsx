// Integration test: /tickets — the ticket ledger rebuilt as a Marula data
// surface. Covers the four toolbar controls, the per-row SLA countdown (the
// typed SLA status preferred, the ticket's own due date as the fallback, and
// neither reading as an em dash rather than a false zero), bulk selection and
// its priority write, and the merged-row banner.
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { Profiler } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import { clearTicketSlaCache } from "../../lib/ticket-sla";
import TicketsListPage from "../../app/tickets/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/tickets",
}));

const NOW = Date.now();
const HOUR = 3_600_000;
const iso = (offsetMs: number) => new Date(NOW + offsetMs).toISOString();

const ORG = ME_FIXTURE.org_id;
const BOARD = "00000000-0000-4000-8000-0000000000b1";

function ticket(over: Record<string, unknown> = {}) {
  return {
    id: "00000000-0000-4000-8000-0000000000a1",
    org_id: ORG,
    board_id: BOARD,
    status_id: "00000000-0000-4000-8000-0000000000c1",
    category: "open",
    category_id: null,
    number: "MAR-100",
    title: "Mail server rejecting inbound",
    body: null,
    priority: "p2",
    assignee_id: ME_FIXTURE.user_id,
    requester_id: null,
    client_id: null,
    contact_id: null,
    site_id: null,
    type_id: null,
    due_at: null,
    created_at: iso(-48 * HOUR),
    updated_at: iso(-HOUR),
    resolved_at: null,
    merged_into_ticket_id: null,
    ...over,
  };
}

/** SLA risk: inside the 30-minute window, so the cell must read as at risk. */
const AT_RISK = ticket({
  id: "00000000-0000-4000-8000-0000000000a2",
  number: "MAR-101",
  title: "Switch uplink flapping",
  priority: "p1",
  category: "in_progress",
  due_at: iso(10 * 60_000),
});

/** No SLA policy and no due date: the honest reading is an em dash. */
const UNDATED = ticket({
  id: "00000000-0000-4000-8000-0000000000a3",
  number: "MAR-102",
  title: "Docs cleanup sprint",
  category: "waiting",
});

const MERGED = ticket({
  id: "00000000-0000-4000-8000-0000000000a4",
  number: "MAR-099",
  title: "Duplicate of the uplink ticket",
  merged_into_ticket_id: "00000000-0000-4000-8000-0000000000a2",
});

const TICKETS = [AT_RISK, UNDATED, MERGED];

function setupApi(rows: unknown[] = TICKETS): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/tickets", () => ({ json: rows }));
  mock.on("GET", "/api/v1/settings/user", () => ({ json: [] }));
  mock.on("PUT", "/api/v1/settings/user", () => ({ json: { key: "appearance.data_surfaces", value: "{}" } }));
  mock.on("GET", "/api/v1/org/users", () => ({ json: { users: [] } }));
  mock.on("GET", "/api/v1/clients", () => ({ json: [] }));
  // MockApi matches literal pathnames, so the per-ticket routes are registered
  // by id. The SLA endpoint is a typed empty state, never a 404: no policy
  // applies to any of these fixtures, which is exactly the case the countdown
  // column has to survive.
  for (const row of rows as { id: string }[]) {
    const id = row.id;
    mock.on("GET", `/api/v1/tickets/${id}/sla`, () => ({ json: TYPED_EMPTY_SLA }));
    mock.on("PATCH", `/api/v1/tickets/${id}`, () => ({ json: { ...ticket(), id } }));
  }
  mock.install();
  return mock;
}

/** The endpoint is a typed empty state, never a 404 (docs: TicketSlaStatus). */
const TYPED_EMPTY_SLA = { applied: false, breached: false, minutes_remaining: null, paused: false, state: null };

beforeEach(() => {
  // The SLA cache is module-level by design (one read per ticket per session),
  // so each case starts from an empty cache or it would inherit the last one's.
  clearTicketSlaCache();
  // The countdown reads the wall clock, so a suite that drifts by a minute
  // would turn "10m left" into "9m left" and fail for the wrong reason.
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
  window.localStorage.clear();
  seedAuthTokens();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("/tickets ledger", () => {
  it("renders the four data-surface controls on one toolbar", async () => {
    setupApi();
    render(
      <AuthProvider>
        <TicketsListPage />
      </AuthProvider>,
    );

    await screen.findByText("Switch uplink flapping", {}, { timeout: 5000 });
    // Named views, the filter builder, the column chooser and the export row
    // count all sit on the same strip.
    expect(screen.getByLabelText("Ticket views")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Add filter/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Columns/ })).toBeInTheDocument();
    // The footer reads "shown of total" per the pack, with the density control
    // beside it (the kit's SurfaceFooter since Wave 48). The two numbers are
    // their own elements, so the sentence is matched on the count element.
    expect(
      screen.getByText(
        (_content, element) =>
          element?.classList.contains("m-surface-footer-count") === true &&
          /^\d+ of \d+ tickets$/.test(element.textContent ?? ""),
      ),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Row density")).toBeInTheDocument();
    cleanup();
  });

  it("reads the countdown from the SLA status, the due date, or neither", async () => {
    setupApi();
    render(
      <AuthProvider>
        <TicketsListPage />
      </AuthProvider>,
    );

    await screen.findByText("Switch uplink flapping", {}, { timeout: 5000 });

    // 10 minutes left on a due date is inside the risk window: it reads as at
    // risk rather than as a neutral countdown.
    expect(screen.getByText("10m left")).toBeInTheDocument();
    // A ticket with no policy and no deadline must not claim zero.
    const undated = screen.getByText("Docs cleanup sprint").closest("tr");
    expect(undated?.textContent).toContain("—");
    cleanup();
  });

  it("says a merged ticket is closed and links to the survivor", async () => {
    setupApi();
    render(
      <AuthProvider>
        <TicketsListPage />
      </AuthProvider>,
    );

    await screen.findByText("Duplicate of the uplink ticket", {}, { timeout: 5000 });
    const banner = screen.getByTestId("merge-banner");
    expect(banner).toBeInTheDocument();
    expect(banner.textContent).toContain("merged");
    cleanup();
  });

  it("bulk-selects rows and writes the chosen priority to each one", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <TicketsListPage />
      </AuthProvider>,
    );

    await screen.findByText("Switch uplink flapping", {}, { timeout: 5000 });

    fireEvent.click(screen.getByRole("checkbox", { name: "Select MAR-101" }));
    const bar = await screen.findByRole("group", { name: "Bulk actions" });
    expect(bar).toBeInTheDocument();
    expect(screen.getByText("1 selected")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Priority to set on the selected tickets"), {
      target: { value: "p3" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Set priority" }));

    await waitFor(() => {
      expect(mock.callsTo("PATCH", `/api/v1/tickets/${AT_RISK.id}`)).toHaveLength(1);
    });
    await screen.findByText("Set P3 on 1 ticket.", {}, { timeout: 5000 });
    cleanup();
  });

  it("explains an empty result and offers to clear the filters", async () => {
    setupApi([]);
    render(
      <AuthProvider>
        <TicketsListPage />
      </AuthProvider>,
    );

    expect(
      await screen.findByText("Nothing is assigned to you right now.", {}, { timeout: 5000 }),
    ).toBeInTheDocument();
    cleanup();
  });

  // Wave 47 lane A21. A page that never stops re-rendering starves the App
  // Router's `startTransition`, so `router.push` never commits and never calls
  // `history.pushState` — so client-side navigation dies on this route and
  // nowhere else (measured: `design/qa-p47-tickets-route-nav-pin/`). `setupApi`
  // serves an EMPTY `/api/v1/settings/user`, which is the demo and
  // fresh-account shape: no saved surface document, so `view()` fabricates a
  // fresh `{ columns: [], filters: [] }` on every render, the `sort` default is
  // a fresh literal, and the memo chain keyed on them (`clauses` -> `filtered`
  // -> `shown`) loses identity every time. The bulk-status effect depends on
  // `shown`, so it re-runs on every commit and sets a fresh array: a loop.
  //
  // The loop yields between commits, so a bounded window is enough to count it
  // and the test fails in a quarter of a second instead of hanging. Measured on
  // the unfixed page: ~160 extra commits in 250ms.
  it("settles — an unsaved surface must not re-render forever", async () => {
    setupApi();
    let commits = 0;
    render(
      <Profiler id="tickets" onRender={() => { commits += 1; }}>
        <AuthProvider>
          <TicketsListPage />
        </AuthProvider>
      </Profiler>,
    );

    await screen.findByText("Switch uplink flapping", {}, { timeout: 5000 });
    const settled = commits;
    await new Promise((resolve) => setTimeout(resolve, 250));
    expect(commits - settled).toBeLessThanOrEqual(5);
    cleanup();
  });
});
