// Integration test: /sales/pipeline — kanban columns with counts/value,
// deal creation through the envelope, and an optimistic stage move that
// reverts on a server rejection.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import PipelinePage from "../../app/sales/pipeline/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/sales/pipeline",
}));

const S1 = "60000000-0000-4000-8000-000000000001";
const S2 = "60000000-0000-4000-8000-000000000002";
const CLIENT = "61000000-0000-4000-8000-000000000001";

const stage = (id: string, name: string, stage_type: string, position: number) => ({
  id,
  org_id: "org",
  name,
  stage_type,
  position,
  active: true,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
});

const deal = (id: string, stage_id: string, name: string) => ({
  id,
  org_id: "org",
  client_id: CLIENT,
  stage_id,
  name,
  amount_minor: 50000,
  currency: "USD",
  status: "open",
  created_at: "2026-08-01T00:00:00Z",
  updated_at: "2026-08-01T00:00:00Z",
  closed_at: null,
  created_by: null,
  owner_user_id: null,
  quote_id: null,
  expected_close_on: "2026-10-01",
  notes: null,
});

function setupApi(): MockApi {
  const created: Record<string, unknown>[] = [];
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/pipeline/stages", () => ({
    json: [stage(S1, "Qualifying", "open", 0), stage(S2, "Won", "won", 10)],
  }));
  mock.on("GET", "/api/v1/pipeline/deals", () => ({
    json: [deal("d1", S1, "Network refresh"), ...created],
  }));
  mock.on("GET", "/api/v1/clients", () => ({
    json: [{ id: CLIENT, name: "Acme Corp", display_name: null, notes: null, created_at: "2026-01-01T00:00:00Z" }],
  }));
  mock.on("POST", "/api/v1/pipeline/deals", ({ body }) => {
    const b = body as { name: string };
    const row = {
      id: "d2",
      org_id: "org",
      client_id: CLIENT,
      stage_id: S1,
      name: b.name,
      amount_minor: 25000,
      currency: "USD",
      status: "open",
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
      closed_at: null,
      created_by: null,
      owner_user_id: null,
      quote_id: null,
      expected_close_on: null,
      notes: null,
    };
    created.push(row);
    return { json: row };
  });
  mock.on("POST", "/api/v1/pipeline/deals/d1/stage", ({ body }) => {
    const to = (body as { to_stage_id?: string }).to_stage_id;
    if (to === S2) {
      // Server is the transition gate — closed stages refuse downstream moves
      // (typed rejection the lane must render and roll back).
      return { status: 409, json: { error: { code: 409, message: "already closed" } } };
    }
    return {
      json: {
        id: "d1",
        stage_id: to,
        mutation: { after: null, before: null, seq: 4, updated_at: new Date().toISOString(), version: 4, idempotency_key: "k" },
      },
    };
  });
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/sales/pipeline integration", () => {
  it("renders kanban stage columns with count chips and single-currency value", async () => {
    setupApi();
    render(
      <AuthProvider>
        <PipelinePage />
      </AuthProvider>,
    );
    expect(await screen.findByTestId("pipeline-stage-Qualifying")).toBeInTheDocument();
    expect(screen.getByTestId("pipeline-stage-Won")).toBeInTheDocument();
    expect(screen.getByTestId("pipeline-count-Qualifying")).toHaveTextContent("1");
    // Card amount and the single-currency column value both render the same
    // money() string — assert at least one, count chips assert the split.
    expect(screen.getAllByText("$500.00").length).toBeGreaterThan(0);
    cleanup();
  });

  it("creates a deal through the envelope with an idempotency key", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <PipelinePage />
      </AuthProvider>,
    );
    await screen.findByText("Network refresh");
    await user.click(screen.getByTestId("pipeline-new-deal"));
    await user.type(screen.getByTestId("pipeline-deal-name"), "Add-on project");
    // Client select (radix): open, then pick the client option.
    const u2 = userEvent.setup({ pointerEventsCheck: 0 });
    await u2.click(screen.getByTestId("pipeline-deal-client"));
    const clientOptions = await screen.findAllByRole("option", { name: "Acme Corp" });
    await u2.click(clientOptions[0] as HTMLElement);
    await user.click(screen.getByTestId("pipeline-deal-save"));

    await waitFor(() => expect(mock.callsTo("POST", "/api/v1/pipeline/deals").length).toBe(1));
    const body = mock.lastBody<{ idempotency_key: string }>("POST", "/api/v1/pipeline/deals");
    expect(body.idempotency_key).toBeTruthy();
    await screen.findByText("Add-on project");
    cleanup();
  });

  it("moves a stage optimistically and reverts with a typed banner on rejection", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <PipelinePage />
      </AuthProvider>,
    );
    await screen.findByText("Network refresh");
    // Move via the keyboard-equivalent select into the terminal Won column
    // (radix listbox: open the trigger, pick the option).
    const trigger = await screen.findByTestId("pipeline-move-d1");
    const user2 = userEvent.setup({ pointerEventsCheck: 0 });
    await user2.click(trigger);
    const options = await screen.findAllByRole("option", { name: "Won" });
    await user2.click(options[0] as HTMLElement);

    await waitFor(() =>
      expect(mock.callsTo("POST", "/api/v1/pipeline/deals/d1/stage").length).toBe(1),
    );
    // The card must have reverted to its source column and the banner names it.
    await screen.findByTestId("pipeline-move-error");
    expect(screen.getByTestId("pipeline-count-Qualifying")).toHaveTextContent("1");
    expect(screen.getByTestId("pipeline-count-Won")).toHaveTextContent("0");
    cleanup();
  });
});
