// Command palette — the no-grant path and the search keyboard model.
//
// The demo org has AI off, so this is the live path: Cmd/Ctrl+K opens a search
// palette with no Ask switch, no tab, no hint and no gap where one would be.
// The cursor starts on the first result, arrows move it, Enter opens it, and
// Esc closes the overlay. The full /search page stays one link away.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import { CommandPaletteHost } from "../../components/command-palette";
import { actions } from "../../components/command-palette/store";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

const push = vi.hoisted(() => vi.fn());

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push, replace: vi.fn(), prefetch: vi.fn() }),
  usePathname: () => "/tickets",
}));

const TICKET_ONE = "71000000-0000-4000-8000-000000000001";
const TICKET_TWO = "71000000-0000-4000-8000-000000000002";

function setupApi(granted: boolean): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/ai/me/surfaces", () => ({
    json: {
      surfaces: [
        {
          surface_key: "general_chat",
          label: "General chat",
          kind: "chat",
          granted,
          models_available: true,
          picked: null,
          models: [],
        },
      ],
    },
  }));
  mock.on("GET", "/api/v1/search", () => ({
    json: {
      query: "vpn",
      total: 2,
      total_capped: false,
      kinds: ["ticket"],
      limit: 30,
      offset: 0,
      next_cursor: null,
      results: [
        {
          entity_kind: "ticket",
          entity_id: TICKET_ONE,
          parent_id: null,
          title: "#18432 VPN disconnects every 20–30 minutes",
          snippet: "Acme Corporation · In progress",
          score: 0.9,
        },
        {
          entity_kind: "ticket",
          entity_id: TICKET_TWO,
          parent_id: null,
          title: "#18401 VPN profile sync fails after reboot",
          snippet: "Northwind Legal · Resolved",
          score: 0.7,
        },
      ],
    },
  }));
  return mock.install();
}

function renderHost() {
  seedAuthTokens();
  return render(
    <AuthProvider>
      <CommandPaletteHost />
    </AuthProvider>,
  );
}

describe("command palette without the general_chat grant", () => {
  beforeEach(() => {
    actions.resetForTests();
    push.mockClear();
  });

  it("opens on Cmd/Ctrl+K with the field focused and no trace of AI", async () => {
    const api = setupApi(false);
    renderHost();
    const user = userEvent.setup();

    // Let the grant read settle first: absence must hold with the answer in
    // hand, not only while the read is in flight.
    await waitFor(() => {
      expect(api.callsTo("GET", "/api/v1/ai/me/surfaces").length).toBe(1);
    });

    await user.keyboard("{Meta>}k{/Meta}");

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toBeTruthy();
    const input = within(dialog).getByRole("combobox", { name: "Search or jump to" });
    expect(document.activeElement).toBe(input);
    expect(within(dialog).queryByRole("radio", { name: "Ask" })).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "Ask" })).toBeNull();
    expect(within(dialog).queryByText(/ask instead/i)).toBeNull();
    expect(within(dialog).queryByText(/Ask about this screen/i)).toBeNull();

    // Tab keeps its normal focus behaviour without the grant.
    await user.keyboard("{Tab}");
    expect(within(dialog).queryByTestId("palette-context-chip")).toBeNull();

    // The chord toggles the palette closed again.
    await user.keyboard("{Meta>}k{/Meta}");
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });

  it("groups results, selects the first, moves the cursor and opens on Enter", async () => {
    setupApi(false);
    renderHost();
    const user = userEvent.setup();
    await user.keyboard("{Meta>}k{/Meta}");

    const dialog = await screen.findByRole("dialog");
    const input = within(dialog).getByRole("combobox", { name: "Search or jump to" });
    await user.type(input, "vpn");

    await waitFor(() => {
      expect(within(dialog).getByText("Tickets · 2")).toBeTruthy();
    });
    const options = within(dialog).getAllByRole("option");
    expect(options[0]?.getAttribute("aria-selected")).toBe("true");

    await user.keyboard("{ArrowDown}");
    expect(options[1]?.getAttribute("aria-selected")).toBe("true");

    await user.keyboard("{Enter}");
    expect(push).toHaveBeenCalledWith(`/tickets/${TICKET_TWO}`);
  });

  it("closes on Escape and leaves the full search page reachable", async () => {
    setupApi(false);
    renderHost();
    const user = userEvent.setup();
    await user.keyboard("{Meta>}k{/Meta}");

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("link", { name: "Full search page" }).getAttribute("href")).toBe(
      "/search",
    );

    await user.keyboard("{Escape}");
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });
});
