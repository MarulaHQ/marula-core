# ADR 0033: Record links, external references, and vendor identity

Status: proposed (2026-11-14)
Companions: [ADR 0032](0032-connector-connection-model.md) (connections),
`docs/reference/connector-capabilities.md` (what reads this data),
`docs/reference/connector-surface.md` (the surfaces).

## Context

Three questions the product already asks have no shared answer in the schema:

- *Which systems is this configuration in?* `config_source_links`
  (migration 0061) answers it for configurations only, keyed by a
  `source_system` **brand string**, with no connection and therefore no way
  to resolve a base URL or a credential when an MSP runs two screen-sharing
  tools, two RMMs, or two NinjaOne organisations.
- *Where did this imported row come from?* `external_ids` (migration 0027)
  answers it for imports, keyed by `(org, entity_kind, external_key)` with
  **no source system at all**, so a ConnectWise key and a Syncro key can
  collide.
- *Which invoice did we send to QuickBooks?* Nothing answers it. The export
  adapters return an `ExportReceipt` that callers store themselves, with no
  schema to store it in.

And one question it is about to ask: *which records are related to this
one?* Today a ticket cannot even name the configuration it is about.

Three near-identical mechanisms, three vocabularies, two of them missing
the one column the UI needs. That is the duplication this ADR removes.

## Decision

Four tables, each with exactly one job.

### 1. `external_refs` — this Marula record is that vendor object

Consolidates `config_source_links` and `external_ids`. One row per
(record, vendor object) claim.

```sql
CREATE TABLE external_refs (
  id            uuid PRIMARY KEY,
  org_id        uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  record_type   text NOT NULL CHECK (record_type IN (
                  'configuration','client','site','contact','ticket',
                  'kb_article','invoice','payment','credit_note','quote',
                  'purchase_order','domain')),
  record_id     uuid NOT NULL,
  -- NULL for an import-era key that predates any live connection.
  connection_id uuid REFERENCES connector_connections(id) ON DELETE CASCADE,
  -- Display + import vocabulary: 'ninjaone', 'qbo', 'connectwise'.
  source_system text NOT NULL,
  external_kind text NOT NULL,           -- 'device','invoice','company','ticket'…
  external_id   text NOT NULL,
  -- A canonical document URL learned from the vendor itself (a payload field,
  -- a report row). Used verbatim. Never templated, never guessed. NULL when
  -- the vendor published none, which is the honest state for QBO invoices.
  external_url  text,
  source_guid   text,
  payload_ref   text,
  first_seen_at timestamptz NOT NULL DEFAULT now(),
  -- NULL for a one-time import observation that no sync refreshes.
  last_seen_at  timestamptz
);

-- The vendor object itself cannot be bound to two Marula records. This is
-- what turns a merge candidate into a decision instead of a duplicate.
CREATE UNIQUE INDEX external_refs_identity_live
  ON external_refs (org_id, connection_id, external_kind, external_id)
  WHERE connection_id IS NOT NULL;
CREATE UNIQUE INDEX external_refs_identity_import
  ON external_refs (org_id, source_system, external_kind, external_id)
  WHERE connection_id IS NULL;

-- One object of a kind per record per system.
CREATE UNIQUE INDEX external_refs_one_per_record
  ON external_refs (org_id, record_type, record_id, source_system, external_kind);

-- The two access paths: record → its systems, connection → what it feeds.
CREATE INDEX external_refs_record ON external_refs (org_id, record_type, record_id);
CREATE INDEX external_refs_connection ON external_refs (org_id, connection_id)
  WHERE connection_id IS NOT NULL;
```

### 2. `record_links` — this Marula record is related to that one

```sql
CREATE TABLE record_links (
  id         uuid PRIMARY KEY,
  org_id     uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  from_kind  text NOT NULL CHECK (from_kind IN ('ticket','kb_article')),
  from_id    uuid NOT NULL,
  to_kind    text NOT NULL CHECK (to_kind IN ('configuration','kb_article')),
  to_id      uuid NOT NULL,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT record_links_pair_allowed CHECK (
    (from_kind = 'ticket' AND to_kind IN ('configuration','kb_article'))
    OR (from_kind = 'kb_article' AND to_kind = 'configuration')
  )
);
CREATE UNIQUE INDEX record_links_once
  ON record_links (org_id, from_kind, from_id, to_kind, to_id);
CREATE INDEX record_links_from ON record_links (org_id, from_kind, from_id);
CREATE INDEX record_links_to   ON record_links (org_id, to_kind, to_id);
```

The two-column shape is `document_links`' (migration 0053), deliberately. It
is still a separate table: its sources are records rather than documents, its
permitted pairs are narrower, and `document_links`' resolver carries a
broken-target policy this edge does not need. Keeping the same shape means the
two can be merged mechanically if the docs platform ever adopts record types
beyond documents and passwords — and a reviewer can see they are the same
idea, not two.

