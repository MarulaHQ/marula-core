// Integration test: /contracts/[id] — the contract record. Covers the
// record composition (DESIGN-NOTES-clients §7: the contract card becomes
// the page, the ledger is its invoices) and the hard truth law: the
// invoices tab reads the REAL invoices endpoint, and nothing invents an
// invoice. Wave 44 lane 4 (p44-ui-crm).
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import ContractDetailPage from "../../app/contracts/[id]/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/contracts/x",
  useParams: () => ({ id: CONTRACT_ID }),
}));

const ORG = ME_FIXTURE.org_id;
const CLIENT_ID = "00000000-0000-4000-8000-0000000000c1";
const CONTRACT_ID = "00000000-0000-4000-8000-0000000000k1";
const INV_ISSUED = "00000000-0000-4000-8000-0000000000i1";
const INV_PAID = "00000000-0000-4000-8000-0000000000i2";

const CLIENT = {
  id: CLIENT_ID,
  org_id: ORG,
  name: "Acme Corporation",
  restricted_to: [],
  created_at: "2021-03-14T09:00:00Z",
  updated_at: "2026-09-30T09:00:00Z",
};

const CONTRACT = {
  id: CONTRACT_ID,
  org_id: ORG,
  client_id: CLIENT_ID,
  number: "CTR-2026-04",
  name: "Managed Gold",
  contract_type: "recurring",
  status: "active",
  currency: "USD",
  amount_minor: 386_000,
  billing_frequency: "monthly",
  billing_anchor: null,
  effective_from: "2026-01-01",
  effective_until: null,
  po_number: null,
  hourly_rate_minor: null,
  included_minutes: null,
  overage_rate_minor: null,
  unit_price_minor: null,
  unit_basis: null,
  incident_price_minor: null,
  incident_priority: null,
  minimum_commitment_minor: null,
  escalator: null,
  notes: null,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  rates: [],
  fees: [],
  blocks: [],
  caps: [],
  change_orders: [],
  sla_credits: [],
};

const INVOICES = [
  {
    id: INV_ISSUED,
    org_id: ORG,
    client_id: CLIENT_ID,
    contract_id: CONTRACT_ID,
    number: "INV-000042",
    status: "issued",
    currency: "USD",
    total_minor: 386_000,
    total_paid_minor: 0,
    due_on: "2026-09-10",
    issued_at: "2026-08-11",
    period_from: "2026-08-01",
    period_to: "2026-08-31",
    dunning_state: "none",
    created_at: "2026-08-11T00:00:00Z",
    updated_at: "2026-08-11T00:00:00Z",
  },
  {
    id: INV_PAID,
    org_id: ORG,
    client_id: CLIENT_ID,
    contract_id: CONTRACT_ID,
    number: "INV-000041",
    status: "paid",
    currency: "USD",
    total_minor: 386_000,
    total_paid_minor: 386_000,
    due_on: "2026-08-10",
    issued_at: "2026-07-11",
    period_from: "2026-07-01",
    period_to: "2026-07-31",
    dunning_state: "none",
    created_at: "2026-07-11T00:00:00Z",
    updated_at: "2026-07-20T00:00:00Z",
  },
];

function setupApi(invoices: unknown[] = INVOICES): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", `/api/v1/contracts/${CONTRACT_ID}`, () => ({ json: CONTRACT }));
  mock.on("GET", "/api/v1/clients", () => ({ json: [CLIENT] }));
  mock.on("GET", "/api/v1/invoices", () => ({ json: invoices }));
  return mock.install();
}

beforeEach(() => {
  seedAuthTokens();
});

function mount() {
  return render(
    <AuthProvider>
      <ContractDetailPage />
    </AuthProvider>,
  );
}

describe("ContractDetailPage", () => {
  it("carries the record's tab set under the title", async () => {
    setupApi();
    mount();
    const title = await screen.findByRole("heading", { level: 1 });
    expect(title).toHaveTextContent("Managed Gold");
    expect(screen.getByTestId("record-tab-overview")).toHaveTextContent("Overview");
    expect(screen.getByTestId("record-tab-agreement")).toHaveTextContent("Agreement");
    expect(screen.getByTestId("record-tab-invoices")).toHaveTextContent("Invoices");
    // The title is the contract's name, the number rides quietly beside it.
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Managed Gold");
    // The number shows on the header and in the customer column.
    expect(screen.getAllByText("CTR-2026-04").length).toBeGreaterThan(0);
    cleanup();
  });

  it("the Invoices tab reads the REAL ledger, scoped to this contract", async () => {
    const mock = setupApi();
    mount();
    fireEvent.click(await screen.findByTestId("record-tab-invoices"));
    const ledger = await screen.findByTestId("contract-invoice-ledger");
    expect(within(ledger).getByText("INV-000042")).toBeInTheDocument();
    expect(within(ledger).getByText("INV-000041")).toBeInTheDocument();
    // The real statuses: the paid one is paid because the read model said
    // so, not because a contract badge was relabelled.
    expect(within(ledger).getByText("Paid")).toBeInTheDocument();
    expect(within(ledger).getAllByText("$3,860.00")).toHaveLength(2);
    const calls = mock.callsTo("GET", "/api/v1/invoices");
    expect(calls.length).toBeGreaterThan(0);
    expect(calls[0]!.query.contract_id).toBe(CONTRACT_ID);
    cleanup();
  });

  it("a contract with no invoices yet says so in words", async () => {
    setupApi([]);
    mount();
    fireEvent.click(await screen.findByTestId("record-tab-invoices"));
    expect(
      await screen.findByText("No invoices yet. An invoice run over the contract's billing window issues the first one, and every issued invoice appears here with its real status."),
    ).toBeInTheDocument();
    cleanup();
  });

  it("the agreement editors stay on the Agreement tab", async () => {
    setupApi();
    mount();
    fireEvent.click(await screen.findByTestId("record-tab-agreement"));
    // Every editor the record always carried is still reachable: the
    // agreement form with its save, the change-order queue on Overview.
    expect(await screen.findByRole("button", { name: /Save changes/ })).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("record-tab-overview"));
    expect(await screen.findByTestId("contract-change-orders")).toBeInTheDocument();
    expect(screen.getByTestId("contract-sla-credits")).toBeInTheDocument();
    cleanup();
  });
});
