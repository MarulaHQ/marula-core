// Integration test: the client-first knowledge home (/kb).
//
// Covers the three things the brief says the home must answer: which clients
// exist and how complete they are, what is about to expire, and that search
// reaches every documentation store. It also proves the completeness copy is a
// count with reasons rather than a bare percentage.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import KnowledgeHomePage from "../../app/kb/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

// The page reads the client scope from the query string, so the mock keeps one
// instance the tests can point at a client before rendering.
const nav = vi.hoisted(() => ({ params: new URLSearchParams(), push: vi.fn() }));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: nav.push }),
  usePathname: () => "/kb",
  useSearchParams: () => nav.params,
  useParams: () => ({}),
}));

const CLIENT_ID = "50000000-0000-4000-8000-000000000001";

// Local calendar days: `daysUntil` compares local dates, so a `toISOString()`
// fixture would shift a day whenever the runner's clock is past 16:00 local.
function isoDaysFromNow(days: number): string {
  const date = new Date();
  date.setDate(date.getDate() + days);
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

const HOME = {
  clients: [
    {
      id: CLIENT_ID,
      name: "Acme Corporation",
      sites_n: 2,
      assets_n: 41,
      documents_n: 3,
      articles_n: 0,
      secrets_n: 12,
      racks_n: 1,
      networks_n: 2,
      domains_n: 1,
      procedures_n: 0,
      stale_secrets_n: 2,
      expiring_n: 1,
      completeness: {
        complete: 6,
        total: 8,
        gaps: [
          "Write or link a document, so the process is written down",
          "Write a procedure, so the next person can repeat it",
        ],
      },
      last_activity: "2026-11-10T09:00:00Z",
    },
  ],
  org: {
    published_articles_n: 14,
    draft_articles_n: 2,
    org_documents_n: 1,
    templates_n: 3,
    org_secrets_n: 6,
  },
  expiring_soon_n: 1,
};

const EXPIRIES = [
  {
    kind: "domain_tls",
    title: "acme.example",
    client_id: CLIENT_ID,
    client_name: "Acme Corporation",
    due_on: isoDaysFromNow(10),
    owner: "DigiCert",
    source_id: "60000000-0000-4000-8000-000000000001",
    provenance: "certificate record",
  },
];

const SEARCH_HITS = [
  {
    kind: "secret",
    id: "70000000-0000-4000-8000-000000000001",
    title: "Firewall admin",
    subtitle: "Credential · value hidden",
    client_id: CLIENT_ID,
    client_name: "Acme Corporation",
    href: "/kb/secrets?record=70000000-0000-4000-8000-000000000001",
  },
  {
    kind: "asset",
    id: "80000000-0000-4000-8000-000000000001",
    title: "ACME-FW01",
    subtitle: "Firewall",
    client_id: CLIENT_ID,
    client_name: "Acme Corporation",
    href: "/assets/80000000-0000-4000-8000-000000000001",
  },
];

function renderPage(): void {
  render(
    <AuthProvider>
      <KnowledgeHomePage />
    </AuthProvider>,
  );
}

describe("/kb knowledge home", () => {
  beforeEach(() => {
    cleanup();
    seedAuthTokens();
    nav.params = new URLSearchParams();
    nav.push.mockClear();
  });

  it("leads with clients and their counted completeness", async () => {
    const mock = new MockApi()
      .on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }))
      .on("GET", "/api/v1/knowledge/home", () => ({ json: HOME }))
      .on("GET", "/api/v1/knowledge/expirations", () => ({ json: EXPIRIES }))
      .install();
    renderPage();

    expect((await screen.findAllByText("Acme Corporation")).length).toBeGreaterThan(0);
    expect(screen.getByText("6 of 8 sections")).toBeTruthy();
    const card = screen.getByTestId("knowledge-client-card");
    expect(card.getAttribute("href")).toBe(`/kb/clients/${CLIENT_ID}`);
    // The expiry is named with words, not just a colour.
    expect(await screen.findByText("Due in 10 days")).toBeTruthy();
    expect(mock.callsTo("GET", "/api/v1/knowledge/home")).toHaveLength(1);
  });

  it("searches every store and narrows by scope", async () => {
    const mock = new MockApi()
      .on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }))
      .on("GET", "/api/v1/knowledge/home", () => ({ json: HOME }))
      .on("GET", "/api/v1/knowledge/expirations", () => ({ json: EXPIRIES }))
      .on("GET", "/api/v1/knowledge/search", () => ({ json: SEARCH_HITS }))
      .install();
    renderPage();

    await screen.findAllByText("Acme Corporation");
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Search knowledge"), "acme fw");

    await waitFor(() => {
      expect(screen.getByText("Firewall admin")).toBeTruthy();
    });
    expect(mock.callsTo("GET", "/api/v1/knowledge/search")[0]?.query.q).toBe("acme fw");

    await user.click(screen.getByRole("button", { name: "Configurations" }));
    await waitFor(() => {
      expect(screen.queryByText("Firewall admin")).toBeNull();
    });
    expect(screen.getByText("ACME-FW01")).toBeTruthy();
  });

  it("sends the selected client scope with the search", async () => {
    // A scoped page (?client_id=…) must ask the server for that client, not
    // filter a global answer afterwards — the limit would already be spent.
    nav.params = new URLSearchParams({ client_id: CLIENT_ID });
    const mock = new MockApi()
      .on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }))
      .on("GET", "/api/v1/knowledge/home", () => ({ json: HOME }))
      .on("GET", "/api/v1/knowledge/expirations", () => ({ json: EXPIRIES }))
      .on("GET", "/api/v1/knowledge/search", () => ({ json: SEARCH_HITS }))
      .install();
    renderPage();

    await screen.findAllByText("Acme Corporation");
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("Search knowledge"), "acme fw");

    await waitFor(() => {
      expect(mock.callsTo("GET", "/api/v1/knowledge/search")).toHaveLength(1);
    });
    const sent = mock.callsTo("GET", "/api/v1/knowledge/search")[0];
    expect(sent?.query.q).toBe("acme fw");
    expect(sent?.query.client_id).toBe(CLIENT_ID);
  });

  it("offers a client scope picker and puts the choice in the URL", async () => {
    const mock = new MockApi()
      .on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }))
      .on("GET", "/api/v1/knowledge/home", () => ({ json: HOME }))
      .on("GET", "/api/v1/knowledge/expirations", () => ({ json: EXPIRIES }))
      .on("GET", "/api/v1/clients", () => ({
        json: [{ id: CLIENT_ID, name: "Acme Corporation" }],
      }))
      .install();
    renderPage();

    const picker = await screen.findByLabelText("Scope");
    expect(within(picker).getByRole("option", { name: "All clients" })).toBeTruthy();
    const user = userEvent.setup();
    await user.selectOptions(picker, CLIENT_ID);

    expect(nav.push).toHaveBeenCalledTimes(1);
    expect(nav.push.mock.calls[0]?.[0]).toContain(`client_id=${CLIENT_ID}`);
    expect(mock.callsTo("GET", "/api/v1/clients")).toHaveLength(1);
  });
});
