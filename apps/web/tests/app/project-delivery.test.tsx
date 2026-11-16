// Integration tests for the project-delivery surfaces: the template gallery,
// the change-order record and the portfolio quarter. Each mounts the real
// component against the real fetch path, so a wrong path or a missing state
// fails here rather than in the browser.
import { describe, expect, it, vi, beforeEach } from "vitest";
import { useState } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import { TemplateGallery } from "../../components/projects/template-gallery";
import { ChangeOrdersPanel } from "../../components/projects/change-orders-panel";
import { PortfolioQuarter } from "../../components/projects/portfolio-quarter";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";
import { currentFetchRouter, setFetchRouter } from "../helpers/fetch-router";

const search = vi.hoisted(() => ({ params: new URLSearchParams() }));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/projects",
  useSearchParams: () => search.params,
  useParams: () => ({ id: "p1" }),
}));

const ORG = ME_FIXTURE.org_id;
const PROJECT = "00000000-0000-4000-8000-0000000000a1";

function template(over: Record<string, unknown> = {}) {
  return {
    id: "t1",
    name: "Office move",
    description: "Phased move with a cutover weekend",
    category: "Moves",
    typical_days: 42,
    effort_min_minutes: 3600,
    effort_max_minutes: 5400,
    archived: false,
    is_shipped: true,
    current_version: 1,
    forked_from_version_id: null,
    phase_count: 4,
    task_count: 18,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...over,
  };
}

function installBase(mock: MockApi, permissions: string[] = []): void {
  mock.on("GET", "/api/v1/me", () => ({
    json: { ...ME_FIXTURE, permissions: [...ME_FIXTURE.permissions, ...permissions] },
  }));
}

