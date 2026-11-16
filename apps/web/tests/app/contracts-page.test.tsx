// /contracts — the ledger, the split New control, the describe door, and the
// renewal path into a prefilled panel.
//
// Purpose: pin the contract-specific rules. A renewal nearing its end is blush
// and never red; the Describe door and its menu item are absent without the
// grant; a renewal from a contract opens the panel already holding the current
// contract's words; and the clause column keeps its legal-review sentence with
// no grant, because a changed clause needs review whoever wrote it.
//
// Non-purpose: the record page (`/contracts/[id]`), which has its own suite.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { AuthProvider } from "../../lib/api";
import ContractsPage from "../../app/contracts/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/contracts",
  useParams: () => ({}),
}));

const CLIENT = "70000000-0000-4000-8000-000000000001";

const client = {
  id: CLIENT,
  name: "Acme Corporation",
  display_name: null,
  notes: null,
  created_at: "2026-01-01T00:00:00Z",
};

const contract = (id: string, patch: Record<string, unknown> = {}) => ({
  id,
  org_id: "org",
  client_id: CLIENT,
  number: `C-${id}`,
  name: "Managed Gold",
  contract_type: "recurring",
  status: "active",
  currency: "USD",
  effective_from: "2026-01-01",
  effective_until: "2026-12-12",
  billing_frequency: "monthly",
  billing_anchor: null,
  amount_minor: 940_000,
  hourly_rate_minor: null,
  included_minutes: null,
  overage_rate_minor: null,
  notes: null,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  ...patch,
});

const CLAUSES = [
  {
    clause_id: "0c100000-0000-0000-0000-000000000005",
    key: "term",
    title: "Term and renewal",
    body: "This agreement starts on the start date in the schedule.",
    version: 1,
    status: "standard",
    review_required: false,
    parameter_label: "Term",
    parameter_value: null,
    standard_value: "12 months",
  },
];

/** The draft the author route hands back: a one- or twelve-month contract. */
function authorDraft(over: Record<string, unknown> = {}) {
  return {
    name: "Managed Silver, Fernwood Pharmacy",
    contract_type: "recurring",
    billing_frequency: "monthly",
    value_minor: 62_400,
    currency: "USD",
    term_months: 12,
    effective_from: "2026-11-01",
    notes: null,
    clauses: ["Term and renewal"],
    clause_refs: [CLAUSES[0]],
    pricing: [],
    client_id: CLIENT,
    client_name: "Acme Corporation",
    assumptions: [],
    understanding: { statement: "", facts: [], questions: [], assumptions: [] },
    checks: [],
    ...over,
  };
}

function authorResponse(over: Record<string, unknown> = {}) {
  return {
    json: {
      valid: true,
      saved: false,
      draft: authorDraft(over),
      assumptions: [],
      issues: [],
      usage: { prompt_tokens: 1, completion_tokens: 1 },
      model_info: { provider: "test", model: "test" },
    },
  };
}

function setup(options: { granted?: boolean; contracts?: unknown[] } = {}): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/ai/me/surfaces", () => ({
    json: {
      surfaces: [
        {
          surface_key: "contract_authoring",
          granted: options.granted ?? true,
          models_available: options.granted ?? true,
        },
      ],
    },
  }));
  mock.on("GET", "/api/v1/contracts", () => ({ json: options.contracts ?? [contract("101")] }));
  mock.on("GET", "/api/v1/clients", () => ({ json: [client] }));
  mock.on("GET", "/api/v1/contracts/clauses", () => ({ json: CLAUSES }));
  mock.on("POST", "/api/v1/contracts/extract", () => ({ json: { fields: [], characters: 0 } }));
  mock.install();
  return mock;
}

beforeEach(() => {
  cleanup();
  window.localStorage.clear();
  seedAuthTokens();
  window.history.replaceState({}, "", "/contracts");
});

