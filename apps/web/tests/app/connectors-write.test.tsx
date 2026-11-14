// Integration test: the connector WRITE path (§2 Connect, §3 Settings,
// Destinations, Access).
//
// What these tests are the record of:
//
// - `Connect` opens the create form for that installation, and the request
//   splits the manifest's `writeOnly` fields into `secrets` — a credential in
//   `settings` is refused by the server, so the split is the contract.
// - An edit sends a per-key settings patch and leaves an untouched credential
//   out of the body entirely.
// - A 409 from a connection delete is rendered in place, naming what still
//   points at it, and the row is not removed.
// - A duplicate destination lands on the paste field that caused it.
// - An empty audience reads as "all staff with access", and picking roles
//   posts role ids.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import ConnectorsSettingsPage from "../../app/settings/connectors/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";
import type { ConnectionView, ConnectorInstallationView } from "../../lib/connectors/types";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/connectors",
}));

const SLACK_SCHEMA = {
  type: "object",
  required: ["workspace_url"],
  properties: {
    workspace_url: { type: "string", pattern: "^https://", title: "Workspace URL" },
    notify_on_share: { type: "boolean", title: "Notify on share" },
    bot_token: { type: "string", writeOnly: true, title: "Bot token" },
  },
};

const ROLES = {
  roles: [
    { id: "r-dispatcher", name: "Dispatcher", is_system: true, permissions: [] },
    { id: "r-senior", name: "Senior technician", is_system: true, permissions: [] },
  ],
  presets: [],
  portal_tiers: [],
};

function connection(overrides: Partial<ConnectionView> = {}): ConnectionView {
  return {
    id: "c-slack-1",
    installation_id: "inst-slack",
    connector: "slack",
    label: "Acme workspace",
    scope_client_id: null,
    scope_client_name: null,
    status: "connected",
    status_detail: null,
    status_detail_vendor: null,
    last_success_at: "2026-11-13T10:00:00Z",
    last_failure_at: null,
    bindings_count: 0,
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
    installation_id: "inst-slack",
    connector: "slack",
    description: "Chat and notifications",
    product_kinds: ["notifications"],
    connections: [connection()],
    ...overrides,
  };
}

function baseApi(connectors: ConnectorInstallationView[]): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/connectors/roster", () => ({ json: { connectors } }));
  mock.on("GET", "/api/v1/plugins/installed/inst-slack/settings", () => ({
    json: { slug: "slack", version: "1.0.0", settings_schema: SLACK_SCHEMA, config: {} },
  }));
  mock.on("GET", "/api/v1/clients", () => ({ json: [] }));
  mock.on("GET", "/api/v1/roles", () => ({ json: ROLES }));
  mock.on("GET", "/api/v1/connector-connections/c-slack-1/bindings", () => ({
    json: { bindings: [] },
  }));
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