Integrity without polymorphic foreign keys: the record-kind registry in Rust
(single source, with a conformance test against the migration's CHECK lists)
generates one trigger per owning table, parameterised by record type, that
deletes that record's links in the same transaction as the delete. A
nightly sweep counts orphans and fails loudly if it finds any. A sweep that
finds none is the evidence the trigger exists.

### 3. `user_external_identities` — this person is that vendor identity

One table, three jobs that were about to become three: the Slack user id a
button press authenticates as, the PBX extension a click-to-call dials
from, and the RMM technician id an action attributes to.

```sql
CREATE TABLE user_external_identities (
  id            uuid PRIMARY KEY,
  org_id        uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  connection_id uuid NOT NULL REFERENCES connector_connections(id) ON DELETE CASCADE,
  identity_kind text NOT NULL CHECK (identity_kind IN (
                  'slack_user','teams_user','pbx_extension','rmm_technician')),
  external_id   text NOT NULL,
  display       text,
  verified_at   timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX user_external_identities_vendor
  ON user_external_identities (org_id, connection_id, identity_kind, external_id);
CREATE UNIQUE INDEX user_external_identities_user
  ON user_external_identities (org_id, user_id, connection_id, identity_kind);
```

### 4. `capability_defaults` — which provider a chip runs first

```sql
CREATE TABLE capability_defaults (
  id              uuid PRIMARY KEY,
  org_id          uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- NULL is the org default. A row here overrides it for one client.
  scope_client_id uuid REFERENCES clients(id) ON DELETE CASCADE,
  capability      text NOT NULL,
  connection_id   uuid NOT NULL REFERENCES connector_connections(id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX capability_defaults_org
  ON capability_defaults (org_id, capability) WHERE scope_client_id IS NULL;
CREATE UNIQUE INDEX capability_defaults_client
  ON capability_defaults (org_id, scope_client_id, capability)
  WHERE scope_client_id IS NOT NULL;
```

Two partial unique indexes rather than a primary key over a nullable column:
a NULL in a key makes uniqueness silently weaker, and this is the kind of
table where "two primaries" is an outage.

## The five axes

| Axis | How this satisfies it |
|---|---|
| **Efficient to maintain** | Four tables, one job each, no `jsonb` grab-bag, no per-record-type table. Two existing tables *fold in*, so the schema shrinks from three identity mechanisms to one. A new record type is one enum line, one registry entry, one trigger attach. |
| **Migrations in and out** | Export carries natural keys only: `(link_kind, from natural key, to natural key)` for links, and `(record type, natural key, source system, external kind, external id)` for references. No local uuid ever leaves the system. `external_refs` are observations and can be omitted from an export entirely — a hosted import re-derives them from the first sync. `record_links` are user data and must travel. `external_ids.entity_kind` collides across import sources today; `source_system` in the key fixes it on the way in. |
| **Speed** | Every read is one index range scan on one table: a record's systems by `(org, record_type, record_id)`, a record's links by `(org, link_kind, from_id)`. No polymorphic join to a union, no N+1: the external-actions resolver is one query joining `external_refs` → `connector_connections` → installation. |
| **Security** | RLS by `org_id` on all four (the repo's standing gate parses for it). No secrets and no tokens live here — only `connection_id`, which points at the sealed credential row. Cross-client binding is refused at write time: both ends of a link must be readable by the writer, and a client-scoped connection cannot bind a record belonging to another client. Every link and ref write is audited by the existing entity event. |
| **Plugin devs** | They never touch these tables. Two endpoints: `POST /api/v1/connections/{id}/refs` (idempotent upsert keyed on the identity unique) and `GET /api/v1/records/{type}/{id}/external-actions`. An unknown `record_type` is a typed refusal naming the allowed set, not a 500. A new capability is a manifest declaration. A new *record type* is an RFC, because a record type core cannot show is a feature the UI has to grow. |

## Migration path

1. Create the four tables.
2. Backfill `external_refs` from `external_ids` (`connection_id` NULL,
   `record_type = entity_kind`), then from `config_source_links`
   (`source_system` kept, `connection_id` resolved by matching the org's
   connection of that brand. Where the org has two, leave NULL and emit an
   unresolved-link report rather than guessing).
3. Point the import framework and `ci-feeders` writes at the new table — one
   writer each.
4. Replace both old tables with read-only views for one release so an old
   binary and a rollback still work.
5. Drop the views and `config_source_links.source_system`'s text key in the
   following release.

## Refused

- **One polymorphic `links` table for both internal and external edges.**
  The uniqueness contracts differ (a vendor object is unique per connection. A link is unique per pair), the lifecycles differ (refs are refreshed by
  sync, links are user data), and merging them puts a sync's upsert and a
  user's drag-and-drop in one unique index.
- **Per-link-type tables** (`ticket_configurations`,
  `kb_article_configurations`, …). Four tables today, twelve after a year,
  each with its own repository function, and the ticket strip would need a
  union to draw one row.
- **Folding `record_links` into `document_links`.** They are the same shape
  and that is deliberate, but merging them puts a document's broken-target
  policy and gate on a ticket edge, and widens a working resolver's
  `from_kind` to records it was never written to authorise. If the docs
  platform ever needs record types beyond documents and passwords, the merge
  is mechanical — the shapes already match.
- **Foreign keys on `record_id`.** Postgres cannot reference several
  parents. The trigger-and-sweep pair is the honest substitute, and the
  sweep failing loud is what keeps it honest.
- **A `jsonb` attributes column** on `external_refs` for "whatever the
  connector wants". Observed vendor fields already live in the feed payload
  store behind `payload_ref`. A second home for them invites two truths.
