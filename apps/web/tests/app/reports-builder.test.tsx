// Integration test: /reports — R6 builder v1. Catalog-driven config, live
// preview through POST /api/v1/reports/run-spec, save through the
// instant-first definition envelope, run/publish of saved reports through
// GET /api/v1/report-definitions/{id}/run, and friendly failure copy.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import ReportsPage from "../../app/reports/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";
import { encodeSpecToQuery } from "../../lib/report-spec-codec";

vi.mock("next/navigation", () => {
  let lastSearch: string | null = null;
  let lastParams: URLSearchParams | null = null;
  return {
    useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
    usePathname: () => "/reports",
    useSearchParams: () => {
      const s = window.location.search;
      if (lastSearch !== s) {
        lastSearch = s;
        lastParams = new URLSearchParams(s);
      }
      return lastParams;
    },
  };
});

const DEF_ID = "00000000-0000-4000-8000-00000000d001";

const CATALOG = {
  views: [
    {
      view: "tickets" as const,
      columns: [
        { name: "org_id", value_type: "uuid" },
        { name: "number", value_type: "text" },
        { name: "client_name", value_type: "text" },
        { name: "priority", value_type: "text" },
        { name: "created_at", value_type: "timestamp" },
        { name: "breached", value_type: "bool" },
      ],
      filterable: [
        { name: "priority", value_type: "text" },
        { name: "client_id", value_type: "uuid" },
        { name: "breached", value_type: "bool" },
      ],
    },
    {
      view: "time_entries" as const,
      columns: [
        { name: "org_id", value_type: "uuid" },
        { name: "user_display_name", value_type: "text" },
        { name: "minutes", value_type: "int" },
        { name: "billable", value_type: "bool" },
      ],
      filterable: [
        { name: "billable", value_type: "bool" },
        { name: "user_id", value_type: "uuid" },
      ],
    },
  ],
};

const RUN_PAGE = {
  kind: "tickets",
  view: "ops.v1_tickets",
  columns: ["priority", "count"],
  rows: [
    { priority: "P1", count: 12 },
    { priority: "P2", count: 30 },
  ],
  summary: { count: 42 },
  next_cursor: null,
};

const DEFINITIONS = {
  definitions: [
    {
      id: DEF_ID,
      name: "Tickets by priority",
      description: null,
      spec: {
        view: "tickets",
        filters: [],
        group_by: ["priority"],
        measures: [{ agg: "count" }],
        viz: "bar",
      },
      scope: "self",
      created_by: ME_FIXTURE.user_id,
      created_at: "2026-09-01T00:00:00Z",
      updated_at: "2026-09-12T00:00:00Z",
    },
  ],
};

const SCHEDULES = { schedules: [], next_cursor: null };

