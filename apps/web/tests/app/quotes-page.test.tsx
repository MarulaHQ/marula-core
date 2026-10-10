// Integration test: /sales/quotes (the canonical ledger, which ends at STATUS
// and carries no per-row actions) and /sales/quotes/[id] (the hand-built
// editor, pack quoting surface state Q13).
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import QuotesPage from "../../app/sales/quotes/page";
import QuoteDetailPage from "../../app/sales/quotes/[id]/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

let currentQuoteId = "q1";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => `/sales/quotes/${currentQuoteId}`,
  useParams: () => ({ id: currentQuoteId }),
}));

const CLIENT = "70000000-0000-4000-8000-000000000001";
const ITEM = "70000000-0000-4000-8000-000000000002";
const INV = "70000000-0000-4000-8000-000000000003";

const client = { id: CLIENT, name: "Acme Corp", display_name: null, notes: null, created_at: "2026-01-01T00:00:00Z" };

const catalogItem = {
  id: ITEM,
  org_id: "org",
  name: "Block hours",
  unit: "hour",
  currency: "USD",
  unit_price_minor: 15000,
  cost_minor: null,
  taxable: true,
  active: true,
  category: "Services",
  group_name: null,
  sku: null,
  description: null,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  created_by: null,
};

const quote = (id: string, status: string, extra: Record<string, unknown> = {}) => ({
  id,
  org_id: "org",
  number: `Q-${id.replace("q", "0").padStart(4, "0")}`,
  client_id: CLIENT,
  status,
  currency: "USD",
  issued_on: "2026-09-01",
  valid_until: "2026-12-31",
  subtotal_minor: 30000,
  tax_rate_bps: 800,
  tax_minor: 2400,
  total_minor: 32400,
  created_at: "2026-09-01T00:00:00Z",
  updated_at: "2026-09-01T00:00:00Z",
  sent_at: null,
  decided_at: null,
  converted_at: null,
  converted_invoice_id: null,
  converted_contract_id: null,
  notes: null,
  terms: null,
  created_by: null,
  ...extra,
});

const line = (extra: Record<string, unknown> = {}) => ({
  id: "line-1",
  org_id: "org",
  quote_id: "q1",
  catalog_item_id: null,
  description: "Labour",
  unit: "hour",
  quantity_milli: 1000,
  unit_amount_minor: 30000,
  amount_minor: 30000,
  taxable: true,
  position: 0,
  ...extra,
});

function setupApi(): MockApi {
  let q3Converted = false;
  const created: Record<string, unknown>[] = [];
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/catalog-items", () => ({ json: [catalogItem] }));
  mock.on("GET", "/api/v1/clients", () => ({ json: [client] }));
  mock.on("GET", "/api/v1/quotes", () => ({
    json: [
      quote("q1", "draft"),
      quote("q2", "sent", { valid_until: "2026-07-01" }),
      quote("q3", "accepted", q3Converted ? { converted_invoice_id: INV } : {}),
      quote("q4", "accepted", { converted_invoice_id: INV }),
      ...created,
    ],
  }));
  mock.on("POST", "/api/v1/quotes", () => ({ json: quote("q9", "draft") }));
  mock.on("POST", "/api/v1/quotes/q3/convert-to-invoice", () => {
    q3Converted = true;
    return {
      json: {
        id: "q3",
        converted_invoice_id: INV,
        mutation: { after: null, before: null, seq: 9, updated_at: new Date().toISOString(), version: 9, idempotency_key: "k" },
      },
    };
  });
  mock.install();
  return mock;
}

