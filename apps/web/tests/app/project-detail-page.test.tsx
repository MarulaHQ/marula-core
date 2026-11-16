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

const PROJECT = {
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
  client_hold_reason: null as string | null,
};

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", `/api/v1/projects/${PROJECT_ID}`, () => ({ json: PROJECT }));
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

/** The record's title. The breadcrumb carries the same name, so a text
 * query matches twice — the heading is the title and only the title. */
function loaded() {
  return screen.findByRole("heading", { level: 1 });
}

describe("ProjectDetailPage", () => {
  it("renders the roll-up strip, phase rail, and risk chips", async () => {
    setupApi();
    mount();
    expect(await loaded()).toHaveTextContent("Exchange migration");
    expect(screen.getByTestId("rollup-phases")).toHaveTextContent("1/ 2 phases");
    expect(screen.getByTestId("rollup-tasks")).toHaveTextContent("0/ 3 tasks");
    expect(screen.getByTestId("rollup-milestones")).toHaveTextContent("0/ 1 milestones");
    expect(screen.getByTestId("phase-rail")).toHaveTextContent("Discovery");
    expect(screen.getByTestId("phase-rail")).toHaveTextContent("Migration");
    expect(screen.getByTestId("risk-overdue")).toHaveTextContent("1 overdue task");
    expect(screen.getByTestId("risk-blocked")).toHaveTextContent("1 blocked");
    cleanup();
  });

  it("carries the pack's three tabs under the title, Plan open first", async () => {
    setupApi();
    mount();
    await loaded();
    // DESIGN-NOTES-clients §7 gives the project record Plan, Tasks, and
    // Time and budget. They sit under the title, so the header does not
    // change between them.
    expect(screen.getByTestId("record-tab-plan")).toHaveTextContent("Plan");
    expect(screen.getByTestId("record-tab-tasks")).toHaveTextContent("Tasks");
    expect(screen.getByTestId("record-tab-money")).toHaveTextContent("Time and budget");
    // Plan is the default view: the phase rail and the schedule are on
    // screen before any click, and the task groups are not.
    expect(screen.getByTestId("project-schedule")).toBeInTheDocument();
    expect(screen.queryByTestId("project-tasks")).toBeNull();
    cleanup();
  });

  it("groups tasks under phases and flags blocked-by with the blocker titles", async () => {
    setupApi();
    mount();
    // The record's tabs (DESIGN-NOTES-clients §7): the task groups live on
    // the Tasks tab, under a header that does not change between tabs.
    await loaded();
    fireEvent.click(screen.getByTestId("record-tab-tasks"));
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
    await loaded();
    fireEvent.click(screen.getByTestId("record-tab-tasks"));
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
    await loaded();
    fireEvent.click(screen.getByTestId("record-tab-tasks"));
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

  it("keeps minutes but hides a redacted phase-burn amount", async () => {
    const mock = setupApi();
    mock.on("GET", `/api/v1/projects/${PROJECT_ID}/phase-burn`, () => ({
      json: [
        {
          phase_id: PHASE_ACTIVE,
          phase_name: "Migration",
          phase_status: "active",
          consumed_minutes: 90,
          consumed_minor: null,
          budget_minor: null,
          currency: null,
          alert: "none",
        },
        {
          phase_id: PHASE_DONE,
          phase_name: "Discovery",
          phase_status: "completed",
          consumed_minutes: 30,
          consumed_minor: 12_000,
          budget_minor: 20_000,
          currency: "USD",
          alert: "warn",
        },
      ],
    }));
    mount();
    await loaded();
    fireEvent.click(screen.getByTestId("record-tab-money"));

    // Without financial:read the route nulls the money and the alert. The
    // minutes are not money, so they stay; "$0.00", "no budget" and an alert
    // badge would all be claims the caller is not allowed to read.
    const redacted = await screen.findByTestId(`phase-burn-${PHASE_ACTIVE}`);
    expect(redacted).toHaveTextContent("Amount hidden");
    expect(redacted).toHaveTextContent("90 min");
    expect(redacted).not.toHaveTextContent("$0.00");
    expect(redacted).not.toHaveTextContent("no budget");
    expect(within(redacted).queryByText("warn")).toBeNull();
    expect(within(redacted).queryByText("over")).toBeNull();

    // A reader with the money sees the amount, the budget it is measured
    // against, and the warning badge, unchanged.
    const visible = screen.getByTestId(`phase-burn-${PHASE_DONE}`);
    expect(visible).toHaveTextContent("$120.00 of $200.00");
    expect(visible).toHaveTextContent("warn");
    cleanup();
  });

  it("puts a project on hold with the reason the client may read", async () => {
    const mock = setupApi();
    let hold: string | null = null;
    mock.on("GET", `/api/v1/projects/${PROJECT_ID}`, () => ({
      json: { ...PROJECT, status: hold === null ? "active" : "on_hold", client_hold_reason: hold },
    }));
    mock.on("PATCH", `/api/v1/projects/${PROJECT_ID}`, ({ body }) => {
      const patch = body as { client_hold_reason?: string | null };
      hold = patch.client_hold_reason ?? null;
      return {
        json: {
          id: PROJECT_ID,
          mutation: { idempotency_key: "k", seq: 1, version: 2, prior: null, applied_at: "" },
        },
      };
    });
    mock.install();
    mount();
    await loaded();

    fireEvent.click(screen.getByTestId("project-hold"));
    fireEvent.change(await screen.findByTestId("hold-reason"), {
      target: { value: "Waiting on the new switchgear delivery" },
    });
    fireEvent.click(screen.getByTestId("hold-confirm"));

    await waitFor(() =>
      expect(mock.callsTo("PATCH", `/api/v1/projects/${PROJECT_ID}`)).toHaveLength(1),
    );
    const body = mock.lastBody<{ status: string; client_hold_reason: string | null }>(
      "PATCH",
      `/api/v1/projects/${PROJECT_ID}`,
    );
    expect(body.status).toBe("on_hold");
    expect(body.client_hold_reason).toBe("Waiting on the new switchgear delivery");

    // The refreshed read shows the stored reason on the staff record, and the
    // action flips from "Put on hold" to "Resume work".
    expect(await screen.findByTestId("project-hold-note")).toHaveTextContent(
      "Client reason: Waiting on the new switchgear delivery",
    );
    expect(screen.getByTestId("project-resume")).toBeInTheDocument();
    expect(screen.queryByTestId("project-hold")).toBeNull();
    cleanup();
  });
});
