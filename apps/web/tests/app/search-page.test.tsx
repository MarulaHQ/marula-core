// Integration test: /search — the P4 rule. Search is the one surface where
// the AI is INVISIBLE: no sparkle, no hint, no "ask", no assistant stub, and
// a no-results state that never suggests another way to ask. Reranking
// happens where the user must never know it happened; the audit trail is
// the only trace.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import SearchPage from "../../app/search/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/search",
}));

const HIT = {
  entity_kind: "ticket",
  entity_id: "81000000-0000-4000-8000-000000000001",
  title: "VPN disconnects every 20 to 30 minutes",
  snippet: "Acme Corporation reported drops every half hour",
  parent_id: null,
  score: 0.91,
};

function setupApi(results: unknown[]): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/search", () => ({
    json: {
      query: "vpn",
      results,
      total: results.length,
      next_cursor: null,
    },
  }));
  mock.on("GET", "/api/v1/search/pins", () => ({ json: { pins: [] } }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

/** Everything the pack forbids on this surface, as one assertion. */
function expectNoAiAffordance(): void {
  const forbidden = /ask ai|assistant|sparkle|spark|try ai|let ai|ai suggestion|generate/i;
  for (const node of Array.from(document.querySelectorAll("body *"))) {
    const text = node.textContent?.trim() ?? "";
    if (text.length === 0) continue;
    expect(text).not.toMatch(forbidden);
  }
  // The stub pattern from the shared kit (a 40px spark with a count).
  expect(document.querySelector(".m-assistant-stub")).toBeNull();
}

describe("/search integration", () => {
  it("renders grouped results with counts and no AI trace", async () => {
    setupApi([HIT, { ...HIT, entity_kind: "client", title: "Acme Corporation" }]);
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <SearchPage />
      </AuthProvider>,
    );
    await user.type(await screen.findByLabelText("Search"), "vpn");
    await screen.findByText("VPN disconnects every 20 to 30 minutes");
    // Grouped by kind, each group carrying its count.
    expect(screen.getByText("Tickets · 1")).toBeInTheDocument();
    expect(screen.getByText("Clients · 1")).toBeInTheDocument();
    expect(
      screen.getByText((_, el) => el?.textContent === "2 matches · Tickets 1 · Clients 1"),
    ).toBeInTheDocument();
    expectNoAiAffordance();
    cleanup();
  });

  it("keeps the no-results state plain and never suggests AI", async () => {
    setupApi([]);
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <SearchPage />
      </AuthProvider>,
    );
    await user.type(await screen.findByLabelText("Search"), "zzzz");
    await screen.findByTestId("search-empty");
    expect(screen.getByText(/Nothing matches/)).toBeInTheDocument();
    // It points at the query, never at another way to ask.
    expect(screen.getByText(/Check the spelling/)).toBeInTheDocument();
    expectNoAiAffordance();
    cleanup();
  });

  it("states the resting shape under two characters without offering anything else", async () => {
    setupApi([]);
    render(
      <AuthProvider>
        <SearchPage />
      </AuthProvider>,
    );
    await screen.findByText("Type at least two characters");
    await waitFor(() => {
      expect(screen.queryByTestId("search-empty")).not.toBeInTheDocument();
    });
    expectNoAiAffordance();
    cleanup();
  });
});
