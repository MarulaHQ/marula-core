// Integration test: /clients/[id] — the record template. Covers the hard
// truth law this lane enforces: the billing ledger and summary read the
// REAL invoices endpoint (a contract is never relabelled a paid invoice),
// the site choice filters the device table, and the Tickets tab reads the
// client's real tickets. Wave 44 lane 4 (p44-ui-clients).
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import ClientDetailPage from "../../app/clients/[id]/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/clients/x",
  useParams: () => ({ id: CLIENT_ID }),
  useSearchParams: () => new URLSearchParams(),
}));

const ORG = ME_FIXTURE.org_id;
const CLIENT_ID = "00000000-0000-4000-8000-0000000000c1";
const SITE_A = "00000000-0000-4000-8000-0000000000s1";
const SITE_B = "00000000-0000-4000-8000-0000000000s2";
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

const SITES = [
  { id: SITE_A, org_id: ORG, client_id: CLIENT_ID, name: "Head office", city: null, address: null, created_at: "2024-01-01T00:00:00Z", updated_at: "2024-01-01T00:00:00Z" },
  { id: SITE_B, org_id: ORG, client_id: CLIENT_ID, name: "Warehouse", city: null, address: null, created_at: "2024-01-01T00:00:00Z", updated_at: "2024-01-01T00:00:00Z" },
];

const CONTACTS = [
  { id: "00000000-0000-4000-8000-0000000000p1", org_id: ORG, client_id: CLIENT_ID, name: "Priya Naidoo", email: "priya@acme.test", phone: null, title: "IT lead", is_primary: true, created_at: "", updated_at: "" },
];

/** Two assets at the head office — one of them offline — and two at the
 * warehouse. The site choice must filter the table to the chosen site. */
const ASSETS = [
  { id: "00000000-0000-4000-8000-0000000000a1", org_id: ORG, client_id: CLIENT_ID, site_id: SITE_A, name: "Exchange server", kind: "server", tag: "EXCH-01", status: "active", created_at: "", updated_at: "" },
  { id: "00000000-0000-4000-8000-0000000000a2", org_id: ORG, client_id: CLIENT_ID, site_id: SITE_A, name: "Backup laptop", kind: "laptop", tag: "LAP-07", status: "offline", created_at: "", updated_at: "" },
  { id: "00000000-0000-4000-8000-0000000000a3", org_id: ORG, client_id: CLIENT_ID, site_id: SITE_B, name: "Warehouse PC", kind: "desktop", tag: "WS-01", status: "active", created_at: "", updated_at: "" },
  { id: "00000000-0000-4000-8000-0000000000a4", org_id: ORG, client_id: CLIENT_ID, site_id: SITE_B, name: "Handheld scanner", kind: "scanner", tag: "SC-22", status: "active", created_at: "", updated_at: "" },
];

const TICKETS = [
  {
    id: "00000000-0000-4000-8000-0000000000t1",
    org_id: ORG,
    client_id: CLIENT_ID,
    board_id: "00000000-0000-4000-8000-0000000000b1",
    number: "18432",
    title: "Mail flow broken",
    priority: "p2",
    category: "in_progress",
    status_id: "00000000-0000-4000-8000-0000000000st1",
    due_at: null,
    created_at: "2026-09-20T08:00:00Z",
    updated_at: "2026-10-01T08:00:00Z",
    custom_fields: [],
  },
  {
    id: "00000000-0000-4000-8000-0000000000t2",
    org_id: ORG,
    client_id: CLIENT_ID,
    board_id: "00000000-0000-4000-8000-0000000000b1",
    number: "18504",
    title: "New starter accounts",
    priority: "p3",
    category: "open",
    status_id: "00000000-0000-4000-8000-0000000000st2",
    due_at: null,
    created_at: "2026-09-28T08:00:00Z",
    updated_at: "2026-09-29T08:00:00Z",
    custom_fields: [],
  },
];

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
  created_at: "",
  updated_at: "",
};

/** Two real invoices: one issued and past its due date, one paid. The
 * ledger renders the statuses the read model returned — and never derives
 * "Paid" from a contract's status. */
const pastDue = new Date();
pastDue.setDate(pastDue.getDate() - 9);
/** A date-only string built from the date's own local calendar parts, which is
 * what `setDate` above subtracted. Slicing `toISOString()` prints the UTC date
 * instead, and that disagrees with `setDate` for the hours where the UTC date
 * still lags the local one — the overdue age then reads a day high. */
