-- 0185 — the connector connection model, record links and external references.
--
-- Three things that were about to become three unrelated mechanisms land
-- here as one, per ADR 0032 and ADR 0033:
--
--   * `connector_connections` + `connector_bindings` — a configured use of an
--     installed connector (scoped to the org or to one client), and the named
--     destinations it may post to. This is what `connector_installations`
--     could not express: it is unique per (org, type) with one sealed blob,
--     so an MSP running two ScreenConnect instances, two NinjaOne orgs, or
--     one Hostifi and one on-prem UniFi had nowhere to put the second.
--
--   * `external_refs` — "this Marula record IS that vendor object". It
--     consolidates `config_source_links` (0061, configurations only, keyed by
--     a brand string with no connection) and `external_ids` (0027, imports
--     only, with no source system at all — so a ConnectWise key and a Syncro
--     key with the same text collide). Both are superseded; the backfill is
--     a separate, reversible step so this migration can land without
--     rewriting live rows.
--
--   * `record_links`, `user_external_identities`, `capability_defaults` —
--     the record-to-record edge a ticket needs to name its configurations,
--     the person-to-vendor identity a Slack button or a click-to-call needs
--     to act as the person pressing it, and the per-capability primary a
--     chip with two providers needs.
--
-- Why one migration rather than four. `external_refs` cannot exist without
-- `connector_connections` (the FK is the whole point: a reference resolves
-- to exactly one base URL and one credential), and every table here is read
-- by the same resolver. Splitting them would ship a schema where half the
-- join is missing.
--
-- Primary keys are app-minted UUIDv7, matching every table since 0146 and
-- the Rust write path (`Uuid::now_v7()`). No `DEFAULT uuidv7()` here on
-- purpose: a default would mint a second timestamp for a row the application
-- already stamped, so the row and its audit sibling could disagree about
-- order. ADR 0018 asks for v7 ids, not for the default.
--
-- RLS doctrine: org-scoped user data, org-bound reads. `assert_rls_coverage`
-- requires the column and the policy together, and every table below carries
-- both. Client scoping is enforced in the query layer, not by RLS: RLS is
-- org-level in this product, and a policy that also encoded client access
-- would duplicate `core_db::client_access` in a second language.

-- 1. Connections -----------------------------------------------------------

CREATE TABLE connector_connections (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The install that defines this connection: manifest, granted scopes,
  -- version. Consent lives on the install; credentials live here.
  installation_id uuid NOT NULL REFERENCES plugin_installs(id) ON DELETE CASCADE,
  -- NULL is the org-wide connection (one Slack workspace, one QBO company).
  -- A client id is a per-client instance (a Hostifi controller, an on-prem
  -- PBX), which is the G11 case: two clients, two credentials, one roster row.
  scope_client_id uuid REFERENCES clients(id) ON DELETE CASCADE,
  -- What the operator called it. Two connections of the same install differ
  -- by label, so "NinjaOne — Acme" and "NinjaOne — Contoso" are both legal.
  label text NOT NULL CHECK (length(btrim(label)) BETWEEN 1 AND 120),
  -- Non-secret settings that satisfy the manifest's settings_schema. Queryable
  -- so the resolver can read a base URL without unsealing anything.
  settings jsonb NOT NULL DEFAULT '{}'::jsonb,
  -- The credential document, sealed exactly as connector_installations did.
  -- NULL means a connection that needs no secret (a public webhook target).
  secrets_encrypted bytea,
  -- The four states plus `disabled`, which is a choice rather than a fault.
  status text NOT NULL DEFAULT 'needs_attachment'
    CHECK (status IN ('connected', 'needs_attachment', 'auth_failed', 'degraded', 'disabled')),
  -- A typed reason code the UI can branch on (`otp_required`, `expired`, …).
  status_detail text,
  -- The vendor's own words, verbatim. Never parsed into prose: a paraphrase
  -- of an auth error is how an operator is sent to the wrong knowledge base.
  status_detail_vendor text,
  -- A status without a timestamp is a claim; these two make it evidence.
  last_success_at timestamptz,
  last_failure_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- Two org-wide connections may both exist, so scope alone cannot be the key;
  -- the label disambiguates. NULLS NOT DISTINCT (PG15+) is what makes the
  -- org-wide row participate in the same unique index as the client rows —
  -- a plain UNIQUE would treat every NULL scope as distinct and enforce
  -- nothing.
  CONSTRAINT connector_connections_scope_label_unique
    UNIQUE NULLS NOT DISTINCT (org_id, installation_id, scope_client_id, label)
);

