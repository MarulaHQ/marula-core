// Roster derivation tests (§2): install-first cards, worst-status-wins, the
// action rule and the search filter. These are the rules the page must not
// re-decide.
import { describe, expect, it } from "vitest";
import {
  buildRoster,
  connectorDisplayName,
  connectorPurpose,
  filterRoster,
  purposeOf,
  scopedClientNames,
} from "./roster";
import type { ConnectionView, ConnectorInstallationView } from "./types";

function connection(overrides: Partial<ConnectionView> = {}): ConnectionView {
  return {
    id: "c-1",
    installation_id: "i-1",
    connector: "ninjaone",
    label: "Acme RMM",
    scope_client_id: null,
    scope_client_name: null,
    status: "connected",
    status_detail: null,
    status_detail_vendor: null,
    last_success_at: null,
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
    installation_id: "i-1",
    connector: "ninjaone",
    description: "Remote monitoring and management",
    product_kinds: ["rmm"],
    connections: [connection()],
    ...overrides,
  };
}

describe("connectorDisplayName", () => {
  it("keeps the brand casings a generic humaniser would break", () => {
    expect(connectorDisplayName("ninjaone")).toBe("NinjaOne");
    expect(connectorDisplayName("screenconnect")).toBe("ScreenConnect");
    expect(connectorDisplayName("3cx")).toBe("3CX");
  });

  it("humanises an unknown slug", () => {
    expect(connectorDisplayName("inventory_control")).toBe("Inventory Control");
  });
});

describe("connectorPurpose", () => {
  it("names the declared product kinds when the manifest has no description", () => {
    expect(connectorPurpose(["rmm"])).toBe("Remote monitoring and management");
    expect(connectorPurpose(["rmm", "remote_access"])).toBe(
      "Remote monitoring and management · Remote sessions",
    );
    expect(connectorPurpose([])).toBe("Connected system");
  });
});

describe("purposeOf", () => {
  it("uses the manifest description when it is present", () => {
    expect(purposeOf(installation({ description: "Chat and notifications" }))).toBe(
      "Chat and notifications",
    );
  });

  it("falls back to the declared kinds when the description is empty", () => {
    expect(purposeOf(installation({ description: "  ", product_kinds: ["remote_access"] }))).toBe(
      "Remote sessions",
    );
  });
});

describe("scopedClientNames", () => {
  it("names each client once and skips unscoped connections", () => {
    expect(
      scopedClientNames([
        connection({ scope_client_id: "client-1", scope_client_name: "Acme" }),
        connection({ id: "c-2", scope_client_id: "client-1", scope_client_name: "Acme" }),
        connection({ id: "c-3", scope_client_id: "client-2", scope_client_name: "Beta" }),
        connection({ id: "c-4" }),
      ]),
    ).toEqual(["Acme", "Beta"]);
  });
});

describe("buildRoster", () => {
  it("keeps an installed connector with no connections as a card that can Connect", () => {
    const [card] = buildRoster([installation({ connections: [] })]);
    expect(card?.total).toBe(0);
    expect(card?.status).toBe("not_connected");
    expect(card?.action).toBe("connect");
  });

  it("derives Needs attention and Fix from one failing connection", () => {
    const [card] = buildRoster([
      installation({
        connections: [connection(), connection({ id: "c-2", status: "auth_failed" })],
      }),
    ]);
    expect(card?.status).toBe("needs_attention");
    expect(card?.action).toBe("fix");
    expect(card?.unhealthy).toBe(1);
    expect(card?.total).toBe(2);
  });

  it("treats needs_attachment as repair work, not as Not connected", () => {
    const [card] = buildRoster([
      installation({ connections: [connection({ status: "needs_attachment" })] }),
    ]);
    expect(card?.status).toBe("needs_attention");
    expect(card?.action).toBe("fix");
  });

  it("offers Manage for a healthy connector", () => {
    const [card] = buildRoster([installation()]);
    expect(card?.status).toBe("connected");
    expect(card?.action).toBe("manage");
  });

  it("keeps Manage for a disabled connector — there is still a record to manage", () => {
    const [card] = buildRoster([
      installation({ connections: [connection({ status: "disabled" })] }),
    ]);
    expect(card?.action).toBe("manage");
    expect(card?.status).toBe("disabled");
  });

  it("carries the scoped client names onto the card", () => {
    const [card] = buildRoster([
      installation({
        connections: [
          connection({ id: "c-1", scope_client_id: "client-1", scope_client_name: "Acme" }),
          connection({ id: "c-2", scope_client_id: "client-2", scope_client_name: "Beta" }),
        ],
      }),
    ]);
    expect(card?.scopedClients).toEqual(["Acme", "Beta"]);
  });

  it("sorts cards by display name", () => {
    const cards = buildRoster([
      installation({ installation_id: "i-2", connector: "screenconnect" }),
      installation(),
    ]);
    expect(cards.map((card) => card.name)).toEqual(["NinjaOne", "ScreenConnect"]);
  });
});

describe("filterRoster", () => {
  it("matches name and description, case-insensitively", () => {
    const cards = buildRoster([
      installation(),
      installation({
        installation_id: "i-2",
        connector: "screenconnect",
        description: "Remote sessions for client machines",
        product_kinds: ["remote_access"],
      }),
    ]);
    expect(filterRoster(cards, "ninja").map((card) => card.connector)).toEqual(["ninjaone"]);
    expect(filterRoster(cards, "REMOTE SESSIONS").map((card) => card.connector)).toEqual([
      "screenconnect",
    ]);
    expect(filterRoster(cards, "  ")).toHaveLength(2);
  });
});
