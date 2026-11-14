// Integration test: /settings/connectors — the connector roster (§2) and the
// connection detail (§3). The roster read is installation-first: a card is an
// installed connector with its connections nested, so a connector with none
// still appears. The contract is typed by hand (lib/connectors/types.ts)
// because the route is not in the generated client yet, so these tests are the
// record of what the page reads.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import ConnectorsSettingsPage from "../../app/settings/connectors/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";
import type {
  ConnectionView,
  ConnectorInstallationView,
} from "../../lib/connectors/types";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/connectors",
}));

function connection(overrides: Partial<ConnectionView> = {}): ConnectionView {
  return {
    id: "c-ninja-1",
    installation_id: "inst-ninja",
    connector: "ninjaone",
    label: "Acme RMM",
    scope_client_id: null,
    scope_client_name: null,
    status: "connected",
    status_detail: null,
    status_detail_vendor: null,
    last_success_at: "2026-11-13T10:00:00Z",
    last_failure_at: null,
    bindings_count: 2,
    settings: {},
    secrets_set: [],
    audience_roles: [],
    ...overrides,
  };
}

function installation(
  overrides: Partial<ConnectorInstallationView> = {},
): ConnectorInstallationView {
  return {
    installation_id: "inst-ninja",
    connector: "ninjaone",
    description: "Remote monitoring and management",
    product_kinds: ["rmm"],
    connections: [connection()],
    ...overrides,
  };
}

function setupApi(connectors: ConnectorInstallationView[]): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/connectors/roster", () => ({ json: { connectors } }));
  mock.install();
  return mock;
}