function setupApi(me = ME_FIXTURE, runStatus = 200) {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: me }));
  mock.on("GET", "/api/v1/report-catalog", () => ({ json: CATALOG }));
  mock.on("GET", "/api/v1/report-definitions", () => ({ json: DEFINITIONS }));
  mock.on("GET", "/api/v1/report-schedules", () => ({ json: SCHEDULES }));
  mock.on("POST", "/api/v1/reports/run-spec", () => ({ json: RUN_PAGE }));
  mock.on("GET", `/api/v1/report-definitions/${DEF_ID}/run`, () =>
    runStatus === 200 ? { json: RUN_PAGE } : { status: runStatus, json: { error: { reason: "forbidden", message: "no" } } },
  );
  mock.on("POST", "/api/v1/report-definitions", (ctx) => ({
    json: { ...(DEFINITIONS.definitions[0] as object), id: DEF_ID, name: (ctx.body as { name: string }).name },
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  cleanup();
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/reports builder integration", () => {
  it("builds a spec from the catalog and previews through run-spec", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );

    // Builder tab is the default surface.
    expect(await screen.findByRole("tab", { name: "Builder" }, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.getByLabelText("Report data view")).toBeInTheDocument();

    // Wait for the catalog to load (grouping adds are disabled before it).
    const addGrouping = await screen.findByRole("button", { name: "Add grouping" }, { timeout: 5000 });
    await waitFor(() => expect(addGrouping).not.toBeDisabled());
    fireEvent.click(addGrouping);
    fireEvent.click(screen.getByRole("button", { name: "Run preview" }));

    await waitFor(() => expect(mock.callsTo("POST", "/api/v1/reports/run-spec").length).toBe(1));
    const body = mock.lastBody<{ spec: { group_by: string[]; measures: { agg: string }[]; viz: string } }>(
      "POST",
      "/api/v1/reports/run-spec",
    );
    expect(body.spec.group_by).toEqual(["number"]);
    expect(body.spec.measures).toEqual([{ agg: "count" }]);
    expect(body.spec.viz).toBe("bar");

    // Preview renders rows + summary from the compiled page.
    expect(await screen.findByText("P1", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.getByText("42")).toBeInTheDocument();
    cleanup();
  });

  it("blocks the preview while a filter operand is invalid, and explains why", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );

    const addFilter = await screen.findByRole("button", { name: "Add filter" }, { timeout: 5000 });
    await waitFor(() => expect(addFilter).not.toBeDisabled());
    fireEvent.click(addFilter);
    // Default operand (priority, text) is empty → operator takes a value.
    expect(screen.getByRole("status")).toHaveTextContent(/enter a valid/i);
    expect(screen.getByRole("button", { name: "Run preview" })).toBeDisabled();
    expect(mock.callsTo("POST", "/api/v1/reports/run-spec").length).toBe(0);
    cleanup();
  });

  it("saves a report through the instant-first envelope with an idempotency key", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );

    // Wait for the catalog before opening the save dialog (save is gated
    // on a sendable spec).
    const addGrouping = await screen.findByRole("button", { name: "Add grouping" }, { timeout: 5000 });
    await waitFor(() => expect(addGrouping).not.toBeDisabled());
    fireEvent.click(await screen.findByRole("button", { name: "Save report" }, { timeout: 5000 }));
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "Open tickets by client" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(mock.callsTo("POST", "/api/v1/report-definitions").length).toBe(1));
    const body = mock.lastBody<{ name: string; idempotency_key: string; spec: { view: string } }>(
      "POST",
      "/api/v1/report-definitions",
    );
    expect(body.name).toBe("Open tickets by client");
    expect(body.idempotency_key).toMatch(/-/);
    expect(body.spec.view).toBe("tickets");
    cleanup();
  });

  it("runs a saved report through its definition run endpoint and renders rows", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );

    fireEvent.click(await screen.findByRole("tab", { name: "Saved reports" }, { timeout: 5000 }));
    fireEvent.click(await screen.findByRole("button", { name: "Run" }, { timeout: 5000 }));

    await waitFor(() =>
      expect(mock.callsTo("GET", `/api/v1/report-definitions/${DEF_ID}/run`).length).toBe(1),
    );
    expect(await screen.findByText("P2", {}, { timeout: 5000 })).toBeInTheDocument();
    cleanup();
  });

  it("renders friendly reason-code copy when a run is forbidden (never raw JSON)", async () => {
    setupApi(ME_FIXTURE, 403);
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );

    fireEvent.click(await screen.findByRole("tab", { name: "Saved reports" }, { timeout: 5000 }));
    fireEvent.click(await screen.findByRole("button", { name: "Run" }, { timeout: 5000 }));

    await waitFor(
      () => {
        const alert = screen.queryByRole("alert");
        expect(alert).not.toBeNull();
        expect(alert!.textContent).not.toMatch(/\{|\}/);
      },
      { timeout: 5000 },
    );
    cleanup();
  });
});
describe("/reports deep link (Wave 23 lane 1, items 2+3)", () => {
  it("seeds and auto-runs a builder preview from ?spec=", async () => {
    const mock = new MockApi();
    mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
    mock.on("GET", "/api/v1/report-catalog", () => ({ json: CATALOG }));
    mock.on("GET", "/api/v1/report-definitions", () => ({ json: DEFINITIONS }));
    mock.on("GET", "/api/v1/report-schedules", () => ({ json: SCHEDULES }));
    mock.on("POST", "/api/v1/reports/run-spec", () => ({ json: RUN_PAGE }));
    mock.install();

    const specQuery = encodeSpecToQuery({
      view: "tickets",
      filters: [{ column: "priority", op: "eq", value: { text: "p1" } }],
      group_by: ["priority"],
      measures: [{ agg: "count" }],
      viz: "bar",
    } as never);
    window.history.replaceState(null, "", `/reports?spec=${specQuery}`);

    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );

    // The linked spec auto-runs once, without anyone pressing Run preview.
    await waitFor(
      () => expect(mock.callsTo("POST", "/api/v1/reports/run-spec").length).toBe(1),
      { timeout: 5000 },
    );
    const body = mock.lastBody<{ spec: { filters: { column: string }[]; group_by: string[] } }>(
      "POST",
      "/api/v1/reports/run-spec",
    );
    expect(body.spec.group_by).toEqual(["priority"]);
    expect(body.spec.filters?.[0]).toMatchObject({ column: "priority" });
    // Rows from the linked spec render in the preview table.
    expect(await screen.findByText("P1", {}, { timeout: 5000 })).toBeInTheDocument();

    // A second navigation without the param does NOT re-run anything new.
    window.history.replaceState(null, "", "/reports");
    expect(mock.callsTo("POST", "/api/v1/reports/run-spec").length).toBe(1);
    cleanup();
  });
});