describe("create a connection", () => {
  it("opens from the card's Connect and splits credentials out of settings", async () => {
    const user = userEvent.setup();
    const created = connection({
      id: "c-new",
      label: "Acme workspace",
      settings: { workspace_url: "https://acme.slack.com", notify_on_share: true },
      secrets_set: ["bot_token"],
    });
    const mock = baseApi([
      installation({ connector: "slack", description: "Chat and notifications", connections: [] }),
    ]);
    mock.on("POST", "/api/v1/connector-installations/inst-slack/connections", () => ({
      status: 201,
      json: created,
    }));
    renderPage();

    const card = await screen.findByTestId("connector-card-slack");
    await user.click(within(card).getByRole("button", { name: "Connect" }));

    // The form is the manifest's, not Core's: these labels come from the schema.
    expect(await screen.findByLabelText(/Workspace URL/)).toBeInTheDocument();
    const secret = screen.getByLabelText(/Bot token/);
    expect(secret).toHaveAttribute("type", "password");
    expect(secret).toHaveValue("");

    await user.type(screen.getByLabelText("Name"), "Acme workspace");
    await user.type(screen.getByLabelText(/Workspace URL/), "https://acme.slack.com");
    await user.click(screen.getByRole("switch", { name: /Notify on share/ }));
    await user.type(secret, "xoxb-token");
    await user.click(screen.getByTestId("create-connection-save"));

    await waitFor(() =>
      expect(
        mock.callsTo("POST", "/api/v1/connector-installations/inst-slack/connections"),
      ).toHaveLength(1),
    );
    const body = mock.lastBody<{
      label: string;
      scope_client_id: string | null;
      settings: Record<string, unknown>;
      secrets: Record<string, string>;
      audience_roles: string[];
    }>("POST", "/api/v1/connector-installations/inst-slack/connections");
    expect(body.label).toBe("Acme workspace");
    expect(body.scope_client_id).toBeNull();
    expect(body.settings).toEqual({
      workspace_url: "https://acme.slack.com",
      notify_on_share: true,
    });
    // The credential travelled in `secrets`; `settings` never carries it.
    expect(body.secrets).toEqual({ bot_token: "xoxb-token" });
    expect(body.settings).not.toHaveProperty("bot_token");
    expect(body.audience_roles).toEqual([]);

    // The server-confirmed row becomes the detail, not an optimistic copy.
    expect(await screen.findByTestId("connection-detail")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Acme workspace" })).toBeInTheDocument();
  });

  it("renders a 422 on the field the server names and keeps the dialog open", async () => {
    const user = userEvent.setup();
    const mock = baseApi([installation({ connections: [] })]);
    mock.on("POST", "/api/v1/connector-installations/inst-slack/connections", () => ({
      status: 422,
      json: {
        error: {
          code: 422,
          reason: "validation_failed",
          message: 'settings key "workspace_url" does not match its declared pattern',
        },
      },
    }));
    renderPage();

    const card = await screen.findByTestId("connector-card-slack");
    await user.click(within(card).getByRole("button", { name: "Connect" }));
    await user.type(await screen.findByLabelText("Name"), "Acme workspace");
    await user.type(screen.getByLabelText(/Workspace URL/), "https://acme.slack.com");
    await user.click(screen.getByTestId("create-connection-save"));

    const alert = await screen.findByTestId("create-connection-error");
    expect(alert).toHaveTextContent("does not match its declared pattern");
    expect(screen.getByTestId("create-connection")).toBeInTheDocument();
  });
});

describe("edit a connection", () => {
  it("patches only what changed and leaves a configured credential alone", async () => {
    const user = userEvent.setup();
    const current = connection({
      settings: { workspace_url: "https://acme.slack.com", notify_on_share: true },
      secrets_set: ["bot_token"],
    });
    const mock = baseApi([installation({ connections: [current] })]);
    mock.on("PATCH", "/api/v1/connector-connections/c-slack-1", () => ({
      json: {
        ...current,
        settings: { workspace_url: "https://acme.slack.com", notify_on_share: false },
      },
    }));
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Manage" }));

    const secret = await screen.findByLabelText(/Bot token/);
    // Configured, never read back, never pre-filled.
    expect(secret).toHaveValue("");
    expect(secret).toHaveAttribute("placeholder", "Configured — type to replace");

    await user.click(screen.getByRole("switch", { name: /Notify on share/ }));
    await user.click(screen.getByTestId("connection-settings-save"));

    await waitFor(() =>
      expect(mock.callsTo("PATCH", "/api/v1/connector-connections/c-slack-1")).toHaveLength(1),
    );
    const body = mock.lastBody<{ settings?: Record<string, unknown>; secrets?: unknown }>(
      "PATCH",
      "/api/v1/connector-connections/c-slack-1",
    );
    expect(body.settings).toEqual({ notify_on_share: false });
    expect(body).not.toHaveProperty("secrets");
  });
});

describe("remove a connection", () => {
  it("renders the server's 409 in place and keeps the connection", async () => {
    const user = userEvent.setup();
    const mock = baseApi([installation()]);
    mock.on("DELETE", "/api/v1/connector-connections/c-slack-1", () => ({
      status: 409,
      json: {
        error: {
          code: 409,
          reason: "conflict",
          message:
            "connection is still referenced: 2 destination(s), 1 external reference(s), 0 vendor identity(ies) — revoke the installation to remove those",
        },
      },
    }));
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Manage" }));
    await user.click(screen.getByTestId("connection-remove"));
    await user.click(screen.getByTestId("confirm-dialog-confirm"));

    const error = await screen.findByTestId("confirm-dialog-error");
    expect(error).toHaveTextContent("2 destination(s)");
    expect(screen.getByTestId("connection-detail")).toBeInTheDocument();
  });
});

describe("destinations", () => {
  it("lands a duplicate 409 on the vendor-id field", async () => {
    const user = userEvent.setup();
    const mock = baseApi([installation()]);
    mock.on("POST", "/api/v1/connector-connections/c-slack-1/bindings", () => ({
      status: 409,
      json: {
        error: { code: 409, reason: "conflict", message: "that destination is already bound" },
      },
    }));
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Manage" }));
    await screen.findByTestId("destination-editor");
    await user.type(await screen.findByLabelText("Label"), "#helpdesk-alerts");
    await user.type(screen.getByLabelText("Vendor id"), "C01234567");
    await user.click(screen.getByTestId("destination-add"));

    expect(await screen.findByText(/that destination is already bound/)).toBeInTheDocument();
    expect(screen.getByLabelText("Vendor id")).toHaveAttribute("aria-invalid", "true");
  });

  it("shows the disabled state of an existing destination and the fixed vendor id", async () => {
    const user = userEvent.setup();
    const mock = baseApi([installation()]);
    mock.on("GET", "/api/v1/connector-connections/c-slack-1/bindings", () => ({
      json: {
        bindings: [
          {
            id: "b-1",
            connection_id: "c-slack-1",
            binding_kind: "slack_channel",
            label: "#helpdesk-alerts",
            external_id: "C01234567",
            external_parent: "T099",
            vendor_ids: {},
            enabled: false,
          },
        ],
      },
    }));
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Manage" }));
    const row = await screen.findByTestId("destination-slack_channel");
    expect(within(row).getByText("#helpdesk-alerts")).toBeInTheDocument();
    expect(within(row).getByText("Disabled")).toBeInTheDocument();
    expect(within(row).getByText(/C01234567/)).toBeInTheDocument();
    // Identity is not editable: no field carries the vendor id as an input.
    expect(screen.queryByDisplayValue("C01234567")).not.toBeInTheDocument();
    expect(mock.callsTo("GET", "/api/v1/connector-connections/c-slack-1/bindings")).toHaveLength(1);
  });
});

describe("access", () => {
  it("reads an empty audience as all staff and posts role ids when narrowed", async () => {
    const user = userEvent.setup();
    const mock = baseApi([installation()]);
    mock.on("PATCH", "/api/v1/connector-connections/c-slack-1", () => ({
      json: connection({ audience_roles: ["r-dispatcher"] }),
    }));
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Manage" }));
    expect((await screen.findAllByText(/All staff with access/)).length).toBeGreaterThan(0);
    expect(screen.queryByText(/No access/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("radio", { name: /Only these roles/ }));
    await user.click(await screen.findByRole("checkbox", { name: "Dispatcher" }));
    await user.click(screen.getByTestId("connection-audience-save"));

    await waitFor(() =>
      expect(mock.callsTo("PATCH", "/api/v1/connector-connections/c-slack-1")).toHaveLength(1),
    );
    expect(
      mock.lastBody<{ audience_roles: string[] }>(
        "PATCH",
        "/api/v1/connector-connections/c-slack-1",
      ).audience_roles,
    ).toEqual(["r-dispatcher"]);
  });

  it("blocks Save when 'Only these roles' is chosen with none ticked", async () => {
    const user = userEvent.setup();
    baseApi([installation()]);
    renderPage();

    await user.click(await screen.findByRole("button", { name: "Manage" }));
    await user.click(await screen.findByRole("radio", { name: /Only these roles/ }));

    expect(await screen.findByText(/Pick at least one role/)).toBeInTheDocument();
    expect(screen.getByTestId("connection-audience-save")).toBeDisabled();
  });
});
