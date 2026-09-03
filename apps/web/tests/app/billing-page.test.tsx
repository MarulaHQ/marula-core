// Integration test: /billing — invoice list filtering, invoice-run
// preview + create through the instant-first envelope, and run history.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import BillingPage from "../../app/billing/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/billing",
}));

const INV_ID = "40000000-0000-4000-8000-000000000001";
const RUN_ID = "41000000-0000-4000-8000-000000000001";

/** Run rows the mock serves; the POST handler prepends the new run. */
const runRows: Record<string, unknown>[] = [
  {
    id: RUN_ID,
    org_id: "org",
    number: "RUN-000001",
    period_from: "2026-08-01",
    period_to: "2026-08-31",
    due_in_days: 30,
    invoice_count: 2,
    total_minor: 15000,
    currency: "USD",
    created_at: "2026-09-01T00:00:00Z",
    created_by: null,
  },
];

function invoice(overrides: Record<string, unknown> = {}) {
  return {
    id: INV_ID,
    org_id: "org",
    number: "INV-000001",
    client_id: "c1",
    contract_id: "ct1",
    currency: "USD",
    status: "issued",
    issued_at: "2026-09-01",
    due_on: "2026-10-01",
    total_minor: 15000,
    total_paid_minor: 5000,
    dunning_state: "none",
    dunning_note: null,
    dunning_updated_at: null,
    period_from: "2026-08-01",
    period_to: "2026-08-31",
    run_id: RUN_ID,
    notes: null,
    created_at: "2026-09-01T00:00:00Z",
    created_by: null,
    updated_at: "2026-09-01T00:00:00Z",
    ...overrides,
  };
}

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/invoices", () => ({
    json: [
      invoice(),
      invoice({ id: "40000000-0000-4000-8000-000000000002", number: "INV-000002", status: "paid", dunning_state: "overdue" }),
    ],
  }));
  mock.on("GET", "/api/v1/billing/invoice-runs", () => ({ json: runRows }));
  mock.on("GET", "/api/v1/billing/preview", ({ query }) => ({
    json: {
      from: query.from ?? "2026-08-01",
      to: query.to ?? "2026-08-31",
      total_minor: 42000,
      evaluations: [
        {
          contract_id: "ct1",
          contract_number: "CT-001",
          client_id: "c1",
          contract_type: "tm",
          currency: "USD",
          from: "2026-08-01",
          to: "2026-08-31",
          lines: [],
          total_minor: 42000,
        },
      ],
    },
  }));
  mock.on("POST", "/api/v1/billing/invoice-runs", () => {
    const run = {
      id: RUN_ID,
      org_id: "org",
      number: "RUN-000002",
      period_from: "2026-08-01",
      period_to: "2026-08-31",
      due_in_days: 30,
      invoice_count: 1,
      total_minor: 42000,
      currency: "USD",
      created_at: "2026-09-03T00:00:00Z",
      created_by: null,
    };
    runRows.unshift(run);
    return {
      json: {
        run,
        invoices: [],
        mutation: {
          after: null,
          before: null,
          seq: 7,
          updated_at: "2026-09-03T00:00:00Z",
          version: 7,
          idempotency_key: "k",
        },
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

describe("/billing integration", () => {
  it("lists invoices with status/dunning columns and run history", async () => {
    setupApi();
    render(
      <AuthProvider>
        <BillingPage />
      </AuthProvider>,
    );
    await screen.findByText("INV-000001");
    expect(screen.getByText("INV-000002")).toBeInTheDocument();
    expect(screen.getByText("RUN-000001")).toBeInTheDocument();
    cleanup();
  });

  it("previews a window then issues a run with idempotency key, reconciling from the envelope", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <BillingPage />
      </AuthProvider>,
    );
    await screen.findByText("INV-000001");
    await user.click(screen.getByRole("button", { name: "New invoice run" }));
    await user.click(screen.getByRole("button", { name: /Preview/ }));
    await screen.findByText(/1 billable contract/);
    const fromValue = (screen.getByLabelText("From") as HTMLInputElement).value;
    const toValue = (screen.getByLabelText("To") as HTMLInputElement).value;
    await user.click(screen.getByRole("button", { name: /Issue invoices/ }));

    await waitFor(() => expect(mock.callsTo("POST", "/api/v1/billing/invoice-runs").length).toBe(1));
    const body = mock.lastBody<{ idempotency_key: string; from: string; to: string }>(
      "POST",
      "/api/v1/billing/invoice-runs",
    );
    expect(body.idempotency_key).toBeTruthy();
    expect(body.from).toBe(fromValue);
    expect(body.to).toBe(toValue);
    // The envelope reconciles: the authoritative run number replaces the row.
    await screen.findByText("RUN-000002");
    cleanup();
  });
});
