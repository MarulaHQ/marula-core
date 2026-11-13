// Integration test: /dashboards — defaults-first presets, RBAC-driven widget
// visibility, widget add/remove persistence, the card size and order controls
// read from `GET /api/report-catalog`, the version 1 migration, the read-only
// fallback when the catalogue cannot be read, and clean empty states. Reports
// run against SDK-typed mock routes mirroring the R1 API shapes.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import DashboardsPage from "../../app/dashboards/page";
import { decodeSpecFromQuery } from "../../lib/report-spec-codec";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/dashboards",
}));

const ME_ID = ME_FIXTURE.user_id;

// recharts measures text by parking one `<span id="recharts_measurement_span">`
// on `document.body`, 20000px offscreen and `aria-hidden`, and leaving the last
// measured string in it. That stub is not part of the page, and which string it
// ends on depends on how far recharts got through layout — under jsdom 26 it
// stopped after a single word ("Corp"), so a bare `getByText("Acme Corp")` found
// only the chart's `<tspan>`; jsdom 31 lets recharts size the whole tick, the
// stub then reads "Acme Corp", and the same query matches two nodes. Excluding
// it is what the query always meant: the page says the client's name, the
// measurement stub is the ruler recharts drew with.
const PAGE_TEXT_QUERY = { ignore: "script, style, #recharts_measurement_span" } as const;

const MANAGER_ME = {
  ...ME_FIXTURE,
  permissions: ["special:manage_users"],
};
const MEMBER_ME = {
  ...ME_FIXTURE,
  permissions: ["tickets:view"],
};

// `GET /api/v1/report-catalog` as lane 1 serves it, trimmed to the members the
// dashboard reads. Values match crates/core-reports SHAPES.
const CATALOG = {
  views: [],
  spans: ["quarter", "third", "half", "full"],
  heights: ["compact", "regular", "tall"],
  shapes: [
    { shape: "chart", viz: ["bar", "line", "pie"], default_span: "half", default_height: "regular" },
    { shape: "stat", viz: ["stat"], default_span: "third", default_height: "compact" },
    { shape: "table", viz: ["table"], default_span: "full", default_height: "tall" },
  ],
  card_rules: ["A card must draw something."],
};

function page(rows: unknown[], summary: Record<string, unknown>, kind: string) {
  return { columns: [], kind, view: "ops.v1_test", rows, summary, next_cursor: null };
}

// A saved report: the card it stores is what the dashboard must draw with.
const DEFINITION_ID = "00000000-0000-4000-8000-00000000a001";
const DEFINITIONS = {
  definitions: [
    {
      id: DEFINITION_ID,
      name: "Weekly breaches",
      description: null,
      spec: {
        view: "tickets",
        filters: [],
        group_by: ["priority"],
        measures: [{ agg: "count" }],
        viz: "table",
      },
      card: {
        size: { span: "full", height: "tall" },
        window_days: 7,
        refresh: { every_minutes: 15, reach_days: 0 },
      },
      scope: "self",
      created_by: ME_ID,
      created_at: "2026-10-01T00:00:00Z",
      updated_at: "2026-11-01T00:00:00Z",
    },
  ],
};

const DEFINITION_RUN = {
  columns: ["priority", "count"],
  kind: "tickets",
  view: "ops.v1_tickets",
  rows: [{ priority: "P1", count: 4 }],
  summary: { count: 4 },
  next_cursor: null,
};

const AUTHORED_CARD = {
  size: { span: "third", height: "compact" },
  window_days: 14,
  refresh: { every_minutes: 15, reach_days: 0 },
};

const AUTHORED = {
  valid: true,
  saved: false,
  draft: {
    name: "Aging P1 tickets",
    spec: {
      view: "tickets",
      filters: [],
      group_by: ["priority"],
      measures: [{ agg: "count" }],
      viz: "bar",
    },
    card: AUTHORED_CARD,
    shape: "chart",
    reads: "A chart of tickets by priority over the last 14 days.",
  },
  issues: [],
  diff: [],
  tool_calls: [],
  usage: { prompt_tokens: 1, completion_tokens: 1, steps: 1 },
  model_info: { provider: "openai", model: "marula-test-chat", resolution: "requested" },
  catalogue: { views: 1, shapes: 3, rules: 3, prompt_chars: 100 },
};