describe("/contracts", () => {
  it("renders the ledger with the renewal signal as a blush pill", async () => {
    // A term inside the renewal window, counted from the clock the ledger uses,
    // so the assertion does not depend on the day the suite runs.
    const endsIn = (days: number): string => {
      const d = new Date();
      d.setUTCHours(0, 0, 0, 0);
      d.setUTCDate(d.getUTCDate() + days);
      return d.toISOString().slice(0, 10);
    };
    setup({ contracts: [contract("101", { effective_until: endsIn(20) })] });
    render(
      <AuthProvider>
        <ContractsPage />
      </AuthProvider>,
    );
    expect(await screen.findByText("C-101")).toBeInTheDocument();
    const row = await screen.findByTestId("contract-row-101");
    expect(row).toBeInTheDocument();
    // The signal is required and it is in words, not colour alone.
    const label = await within(row).findByText(/\d+ days left/);
    const pill = label.closest('[data-slot="pill"]');
    expect(pill).not.toBeNull();
    // A renewal inside its window is the due pair (blush), never the danger
    // pair: red is a breach, a destructive action or a P1.
    expect(pill?.className).toContain("bg-mc-attn");
    expect(pill?.className).not.toContain("bg-mc-st-breach");
  });

  it("offers Describe it as a third way to start when the grant exists", async () => {
    setup();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ContractsPage />
      </AuthProvider>,
    );
    expect(await screen.findByTestId("contracts-describe")).toBeInTheDocument();
    await user.click(await screen.findByTestId("new-contract-menu"));
    expect(await screen.findByTestId("new-contract-describe")).toBeInTheDocument();
    expect(screen.getByTestId("new-contract-blank")).toBeInTheDocument();
  });

  it("has no door and no menu item without the grant", async () => {
    setup({ granted: false });
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ContractsPage />
      </AuthProvider>,
    );
    // Wait for the surfaces answer to settle before asserting absence.
    await waitFor(() => expect(screen.queryByTestId("contracts-describe")).not.toBeInTheDocument());
    await user.click(await screen.findByTestId("new-contract-menu"));
    expect(screen.queryByTestId("new-contract-describe")).not.toBeInTheDocument();
    expect(screen.queryByText(/Describe it/i)).not.toBeInTheDocument();
  });

  it("builds a draft with clauses from the library and flags the changed one", async () => {
    const mock = setup();
    mock.on("POST", "/api/v1/contracts/author", () => ({
      json: {
        valid: true,
        saved: false,
        draft: {
          name: "Managed Silver, Fernwood Pharmacy",
          contract_type: "recurring",
          billing_frequency: "monthly",
          value_minor: 62_400,
          currency: "USD",
          term_months: 12,
          effective_from: "2026-11-01",
          notes: null,
          clauses: ["Term and renewal", "Notice to cancel or change"],
          clause_refs: [
            {
              clause_id: "0c100000-0000-0000-0000-000000000005",
              key: "term",
              title: "Term and renewal",
              body: "This agreement starts on the start date in the schedule.",
              version: 1,
              status: "standard",
              review_required: false,
              parameter_label: "Term",
              parameter_value: null,
              standard_value: "12 months",
            },
            {
              clause_id: "0c100000-0000-0000-0000-000000000006",
              key: "renewal_notice",
              title: "Notice to cancel or change",
              body: "Either party may end this agreement at the end of a term.",
              version: 1,
              status: "variant",
              review_required: true,
              parameter_label: "Notice period",
              parameter_value: "60 days",
              standard_value: "90 days",
            },
          ],
          pricing: [
            { label: "Users", quantity_milli: 12_000, unit_price_minor: 3800 },
            { label: "Devices", quantity_milli: 14_000, unit_price_minor: 1200 },
          ],
          client_id: CLIENT,
          client_name: "Acme Corporation",
          assumptions: [],
          understanding: {
            statement: "A 12 month Managed Silver contract for Acme Corporation.",
            facts: [{ label: "Template", value: "Managed Silver, version 4" }],
            questions: [
              {
                id: "signer",
                text: "Who signs for Acme Corporation?",
                options: ["Lindiwe Dlamini (owner)", "Someone else"],
                chosen: "Lindiwe Dlamini (owner)",
              },
            ],
            assumptions: [
              "I wrote no new legal wording. Every clause is from your approved library.",
              "Your 60 day notice changes the standard notice period, so that clause is marked for legal review.",
            ],
          },
          checks: ["Notice to cancel or change differs from the standard wording. Legal review is advised."],
        },
        assumptions: [],
        issues: [],
        usage: { prompt_tokens: 1, completion_tokens: 1 },
        model_info: { provider: "test", model: "test" },
      },
    }));
    mock.install();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ContractsPage />
      </AuthProvider>,
    );
    await screen.findByText("C-101");
    await user.click(screen.getByTestId("contracts-describe"));
    await user.type(
      await screen.findByTestId("describe-transcript"),
      "Managed Silver for Acme, 12 months, 12 users and 14 devices",
    );
    await user.click(screen.getByTestId("describe-draft"));
    // The check step for a contract says it writes no legal wording, and its
    // one question already has the likeliest answer chosen.
    await screen.findByTestId("describe-understanding");
    expect(await screen.findByText(/I wrote no new legal wording/i)).toBeInTheDocument();
    expect(
      screen.getByTestId("describe-answer-signer-Lindiwe Dlamini (owner)"),
    ).toHaveAttribute("aria-pressed", "true");
    await user.click(screen.getByTestId("describe-build"));

    // The draft: pricing from quantities, clauses from the library, one flagged.
    expect(await screen.findByTestId("contract-pricing")).toBeInTheDocument();
    expect(screen.getByText("Users")).toBeInTheDocument();
    expect(screen.getByText("Devices")).toBeInTheDocument();
    expect(await screen.findByText("Changed: needs review")).toBeInTheDocument();
    expect(screen.getByText("Standard")).toBeInTheDocument();
    expect(screen.getByTestId("contract-review")).toHaveTextContent(/Legal review is advised/i);
    // The value is computed from the quantities, not stated by the model.
    expect(screen.getByTestId("contract-total")).toHaveTextContent("624.00");
    expect(mock.callsTo("POST", "/api/v1/contracts")).toHaveLength(0);
  });

  it("treats a cleared end date as open-ended, in the summary and in the payload", async () => {
    const mock = setup();
    mock.on("POST", "/api/v1/contracts/author", () => authorResponse());
    mock.on("POST", "/api/v1/contracts/author/change", () => ({
      json: {
        valid: true,
        saved: false,
        sentence: "Raises the price by 5%.",
        operations: [
          { path: "pricing", value: [], summary: "Raise the price by 5%", kind: "price" },
        ],
        draft: authorDraft(),
        resulting_total_minor: 65_520,
        currency: "USD",
        issues: [],
        usage: { prompt_tokens: 1, completion_tokens: 1 },
        model_info: { provider: "test", model: "test" },
      },
    }));
    mock.install();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ContractsPage />
      </AuthProvider>,
    );
    await screen.findByText("C-101");
    await user.click(screen.getByTestId("contracts-describe"));
    await user.type(
      await screen.findByTestId("describe-transcript"),
      "Managed Silver for Acme, 12 months from 1 November",
    );
    await user.click(screen.getByTestId("describe-draft"));
    await screen.findByTestId("describe-understanding");
    await user.click(screen.getByTestId("describe-build"));

    // The draft applied a term, so the summary reads a year.
    expect(await screen.findByText(/over 12 months/i)).toBeInTheDocument();
    const until = screen.getByLabelText("Effective until");
    expect(until).toHaveValue("2027-11-01");

    // Clearing the date makes the contract open-ended. The term the draft
    // arrived with is not allowed to survive and turn it back into a year.
    fireEvent.change(until, { target: { value: "" } });
    expect(screen.queryByText(/over 12 months/i)).not.toBeInTheDocument();

    // The payload the change call carries says the same: no term.
    await user.click(screen.getByTestId("draft-tab-describe"));
    await user.type(screen.getByTestId("change-instruction"), "raise it 5%{Enter}");
    await waitFor(() => {
      const body = mock.lastBody<{ draft: { term_months: number | null } }>(
        "POST",
        "/api/v1/contracts/author/change",
      );
      expect(body.draft.term_months).toBeNull();
    });
  });

  it("refuses a proposal read before the end date moved, even inside the same month", async () => {
    const mock = setup();
    mock.on("POST", "/api/v1/contracts/author", () =>
      authorResponse({ term_months: 1 }),
    );
    mock.on("POST", "/api/v1/contracts/author/change", () => ({
      json: {
        valid: true,
        saved: false,
        sentence: "Adds a clause value.",
        operations: [
          { path: "clause_refs", value: [], summary: "Set the term to 1 month", kind: "term" },
        ],
        draft: authorDraft({ term_months: 1 }),
        resulting_total_minor: 62_400,
        currency: "USD",
        issues: [],
        usage: { prompt_tokens: 1, completion_tokens: 1 },
        model_info: { provider: "test", model: "test" },
      },
    }));
    mock.install();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ContractsPage />
      </AuthProvider>,
    );
    await screen.findByText("C-101");
    await user.click(screen.getByTestId("contracts-describe"));
    await user.type(
      await screen.findByTestId("describe-transcript"),
      "Managed Silver for Acme, one month from 1 November",
    );
    await user.click(screen.getByTestId("describe-draft"));
    await screen.findByTestId("describe-understanding");
    await user.click(screen.getByTestId("describe-build"));

    const until = await screen.findByLabelText("Effective until");
    expect(until).toHaveValue("2026-12-01");

    await user.click(screen.getByTestId("draft-tab-describe"));
    await user.type(screen.getByTestId("change-instruction"), "make the term clear{Enter}");
    const apply = await screen.findByTestId("change-apply");
    expect(apply).toBeEnabled();

    // One day later is still "1 month" to the term arithmetic, so the term
    // alone would call this proposal applicable and Apply would put the old
    // date back.
    fireEvent.change(until, { target: { value: "2026-12-02" } });
    await waitFor(() => expect(screen.getByTestId("change-apply")).toBeDisabled());
    expect(screen.getAllByText(/out of date/i).length).toBeGreaterThan(0);
  });

  it("prefills the panel from the contract a renewal starts at", async () => {
    window.history.replaceState({}, "", "/contracts?renew=101");
    setup();
    render(
      <AuthProvider>
        <ContractsPage />
      </AuthProvider>,
    );
    const panel = await screen.findByTestId("contracts-describe-panel");
    await waitFor(() => expect(panel).not.toHaveAttribute("hidden"));
    expect(screen.getByLabelText("Describe it")).toBeInTheDocument();
    // What it will use, in words, before anything is read.
    expect(await screen.findByText("Using")).toBeInTheDocument();
    expect(await screen.findByText("Contract C-101")).toBeInTheDocument();
    expect(screen.getByDisplayValue(/Renew Managed Gold for Acme Corporation/i)).toBeInTheDocument();
  });
});