function renderPage(): void {
  render(
    <AuthProvider>
      <ConnectorsSettingsPage />
    </AuthProvider>,
  );
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/settings/connectors", () => {
  it("renders one card per installed connector, with one status and one action", async () => {
    setupApi([
      installation(),
      installation({
        installation_id: "inst-sc",
        connector: "screenconnect",
        description: "Remote sessions",
        product_kinds: ["remote_access"],
        connections: [
          connection({
            id: "c-sc-1",
            installation_id: "inst-sc",
            connector: "screenconnect",
            label: "Acme remote",
          }),
        ],
      }),
    ]);
    renderPage();

    await screen.findByTestId("connector-card-ninjaone");
    expect(screen.getByTestId("connector-card-screenconnect")).toBeInTheDocument();
    expect(screen.getAllByTestId(/^connector-card-/)).toHaveLength(2);

    const ninjaCard = screen.getByTestId("connector-card-ninjaone");
    expect(within(ninjaCard).getByTestId("status-pill")).toHaveTextContent("Connected");
    expect(within(ninjaCard).getByRole("button", { name: "Manage" })).toBeInTheDocument();
  });

  it("renders an installed connector with no connections as a card that can Connect", async () => {
    setupApi([
      installation({
        installation_id: "inst-slack",
        connector: "slack",
        description: "Chat and notifications",
        product_kinds: ["notifications"],
        connections: [],
      }),
    ]);
    renderPage();

    const card = await screen.findByTestId("connector-card-slack");
    expect(within(card).getByTestId("status-pill")).toHaveTextContent("Not connected");
    expect(within(card).getByText("Installed but not connected yet")).toBeInTheDocument();
    expect(within(card).getByRole("button", { name: "Connect" })).toBeInTheDocument();
  });

  it("reads Needs attention and offers Fix when one of several connections failed", async () => {
    setupApi([
      installation({
        connections: [
          connection({ id: "c-1", label: "Acme RMM" }),
          connection({
            id: "c-2",
            label: "Beta RMM",
            status: "auth_failed",
            status_detail_vendor: "401 token expired",
            last_failure_at: "2026-11-13T09:00:00Z",
          }),
          connection({ id: "c-3", label: "Gamma RMM", scope_client_id: "client-3" }),
          connection({ id: "c-4", label: "Delta RMM" }),
        ],
      }),
    ]);
    renderPage();

    const card = await screen.findByTestId("connector-card-ninjaone");
    expect(within(card).getByTestId("status-pill")).toHaveTextContent("Needs attention");
    expect(card).toHaveTextContent("1 of 4");
    expect(within(card).getByRole("button", { name: "Fix" })).toBeInTheDocument();
    expect(within(card).queryByRole("button", { name: "Manage" })).not.toBeInTheDocument();
  });

  it("shows a scoped client by name on the card and in the detail", async () => {
    const user = userEvent.setup();
    setupApi([
      installation({
        connections: [
          connection({
            id: "c-client",
            label: "Beta RMM",
            scope_client_id: "client-2",
            scope_client_name: "Beta Industries",
          }),
          connection({ id: "c-org", label: "Acme RMM" }),
        ],
      }),
    ]);
    renderPage();

    const card = await screen.findByTestId("connector-card-ninjaone");
    // The secondary line names the client; it never prints the uuid.
    expect(within(card).getByText(/Beta Industries/)).toBeInTheDocument();
    expect(card).not.toHaveTextContent("client-2");

    await user.click(within(card).getByRole("button", { name: "Manage" }));
    expect(screen.getByTestId("connection-scope-client")).toHaveTextContent(
      "Client · Beta Industries",
    );
  });

  it("opens the connection detail from Fix with the four sections in order", async () => {
    const user = userEvent.setup();
    setupApi([
      installation({
        connections: [
          connection({
            status: "auth_failed",
            status_detail_vendor: "401 token expired",
            last_failure_at: "2026-11-13T09:00:00Z",
          }),
        ],
      }),
    ]);
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Fix" }));

    expect(screen.getByTestId("connection-detail")).toBeInTheDocument();
    const sections = screen
      .getAllByTestId(/^detail-section-/)
      .map((node) => node.getAttribute("data-testid"));
    expect(sections).toEqual([
      "detail-section-settings",
      "detail-section-destinations",
      "detail-section-health",
      "detail-section-access",
    ]);
    expect(screen.getByTestId("connection-vendor-error")).toHaveTextContent("401 token expired");
    // A never-failed connection omits the timestamp row rather than printing
    // a dash where a failure time would go.
    expect(screen.getByText("Last failure")).toBeInTheDocument();

    await user.click(screen.getByTestId("connection-detail-back"));
    expect(screen.getByTestId("connector-roster")).toBeInTheDocument();
  });

  it("omits the last-failure row for a connection that has never failed", async () => {
    const user = userEvent.setup();
    setupApi([installation()]);
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Manage" }));
    expect(screen.queryByText("Last failure")).not.toBeInTheDocument();
    expect(screen.getByText("Last success")).toBeInTheDocument();
  });

  it("wires Test connection for a connector the canary endpoint accepts", async () => {
    const user = userEvent.setup();
    const mock = setupApi([
      installation({
        installation_id: "inst-slack",
        connector: "slack",
        description: "Chat and notifications",
        product_kinds: ["notifications"],
        connections: [
          connection({
            id: "c-slack",
            installation_id: "inst-slack",
            connector: "slack",
            label: "Acme workspace",
          }),
        ],
      }),
    ]);
    mock.on("POST", "/api/v1/connectors/slack/test", () => ({
      json: { kind: "slack", outcome: "delivered", detail: null },
    }));
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Manage" }));
    await user.click(screen.getByTestId("connection-test"));

    const result = await screen.findByTestId("connection-test-result");
    expect(result).toHaveTextContent("Delivered");
    expect(mock.callsTo("POST", "/api/v1/connectors/slack/test")).toHaveLength(1);
  });

  it("does not draw a dead canary button for a connector with no test endpoint", async () => {
    const user = userEvent.setup();
    setupApi([installation()]);
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Manage" }));
    expect(screen.queryByTestId("connection-test")).not.toBeInTheDocument();
    expect(screen.getByText(/no canary endpoint/)).toBeInTheDocument();
  });

  it("renders the settings route frame and filters by name and description", async () => {
    const user = userEvent.setup();
    setupApi([
      installation({ description: "Remote monitoring and management" }),
      installation({
        installation_id: "inst-sc",
        connector: "screenconnect",
        description: "Remote sessions for client machines",
        product_kinds: ["remote_access"],
        connections: [
          connection({
            id: "c-sc-1",
            installation_id: "inst-sc",
            connector: "screenconnect",
            label: "Acme remote",
          }),
        ],
      }),
    ]);
    renderPage();

    expect(await screen.findByRole("heading", { name: "Connectors" })).toBeInTheDocument();
    const search = screen.getByRole("searchbox", { name: "Search connectors" });

    // Description, not just the name.
    await user.type(search, "client machines");
    expect(screen.queryByTestId("connector-card-ninjaone")).not.toBeInTheDocument();
    expect(screen.getByTestId("connector-card-screenconnect")).toBeInTheDocument();

    await user.clear(search);
    await user.type(search, "nothing-like-this");
    expect(screen.getByTestId("connectors-no-matches")).toBeInTheDocument();
  });

  it("renders the empty state when no connector is installed", async () => {
    setupApi([]);
    renderPage();

    expect(
      await screen.findByText(/No connectors installed yet/),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("connector-roster")).not.toBeInTheDocument();
  });

  it("shows the roster error with a retry that re-reads the route", async () => {
    const user = userEvent.setup();
    const mock = new MockApi();
    mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
    let attempts = 0;
    mock.on("GET", "/api/v1/connectors/roster", () => {
      attempts += 1;
      return attempts === 1
        ? { status: 500, json: { error: { message: "boom" } } }
        : { json: { connectors: [installation()] } };
    });
    mock.install();
    renderPage();

    expect(await screen.findByTestId("connectors-error")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() =>
      expect(screen.getByTestId("connector-card-ninjaone")).toBeInTheDocument(),
    );
    expect(attempts).toBe(2);
  });
});