const TIME_SELF = page(
  [{ group_id: ME_ID, label: "Alice", metric: 210, billable_minutes: 150, entry_count: 5 }],
  { entries_count: 5, total_minutes: 210, billable_minutes: 150 },
  "time_by_user",
);
const TIME_ORG = page(
  [
    { group_id: ME_ID, label: "Alice", metric: 210, billable_minutes: 150, entry_count: 5 },
    { group_id: "00000000-0000-4000-8000-0000000000aa", label: "Bob", metric: 120, billable_minutes: 90, entry_count: 4 },
  ],
  { entries_count: 9, total_minutes: 330, billable_minutes: 200 },
  "time_by_user",
);
const TIME_BY_CLIENT = page(
  [{ group_id: "00000000-0000-4000-8000-0000000000c1", label: "Acme Corp", metric: 180, billable_minutes: 180, entry_count: 4 }],
  { entries_count: 4, total_minutes: 180, billable_minutes: 180 },
  "time_by_client",
);
const THROUGHPUT = page(
  [
    { day: "2026-08-25", created_count: 2, resolved_count: 1 },
    { day: "2026-08-26", created_count: 1, resolved_count: 2 },
  ],
  { created_total: 3, resolved_total: 3 },
  "ticket_throughput",
);
const AGING = page(
  [
    { bucket: "under_1d", bucket_ordinal: 0, open_count: 3, oldest_age_days: 0, avg_age_days: 0.5 },
    { bucket: "d1_3", bucket_ordinal: 1, open_count: 2, oldest_age_days: 3, avg_age_days: 2 },
  ],
  { open_total: 5, oldest_age_days: 3, breached_open: 1 },
  "ticket_aging",
);
const FIRST_RESPONSE = page(
  [{ ticket_id: "00000000-0000-4000-8000-00000000000f1", ticket_number: "MAR-7" }],
  { tickets_total: 4, answered_count: 3, unanswered_count: 1, response_p50_secs: 260, response_p90_secs: 900 },
  "first_response",
);
const ATTAINMENT = page(
  [{ group_id: "00000000-0000-4000-8000-0000000000p1", label: "Gold SLA", metric: 10, resolutions_met: 9, attainment_pct: 90 }],
  { resolutions_total: 10, resolutions_met: 9, attainment_pct: 90 },
  "sla_attainment",
);
const BREACHES = page(
  [{ ticket_id: "00000000-0000-4000-8000-0000000000b1", ticket_number: "MAR-42", priority: "P1", policy_name: "Gold SLA", resolution_due: "2026-08-20T17:00:00Z", resolution_met: false }],
  { breaches_total: 1, paused_business_secs_total: 3600 },
  "sla_breaches",
);

function setupApi(
  me: typeof ME_FIXTURE,
  catalog: unknown = CATALOG,
  author: () => { status?: number; json: unknown } = () => ({ json: AUTHORED }),
): MockApi & { setStored: (v: string | null) => void } {
  const mock = new MockApi() as MockApi & { setStored: (v: string | null) => void };
  let stored: string | null = null;
  mock.setStored = (v) => {
    stored = v;
  };
  mock.on("GET", "/api/v1/me", () => ({ json: me }));
  mock.on("GET", "/api/v1/settings/user", () => ({
    json: stored === null ? [] : [{ key: "dashboards.widgets", value: stored }],
  }));
  mock.on("PUT", "/api/v1/settings/user", (ctx) => ({
    json: { key: (ctx.body as { key: string }).key, value: (ctx.body as { value: string }).value },
  }));
  mock.on("GET", "/api/v1/report-catalog", () => ({ json: catalog }));
  mock.on("GET", "/api/v1/report-definitions", () => ({ json: DEFINITIONS }));
  mock.on("GET", `/api/v1/report-definitions/${DEFINITION_ID}/run`, () => ({
    json: DEFINITION_RUN,
  }));
  mock.on("POST", "/api/v1/reports/author", author);
  mock.on("POST", "/api/v1/report-definitions", () => ({
    json: { ...DEFINITIONS.definitions[0], id: DEFINITION_ID, name: AUTHORED.draft.name },
  }));
  mock.on("GET", "/api/v1/plugins/surface/widgets", () => ({ json: { entries: [] } }));
  mock.on("GET", "/api/v1/reports/time_by_user", (ctx) => ({
    json: ctx.query.user_id === ME_ID ? TIME_SELF : TIME_ORG,
  }));
  mock.on("GET", "/api/v1/reports/time_by_client", () => ({ json: TIME_BY_CLIENT }));
  mock.on("GET", "/api/v1/reports/ticket_throughput", () => ({ json: THROUGHPUT }));
  mock.on("GET", "/api/v1/reports/ticket_aging", () => ({ json: AGING }));
  mock.on("GET", "/api/v1/reports/first_response", () => ({ json: FIRST_RESPONSE }));
  mock.on("GET", "/api/v1/reports/sla_attainment", () => ({ json: ATTAINMENT }));
  mock.on("GET", "/api/v1/reports/sla_breaches", () => ({ json: BREACHES }));
  mock.install();
  return mock;
}