CREATE INDEX connector_connections_install_idx
  ON connector_connections (org_id, installation_id);
CREATE INDEX connector_connections_client_idx
  ON connector_connections (org_id, scope_client_id)
  WHERE scope_client_id IS NOT NULL;
-- The roster's "needs attention" count reads only the unhealthy rows.
CREATE INDEX connector_connections_attention_idx
  ON connector_connections (org_id, status)
  WHERE status <> 'connected';

-- 2. Bindings — the named destinations a connection may post to --------------

CREATE TABLE connector_bindings (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  connection_id uuid NOT NULL REFERENCES connector_connections(id) ON DELETE CASCADE,
  binding_kind text NOT NULL
    CHECK (binding_kind IN ('slack_channel', 'teams_conversation', 'rewst_workflow', 'voip_route')),
  -- What the operator picked from the vendor's own list ("#helpdesk-alerts").
  label text NOT NULL CHECK (length(btrim(label)) BETWEEN 1 AND 200),
  -- The vendor's id for the destination. Slack: channel id (C…), because the
  -- `slack://` and `app_redirect` forms both key on the id and neither
  -- resolves a name safely (a wrong or private channel is a generic error).
  external_id text NOT NULL,
  -- The workspace/tenant the destination sits in, when the vendor needs it.
  -- Slack: team id (T…); Teams: tenant id.
  external_parent text,
  -- Vendor ids the deep link needs and that differ per kind — Teams wants
  -- tenantId, groupId, channelId and the display names for its `/l/message`
  -- form. Kept as ids only; nothing user-authored lives here.
  vendor_ids jsonb NOT NULL DEFAULT '{}'::jsonb,
  enabled boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT connector_bindings_destination_unique
    UNIQUE (org_id, connection_id, binding_kind, external_id)
);

CREATE INDEX connector_bindings_connection_idx
  ON connector_bindings (org_id, connection_id)
  WHERE enabled;

-- 3. External references — this record IS that vendor object ----------------

CREATE TABLE external_refs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- A closed list, because the resolver and every chip row branches on it.
  -- A record type Core cannot show has no surface, so it is refused here
  -- rather than stored and discovered later as a dead link.
  record_type text NOT NULL CHECK (record_type IN (
    'configuration', 'client', 'site', 'contact', 'ticket',
    'kb_article', 'invoice', 'payment', 'credit_note', 'quote',
    'purchase_order', 'domain')),
  record_id uuid NOT NULL,
  -- NULL for an import-era key that predates any live connection. The two
  -- identity indexes below are partial for exactly that reason: a live
  -- reference is unique per connection, an imported one per source system.
  connection_id uuid REFERENCES connector_connections(id) ON DELETE CASCADE,
  -- Display and import vocabulary ('ninjaone', 'qbo', 'connectwise'). Kept
  -- even when a connection is present, so a reference still reads correctly
  -- after the connection is deleted and the row survives as history.
  source_system text NOT NULL,
  external_kind text NOT NULL,
  external_id text NOT NULL,
  -- A canonical document URL the connector LEARNED from the vendor (a payload
  -- field, a report row). Used verbatim; never templated, never guessed.
  -- NULL is the honest state where the vendor publishes no deep link — QBO
  -- invoices, for instance (Intuit: transaction links are unsupported and
  -- txnId is not the API Id).
  external_url text,
  source_guid text,
  payload_ref text,
  first_seen_at timestamptz NOT NULL DEFAULT now(),
  -- NULL for a one-time import observation that no sync refreshes. A live
  -- reference always carries it, which is how "the connector went quiet" is
  -- distinguishable from "this row arrived once".
  last_seen_at timestamptz
);

