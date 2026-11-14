// Roster derivation for Settings → Connectors (§2).
//
// Purpose: the read is installation-first — one entry per installed connector,
// each with its connections nested — so the card is the installation. This
// module derives, per card: the worst status across its connections, the
// "N of M needing attention" line when there are several, the one action that
// state allows, and the scoped-client names §2 asks for. Kept out of the page
// so the arithmetic (worst status wins, action follows status) is testable
// without rendering.
//
// Two deliberate readings of the contract:
//
// 1. `description` is the manifest's own line and wins. Only when it is empty
//    does the purpose fall back to the declared product kinds, which is still
//    real data from the response rather than a brand-name table that drifts
//    from the marketplace.
// 2. A connector with no connections is still a card. It reads `Not connected`
//    with a `Connect` action — the install-first shape exists so that state is
//    visible rather than absent.
import { statusOfConnection, statusOfConnector, type ConnectorStatus } from "@marula-core/ui";
import type { ConnectionView, ConnectorInstallationView } from "./types";

/** What the connector's one action is, driven by its connections. */
export type RosterAction = "connect" | "manage" | "fix";

export interface ConnectorCardModel {
  installationId: string;
  connector: string;
  /** Humanised connector slug; the contract carries no separate display name. */
  name: string;
  /** The manifest description, or a line built from the product kinds. */
  purpose: string;
  connections: ConnectionView[];
  status: ConnectorStatus;
  /** Connections that need attention; the numerator of "N of M". */
  unhealthy: number;
  total: number;
  /** Distinct client names served by scoped connections, in response order. */
  scopedClients: string[];
  action: RosterAction;
}

/** Acronyms and brand casings a generic humaniser would get wrong. */
const CONNECTOR_NAME: Record<string, string> = {
  ninjaone: "NinjaOne",
  screenconnect: "ScreenConnect",
  connectwise: "ConnectWise",
  hudu: "Hudu",
  quickbooks: "QuickBooks Online",
  xero: "Xero",
  slack: "Slack",
  teams: "Microsoft Teams",
  "3cx": "3CX",
  unifi: "UniFi",
  m365: "Microsoft 365",
  dms: "DMS",
};

/** §3 of the capabilities reference: what each declared kind lights up. */
const KIND_PURPOSE: Record<string, string> = {
  rmm: "Remote monitoring and management",
  remote_access: "Remote sessions",
  dms: "Documentation",
  documentation: "Documentation",
  telephony: "Click-to-call",
  accounting: "Accounting and invoicing",
  dns: "DNS",
  identity: "Identity",
  network: "Network",
  inventory: "Inventory",
  crm: "CRM",
  notifications: "Channel notifications",
};

export function connectorDisplayName(connector: string): string {
  const known = CONNECTOR_NAME[connector.toLowerCase()];
  if (known !== undefined) return known;
  const words = connector.replace(/[_-]+/g, " ").trim();
  if (words === "") return connector;
  return words
    .split(" ")
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

export function connectorPurpose(productKinds: readonly string[]): string {
  const lines = productKinds
    .map((kind) => KIND_PURPOSE[kind.toLowerCase()] ?? kind.replace(/[_-]+/g, " "))
    .filter((line, index, all) => line !== "" && all.indexOf(line) === index);
  if (lines.length === 0) return "Connected system";
  return lines.join(" · ");
}

/** The manifest's own line, falling back to the declared kinds when empty. */
export function purposeOf(installation: ConnectorInstallationView): string {
  const description = installation.description.trim();
  if (description !== "") return description;
  return connectorPurpose(installation.product_kinds);
}

/** Distinct client names, so two connections on one client name it once. */
export function scopedClientNames(connections: readonly ConnectionView[]): string[] {
  const names: string[] = [];
  for (const connection of connections) {
    const name = connection.scope_client_name?.trim();
    if (name !== undefined && name !== "" && !names.includes(name)) names.push(name);
  }
  return names;
}

/** The roster, sorted by display name, each card carrying its derived state. */
export function buildRoster(
  installations: readonly ConnectorInstallationView[],
): ConnectorCardModel[] {
  return installations
    .map((installation) => {
      const status = statusOfConnector(installation.connections);
      const unhealthy = installation.connections.filter(
        (connection) => statusOfConnection(connection.status) === "needs_attention",
      ).length;
      return {
        installationId: installation.installation_id,
        connector: installation.connector,
        name: connectorDisplayName(installation.connector),
        purpose: purposeOf(installation),
        connections: installation.connections,
        status,
        unhealthy,
        total: installation.connections.length,
        scopedClients: scopedClientNames(installation.connections),
        action:
          installation.connections.length === 0
            ? "connect"
            : status === "needs_attention"
              ? "fix"
              : "manage",
      } satisfies ConnectorCardModel;
    })
    .sort((a, b) => a.name.localeCompare(b.name));
}

/** Search filters by connector name and its purpose line (§2). */
export function filterRoster(
  cards: readonly ConnectorCardModel[],
  query: string,
): ConnectorCardModel[] {
  const needle = query.trim().toLowerCase();
  if (needle === "") return [...cards];
  return cards.filter(
    (card) =>
      card.name.toLowerCase().includes(needle) || card.purpose.toLowerCase().includes(needle),
  );
}