const iso = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
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
    due_on: iso(pastDue),
    issued_at: iso(pastDue),
    period_from: "2026-09-01",
    period_to: "2026-09-30",
    dunning_state: "none",
    created_at: `${iso(pastDue)}T00:00:00Z`,
    updated_at: `${iso(pastDue)}T00:00:00Z`,
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
    due_on: "2026-09-10",
    issued_at: "2026-08-11",
    period_from: "2026-08-01",
    period_to: "2026-08-31",
    dunning_state: "none",
    created_at: "2026-08-11T00:00:00Z",
    updated_at: "2026-08-20T00:00:00Z",
  },
];

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", `/api/v1/clients/${CLIENT_ID}`, () => ({ json: CLIENT }));
  mock.on("GET", `/api/v1/clients/${CLIENT_ID}/sites`, () => ({ json: SITES }));
  mock.on("GET", `/api/v1/clients/${CLIENT_ID}/contacts`, () => ({ json: CONTACTS }));
  mock.on("GET", `/api/v1/clients/${CLIENT_ID}/assets`, () => ({ json: ASSETS }));
  mock.on("GET", `/api/v1/clients/${CLIENT_ID}/portal-users`, () => ({ json: [] }));
  mock.on("GET", `/api/v1/clients/${CLIENT_ID}/completeness`, () => ({
    json: { score: 80, basis: "contacts, sites and documents on record", gaps: [] },
  }));
  mock.on("GET", "/api/v1/documents", () => ({ json: [] }));
  mock.on("GET", "/api/v1/documents/layouts", () => ({ json: [] }));
  mock.on("GET", "/api/v1/tickets", () => ({ json: TICKETS }));
  mock.on("GET", "/api/v1/contracts", () => ({ json: [CONTRACT] }));
  mock.on("GET", "/api/v1/invoices", () => ({ json: INVOICES }));
  return mock.install();
}

beforeEach(() => {
  seedAuthTokens();
});

function mount() {
  return render(
    <AuthProvider>
      <ClientDetailPage />
    </AuthProvider>,
  );
}

/** The record's title. The breadcrumb carries the same name, so a text
 * query matches twice — the heading is the title and only the title. */
function loaded() {
  return screen.findByRole("heading", { level: 1 });
}

/** Open one of the record's tabs, waiting for the record to load first. */
function openTab(id: string) {
  return (async () => {
    await loaded();
    fireEvent.click(screen.getByTestId(`record-tab-${id}`));
  })();
}

