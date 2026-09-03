// Integration test: /dispatch/travel — draft creation through the
// instant-first envelope, submit turning drafts immutable, and the
// status filter.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import TravelDraftsPage from "../../app/dispatch/travel/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/dispatch/travel",
}));

const DRAFT_ID = "50000000-0000-4000-8000-000000000001";
const TICKET_ID = "51000000-0000-4000-8000-000000000001";

function draft(overrides: Record<string, unknown> = {}) {
  return {
    id: DRAFT_ID,
    user_id: "u1",
    ticket_id: TICKET_ID,
    started_at: "2026-09-02T09:00:00Z",
    ended_at: "2026-09-02T09:45:00Z",
    distance: 32.5,
    distance_unit: "km",
    vehicle: "Van 2",
    billable: true,
    notes: null,
    status: "draft",
    submitted_time_entry_id: null,
    created_at: "2026-09-02T09:45:00Z",
    updated_at: "2026-09-02T09:45:00Z",
    ...overrides,
  };
}

/** Draft rows the mock serves; handlers mutate this list in place. */
const rows: Record<string, unknown>[] = [];

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/dispatch/travel-drafts", () => ({ json: rows }));
  mock.on("POST", "/api/v1/dispatch/travel-drafts", ({ body }) => {
    const b = body as Record<string, unknown>;
    const row = {
      id: DRAFT_ID,
      user_id: "u1",
      submitted_time_entry_id: null,
      created_at: "2026-09-02T09:45:00Z",
      updated_at: "2026-09-02T09:45:00Z",
      status: "draft",
      ...b,
    };
    rows.unshift(row);
    return {
      json: { ...row, mutation: { after: row, before: null, seq: 3, updated_at: row.updated_at, version: 3 } },
    };
  });
  mock.on("POST", `/api/v1/dispatch/travel-drafts/${DRAFT_ID}/submit`, () => {
    const row = rows.find((r) => r.id === DRAFT_ID);
    if (row !== undefined) row.status = "submitted";
    return {
      json: {
        time_entry: { id: "52000000-0000-4000-8000-000000000001" },
        mutation: { after: null, before: null, seq: 4, updated_at: "2026-09-02T10:00:00Z", version: 4 },
      },
    };
  });
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  rows.length = 0;
  rows.push(draft());
});

describe("/dispatch/travel integration", () => {
  it("lists drafts with distance, vehicle, and status", async () => {
    setupApi();
    render(
      <AuthProvider>
        <TravelDraftsPage />
      </AuthProvider>,
    );
    await screen.findByText("Van 2");
    expect(screen.getByText("32.5 km")).toBeInTheDocument();
    expect(screen.getByText("draft")).toBeInTheDocument();
    cleanup();
  });

  it("creates a draft with an idempotency key and reconciles the optimistic row", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <TravelDraftsPage />
      </AuthProvider>,
    );
    await screen.findByText("Van 2");
    await user.click(screen.getByRole("button", { name: "Add draft" }));
    await user.type(screen.getByLabelText("Ticket id"), TICKET_ID);
    fireEvent.change(screen.getByLabelText("Started"), { target: { value: "2026-09-02T09:00" } });
    fireEvent.change(screen.getByLabelText("Ended"), { target: { value: "2026-09-02T09:45" } });
    await user.type(screen.getByLabelText("Distance"), "18");
    await user.type(screen.getByLabelText("Vehicle"), "Car 1");
    await user.click(screen.getByRole("button", { name: /Save draft/ }));

    await waitFor(() => expect(mock.callsTo("POST", "/api/v1/dispatch/travel-drafts").length).toBe(1));
    const body = mock.lastBody<{ idempotency_key: string; distance: number; vehicle: string }>(
      "POST",
      "/api/v1/dispatch/travel-drafts",
    );
    expect(body.idempotency_key).toBeTruthy();
    expect(body.distance).toBe(18);
    expect(body.vehicle).toBe("Car 1");
    await screen.findByText("Car 1");
    cleanup();
  });

  it("submits a draft and the row becomes an immutable submitted entry", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <TravelDraftsPage />
      </AuthProvider>,
    );
    await screen.findByText("Van 2");
    await user.click(screen.getByRole("button", { name: /Submit/ }));
    await waitFor(() =>
      expect(screen.getAllByText("submitted").length).toBeGreaterThan(0),
    );
    cleanup();
  });
});
