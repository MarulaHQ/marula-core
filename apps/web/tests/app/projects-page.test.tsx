// Integration test: /projects — the portfolio list. Covers the stat
// strip, per-project status/risk signals, phase-progress roll-ups from
// the real phases endpoints, and the client-side search filter
// (Wave 17 lane 4).
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import ProjectsPage from "../../app/projects/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/projects",
}));

const ORG = ME_FIXTURE.org_id;
const CLIENT_ID = "00000000-0000-4000-8000-0000000000c1";
const OVERDUE_ID = "00000000-0000-4000-8000-0000000000p1";
const HEALTHY_ID = "00000000-0000-4000-8000-0000000000p2";
const YESTERDAY = new Date(Date.now() - 86_400_000).toISOString().slice(0, 10);
const NEXT_MONTH = new Date(Date.now() + 30 * 86_400_000).toISOString().slice(0, 10);

function project(over: Record<string, unknown>) {
  return {
    id: OVERDUE_ID,
    org_id: ORG,
    name: "Exchange migration",
    status: "active",
    billing_method: "milestone",
    client_id: CLIENT_ID,
    budget_minor: 500_000,
    currency: "USD",
    description: null,
    start_date: null,
    due_date: YESTERDAY,
    created_at: "",
    updated_at: "",
    ...over,
  };
}

function phases(total: number, completed: number) {
  return Array.from({ length: total }, (_, i) => ({
    id: `ph${i}`,
    org_id: ORG,
    project_id: "p",
    name: `Phase ${i + 1}`,
    sort_order: i,
    status: i < completed ? "completed" : "pending",
    is_milestone: false,
    milestone_amount_minor: null,
    milestone_currency: null,
    description: null,
    completed_at: null,
    created_at: "",
    updated_at: "",
  }));
}

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/projects", () => ({
    json: [
      project({}),
      project({
        id: HEALTHY_ID,
        name: "Onboard Initech",
        status: "planning",
        due_date: NEXT_MONTH,
      }),
    ],
  }));
  mock.on("GET", "/api/v1/clients", () => ({
    json: [{ id: CLIENT_ID, org_id: ORG, name: "Acme Corp", created_at: "", updated_at: "", visibility: "private" }],
  }));
  mock.on("GET", `/api/v1/projects/${OVERDUE_ID}/phases`, () => ({ json: phases(4, 2) }));
  mock.on("GET", `/api/v1/projects/${HEALTHY_ID}/phases`, () => ({ json: phases(0, 0) }));
  return mock.install();
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("ProjectsPage", () => {
  it("renders the stat strip, status tones, and the overdue risk chip", async () => {
    setupApi();
    render(
      <AuthProvider>
        <ProjectsPage />
      </AuthProvider>,
    );
    expect(await screen.findByText("Exchange migration")).toBeInTheDocument();
    expect(screen.getByTestId("stat-active")).toHaveTextContent("1");
    expect(screen.getByTestId("stat-at-risk")).toHaveTextContent("1");
    const overdueRow = screen.getByTestId(`project-row-${OVERDUE_ID}`);
    expect(within(overdueRow).getByText("Active")).toBeInTheDocument();
    expect(within(overdueRow).getByText(/overdue 1d/)).toBeInTheDocument();
    const healthyRow = screen.getByTestId(`project-row-${HEALTHY_ID}`);
    expect(within(healthyRow).getByText("Planning")).toBeInTheDocument();
    expect(within(healthyRow).queryByText(/overdue/)).toBeNull();
    cleanup();
  });

  it("loads per-project phase progress from the real phases endpoints", async () => {
    setupApi();
    render(
      <AuthProvider>
        <ProjectsPage />
      </AuthProvider>,
    );
    const bar = await screen.findByTestId("phase-progress-2-4");
    expect(bar).toHaveTextContent("2/4 phases");
    await waitFor(() => {
      expect(screen.getByTestId("phase-progress-empty")).toHaveTextContent("no phases");
    });
    cleanup();
  });

  it("narrows rows through the client-side search", async () => {
    setupApi();
    render(
      <AuthProvider>
        <ProjectsPage />
      </AuthProvider>,
    );
    await screen.findByText("Exchange migration");
    fireEvent.change(screen.getByLabelText("Search projects"), { target: { value: "initech" } });
    await waitFor(() => {
      expect(screen.queryByText("Exchange migration")).toBeNull();
    });
    expect(screen.getByText("Onboard Initech")).toBeInTheDocument();
    expect(screen.getByText("1 project")).toBeInTheDocument();
    cleanup();
  });
});