-- The vendor object cannot be bound to two Marula records. This index is what
-- turns a duplicate into a merge candidate the technician decides, instead of
-- two records quietly claiming the same device.
CREATE UNIQUE INDEX external_refs_identity_live
  ON external_refs (org_id, connection_id, external_kind, external_id)
  WHERE connection_id IS NOT NULL;
-- The import-era contract (`external_ids`): the same key re-pushed resolves
-- to the same row. `source_system` is part of the key, which fixes the
-- cross-source collision the old table had.
CREATE UNIQUE INDEX external_refs_identity_import
  ON external_refs (org_id, source_system, external_kind, external_id)
  WHERE connection_id IS NULL;
-- One object of a kind per record per connection. The connection is in the
-- key because the same device legitimately appears in two instances of the
-- same vendor during a migration, and forbidding that would block the very
-- move this schema exists to make safe. NULLS NOT DISTINCT keeps the
-- import-era (connection-less) rows to one per system.
CREATE UNIQUE INDEX external_refs_one_per_record
  ON external_refs (org_id, record_type, record_id, connection_id, source_system, external_kind)
  NULLS NOT DISTINCT;
-- The resolver's read: one record's presences.
CREATE INDEX external_refs_record_idx
  ON external_refs (org_id, record_type, record_id);
-- The connection's read: what it feeds, and what went stale.
CREATE INDEX external_refs_connection_idx
  ON external_refs (org_id, connection_id, last_seen_at DESC)
  WHERE connection_id IS NOT NULL;

-- 4. Record links — this record IS RELATED TO that one ----------------------

CREATE TABLE record_links (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Same two-column shape as `document_links` (0053), on purpose: one mental
  -- model for a record edge, and a mechanical path to merge the two if the
  -- docs platform ever adopts record types beyond documents and passwords.
  -- It is a separate table because its sources are different (a ticket is not
  -- a document), its allowed pairs are narrower, and `document_links`'s
  -- resolver carries a broken-target policy this edge does not need.
  from_kind text NOT NULL CHECK (from_kind IN ('ticket', 'kb_article')),
  from_id uuid NOT NULL,
  to_kind text NOT NULL CHECK (to_kind IN ('configuration', 'kb_article')),
  to_id uuid NOT NULL,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  -- The permitted pairs, stated once. Two independent CHECKs would allow
  -- kb_article → kb_article, which no surface draws.
  CONSTRAINT record_links_pair_allowed CHECK (
    (from_kind = 'ticket' AND to_kind IN ('configuration', 'kb_article'))
    OR (from_kind = 'kb_article' AND to_kind = 'configuration')
  ),
  CONSTRAINT record_links_not_self
    CHECK (NOT (from_kind = to_kind AND from_id = to_id))
);

CREATE UNIQUE INDEX record_links_once
  ON record_links (org_id, from_kind, from_id, to_kind, to_id);
CREATE INDEX record_links_from_idx
  ON record_links (org_id, from_kind, from_id);
CREATE INDEX record_links_to_idx
  ON record_links (org_id, to_kind, to_id);

-- Polymorphic ids cannot carry a foreign key, so the owner deletes its own
-- edges in the same transaction. One function, attached per owner with its
-- record type as the argument, means adding a link pair is one trigger line
-- rather than a new hand-written function that can drift.
CREATE FUNCTION record_links_prune() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  owner_type text := TG_ARGV[0];
BEGIN
  DELETE FROM record_links
   WHERE (from_kind = owner_type AND from_id = OLD.id)
      OR (to_kind   = owner_type AND to_id   = OLD.id);
  RETURN OLD;
END;
$$;

CREATE TRIGGER tickets_record_links_prune
  AFTER DELETE ON tickets FOR EACH ROW
  EXECUTE FUNCTION record_links_prune('ticket');
CREATE TRIGGER kb_articles_record_links_prune
  AFTER DELETE ON kb_articles FOR EACH ROW
  EXECUTE FUNCTION record_links_prune('kb_article');
CREATE TRIGGER device_assets_record_links_prune
  AFTER DELETE ON device_assets FOR EACH ROW
  EXECUTE FUNCTION record_links_prune('configuration');

-- 5. Vendor identity of a person -------------------------------------------