describe("ClientDetailPage", () => {
  it("keeps the pack's tab set with real counts, Tickets included", async () => {
    setupApi();
    mount();
    await loaded();
    expect(await loaded()).toHaveTextContent("Acme Corporation");
    for (const id of ["overview", "contacts", "sites", "contracts", "tickets", "assets", "documents"]) {
      expect(screen.getByTestId(`record-tab-${id}`)).toBeInTheDocument();
    }
    expect(screen.getByTestId("record-tab-tickets")).toHaveTextContent("Tickets");
    expect(screen.getByTestId("record-tab-tickets")).toHaveTextContent("2");
    cleanup();
  });

  it("bills history from the REAL invoices endpoint, never a relabelled contract", async () => {
    const mock = setupApi();
    mount();
    await openTab("contracts");
    const ledger = await screen.findByTestId("client-billing-history");
    // The two real invoices, with their real numbers and statuses.
    expect(within(ledger).getByText("INV-000042")).toBeInTheDocument();
    expect(within(ledger).getByText("INV-000041")).toBeInTheDocument();
    expect(within(ledger).getByText("Paid")).toBeInTheDocument();
    // The issued invoice is past due: blush with its clock and the number
    // of days, never colour alone.
    expect(within(ledger).getByText(/d late/)).toBeInTheDocument();
    // Both invoices carry the contract's real per-period figure.
    expect(within(ledger).getAllByText("$3,860.00")).toHaveLength(2);
    // The request was scoped to this client by the endpoint's own filter.
    const calls = mock.callsTo("GET", "/api/v1/invoices");
    expect(calls.length).toBeGreaterThan(0);
    expect(calls[0]!.query.client_id).toBe(CLIENT_ID);
    cleanup();
  });

  it("the billing summary shows the real balance due and overdue age", async () => {
    setupApi();
    mount();
    const billing = await screen.findByTestId("summary-card-billing");
    expect(within(billing).getByText("Balance due")).toBeInTheDocument();
    expect(within(billing).getByText("$3,860.00")).toBeInTheDocument();
    expect(within(billing).getByText(/9d/)).toBeInTheDocument();
    expect(within(billing).getByText("INV-000042")).toBeInTheDocument();
    cleanup();
  });

  it("the site choice filters the device table and counts per site", async () => {
    setupApi();
    mount();
    await openTab("sites");
    const devices = await screen.findByTestId("client-devices-card");
    // The head office is the default choice: its own devices only.
    expect(within(devices).getByText("EXCH-01")).toBeInTheDocument();
    expect(within(devices).getByText("LAP-07")).toBeInTheDocument();
    expect(within(devices).queryByText("WS-01")).toBeNull();
    expect(within(devices).getByText("Showing 2 of 2")).toBeInTheDocument();
    // The choice itself carries that site's real counts, offline included.
    const choiceB = await screen.findByTestId(`site-choice-${SITE_B}`);
    expect(choiceB).toHaveTextContent("2 devices");
    fireEvent.click(choiceB);
    await waitFor(() => {
      expect(within(screen.getByTestId("client-devices-card")).getByText("WS-01")).toBeInTheDocument();
    });
    expect(within(screen.getByTestId("client-devices-card")).queryByText("EXCH-01")).toBeNull();
    // The left column counts each site on its own, with the offline figure.
    const context = screen.getByTestId("record-context");
    expect(within(context).getByText("Head office")).toBeInTheDocument();
    expect(within(context).getByText(/2 devices · 1 offline/)).toBeInTheDocument();
    expect(within(context).getByText("Warehouse")).toBeInTheDocument();
    cleanup();
  });

  it("the Tickets tab lists the client's real tickets", async () => {
    setupApi();
    mount();
    await openTab("tickets");
    const card = await screen.findByTestId("client-tickets-card");
    expect(within(card).getByText("18432")).toBeInTheDocument();
    expect(within(card).getByText("18504")).toBeInTheDocument();
    expect(within(card).getByText("Mail flow broken")).toBeInTheDocument();
    expect(within(card).getByText("In progress")).toBeInTheDocument();
    expect(within(card).getByText("Open")).toBeInTheDocument();
    cleanup();
  });

  it("an unreadable invoices endpoint leaves the ledger honest, not invented", async () => {
    const mock = new MockApi();
    mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
    mock.on("GET", `/api/v1/clients/${CLIENT_ID}`, () => ({ json: CLIENT }));
    mock.on("GET", `/api/v1/clients/${CLIENT_ID}/sites`, () => ({ json: SITES }));
    mock.on("GET", `/api/v1/clients/${CLIENT_ID}/contacts`, () => ({ json: CONTACTS }));
    mock.on("GET", `/api/v1/clients/${CLIENT_ID}/assets`, () => ({ json: ASSETS }));
    mock.on("GET", `/api/v1/clients/${CLIENT_ID}/portal-users`, () => ({ json: [] }));
    mock.on("GET", `/api/v1/clients/${CLIENT_ID}/completeness`, () => ({ status: 403, json: { error: { message: "forbidden" } } }));
    mock.on("GET", "/api/v1/documents", () => ({ json: [] }));
    mock.on("GET", "/api/v1/documents/layouts", () => ({ json: [] }));
    mock.on("GET", "/api/v1/tickets", () => ({ json: TICKETS }));
    mock.on("GET", "/api/v1/contracts", () => ({ json: [CONTRACT] }));
    mock.on("GET", "/api/v1/invoices", () => ({ status: 403, json: { error: { message: "forbidden" } } }));
    mock.install();
    mount();
    await openTab("contracts");
    const denied = "Invoices are not available in your scope.";
    // The ledger says it...
    await screen.findByTestId("client-billing-history");
    expect(
      within(screen.getByTestId("client-billing-history")).getAllByText(denied).length,
    ).toBeGreaterThan(0);
    // ...and the summary column is honest the same way, rather than
    // inventing a figure in place of the ledger.
    expect(
      within(screen.getByTestId("summary-card-billing")).getAllByText(denied).length,
    ).toBeGreaterThan(0);
    cleanup();
  });
});
