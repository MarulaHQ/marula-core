// Integration test: /dashboards — defaults-first presets, RBAC-driven
// widget visibility, widget add/remove persistence round-trip through the
// settings-registry machinery, and clean empty states. Reports run against
// SDK-typed mock routes mirroring the R1 API shapes.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import DashboardsPage from "../../app/dashboards/page";
import { decodeSpecFromQuery } from "../../lib/report-spec-codec";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/dashboards",
}));

const ME_ID = ME_FIXTURE.user_id;

const MANAGER_ME = {
  ...ME_FIXTURE,
  permissions: ["special:manage_users"],
};
const MEMBER_ME = {
  ...ME_FIXTURE,
  permissions: ["tickets:view"],
};

function page(rows: unknown[], summary: Record<string, unknown>, kind: string) {
  return { columns: [], kind, view: "ops.v1_test", rows, summary, next_cursor: null };
}

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
  [{ ticket_id: "00000000-0000-4000-8000-00000000000f1", ticket_number: "MER-7" }],
  { tickets_total: 4, answered_count: 3, unanswered_count: 1, response_p50_secs: 260, response_p90_secs: 900 },
  "first_response",
);
const ATTAINMENT = page(
  [{ group_id: "00000000-0000-4000-8000-0000000000p1", label: "Gold SLA", metric: 10, resolutions_met: 9, attainment_pct: 90 }],
  { resolutions_total: 10, resolutions_met: 9, attainment_pct: 90 },
  "sla_attainment",
);
const BREACHES = page(
  [{ ticket_id: "00000000-0000-4000-8000-0000000000b1", ticket_number: "MER-42", priority: "P1", policy_name: "Gold SLA", resolution_due: "2026-08-20T17:00:00Z", resolution_met: false }],
  { breaches_total: 1, paused_business_secs_total: 3600 },
  "sla_breaches",
);

function setupApi(me: typeof ME_FIXTURE): MockApi & { setStored: (v: string | null) => void } {
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
    expect(screen.getByRole("button", { name: "Apply Team load preset" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Apply SLA health preset" })).toBeInTheDocument();

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
    expect(screen.queryByRole("button", { name: "Time by client" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "SLA breaches" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "My SLA attainment" })).toBeInTheDocument();
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

    // Add an org widget through the picker.
    fireEvent.click(screen.getByRole("button", { name: "Add widget" }));
    fireEvent.click(screen.getByRole("button", { name: "Time by client" }));
    await waitFor(() => {
      expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(1);
    });
    const added = mock.lastBody<{ key: string; value: string }>("PUT", "/api/v1/settings/user");
    expect(added.key).toBe("dashboards.widgets");
    expect(JSON.parse(added.value)).toEqual({
      version: 1,
      widgets: [
        "time_by_user:self",
        "ticket_throughput:self",
        "ticket_aging:self",
        "first_response:self",
        "time_by_client:org",
      ],
    });
    expect(await screen.findByText("Acme Corp", {}, { timeout: 5000 })).toBeInTheDocument();
    // Org-lens fetches carry no user_id narrowing.
    expect(mock.callsTo("GET", "/api/v1/reports/time_by_client")[0]?.query.user_id).toBeUndefined();

    // Remove it again.
    fireEvent.click(screen.getByRole("button", { name: "Remove Time by client" }));
    await waitFor(() => {
      expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(2);
    });
    const removed = mock.lastBody<{ value: string }>("PUT", "/api/v1/settings/user");
    expect(JSON.parse(removed.value).widgets).not.toContain("time_by_client:org");

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
    expect(screen.queryByText("Acme Corp")).not.toBeInTheDocument();
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

  it("offers restore-from-empty through the My day preset", async () => {
    const mock = setupApi(MANAGER_ME);
    mock.setStored(JSON.stringify({ version: 1, widgets: [] }));
    render(
      <AuthProvider>
        <DashboardsPage />
      </AuthProvider>,
    );

    expect(await screen.findByText("Your dashboard is empty.", {}, { timeout: 5000 })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Restore My day" }));
    await waitFor(() => {
      expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(1);
    });
    const body = mock.lastBody<{ key: string; value: string }>("PUT", "/api/v1/settings/user");
    expect(body.key).toBe("dashboards.widgets");
    expect(JSON.parse(body.value).widgets).toEqual([
      "time_by_user:self",
      "ticket_throughput:self",
      "ticket_aging:self",
      "first_response:self",
    ]);
    cleanup();
  });
});