describe("template gallery", () => {
  beforeEach(() => {
    cleanup();
    seedAuthTokens();
  });

  it("renders the shape of a template and hides authoring without permission", async () => {
    const mock = new MockApi();
    installBase(mock);
    mock.on("GET", "/api/v1/projects/template-gallery", () => ({ json: [template()] }));
    mock.install();

    render(
      <AuthProvider>
        <TemplateGallery />
      </AuthProvider>,
    );

    expect(await screen.findByText("Office move")).toBeInTheDocument();
    expect(screen.getByText(/4 phases · 18 tasks/)).toBeInTheDocument();
    expect(screen.getByText(/You can use and preview templates/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Archive" })).not.toBeInTheDocument();
  });

  it("says so honestly when the gallery is empty", async () => {
    const mock = new MockApi();
    installBase(mock);
    mock.on("GET", "/api/v1/projects/template-gallery", () => ({ json: [] }));
    mock.install();

    render(
      <AuthProvider>
        <TemplateGallery />
      </AuthProvider>,
    );

    expect(await screen.findByText(/No templates yet/)).toBeInTheDocument();
  });
});

describe("change orders", () => {
  beforeEach(() => {
    cleanup();
    seedAuthTokens();
  });

  it("draws approval and the two acts as separate controls", async () => {
    const mock = new MockApi();
    installBase(mock, ["projects:edit", "financial:read"]);
    mock.on("GET", `/api/v1/projects/${PROJECT}/change-orders`, () => ({
      json: [
        {
          id: "co1",
          project_id: PROJECT,
          reference: "CO-ABC123",
          title: "Extra staging",
          status: "approved",
          source: "manual",
          reason: "The client added a floor",
          scope_changes: {},
          client_summary: null,
          evidence: [],
          coverage_note: null,
          confidence: null,
          contract_clause: null,
          amount_minor: 120000,
          currency: "USD",
          submitted_at: null,
          submitted_by: null,
          decided_at: null,
          decided_by: null,
          decision_note: null,
          applied_at: null,
          applied_by: null,
          applied_baseline_id: null,
          billed_at: null,
          billing_release_id: null,
          billing_error: null,
          client_ack_required: false,
          client_ack_at: null,
          created_at: "2026-01-01T00:00:00Z",
          updated_at: "2026-01-01T00:00:00Z",
        },
      ],
    }));
    mock.on("GET", `/api/v1/projects/${PROJECT}/billing-releases`, () => ({ json: [] }));
    mock.on("GET", `/api/v1/projects/${PROJECT}/client-updates`, () => ({ json: [] }));
    mock.install();

    render(
      <AuthProvider>
        <ChangeOrdersPanel projectId={PROJECT} />
      </AuthProvider>,
    );

    expect(await screen.findByText("Extra staging")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Apply to the plan" })).toBeInTheDocument();
    // Approval has already happened, so no approve button and no bill button yet:
    // apply and bill are separate steps.
    expect(screen.queryByRole("button", { name: "Approve" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Release the billing line" })).not.toBeInTheDocument();
  });

  it("shows a failed release with its reason and a retry", async () => {    const mock = new MockApi();
    installBase(mock, ["projects:edit", "financial:read"]);
    const base = {
      id: "co2",
      project_id: PROJECT,
      reference: "CO-DEF456",
      title: "Licence uplift",
      status: "billing_failed",
      source: "scope_signal",
      reason: "Contract clause 4.2",
      scope_changes: {},
      client_summary: null,
      evidence: [],
      coverage_note: null,
      confidence: "fair",
      contract_clause: "4.2",
      amount_minor: 50000,
      currency: "USD",
      submitted_at: null,
      submitted_by: null,
      decided_at: null,
      decided_by: null,
      decision_note: null,
      applied_at: null,
      applied_by: null,
      applied_baseline_id: null,
      billed_at: null,
      billing_release_id: null,
      billing_error: "no active contract for this client",
      client_ack_required: false,
      client_ack_at: null,
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
    };
    mock.on("GET", `/api/v1/projects/${PROJECT}/change-orders`, () => ({ json: [base] }));
    mock.on("GET", `/api/v1/projects/${PROJECT}/billing-releases`, () => ({ json: [] }));
    mock.on("GET", `/api/v1/projects/${PROJECT}/client-updates`, () => ({ json: [] }));
    mock.install();

    render(
      <AuthProvider>
        <ChangeOrdersPanel projectId={PROJECT} />
      </AuthProvider>,
    );

    expect(await screen.findByText(/no active contract/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry billing" })).toBeInTheDocument();
  });

  it("keeps a finished create from the previous project from redrawing it", async () => {
    const OTHER = "00000000-0000-4000-8000-0000000000b2";
    const order = (project: string, title: string) => ({
      id: `co-${project}`,
      project_id: project,
      reference: "CO-ABC123",
      title,
      status: "draft",
      source: "manual",
      reason: "",
      scope_changes: {},
      client_summary: null,
      evidence: [],
      coverage_note: null,
      confidence: null,
      contract_clause: null,
      amount_minor: null,
      currency: null,
      submitted_at: null,
      submitted_by: null,
      decided_at: null,
      decided_by: null,
      decision_note: null,
      applied_at: null,
      applied_by: null,
      applied_baseline_id: null,
      billed_at: null,
      billing_release_id: null,
      billing_error: null,
      client_ack_required: false,
      client_ack_at: null,
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
    });

    const mock = new MockApi();
    installBase(mock, ["projects:edit", "financial:read"]);
    mock.on("GET", `/api/v1/projects/${PROJECT}/change-orders`, () => ({
      json: [order(PROJECT, "A only")],
    }));
    mock.on("GET", `/api/v1/projects/${OTHER}/change-orders`, () => ({
      json: [order(OTHER, "B only")],
    }));
    for (const id of [PROJECT, OTHER]) {
      mock.on("GET", `/api/v1/projects/${id}/billing-releases`, () => ({ json: [] }));
      mock.on("GET", `/api/v1/projects/${id}/client-updates`, () => ({ json: [] }));
    }
    // The create answers only when the test says so, so the move to the other
    // project happens while it is still in flight.
    let release = (): void => undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    mock.on("POST", `/api/v1/projects/${PROJECT}/change-orders`, () => ({
      json: order(PROJECT, "A only"),
    }));
    mock.install();
    const delegate = currentFetchRouter();
    setFetchRouter(async (input, init) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const method = (
        typeof input === "object" && "method" in input && input.method !== undefined
          ? input.method
          : (init?.method ?? "GET")
      ).toUpperCase();
      if (method === "POST" && url.includes("/change-orders")) await gate;
      if (delegate === null) throw new Error("no MockApi router installed");
      return delegate(input, init);
    });

    function Harness() {
      const [id, setId] = useState(PROJECT);
      return (
        <AuthProvider>
          <button type="button" onClick={() => setId(OTHER)}>
            go to the other project
          </button>
          <ChangeOrdersPanel projectId={id} />
        </AuthProvider>
      );
    }

    render(<Harness />);
    expect(await screen.findByText("A only")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "New change order" }));
    fireEvent.change(await screen.findByLabelText("Title"), { target: { value: "Extras" } });
    fireEvent.click(screen.getByRole("button", { name: "Save draft" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Saving…" })).toBeTruthy());

    // The dialog is modal, so the page behind it is hidden from the
    // accessibility tree; the switch control is still in the DOM.
    fireEvent.click(screen.getByText("go to the other project"));
    expect(await screen.findByText("B only")).toBeInTheDocument();

    // The create lands after the move. Its follow-up belongs to the project it
    // was started under, so it must not fetch that project's rows again.
    await act(async () => {
      release();
    });
    expect(
      mock.callsTo("GET", `/api/v1/projects/${PROJECT}/change-orders`),
    ).toHaveLength(1);
    expect(screen.getByText("B only")).toBeInTheDocument();
    expect(screen.queryByText("A only")).not.toBeInTheDocument();
  });
});

describe("portfolio quarter", () => {
  beforeEach(() => {
    cleanup();
    seedAuthTokens();
    search.params = new URLSearchParams();
  });

  it("groups projects and writes the risk reason in words", async () => {
    const mock = new MockApi();
    installBase(mock);
    mock.on("GET", "/api/v1/projects/portfolio", () => ({
      json: {
        from: "2026-01-01",
        to: "2026-03-31",
        zoom: "quarter",
        group_by: "client",
        weeks: [
          { start: "2026-01-01", label: "Wk of Jan 1" },
          { start: "2026-01-08", label: "Wk of Jan 8" },
        ],
        groups: [
          {
            key: "c1",
            label: "Acme",
            project_count: 1,
            at_risk_count: 1,
            projects: [
              {
                id: "p1",
                name: "Exchange migration",
                status: "active",
                client_id: "c1",
                client_name: "Acme",
                start_date: "2026-01-01",
                due_date: "2025-12-01",
                at_risk: true,
                risk_reasons: ["Past its due date (31 days)"],
                has_dated_phase: true,
                phases: [],
              },
            ],
          },
        ],
        unscheduled: [
          {
            id: "p2",
            name: "Waiting on dates",
            status: "planning",
            start_date: null,
            due_date: null,
          },
        ],
      },
    }));
    mock.install();

    render(
      <AuthProvider>
        <PortfolioQuarter />
      </AuthProvider>,
    );

    expect(await screen.findByText("Exchange migration")).toBeInTheDocument();
    expect(screen.getByText("Past its due date (31 days)")).toBeInTheDocument();
    expect(screen.getByText(/Not scheduled \(1\)/)).toBeInTheDocument();
    expect(screen.getByText("Waiting on dates")).toBeInTheDocument();
  });
});