/** The default My day arrangement with the sizes lane 1's payload names. */
function myDayPlacements() {
  return [
    { id: "time_by_user:self", size: { span: "half", height: "regular" } },
    { id: "ticket_throughput:self", size: { span: "half", height: "regular" } },
    { id: "ticket_aging:self", size: { span: "half", height: "regular" } },
    { id: "first_response:self", size: { span: "third", height: "compact" } },
  ];
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/dashboards integration", () => {
  it("renders the My day preset by default with report data (manager)", async () => {
    setupApi(MANAGER_ME);
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    expect(await screen.findByText("My time", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.getByText("My throughput")).toBeInTheDocument();
    expect(screen.getByText("My open tickets")).toBeInTheDocument();
    expect(screen.getByText("My first response")).toBeInTheDocument();
    expect(screen.getByText("Org-wide scope")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Apply Team load preset" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Apply SLA health preset" })).toBeInTheDocument();

    // Widget charts rendered from report fixtures.
    expect(await screen.findByText("Alice", {}, { timeout: 5000 })).toBeInTheDocument();
    // Every widget carries the Builder deep link (items 2+3) with a decodable spec.
    const drill = screen.getByTestId("widget-drill-time_by_user:self");
    expect(drill).toHaveAttribute("href", expect.stringContaining("/reports?spec="));
    const specParams = new URLSearchParams(drill.getAttribute("href")!.split("?")[1]!);
    expect(decodeSpecFromQuery(specParams.get("spec"))?.spec.view).toBe("time_entries");
    expect(screen.getByText("3h 30m total · 2h 30m billable")).toBeInTheDocument();
    expect(screen.getByText("4m 20s")).toBeInTheDocument();
    expect(screen.getByText("15m")).toBeInTheDocument();
    cleanup();
  });

  it("draws each card at the span and height the payload defaults to", async () => {
    setupApi(MANAGER_ME);
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );
    const chart = await screen.findByTestId("widget-time_by_user:self", {}, { timeout: 5000 });
    const stat = screen.getByTestId("widget-first_response:self");
    expect(chart).toHaveAttribute("data-card-span", "half");
    expect(chart).toHaveAttribute("data-card-height", "regular");
    // The span is a grid class on the card's cell; the height is on the card.
    expect(chart.parentElement?.className).toContain("xl:col-span-6");
    expect(stat).toHaveAttribute("data-card-span", "third");
    expect(stat).toHaveAttribute("data-card-height", "compact");
    expect(stat.parentElement?.className).toContain("xl:col-span-4");
    cleanup();
  });

  it("offers only the span and height values the payload lists", async () => {
    // A payload without `third`: the control must not invent it, and a widget
    // whose shape default it cannot name keeps working through another shape.
    setupApi(MANAGER_ME, {
      ...CATALOG,
      spans: ["quarter", "half", "full"],
      shapes: [
        CATALOG.shapes[0],
        { ...CATALOG.shapes[1], default_span: "quarter" },
        CATALOG.shapes[2],
      ],
    });
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );
    const card = await screen.findByTestId("widget-time_by_user:self", {}, { timeout: 5000 });
    fireEvent.click(within(card).getByRole("button", { name: "Size and position for My time" }));

    const width = within(card).getByRole("radiogroup", { name: "My time card width" });
    expect(within(width).getAllByRole("radio").map((r) => r.textContent)).toEqual([
      "Quarter",
      "Half",
      "Full",
    ]);
    expect(within(width).queryByRole("radio", { name: "Third" })).not.toBeInTheDocument();

    const height = within(card).getByRole("radiogroup", { name: "My time card height" });
    expect(within(height).getAllByRole("radio").map((r) => r.textContent)).toEqual([
      "Compact",
      "Regular",
      "Tall",
    ]);
    cleanup();
  });

  it("resizes and reorders a card from the keyboard-reachable controls", async () => {
    const mock = setupApi(MANAGER_ME);
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );
    const card = await screen.findByTestId("widget-time_by_user:self", {}, { timeout: 5000 });

    fireEvent.click(within(card).getByRole("button", { name: "Size and position for My time" }));
    expect(within(card).getByTestId("card-size-time_by_user:self")).toBeInTheDocument();

    fireEvent.click(within(card).getByRole("radio", { name: "Full" }));
    await waitFor(() => {
      expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(1);
    });
    const resized = mock.lastBody<{ key: string; value: string }>("PUT", "/api/v1/settings/user");
    expect(resized.key).toBe("dashboards.widgets");
    const resizedPayload = JSON.parse(resized.value) as {
      version: number;
      widgets: Array<{ id: string; size: { span: string; height: string } }>;
    };
    expect(resizedPayload.version).toBe(2);
    expect(resizedPayload.widgets[0]).toEqual({
      id: "time_by_user:self",
      size: { span: "full", height: "regular" },
    });
    // The other cards kept their size.
    expect(resizedPayload.widgets[1]).toEqual(myDayPlacements()[1]);

    // Move it one slot later.
    fireEvent.click(within(card).getByRole("button", { name: "Move My time later" }));
    await waitFor(() => {
      expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(2);
    });
    const moved = JSON.parse(
      mock.lastBody<{ value: string }>("PUT", "/api/v1/settings/user").value,
    ) as { widgets: Array<{ id: string }> };
    expect(moved.widgets.map((w) => w.id)).toEqual([
      "ticket_throughput:self",
      "time_by_user:self",
      "ticket_aging:self",
      "first_response:self",
    ]);
    cleanup();
  });

  it("keeps personal-lens queries pinned and org queries unscoped", async () => {
    const mock = setupApi(MANAGER_ME);
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );
    await screen.findByText("My time", {}, { timeout: 5000 });
    await waitFor(() => {
      expect(mock.callsTo("GET", "/api/v1/reports/time_by_user").length).toBeGreaterThan(0);
    });
    for (const call of mock.callsTo("GET", "/api/v1/reports/time_by_user")) {
      expect(call.query.user_id).toBe(ME_ID);
    }
    cleanup();
  });

  it("hides org widgets and org presets from non-managers", async () => {
    const mock = setupApi(MEMBER_ME);
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    expect(await screen.findByText("Your data only", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.queryByText("Org-wide scope")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Apply Team load preset" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Apply SLA health preset" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Add widget" }));
    expect(screen.queryByRole("button", { name: "Add Time by client" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add SLA breaches" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add My SLA attainment" })).toBeInTheDocument();
    // Personal-lens widgets still pin self on the wire.
    await waitFor(() => {
      expect(mock.callsTo("GET", "/api/v1/reports/time_by_user").length).toBeGreaterThan(0);
    });
    expect(mock.callsTo("GET", "/api/v1/reports/time_by_user")[0]?.query.user_id).toBe(ME_ID);
    cleanup();
  });

  it("adds and removes a widget with a settings-registry round-trip", async () => {
    const mock = setupApi(MANAGER_ME);
    const { unmount } = render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );
    await screen.findByText("My time", {}, { timeout: 5000 });

    // Add an org widget through the picker; it arrives at its shape default.
    fireEvent.click(screen.getByRole("button", { name: "Add widget" }));
    expect(screen.getByRole("button", { name: "Add Time by client" })).toHaveTextContent(
      "Half wide, Regular",
    );
    fireEvent.click(screen.getByRole("button", { name: "Add Time by client" }));
    await waitFor(() => {
      expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(1);
    });
    const added = mock.lastBody<{ key: string; value: string }>("PUT", "/api/v1/settings/user");
    expect(added.key).toBe("dashboards.widgets");
    expect(JSON.parse(added.value)).toEqual({
      version: 2,
      widgets: [
        ...myDayPlacements(),
        { id: "time_by_client:org", size: { span: "half", height: "regular" } },
      ],
    });
    expect(await screen.findByText("Acme Corp", PAGE_TEXT_QUERY, { timeout: 5000 })).toBeInTheDocument();
    // Org-lens fetches carry no user_id narrowing.
    expect(mock.callsTo("GET", "/api/v1/reports/time_by_client")[0]?.query.user_id).toBeUndefined();

    // Remove it again.
    fireEvent.click(screen.getByRole("button", { name: "Remove Time by client" }));
    await waitFor(() => {
      expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(2);
    });
    const removed = mock.lastBody<{ value: string }>("PUT", "/api/v1/settings/user");
    expect(JSON.parse(removed.value).widgets).not.toContainEqual(
      expect.objectContaining({ id: "time_by_client:org" }),
    );

    // Round-trip: a fresh mount restores the stored arrangement.
    mock.setStored(removed.value);
    unmount();
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );
    expect(await screen.findByText("My time", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.queryByText("Time by client")).not.toBeInTheDocument();
    expect(screen.queryByText("Acme Corp", PAGE_TEXT_QUERY)).not.toBeInTheDocument();
    cleanup();
  });

  it("migrates the shipped version 1 arrangement and keeps its order", async () => {
    const mock = setupApi(MANAGER_ME);
    mock.setStored(
      JSON.stringify({ version: 1, widgets: ["sla_breaches:org", "time_by_user:self"] }),
    );
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    expect(await screen.findByText("SLA breaches", {}, { timeout: 5000 })).toBeInTheDocument();
    const grid = screen.getByTestId("dashboard-grid");
    const cards = Array.from(grid.querySelectorAll("[data-widget-id]"));
    expect(cards.map((c) => c.getAttribute("data-widget-id"))).toEqual([
      "sla_breaches:org",
      "time_by_user:self",
    ]);
    expect(cards[0]).toHaveAttribute("data-card-span", "full");
    cleanup();
  });

  it("falls back to the default arrangement when a stored size no longer decodes", async () => {
    const mock = setupApi(MANAGER_ME);
    mock.setStored(
      JSON.stringify({
        version: 2,
        widgets: [{ id: "time_by_user:self", size: { span: "huge", height: "regular" } }],
      }),
    );
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    expect(await screen.findByText("My time", {}, { timeout: 5000 })).toBeInTheDocument();
    // The whole My day arrangement, at its defaults, and nothing written back.
    expect(screen.getByText("My first response")).toBeInTheDocument();
    expect(screen.getByTestId("widget-time_by_user:self")).toHaveAttribute("data-card-span", "half");
    expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(0);
    cleanup();
  });

  it("treats a failed settings read as unknown, not as no arrangement", async () => {
    const mock = setupApi(MANAGER_ME);
    // The registry read fails while the catalogue is fine. The page knows the
    // size vocabulary, so it could render and write — and that write would
    // replace an arrangement it never saw.
    mock.on("GET", "/api/v1/settings/user", () => ({
      status: 500,
      json: { error: { message: "settings registry unavailable" } },
    }));
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    const notice = await screen.findByTestId("dashboard-readonly", {}, { timeout: 5000 });
    expect(notice).toHaveAttribute("data-reason", "arrangement");
    expect(notice).toHaveTextContent(/saved arrangement could not be loaded/i);
    expect(notice).toHaveTextContent(/nothing has been overwritten/i);

    // Nothing is drawn: some other arrangement under this notice would say one
    // thing and show another. Every mutation surface is absent too.
    expect(screen.queryByTestId("dashboard-grid")).not.toBeInTheDocument();
    expect(screen.queryAllByTestId(/^widget-/)).toHaveLength(0);
    expect(screen.queryByRole("tab", { name: "Apply Team load preset" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Add widget" }));
    expect(screen.getByText(/until your saved arrangement loads/i)).toBeInTheDocument();

    expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(0);
    cleanup();
  });

  it("treats the registry's default document as unset, not as corrupt", async () => {
    // The registry serves every declared user setting with its default, so a
    // fresh user now reads `dashboards.widgets: "{}"` instead of no row at
    // all. That is "nothing written yet": the defaults are the arrangement,
    // and the first change must be able to save.
    const mock = setupApi(MANAGER_ME);
    mock.setStored("{}");
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    const card = await screen.findByTestId("widget-time_by_user:self", {}, { timeout: 5000 });
    expect(screen.queryByTestId("dashboard-readonly")).not.toBeInTheDocument();
    expect(screen.getByTestId("dashboard-grid")).toBeInTheDocument();

    fireEvent.click(within(card).getByRole("button", { name: "Size and position for My time" }));
    fireEvent.click(within(card).getByRole("radio", { name: "Tall" }));
    await waitFor(() => {
      expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(1);
    });
    const saved = mock.lastBody<{ value: string }>("PUT", "/api/v1/settings/user");
    expect(JSON.parse(saved.value).version).toBe(2);
    cleanup();
  });

  it("leaves a stored arrangement this client cannot read alone", async () => {
    const mock = setupApi(MANAGER_ME);
    // A newer client's payload version: readable by nobody here, and the page
    // must not replace it with defaults.
    mock.setStored(JSON.stringify({ version: 3, widgets: [] }));
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    const notice = await screen.findByTestId("dashboard-readonly", {}, { timeout: 5000 });
    expect(notice).toHaveAttribute("data-reason", "arrangement");
    expect(notice).toHaveTextContent(/nothing is shown/i);
    expect(screen.queryByTestId("dashboard-grid")).not.toBeInTheDocument();
    expect(screen.queryAllByTestId(/^widget-/)).toHaveLength(0);
    expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(0);
    cleanup();
  });

  it("renders the default arrangement when nothing is stored", async () => {
    const mock = setupApi(MANAGER_ME);
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    expect(await screen.findByTestId("dashboard-grid", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.getByText("My first response")).toBeInTheDocument();
    const card = screen.getByTestId("widget-time_by_user:self");
    // A stored row is absent, so the page is fully live: size, remove, presets.
    expect(within(card).getByRole("button", { name: "Size and position for My time" })).toBeInTheDocument();
    expect(within(card).getByRole("button", { name: "Remove My time" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Apply Team load preset" })).toBeInTheDocument();
    expect(screen.queryByTestId("dashboard-readonly")).not.toBeInTheDocument();
    expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(0);
    cleanup();
  });

  it("recovers the stored arrangement when a retry succeeds", async () => {
    const mock = setupApi(MANAGER_ME);
    const arrangement = JSON.stringify({
      version: 2,
      widgets: [
        { id: "sla_breaches:org", size: { span: "full", height: "tall" } },
        { id: "time_by_user:self", size: { span: "quarter", height: "compact" } },
      ],
    });
    let reads = 0;
    mock.on("GET", "/api/v1/settings/user", () => {
      reads += 1;
      if (reads === 1) {
        return { status: 500, json: { error: { message: "settings registry unavailable" } } };
      }
      return { json: [{ key: "dashboards.widgets", value: arrangement }] };
    });
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    expect(await screen.findByTestId("dashboard-readonly", {}, { timeout: 5000 })).toHaveAttribute(
      "data-reason",
      "arrangement",
    );
    expect(screen.queryAllByTestId(/^widget-/)).toHaveLength(0);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));

    await waitFor(() => {
      expect(screen.getByTestId("dashboard-grid")).toBeInTheDocument();
    });
    expect(screen.queryByTestId("dashboard-readonly")).not.toBeInTheDocument();
    expect(screen.getByTestId("widget-sla_breaches:org")).toHaveAttribute("data-card-span", "full");
    expect(screen.getByTestId("widget-time_by_user:self")).toHaveAttribute("data-card-span", "quarter");
    expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(0);
    cleanup();
  });

  it("renders read-only with a notice when the catalogue cannot be read", async () => {
    const mock = setupApi(MANAGER_ME);
    mock.on("GET", "/api/v1/report-catalog", () => ({
      status: 500,
      json: { error: { message: "catalogue unavailable" } },
    }));
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    const notice = await screen.findByTestId("dashboard-readonly", {}, { timeout: 5000 });
    expect(notice).toHaveAttribute("data-reason", "catalogue");
    expect(notice).toHaveTextContent(/nothing is shown/i);
    // No card is drawn, so no size is claimed and no control can write.
    expect(screen.queryAllByTestId(/^widget-/)).toHaveLength(0);
    fireEvent.click(screen.getByRole("button", { name: "Add widget" }));
    expect(screen.getByText(/No widget can be added until the card vocabulary loads/i)).toBeInTheDocument();
    expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(0);
    cleanup();
  });

  it("keeps plugin widgets reachable beside the report grid", async () => {
    const mock = setupApi(MANAGER_ME);
    mock.on("GET", "/api/v1/plugins/surface/widgets", () => ({
      json: {
        entries: [
          {
            install_id: "00000000-0000-4000-8000-0000000000aa",
            entry_key: "acme.hello",
            label: "Acme hello",
            definition: {
              kind: "stat",
              slot: "dashboard",
              props_schema: { type: "object", properties: { value: { type: "string" } } },
              default_props: { value: "42" },
            },
          },
        ],
      },
    }));
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    expect(await screen.findByText("Acme hello", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.getByTestId("plugin-widgets-dashboard")).toBeInTheDocument();
    // The sized report grid is still there beside it.
    expect(screen.getByTestId("dashboard-grid")).toBeInTheDocument();
    cleanup();
  });

  it("shows clean empty states when reports have no data yet", async () => {
    const mock = setupApi(MEMBER_ME);
    mock.on("GET", "/api/v1/reports/time_by_user", () => ({ json: page([], {}, "time_by_user") }));
    mock.on("GET", "/api/v1/reports/ticket_throughput", () => ({ json: page([], {}, "ticket_throughput") }));
    mock.on("GET", "/api/v1/reports/ticket_aging", () => ({ json: page([], {}, "ticket_aging") }));
    mock.on("GET", "/api/v1/reports/first_response", () => ({ json: page([], {}, "first_response") }));
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    expect(
      await screen.findByText("No time logged in the last 30 days.", {}, { timeout: 5000 }),
    ).toBeInTheDocument();
    expect(screen.getByText("No tickets created or resolved in the last 30 days.")).toBeInTheDocument();
    expect(screen.getByText("No open tickets assigned to you.")).toBeInTheDocument();
    expect(screen.getByText("No tickets assigned to you in the last 30 days.")).toBeInTheDocument();
    cleanup();
  });

  it("surfaces a save failure in blush instead of swallowing it", async () => {
    const mock = setupApi(MANAGER_ME);
    // The registry refuses the write; the arrangement stays on screen and the
    // user is told, in words, rather than watching a silent drop.
    mock.on("PUT", "/api/v1/settings/user", () => ({
      status: 409,
      json: { error: { message: "settings registry is read-only for this account" } },
    }));
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );
    await screen.findByText("My time", {}, { timeout: 5000 });

    fireEvent.click(screen.getByRole("button", { name: "Add widget" }));
    fireEvent.click(screen.getByRole("button", { name: "Add Time by client" }));

    const notice = await screen.findByTestId("dashboard-save-error", {}, { timeout: 5000 });
    expect(notice).toHaveTextContent(/did not save/i);
    // The status danger pair, never `--m-destructive`: a failed save is an
    // error message, and destructive red stays for things that destroy
    // (docs/theme.md §3.4).
    //
    // Read the declaration, not the computed value. jsdom 26 handed `var(...)`
    // straight back from getComputedStyle; jsdom 31 substitutes it, and with no
    // theme custom properties defined in a test document the substitution falls
    // back to the initial values (`color: rgb(0, 0, 0)`), so `toHaveStyle` sees
    // black and fails on a page that is painted correctly. The inline style is
    // the statement the component makes, and it is where the token lives.
    expect(notice.style.background).toBe("var(--m-status-danger-bg)");
    expect(notice.style.color).toBe("var(--m-status-danger-fg)");
    // The arrangement the user just built is still on screen.
    expect(await screen.findByText("Acme Corp", PAGE_TEXT_QUERY, { timeout: 5000 })).toBeInTheDocument();
    cleanup();
  });

  it("offers restore-from-empty through the My day preset", async () => {
    const mock = setupApi(MANAGER_ME);
    mock.setStored(JSON.stringify({ version: 2, widgets: [] }));
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    expect(await screen.findByText("Your dashboard is empty", {}, { timeout: 5000 })).toBeInTheDocument();
    // An empty arrangement is valid, not a failure: the picker stays live.
    fireEvent.click(screen.getByRole("button", { name: "Add widget" }));
    expect(screen.getByRole("button", { name: "Add Time by client" })).toBeInTheDocument();
    expect(screen.queryByTestId("dashboard-readonly")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Restore My day" }));
    await waitFor(() => {
      expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(1);
    });
    const body = mock.lastBody<{ key: string; value: string }>("PUT", "/api/v1/settings/user");
    expect(body.key).toBe("dashboards.widgets");
    expect(JSON.parse(body.value).widgets).toEqual(myDayPlacements());
    cleanup();
  });
});

// --- Wave 46 lane 6: saved report cards, the describe entry, the gates -------

function storedWidgets(mock: MockApi): Array<{ id: string; size: { span: string; height: string } }> {
  const call = mock.callsTo("PUT", "/api/v1/settings/user").at(-1);
  if (call === undefined) throw new Error("no arrangement was written");
  return (JSON.parse((call.body as { value: string }).value) as {
    widgets: Array<{ id: string; size: { span: string; height: string } }>;
  }).widgets;
}

describe("/dashboards saved reports and describe entry (Wave 46 lane 6)", () => {
  it("places a saved report at its stored card's size and prints that card", async () => {
    const mock = setupApi(MANAGER_ME);
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );
    await screen.findByText("My time", {}, { timeout: 5000 });

    fireEvent.click(screen.getByRole("button", { name: "Add widget" }));
    // The picker shows the stored card before the card is placed.
    const candidate = await screen.findByTestId(
      `definition-candidate-${DEFINITION_ID}`,
      {},
      { timeout: 5000 },
    );
    expect(within(candidate).getByTestId(`definition-candidate-meta-${DEFINITION_ID}`)).toHaveAttribute(
      "data-card-window-days",
      "7",
    );

    fireEvent.click(
      screen.getByRole("button", { name: "Add Weekly breaches to the dashboard" }),
    );
    await waitFor(() =>
      expect(mock.callsTo("PUT", "/api/v1/settings/user").length).toBe(1),
    );

    // The placement stores the definition's own size, not a shape default.
    const widgets = storedWidgets(mock);
    expect(widgets.at(-1)).toEqual({
      id: `definition:${DEFINITION_ID}`,
      size: { span: "full", height: "tall" },
    });

    // The card runs the definition and prints the window, refresh and size.
    const card = await screen.findByTestId(
      `definition-widget-${DEFINITION_ID}`,
      {},
      { timeout: 5000 },
    );
    expect(card).toHaveAttribute("data-card-span", "full");
    expect(card).toHaveAttribute("data-card-height", "tall");
    const meta = within(card).getByTestId(`definition-card-meta-${DEFINITION_ID}`);
    expect(meta).toHaveAttribute("data-card-refresh-minutes", "15");
    expect(meta).toHaveTextContent("Last 7 days");
    expect(meta).toHaveTextContent("every 15 min");
    expect(meta).toHaveTextContent("Full wide, Tall tall");
    expect(mock.callsTo("GET", `/api/v1/report-definitions/${DEFINITION_ID}/run`)).toHaveLength(1);
    expect(await within(card).findByText("P1")).toBeInTheDocument();

    // A reload reads the placement back and draws it again: the load filter
    // must not treat a `definition:` id as an unknown widget.
    mock.setStored(mock.lastBody<{ value: string }>("PUT", "/api/v1/settings/user").value);
    cleanup();
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );
    const reloaded = await screen.findByTestId(
      `definition-widget-${DEFINITION_ID}`,
      {},
      { timeout: 5000 },
    );
    expect(reloaded).toHaveAttribute("data-card-span", "full");
    cleanup();
  });

  it("refuses to place a saved report whose stored size the catalogue lost", async () => {
    setupApi(MANAGER_ME, {
      ...CATALOG,
      heights: ["compact", "regular"],
      shapes: [
        CATALOG.shapes[0],
        { ...CATALOG.shapes[1], default_height: "regular" },
        { ...CATALOG.shapes[2], default_height: "regular", default_span: "full" },
      ],
    });
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );
    await screen.findByText("My time", {}, { timeout: 5000 });
    fireEvent.click(screen.getByRole("button", { name: "Add widget" }));

    const add = await screen.findByRole(
      "button",
      { name: "Add Weekly breaches to the dashboard" },
      { timeout: 5000 },
    );
    // The stored card says tall; the payload no longer offers tall. Nothing is
    // stored rather than silently drawn at another height.
    expect(add).toBeDisabled();
    expect(add).toHaveTextContent(/stored size is not in this catalogue/i);
    cleanup();
  });

  it("describes a card, saves through the definition route, and places the accepted card", async () => {
    const mock = setupApi(MANAGER_ME);
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );
    await screen.findByText("My time", {}, { timeout: 5000 });

    fireEvent.click(screen.getByTestId("dashboard-describe-entry"));
    const box = await screen.findByLabelText("What should this report answer?");
    fireEvent.change(box, { target: { value: "Aging P1 tickets over two weeks." } });
    fireEvent.click(screen.getByRole("button", { name: "Generate a draft" }));

    const row = await screen.findByTestId("report-draft-row", {}, { timeout: 5000 });
    await waitFor(() => expect(row).toHaveAttribute("data-status", "ready"));
    expect(mock.callsTo("POST", "/api/v1/reports/author")).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: /Accept and save/ }));
    await waitFor(() =>
      expect(mock.callsTo("POST", "/api/v1/report-definitions")).toHaveLength(1),
    );
    const saved = mock.lastBody<{ card: { window_days: number } }>(
      "POST",
      "/api/v1/report-definitions",
    );
    expect(saved.card.window_days).toBe(14);

    // The accepted card is placed at the size it was accepted with.
    await waitFor(() =>
      expect(mock.callsTo("PUT", "/api/v1/settings/user").length).toBe(1),
    );
    expect(storedWidgets(mock).at(-1)).toEqual({
      id: `definition:${DEFINITION_ID}`,
      size: { span: "third", height: "compact" },
    });
    expect(await screen.findByTestId("dashboard-authored-note")).toHaveTextContent(
      /on your dashboard with the card you accepted/i,
    );
    cleanup();
  });

  it("states why AI cannot draft, with the remedy, on the dashboard entry", async () => {
    setupApi(MANAGER_ME, CATALOG, () => ({
      status: 403,
      json: {
        error: {
          reason: "ai_no_credential",
          message: "no active 'openai' credential is available for this call (org or personal)",
        },
      },
    }));
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );
    await screen.findByText("My time", {}, { timeout: 5000 });

    fireEvent.click(screen.getByTestId("dashboard-describe-entry"));
    const box = await screen.findByLabelText("What should this report answer?");
    fireEvent.change(box, { target: { value: "anything" } });
    fireEvent.click(screen.getByRole("button", { name: "Generate a draft" }));

    const notice = await screen.findByTestId("ai-gate-notice", {}, { timeout: 5000 });
    expect(notice).toHaveAttribute("data-reason", "ai_no_credential");
    expect(notice).toHaveAttribute("data-surface", "dashboards");
    expect(notice).toHaveTextContent("No model credential is available");
    expect(notice).toHaveTextContent(/provider key under Settings, AI, credentials/i);
    // Nothing was saved and nothing was placed.
    expect(screen.queryByRole("button", { name: /Accept and save/ })).toBeDisabled();
    cleanup();
  });
});

