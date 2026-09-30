// Integration test: /provenance — the data-surface frame (view switch,
// filter bar clauses, column chooser, export, density footer), keyset
// paging via next_cursor, and before/after rendering.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import ProvenancePage from "../../app/provenance/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/provenance",
}));

function event(overrides: Record<string, unknown> = {}) {
  return {
    write_id: "80000000-0000-4000-8000-000000000001",
    entity: "ticket",
    entity_id: "81000000-0000-4000-8000-000000000001",
    seq: 12,
    op: "update",
    actor_kind: "human",
    actor_id: ME_FIXTURE.user_id,
    field: "status",
    before: "open",
    after: "waiting",
    undo: false,
    occurred_at: "2026-09-02T10:00:00Z",
    idempotency_key: null,
    ruleset_hash: null,
    session_id: null,
    skill: null,
    ...overrides,
  };
}

let rows: Record<string, unknown>[];

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/provenance", ({ query }) => {
    let out = rows;
    if (query.actor_kind !== undefined && query.actor_kind !== "") {
      out = out.filter((r) => r.actor_kind === query.actor_kind);
    }
    if (query.cursor !== undefined && query.cursor !== "") {
      out = out.filter((r) => Number(r.seq) > 12);
    }
    if (query.entity !== undefined && query.entity !== "") {
      out = out.filter((r) => r.entity === query.entity);
    }
    return {
      json: {
        events: out,
        next_cursor: out.length > 0 && query.cursor === undefined ? "cur-1" : null,
      },
    };
  });
  mock.install();
  return mock;
}

/** Add one clause through the D12 filter builder. */
async function addClause(
  user: ReturnType<typeof userEvent.setup>,
  fieldLabel: string,
  value: string,
): Promise<void> {
  await user.click(screen.getByRole("button", { name: /add filter/i }));
  await user.selectOptions(screen.getByLabelText("Field"), fieldLabel);
  await user.type(screen.getByLabelText("Value"), value);
  await user.click(screen.getByRole("button", { name: "Add" }));
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  rows = [
    event(),
    event({
      write_id: "80000000-0000-4000-8000-000000000002",
      seq: 14,
      actor_kind: "agent",
      field: "priority",
      before: "p3",
      after: "p2",
      skill: "triage",
      session_id: "82000000-0000-4000-8000-000000000001",
    }),
  ];
});

describe("/provenance integration", () => {
  it("renders field-level before/after rows with actor attribution", async () => {
    setupApi();
    render(
      <AuthProvider>
        <ProvenancePage />
      </AuthProvider>,
    );
    await screen.findByText("status");
    expect(screen.getByText("open")).toBeInTheDocument();
    expect(screen.getByText("waiting")).toBeInTheDocument();
    expect(screen.getByText("priority")).toBeInTheDocument();
    cleanup();
  });

  it("carries the four data-surface controls and no bulk bar", async () => {
    setupApi();
    render(
      <AuthProvider>
        <ProvenancePage />
      </AuthProvider>,
    );
    await screen.findByText("status");
    // View switch, filter bar, column chooser, export and the density step.
    expect(screen.getByRole("tablist", { name: "Provenance views" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /add filter/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Columns" })).toBeInTheDocument();
    // One button per format, and the row count on the strip so the user
    // knows the size before they ask.
    expect(screen.getByRole("button", { name: "Export CSV" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Export XLSX" })).toBeInTheDocument();
    expect(screen.getByText(/\d+ rows?/)).toBeInTheDocument();
    expect(screen.getByRole("radiogroup", { name: "Density" })).toBeInTheDocument();
    // Audit and provenance take every control EXCEPT the selection bar.
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    expect(screen.queryByText(/selected/)).not.toBeInTheDocument();
    cleanup();
  });

  it("pages with the keyset cursor", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ProvenancePage />
      </AuthProvider>,
    );
    await screen.findByText("status");
    await user.click(screen.getByRole("button", { name: "Load 50 more" }));
    await waitFor(() => {
      const calls = mock.callsTo("GET", "/api/v1/provenance");
      expect(calls[calls.length - 1]?.query.cursor).toBe("cur-1");
    });
    cleanup();
  });

  it("filters through the chip bar and shows the empty state when nothing matches", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ProvenancePage />
      </AuthProvider>,
    );
    await screen.findByText("status");
    await addClause(user, "Entity", "invoice");
    // The clause reads back as a sentence chip with a remove button.
    expect(await screen.findByText("Entity contains invoice")).toBeInTheDocument();
    await waitFor(() => {
      const calls = mock.callsTo("GET", "/api/v1/provenance");
      expect(calls[calls.length - 1]?.query.entity).toBe("invoice");
    });
    await screen.findByText(/No writes match these filters/);
    // The empty state never suggests another way to ask.
    expect(screen.queryByText(/ask/i)).not.toBeInTheDocument();
    cleanup();
  });

  it("switches views without a round trip for the client-side ones", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ProvenancePage />
      </AuthProvider>,
    );
    await screen.findByText("status");
    expect(screen.getByText("p3")).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: /Agent/ }));
    await waitFor(() => {
      expect(screen.queryByText("status")).not.toBeInTheDocument();
    });
    expect(screen.getByText("p2")).toBeInTheDocument();
    cleanup();
  });
});
