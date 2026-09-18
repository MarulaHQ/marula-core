// Integration test: /sales/quotes — status filter rail, builder totals
// preview (integer minor-unit math), and lifecycle/convert through the
// envelope with the converted→invoice link.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import QuotesPage from "../../app/sales/quotes/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/sales/quotes",
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

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
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

  it("previews builder totals with integer math and creates the quote", async () => {
    const mock = setupApi();
    const user = userEvent.setup({ pointerEventsCheck: 0 });
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
    // Catalog snapshot: picking the item fills price 150.00, taxable.
    await user.click(await screen.findByTestId("quote-line-item-0"));
    const itemOptions = await screen.findAllByRole("option", { name: "Block hours" });
    await user.click(itemOptions[0] as HTMLElement);
    await screen.findAllByText("$150.00");
    await user.click(screen.getByTestId("quote-save"));
    await waitFor(() => expect(mock.callsTo("POST", "/api/v1/quotes").length).toBe(1));
    const body = mock.lastBody<{ lines: { unit_amount_minor: number }[]; idempotency_key: string }>("POST", "/api/v1/quotes");
    expect(body.idempotency_key).toBeTruthy();
    expect(body.lines[0]?.unit_amount_minor).toBe(15000);
    cleanup();
  });

  it("converts an accepted quote to an invoice through the envelope", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <QuotesPage />
      </AuthProvider>,
    );
    await screen.findByText("Q-0003");
    const convertButtons = screen.getAllByRole("button", { name: "Convert to invoice" });
    await user.click(convertButtons[0] as HTMLElement); // first accepted row = q3
    await waitFor(() =>
      expect(mock.callsTo("POST", "/api/v1/quotes/q3/convert-to-invoice").length).toBe(1),
    );
    // Reconciled list re-renders with the invoice link on the converting row.
    await screen.findByTestId("quote-invoice-link-q3");
    cleanup();
  });
});