// --- Wave 46 lane 6 follow-up: an unavailable card is a stated state --------

const GONE_ID = "00000000-0000-4000-8000-00000000dead";

/** A stored arrangement holding one placement whose definition is not listed. */
function storedWithGoneDefinition() {
  return JSON.stringify({
    version: 2,
    widgets: [
      { id: `definition:${GONE_ID}`, size: { span: "full", height: "tall" } },
      { id: "time_by_user:self", size: { span: "half", height: "regular" } },
    ],
  });
}

describe("/dashboards unavailable saved reports (Wave 46 lane 6 follow-up)", () => {
  it("draws a placeholder at the stored span when the saved report is gone", async () => {
    const mock = setupApi(MANAGER_ME);
    mock.setStored(storedWithGoneDefinition());
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    const tile = await screen.findByTestId(
      `definition-unavailable-${GONE_ID}`,
      {},
      { timeout: 5000 },
    );
    // It occupies the stored footprint, so the grid does not shift.
    expect(tile).toHaveAttribute("data-card-span", "full");
    expect(tile).toHaveAttribute("data-card-height", "tall");
    expect(tile.parentElement?.className).toContain("col-span-12");
    expect(tile).toHaveTextContent(/Saved report unavailable/i);
    expect(tile).toHaveTextContent(/not available to you/i);
    expect(tile).toHaveTextContent(/deleted or is outside your scope/i);

    // The catalogue card beside it still draws, and nothing was run for the
    // definition the list does not resolve.
    expect(await screen.findByText("My time", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(mock.callsTo("GET", `/api/v1/report-definitions/${GONE_ID}/run`)).toHaveLength(0);
    cleanup();
  });

  it("removes the placeholder through the same arrangement write", async () => {
    const mock = setupApi(MANAGER_ME);
    mock.setStored(storedWithGoneDefinition());
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    const tile = await screen.findByTestId(
      `definition-unavailable-${GONE_ID}`,
      {},
      { timeout: 5000 },
    );
    fireEvent.click(
      within(tile).getByRole("button", { name: "Remove the unavailable saved report card" }),
    );

    await waitFor(() =>
      expect(mock.callsTo("PUT", "/api/v1/settings/user").length).toBe(1),
    );
    expect(storedWidgets(mock).map((w) => w.id)).toEqual(["time_by_user:self"]);
    expect(screen.queryByTestId(`definition-unavailable-${GONE_ID}`)).not.toBeInTheDocument();
    cleanup();
  });

  it("states a failed saved-report read above the grid without blocking the catalogue", async () => {
    const mock = setupApi(MANAGER_ME);
    mock.on("GET", "/api/v1/report-definitions", () => ({
      status: 500,
      json: { error: { reason: "internal_error", message: "boom" } },
    }));
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    const note = await screen.findByTestId(
      "dashboard-definitions-error",
      {},
      { timeout: 5000 },
    );
    expect(note).toHaveTextContent(/saved reports could not be loaded/i);
    // The catalogue's own widgets are unaffected.
    expect(await screen.findByText("My time", {}, { timeout: 5000 })).toBeInTheDocument();

    fireEvent.click(within(note).getByRole("button", { name: "Retry" }));
    await waitFor(() =>
      expect(mock.callsTo("GET", "/api/v1/report-definitions").length).toBe(2),
    );
    cleanup();
  });

  it("does not claim deletion when the saved-report list is merely unread", async () => {
    const mock = setupApi(MANAGER_ME);
    mock.setStored(storedWithGoneDefinition());
    mock.on("GET", "/api/v1/report-definitions", () => ({
      status: 500,
      json: { error: { reason: "internal_error", message: "boom" } },
    }));
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    const tile = await screen.findByTestId(
      `definition-unavailable-${GONE_ID}`,
      {},
      { timeout: 5000 },
    );
    expect(tile).toHaveTextContent(/could not be checked/i);
    expect(tile).not.toHaveTextContent(/deleted or is outside your scope/i);
    cleanup();
  });
});