CREATE TABLE user_external_identities (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  connection_id uuid NOT NULL REFERENCES connector_connections(id) ON DELETE CASCADE,
  -- One table, three jobs that were about to become three: the Slack user a
  -- button press authenticates as, the PBX extension a click-to-call dials
  -- from, the RMM technician an action attributes to.
  identity_kind text NOT NULL
    CHECK (identity_kind IN ('slack_user', 'teams_user', 'pbx_extension', 'rmm_technician')),
  external_id text NOT NULL,
  display text,
  -- Set when the link was proven (an OAuth round trip, a verified ping).
  -- NULL means the mapping exists but has not been demonstrated to work.
  verified_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX user_external_identities_vendor
  ON user_external_identities (org_id, connection_id, identity_kind, external_id);
CREATE UNIQUE INDEX user_external_identities_user
  ON user_external_identities (org_id, user_id, connection_id, identity_kind);

-- 6. Which provider a capability runs first --------------------------------

CREATE TABLE capability_defaults (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- NULL is the org default; a row here overrides it for one client, because
  -- a client may mandate their own remote-access tool.
  scope_client_id uuid REFERENCES clients(id) ON DELETE CASCADE,
  capability text NOT NULL,
  connection_id uuid NOT NULL REFERENCES connector_connections(id) ON DELETE CASCADE
);

-- Two partial uniques rather than a primary key over a nullable column: a
-- NULL in a key makes uniqueness silently weaker, and "two primaries for
-- remote access" is an outage, not a cosmetic bug.
CREATE UNIQUE INDEX capability_defaults_org
  ON capability_defaults (org_id, capability)
  WHERE scope_client_id IS NULL;
CREATE UNIQUE INDEX capability_defaults_client
  ON capability_defaults (org_id, scope_client_id, capability)
  WHERE scope_client_id IS NOT NULL;

-- 7. RLS -------------------------------------------------------------------

ALTER TABLE connector_connections ENABLE ROW LEVEL SECURITY;
CREATE POLICY connector_connections_org ON connector_connections
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE connector_bindings ENABLE ROW LEVEL SECURITY;
CREATE POLICY connector_bindings_org ON connector_bindings
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE external_refs ENABLE ROW LEVEL SECURITY;
CREATE POLICY external_refs_org ON external_refs
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE record_links ENABLE ROW LEVEL SECURITY;
CREATE POLICY record_links_org ON record_links
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE user_external_identities ENABLE ROW LEVEL SECURITY;
CREATE POLICY user_external_identities_org ON user_external_identities
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE capability_defaults ENABLE ROW LEVEL SECURITY;
CREATE POLICY capability_defaults_org ON capability_defaults
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- 8. What each table is for, in the catalogue ------------------------------

COMMENT ON TABLE connector_connections IS
  'One configured use of an installed connector: org-wide (scope_client_id '
  'NULL) or per client. Carries the non-secret settings the resolver needs to '
  'build a link, the sealed credential, and a status with the timestamps that '
  'make the status evidence.';
COMMENT ON TABLE connector_bindings IS
  'A named destination a connection may post to (Slack channel, Teams '
  'conversation, Rewst workflow, VoIP route), keyed by the vendor id the deep '
  'link needs — never by a display name.';
COMMENT ON TABLE external_refs IS
  'This Marula record is that vendor object. Supersedes config_source_links '
  'and external_ids. A live reference is unique per connection; an imported '
  'one is unique per source system.';
COMMENT ON TABLE record_links IS
  'A typed edge between two Marula records (ticket ↔ configuration, ticket ↔ '
  'KB article, KB article ↔ configuration). from_kind/to_kind are closed '
  'lists and the permitted pairs are one CHECK; each owner table prunes its '
  'own edges on delete.';
COMMENT ON TABLE user_external_identities IS
  'The vendor identity of a person: Slack user, Teams user, PBX extension, '
  'RMM technician. Required for an inbound action to act as the person who '
  'pressed it, and for click-to-call to dial from their own extension.';
COMMENT ON TABLE capability_defaults IS
  'The primary connection for one capability, per org, optionally overridden '
  'per client. Decides which provider a chip runs; never hides the others.';
