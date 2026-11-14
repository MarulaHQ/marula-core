// Response types for the connector roster (§2) and connection detail (§3).
//
// Purpose: `GET /api/v1/connectors/roster` is installation-first. A card is a
// connector the org installed, and its connections nest inside it, so a
// connector that is installed but not yet configured still appears — which is
// the state every install begins in. The roster types this here rather than
// reaching for a generated schema the route has not joined yet.
//
// Non-purpose: no settings schema, no destination rows and no audience. The
// contract carries a description, a destination *count*, a status and two
// status strings; the detail panel states what that cannot show rather than
// inventing fields. When the route grows those fields, add them here once —
// the panel already reads this file and nothing else.
//
// The route is not in the OpenAPI document yet, so this client reads it with a
// plain `fetch` (the same route `lib/api` uses for other not-yet-generated
// reads). Every field is required in the contract, so nothing here is
// optimistic about what the server returns.

export interface ConnectionView {
  id: string;
  installation_id: string;
  /** Connector slug, e.g. `ninjaone`, `screenconnect`, `slack`. */
  connector: string;
  /** The operator's label for this connection ("Acme workspace"). */
  label: string;
  /** Set for a per-client connection; null for an org-wide one. */
  scope_client_id: string | null;
  /** The client's name, so a card never has to show a bare uuid. */
  scope_client_name: string | null;
  /**
   * Raw status as the API reports it. The contract names `connected`,
   * `needs_attachment`, `auth_failed`, `degraded` and `disabled`; see
   * `statusOfConnection` for the fold onto the four words the UI shows.
   */
  status: string;
  status_detail: string | null;
  status_detail_vendor: string | null;
  last_success_at: string | null;
  /** Null when the connection has never failed — omit the timestamp row, then. */
  last_failure_at: string | null;
  bindings_count: number;
}

/** One installed connector and the connections bound to it. */
export interface ConnectorInstallationView {
  installation_id: string;
  connector: string;
  /** The manifest's own line. Empty string when the manifest has none. */
  description: string;
  /** The manifest's open product-kind list (`rmm`, `remote_access`, …). */
  product_kinds: string[];
  /** Empty for a connector that is installed but has nothing configured yet. */
  connections: ConnectionView[];
}

export interface ConnectorsRosterResponse {
  connectors: ConnectorInstallationView[];
}
