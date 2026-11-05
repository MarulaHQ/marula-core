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
      card: {
        size: { span: "half", height: "regular" },
        window_days: 30,
        refresh: { every_minutes: 60, reach_days: 0 },
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
describe("/reports catalog robustness", () => {
  it("shows the builder's own state when the catalog payload has no views", async () => {
    const mock = new MockApi();
    mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
    // A resolving read can carry a payload the page did not expect. The tab
    // row has to survive it, because the builder sits inside the same surface.
    mock.on("GET", "/api/v1/report-catalog", () => ({ json: { views: undefined } }));
    mock.on("GET", "/api/v1/report-definitions", () => ({ json: DEFINITIONS }));
    mock.on("GET", "/api/v1/report-schedules", () => ({ json: { schedules: [] } }));
    mock.install();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );
    expect(await screen.findByRole("tab", { name: "Saved reports" })).toBeInTheDocument();
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

// --- Wave 46 lane 6: the describe entry, the mic's typed state, the card -----

const AUTHOR_PATH = "/api/v1/reports/author";

/** The accepted draft, as the authoring endpoint returns it. */
const AUTHORED = {
  valid: true,
  saved: false,
  draft: {
    name: "P1 tickets by priority",
    spec: {
      view: "tickets",
      filters: [],
      group_by: ["priority"],
      measures: [{ agg: "count" }],
      viz: "bar",
    },
    card: {
      size: { span: "full", height: "tall" },
      window_days: 7,
      refresh: { every_minutes: 30, reach_days: 0 },
    },
    shape: "chart",
    reads: "A chart of tickets by priority over the last 7 days.",
  },
  issues: [],
  diff: [{ op: "add", path: "view", after: "tickets" }],
  tool_calls: [],
  usage: { prompt_tokens: 12, completion_tokens: 8, steps: 1 },
  model_info: { provider: "openai", model: "marula-test-chat", resolution: "requested" },
  catalogue: { views: 1, shapes: 3, rules: 3, prompt_chars: 120 },
};

function setupAuthoring(
  author: () => { status?: number; json: unknown } = () => ({ json: AUTHORED }),
) {
  const mock = setupApi();
  mock.on("POST", AUTHOR_PATH, author);
  return mock;
}

/** Opens the entry, types a transcript and generates once. */
async function describeThroughEntry(transcript: string) {
  fireEvent.click(await screen.findByTestId("reports-describe-entry", {}, { timeout: 5000 }));
  const box = await screen.findByLabelText("What should this report answer?");
  fireEvent.change(box, { target: { value: transcript } });
  fireEvent.click(screen.getByRole("button", { name: "Generate a draft" }));
}

describe("/reports describe-it entry (Wave 46 lane 6)", () => {
  it("prints the stored card on the saved row, read from the definition", async () => {
    setupApi();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );

    fireEvent.click(await screen.findByRole("tab", { name: "Saved reports" }, { timeout: 5000 }));
    const meta = await screen.findByTestId(`definition-card-meta-${DEF_ID}`);

    // The raw stored members, not a rendering: window, refresh, size.
    expect(meta).toHaveAttribute("data-card", "stored");
    expect(meta).toHaveAttribute("data-card-window-days", "30");
    expect(meta).toHaveAttribute("data-card-refresh-minutes", "60");
    expect(meta).toHaveAttribute("data-card-span", "half");
    expect(meta).toHaveAttribute("data-card-height", "regular");
    expect(meta).toHaveTextContent("Last 30 days");
    expect(meta).toHaveTextContent("every 1 h");
    expect(meta).toHaveTextContent("Half wide, Regular tall");
    cleanup();
  });

  it("says a definition with no card runs with the default window", async () => {
    const mock = setupApi();
    mock.on("GET", "/api/v1/report-definitions", () => ({
      json: { definitions: [{ ...DEFINITIONS.definitions[0], card: null }] },
    }));
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );

    fireEvent.click(await screen.findByRole("tab", { name: "Saved reports" }, { timeout: 5000 }));
    const meta = await screen.findByTestId(`definition-card-meta-${DEF_ID}`);
    expect(meta).toHaveAttribute("data-card", "none");
    expect(meta).toHaveTextContent(/No card stored/);
    cleanup();
  });

  it("opens, generates one draft from one transcript, and saves it with its card", async () => {
    const mock = setupAuthoring();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );

    await describeThroughEntry("Tickets by priority over the last seven days.");

    const row = await screen.findByTestId("report-draft-row", {}, { timeout: 5000 });
    await waitFor(() => expect(row).toHaveAttribute("data-status", "ready"));
    expect(mock.callsTo("POST", AUTHOR_PATH)).toHaveLength(1);
    expect(mock.lastBody<{ transcript: string }>("POST", AUTHOR_PATH).transcript).toBe(
      "Tickets by priority over the last seven days.",
    );
    // The server's read-back is rendered unchanged.
    expect(screen.getByText(AUTHORED.draft.reads)).toBeInTheDocument();
    // Nothing was stored by generating.
    expect(mock.callsTo("POST", "/api/v1/report-definitions")).toHaveLength(0);

    fireEvent.click(screen.getByRole("button", { name: /Accept and save/ }));

    await waitFor(() =>
      expect(mock.callsTo("POST", "/api/v1/report-definitions")).toHaveLength(1),
    );
    const body = mock.lastBody<{
      name: string;
      card: { window_days: number; size: { span: string; height: string } };
      idempotency_key: string;
    }>("POST", "/api/v1/report-definitions");
    expect(body.card.window_days).toBe(7);
    expect(body.card.size).toEqual({ span: "full", height: "tall" });
    expect(body.idempotency_key).toMatch(/-/);

    // The page lands on the saved table and says where the row is.
    expect(await screen.findByTestId("reports-saved-note")).toHaveTextContent(
      /new private report/i,
    );
    cleanup();
  });

  it("states the mic as not available instead of shipping a dead control", async () => {
    setupAuthoring();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );

    fireEvent.click(await screen.findByTestId("reports-describe-entry", {}, { timeout: 5000 }));
    const mic = await screen.findByTestId("transcript-voice-unavailable");

    // It is an affordance with a stated reason, not a live recorder and not a
    // silent omission.
    expect(mic).toHaveAttribute("aria-disabled", "true");
    expect(mic).toHaveAttribute("data-state", "unavailable");
    expect(mic).toHaveTextContent(/voice note/i);
    const describedBy = mic.getAttribute("aria-describedby") ?? "";
    expect(describedBy).not.toBe("");
    expect(document.getElementById(describedBy)).toHaveTextContent(
      /Voice capture is not available yet/i,
    );
    // No audio control exists anywhere on the surface.
    expect(document.querySelector("audio")).toBeNull();
    cleanup();
  });

  it("renders an AI refusal as a reason with its remedy, never as a draft", async () => {
    const mock = setupAuthoring(() => ({
      status: 403,
      json: {
        error: {
          reason: "ai_hard_cap_reached",
          message: "your 'general_chat' usage hit the hard cap for this month",
        },
      },
    }));
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );

    await describeThroughEntry("anything at all");

    const notice = await screen.findByTestId("ai-gate-notice", {}, { timeout: 5000 });
    expect(notice).toHaveAttribute("data-reason", "ai_hard_cap_reached");
    expect(notice).toHaveAttribute("data-surface", "reports");
    expect(notice).toHaveTextContent("AI usage has reached its cap for this period");
    expect(notice).toHaveTextContent(/raise the cap/i);

    // No draft, no read-back, no save.
    expect(screen.queryByText(AUTHORED.draft.reads)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Accept and save/ })).toBeDisabled();
    expect(mock.callsTo("POST", "/api/v1/report-definitions")).toHaveLength(0);
    cleanup();
  });
});