/** The detail page reads one quote by id, plus the ledger it refreshes from. */
function setupApiForDetail(id: string = "q1", status: string = "draft"): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/catalog-items", () => ({ json: [catalogItem] }));
  mock.on("GET", "/api/v1/clients", () => ({ json: [client] }));
  mock.on("GET", "/api/v1/stock-items", () => ({ json: [] }));
  mock.on("GET", "/api/v1/quotes", () => ({ json: [quote(id, status)] }));
  mock.on("GET", `/api/v1/quotes/${id}`, () => ({
    json: { ...quote(id, status), lines: [line()] },
  }));
  mock.on("POST", `/api/v1/quotes/${id}/convert-to-invoice`, () => ({
    json: {
      id,
      converted_invoice_id: INV,
      mutation: { after: null, before: null, seq: 9, updated_at: new Date().toISOString(), version: 9, idempotency_key: "k" },
    },
  }));
  mock.on("PATCH", `/api/v1/quotes/${id}`, () => ({
    json: {
      id,
      mutation: { after: null, before: null, seq: 4, updated_at: new Date().toISOString(), version: 4, idempotency_key: "k" },
    },
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  currentQuoteId = "q1";
});

describe("/sales/quotes integration", () => {
  it("renders the ledger with status tags and the converted→invoice link", async () => {
    setupApi();
    render(
      <AuthProvider>
        <QuotesPage />
      </AuthProvider>,
    );
    expect(await screen.findByText("Q-0001")).toBeInTheDocument();
    expect(screen.getByText("Q-0004")).toBeInTheDocument();
    const link = await screen.findByTestId("quote-invoice-link-q4");
    expect(link).toHaveAttribute("href", `/billing/${INV}`);
    cleanup();
  });

  it("carries the shared ledger shell, not a bare view switch", async () => {
    setupApi();
    render(
      <AuthProvider>
        <QuotesPage />
      </AuthProvider>,
    );
    await screen.findByText("Q-0001");
    // The pack's money-cluster toolbar rule: views, filters, columns, export.
    expect(screen.getByRole("button", { name: /columns/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /export/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /add filter/i })).toBeInTheDocument();
    cleanup();
  });

  it("opens the quote document from the row and keeps the ledger action-free", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <QuotesPage />
      </AuthProvider>,
    );
    await screen.findByText("Q-0001");
    const link = await screen.findByTestId("quote-link-q1");
    expect(link).toHaveAttribute("href", "/sales/quotes/q1");
    // The canonical ledger ends at STATUS: no per-row lifecycle buttons remain.
    expect(screen.queryByRole("button", { name: "Send" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Convert to invoice" })).not.toBeInTheDocument();
    await user.click(link);
    cleanup();
  });

  it("previews builder totals with integer math and creates the quote", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <QuotesPage />
      </AuthProvider>,
    );
    await screen.findByText("Q-0001");
    await user.click(screen.getByTestId("quotes-new"));
    await user.click(screen.getByTestId("quote-client"));
    const clientOptions = await screen.findAllByRole("option", { name: "Acme Corp" });
    await user.click(clientOptions[0] as HTMLElement);
    await user.click(await screen.findByTestId("quote-line-item-0"));
    const itemOptions = await screen.findAllByRole("option", { name: "Block hours" });
    await user.click(itemOptions[0] as HTMLElement);
    await screen.findAllByText("$150.00");
    await user.click(screen.getByTestId("quote-save"));
    await waitFor(() => expect(mock.callsTo("POST", "/api/v1/quotes").length).toBe(1));
    expect(body0Unit(mock)).toBe(15000);
    cleanup();
  });
});

describe("/sales/quotes/[id] integration", () => {
  it("shows the hand-built editor: the summary, and no AI marks or tabs", async () => {
    setupApiForDetail("q1", "draft");
    render(
      <AuthProvider>
        <QuoteDetailPage />
      </AuthProvider>,
    );
    expect(await screen.findByTestId("quote-detail-status")).toBeInTheDocument();
    expect(screen.getByText("Summary")).toBeInTheDocument();
    // The number is in the breadcrumb and the title, so assert the title once.
    expect(screen.getAllByText("Q-0001").length).toBeGreaterThan(0);
    // Q13's rule: no provenance dots, no assumed highlighting, no source tabs.
    expect(screen.queryByText(/from notes/i)).not.toBeInTheDocument();
    expect(screen.queryByRole("tab")).not.toBeInTheDocument();
    // The two manual controls are always there.
    expect(screen.getByTestId("quote-add-item")).toBeInTheDocument();
    expect(screen.getByTestId("quote-from-catalogue")).toBeInTheDocument();
    cleanup();
  });

  it("saves a draft's lines through the envelope as a full replace", async () => {
    const mock = setupApiForDetail("q1", "draft");
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <QuoteDetailPage />
      </AuthProvider>,
    );
    await screen.findByTestId("quote-save");
    await user.clear(screen.getByLabelText("Quantity"));
    await user.type(screen.getByLabelText("Quantity"), "3");    await user.click(screen.getByTestId("quote-save"));
    await waitFor(() => expect(mock.callsTo("PATCH", "/api/v1/quotes/q1").length).toBe(1));
    expect(
      mock.lastBody<{ lines: { quantity_milli: number }[] }>("PATCH", "/api/v1/quotes/q1").lines[0],
    ).toMatchObject({ quantity_milli: 3000 });
    cleanup();
  });

  it("refuses to edit a quote that has left draft", async () => {
    currentQuoteId = "q3";
    setupApiForDetail("q3", "accepted");
    render(
      <AuthProvider>
        <QuoteDetailPage />
      </AuthProvider>,
    );
    expect(await screen.findByTestId("quote-convert")).toBeInTheDocument();
    expect(screen.queryByTestId("quote-save")).not.toBeInTheDocument();
    expect(screen.queryByTestId("quote-add-item")).not.toBeInTheDocument();
    cleanup();
  });

  it("converts an accepted quote to an invoice through the envelope", async () => {
    currentQuoteId = "q3";
    const mock = setupApiForDetail("q3", "accepted");
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <QuoteDetailPage />
      </AuthProvider>,
    );
    await screen.findByTestId("quote-detail-status");
    await user.click(screen.getByTestId("quote-convert"));
    await waitFor(() =>
      expect(mock.callsTo("POST", "/api/v1/quotes/q3/convert-to-invoice").length).toBe(1),
    );
    await screen.findByTestId("quote-detail-note");
    cleanup();
  });
});

function body0Unit(mock: MockApi): number {
  const body = mock.lastBody<{ lines: { unit_amount_minor: number }[] }>("POST", "/api/v1/quotes");
  return body.lines[0]?.unit_amount_minor ?? 0;
}
