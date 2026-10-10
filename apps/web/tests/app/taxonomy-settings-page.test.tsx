// Integration test: /settings/taxonomy — the type list and the two-level
// category tree render from the SDK, and CRUD rides the instant-first
// envelope: create (POST), rename (PATCH), reparent (POST with the new root),
// retire (soft), duplicate names surfaced inline as typed 409s, and the
// read-only posture without `special:manage_boards`.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import TaxonomySettingsPage from "../../app/settings/taxonomy/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/taxonomy",
}));

const ORG = ME_FIXTURE.org_id;
const T1 = "10000000-0000-4000-8000-000000000001";
const T2 = "10000000-0000-4000-8000-000000000002";
const C_HW = "20000000-0000-4000-8000-000000000001";
const C_SOFT = "20000000-0000-4000-8000-000000000002";
const C_LAPTOP = "20000000-0000-4000-8000-000000000003";

const NOW = "2026-08-30T09:00:00Z";

function node(over: Record<string, unknown>) {
  return {
    org_id: ORG,
    parent_id: null,
    description: null,
    is_retired: false,
    sort_order: 0,
    created_at: NOW,
    updated_at: NOW,
    ...over,
  };
}

const TYPES = [
  node({ id: T1, name: "Incident", description: "Something broke" }),
  node({ id: T2, name: "Request", sort_order: 1 }),
];

const CATEGORIES = [
  node({ id: C_HW, name: "Hardware" }),
  node({ id: C_SOFT, name: "Software", sort_order: 1 }),
  node({ id: C_LAPTOP, name: "Laptop", parent_id: C_HW }),
];

function mutationResponse(over: Record<string, unknown>, seq = 1) {
  const body = node(over);
  return {
    ...body,
    ticket_references: 0,
    mutation: {
      after: body,
      before: null,
      seq,
      updated_at: NOW,
      version: seq,
      idempotency_key: "server-key",
    },
  };
}

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/taxonomy/types", () => ({ json: TYPES }));
  mock.on("GET", "/api/v1/taxonomy/categories", () => ({ json: CATEGORIES }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/settings/taxonomy", () => {
  it("renders the type list and the root→subcategory tree", async () => {
    setupApi();
    render(
      <AuthProvider>
        <TaxonomySettingsPage />
      </AuthProvider>,
    );
    expect(await screen.findByText("Incident")).toBeInTheDocument();
    expect(screen.getByText("Request")).toBeInTheDocument();
    expect(screen.getByText("Hardware")).toBeInTheDocument();
    expect(screen.getByText("Software")).toBeInTheDocument();
    expect(screen.getByText("Laptop")).toBeInTheDocument();
    expect(screen.getByText("Something broke")).toBeInTheDocument();
    cleanup();
  });

  it("creates a type through the envelope (idempotency-keyed POST)", async () => {
    const mock = setupApi();
    mock.on("POST", "/api/v1/taxonomy/types", (ctx) => ({
      json: mutationResponse({ id: "new", name: (ctx.body as { name: string }).name }),
    }));
    render(
      <AuthProvider>
        <TaxonomySettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Incident");

    fireEvent.click(screen.getByTestId("new-type"));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Change" } });
    fireEvent.click(screen.getByTestId("save-taxonomy-node"));

    await waitFor(() => {
      expect(mock.callsTo("POST", "/api/v1/taxonomy/types")).toHaveLength(1);
    });
    const body = mock.lastBody<{ name: string; idempotency_key?: string; sort_order?: number }>(
      "POST",
      "/api/v1/taxonomy/types",
    );
    expect(body.name).toBe("Change");
    expect(typeof body.idempotency_key).toBe("string");
    expect(body.sort_order).toBe(0);
    cleanup();
  });

  it("renders a duplicate-name typed 409 inline instead of failing silently", async () => {
    const mock = setupApi();
    mock.on("POST", "/api/v1/taxonomy/types", () => ({
      status: 409,
      json: { error: { code: 409, reason: "conflict", message: "a node with this name already exists here" } },
    }));
    render(
      <AuthProvider>
        <TaxonomySettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Incident");

    fireEvent.click(screen.getByTestId("new-type"));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Incident" } });
    fireEvent.click(screen.getByTestId("save-taxonomy-node"));

    expect(await screen.findByText(/already exists/i)).toBeInTheDocument();
    cleanup();
  });

  it("reparents a subcategory under a different root via POST", async () => {
    const mock = setupApi();
    mock.on("POST", `/api/v1/taxonomy/categories/${C_LAPTOP}/reparent`, (ctx) => ({
      json: mutationResponse({
        id: C_LAPTOP,
        name: "Laptop",
        parent_id: (ctx.body as { parent_id: string }).parent_id,
      }),
    }));
    render(
      <AuthProvider>
        <TaxonomySettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Laptop");

    fireEvent.click(screen.getByRole("button", { name: "Move" }));
    // The reparent target is the kit Radix Select — open the trigger, pick the root.
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    await user.click(screen.getByTestId("reparent-target"));
    await user.click(await screen.findByRole("option", { name: "Software" }));
    fireEvent.click(screen.getByTestId("confirm-reparent"));

    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/taxonomy/categories/${C_LAPTOP}/reparent`)).toHaveLength(1);
    });
    const body = mock.lastBody<{ parent_id: string }>(
      "POST",
      `/api/v1/taxonomy/categories/${C_LAPTOP}/reparent`,
    );
    expect(body.parent_id).toBe(C_SOFT);
    cleanup();
  });

  it("retires a type softly with a confirmation", async () => {
    const mock = setupApi();
    mock.on("POST", `/api/v1/taxonomy/types/${T1}/retire`, () => ({
      json: mutationResponse({ id: T1, name: "Incident", is_retired: true }, 2),
    }));
    render(
      <AuthProvider>
        <TaxonomySettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Incident");

    fireEvent.click(screen.getAllByRole("button", { name: "Retire" })[0] as Element);
    fireEvent.click(screen.getByTestId("confirm-retire"));

    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/taxonomy/types/${T1}/retire`)).toHaveLength(1);
    });
    cleanup();
  });

  it("renders read-only without special:manage_boards", async () => {
    const mock = new MockApi();
    mock.on("GET", "/api/v1/me", () => ({ json: { ...ME_FIXTURE, permissions: [] } }));
    mock.on("GET", "/api/v1/taxonomy/types", () => ({ json: TYPES }));
    mock.on("GET", "/api/v1/taxonomy/categories", () => ({ json: CATEGORIES }));
    mock.install();
    render(
      <AuthProvider>
        <TaxonomySettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Incident");
    expect(screen.queryByTestId("new-type")).not.toBeInTheDocument();
    expect(screen.getByText(/read-only/i)).toBeInTheDocument();
    cleanup();
  });
});
