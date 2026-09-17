// Integration test: /projects/[id] — the delivery surface. Covers the
// roll-up strip, the phase rail, phase-grouped tasks with blocked-by
// chips, the needs-attention triage filter, and the optimistic
// complete-task envelope (Wave 17 lane 4).
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import ProjectDetailPage from "../../app/projects/[id]/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/projects/x",
  useParams: () => ({ id: "00000000-0000-4000-8000-0000000000p1" }),
}));

const ORG = ME_FIXTURE.org_id;
const PROJECT_ID = "00000000-0000-4000-8000-0000000000p1";
const CLIENT_ID = "00000000-0000-4000-8000-0000000000c1";
const PHASE_DONE = "00000000-0000-4000-8000-0000000000h1";
const PHASE_ACTIVE = "00000000-0000-4000-8000-0000000000h2";
const TASK_BLOCKED = "00000000-0000-4000-8000-0000000000t1";
const TASK_BLOCKING = "00000000-0000-4000-8000-0000000000t2";
const TASK_CALM = "00000000-0000-4000-8000-0000000000t3";
const HOUR = 3_600_000;

function task(over: Record<string, unknown>) {
  return {
    id: "t",
    org_id: ORG,
    project_id: PROJECT_ID,
    title: "Task",
    status: "pending",
    sort_order: 0,
    is_milestone: false,
    milestone_amount_minor: null,
    milestone_currency: null,
    phase_id: null,
    assignee_id: null,
    description: null,
    due_at: null,
    completed_at: null,
    created_at: "",
    updated_at: "",
    ...over,
  };
}

const PHASES = [
  {
    id: PHASE_DONE,
    org_id: ORG,
    project_id: PROJECT_ID,
    name: "Discovery",
    sort_order: 0,
    status: "completed",
    is_milestone: false,
    milestone_amount_minor: null,
    milestone_currency: null,
    description: null,
    completed_at: null,
    created_at: "",
    updated_at: "",
  },
  {
    id: PHASE_ACTIVE,
    org_id: ORG,
    project_id: PROJECT_ID,
    name: "Migration",
    sort_order: 1,
    status: "active",
    is_milestone: true,
    milestone_amount_minor: 120_000,
    milestone_currency: "USD",
    description: null,
    completed_at: null,
    created_at: "",
    updated_at: "",
  },
];

const TASKS = [
  task({ id: TASK_BLOCKED, title: "Cut over mail", phase_id: PHASE_ACTIVE, due_at: new Date(Date.now() - HOUR).toISOString() }),
  task({ id: TASK_BLOCKING, title: "Freeze mailbox deltas", phase_id: PHASE_ACTIVE }),
  task({ id: TASK_CALM, title: "Archive project channel", phase_id: PHASE_DONE }),
];

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", `/api/v1/projects/${PROJECT_ID}`, () => ({
    json: {
      id: PROJECT_ID,
      org_id: ORG,
      name: "Exchange migration",
      status: "active",
      billing_method: "milestone",
      client_id: CLIENT_ID,
      budget_minor: 500_000,
      currency: "USD",
      description: "Move Acme to Exchange Online.",
      start_date: null,
      due_date: new Date(Date.now() + 14 * 86_400_000).toISOString().slice(0, 10),
      created_at: "",
      updated_at: "",
    },
  }));
  mock.on("GET", `/api/v1/clients/${CLIENT_ID}`, () => ({
    json: { id: CLIENT_ID, org_id: ORG, name: "Acme Corp", created_at: "", updated_at: "" },
  }));
  mock.on("GET", `/api/v1/projects/${PROJECT_ID}/phases`, () => ({ json: PHASES }));
  mock.on("GET", `/api/v1/projects/${PROJECT_ID}/tasks`, () => ({ json: TASKS }));
  mock.on(
    "GET",
    `/api/v1/projects/${PROJECT_ID}/tasks/${TASK_BLOCKED}/dependencies`,
    () => ({ json: { depends_on: [TASKS[1]] } }),
  );
  mock.on(
    "GET",
    `/api/v1/projects/${PROJECT_ID}/tasks/${TASK_BLOCKING}/dependencies`,
    () => ({ json: { depends_on: [] } }),
  );
  mock.on(
    "GET",
    `/api/v1/projects/${PROJECT_ID}/tasks/${TASK_CALM}/dependencies`,
    () => ({ json: { depends_on: [] } }),
  );
  mock.on("POST", `/api/v1/projects/${PROJECT_ID}/tasks/${TASK_BLOCKING}/complete`, () => ({
    json: {
      id: TASK_BLOCKING,
      mutation: { idempotency_key: "k", seq: 1, version: 1, prior: null, applied_at: "" },
    },
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
      <ProjectDetailPage />
    </AuthProvider>,
  );
}

describe("ProjectDetailPage", () => {
  it("renders the roll-up strip, phase rail, and risk chips", async () => {
    setupApi();
    mount();
    expect(await screen.findByText("Exchange migration")).toBeInTheDocument();
    expect(screen.getByTestId("rollup-phases")).toHaveTextContent("1/ 2 phases");
    expect(screen.getByTestId("rollup-tasks")).toHaveTextContent("0/ 3 tasks");
    expect(screen.getByTestId("rollup-milestones")).toHaveTextContent("0/ 1 milestones");
    expect(screen.getByTestId("phase-rail")).toHaveTextContent("Discovery");
    expect(screen.getByTestId("phase-rail")).toHaveTextContent("Migration");
    expect(screen.getByTestId("risk-overdue")).toHaveTextContent("1 overdue task");
    expect(screen.getByTestId("risk-blocked")).toHaveTextContent("1 blocked");
    cleanup();
  });

  it("groups tasks under phases and flags blocked-by with the blocker titles", async () => {
    setupApi();
    mount();
    const migrationGroup = await screen.findByTestId(`task-group-${PHASE_ACTIVE}`);
    expect(within(migrationGroup).getByText("Cut over mail")).toBeInTheDocument();
    expect(within(migrationGroup).getByText("Freeze mailbox deltas")).toBeInTheDocument();
    expect(migrationGroup).toHaveTextContent("milestone $1,200.00");
    expect(screen.getByTestId(`blocked-by-${TASK_BLOCKED}`)).toHaveTextContent(
      "blocked by Freeze mailbox deltas",
    );
    cleanup();
  });

  it("the needs-attention filter hides calm tasks", async () => {
    setupApi();
    mount();
    await screen.findByText("Cut over mail");
    expect(screen.getByText("Archive project channel")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("attention-filter"));
    await waitFor(() => {
      expect(screen.queryByText("Archive project channel")).toBeNull();
    });
    expect(screen.getByText("Cut over mail")).toBeInTheDocument();
    cleanup();
  });

  it("completing a task posts the idempotency-keyed envelope", async () => {
    const mock = setupApi();
    mount();
    const row = await screen.findByTestId(`task-row-${TASK_BLOCKING}`);
    fireEvent.click(within(row).getByRole("button", { name: "Complete" }));
    await waitFor(() => {
      expect(
        mock.callsTo("POST", `/api/v1/projects/${PROJECT_ID}/tasks/${TASK_BLOCKING}/complete`),
      ).toHaveLength(1);
    });
    const body = mock.lastBody<{ idempotency_key: string }>(
      "POST",
      `/api/v1/projects/${PROJECT_ID}/tasks/${TASK_BLOCKING}/complete`,
    );
    expect(body.idempotency_key).toBeTruthy();
    cleanup();
  });
});
