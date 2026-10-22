-- Plugin state: one namespaced KV table, its quota counter and its erasure
-- ledger (Wave 53 lane 6, `p53-plugin-data`; ADR 0023 Decision 5).
--
-- Why these tables exist. A plugin that runs on every ticket needs somewhere to
-- keep a cursor, a cache or a dedupe key between invocations, and until now it
-- had nowhere: `crates/plugin-host/src/host_fns/kv.rs` answered every `kv_*`
-- call with `plugin_not_implemented`. Three tables, and nothing else, are what
-- Decision 5 asks for.
--
-- The shape is fixed by the ADR and no plugin defines a table, a column or an
-- index. What each table is FOR:
--
--   1. `plugin_kv_entries` — the store. `(install_id, namespace, key)` is the
--      primary key, so `install_id` alone already isolates tenants: an install
--      belongs to exactly one org, so one install cannot address another's rows
--      by construction. `org_id` is carried for the RLS policy and for the
--      quota walk, and it is what makes a mismatched (org, install) pair an
--      INSERT the policy refuses rather than a row that belongs to one tenant
--      and names another.
--
--   2. `plugin_kv_usage` — the quota, as a COUNTER ROW, not a `SUM`. Summing
--      `octet_length(value)` over an install's rows on every `kv_put` is a scan
--      of up to 100,000 rows on the hot path, and the ADR measures a put in
--      microseconds. The counter is updated in the same transaction as the
--      write it accounts for, and it is reconcilable: `plugin_kv::reconcile`
--      recomputes it from the rows and logs a mismatch, so a lost update is a
--      repair rather than a silent overrun.
--
--   3. `plugin_kv_erasure_receipts` — the proof. Erase is `DELETE` by
--      `install_id` plus a counter reset, and a receipt in the shape
--      `rag_deletion_receipts` already produces (0134): what was removed, how
--      much, a content hash per entry, who asked and when. `namespace` is `'*'`
--      because a whole-install erase spans every namespace; the column stays so
--      a narrower erase does not need a new table.
--
-- Why the receipt has no foreign key to `plugin_installs`. An uninstall deletes
-- the install, and a cascade would take the receipt with it — the proof of
-- erasure would be deleted by the erasure. `install_id` is therefore a bare
-- uuid here, and only the ORG cascades. This is the one place the three tables
-- deliberately disagree, and it is the difference between "the rows are gone"
-- and "here is the row that says the rows are gone".
--
-- What this migration deliberately does NOT do:
--
--   * No cross-install namespace and no org-level bucket. Decision 5 is
--     explicit: sharing between two plugins is a mediated call, not a shared
--     table, because a shared bucket has no owner and so no erasure story.
--   * No sweeper. `expires_at` is a per-row TTL, and the retention matrix
--     (`core_db::retention::MATRIX`) enforces per-table age windows through
--     `updated_at`, which is a different mechanism. `plugin_kv::sweep_expired`
--     is the statement; registering it is a follow-up outside this lane's
--     files. Reads filter `expires_at` in the same statement, so an expired key
--     is invisible the moment it expires, sweep or no sweep.
--   * No index on `(install_id, namespace, key)` beyond the primary key. The
--     PK already serves `kv_get`, `kv_delete`, `kv_list`'s prefix scan and the
--     export's `(namespace, key)` ordering, all as prefix scans of one install.
--
-- RLS doctrine: org-scoped user data, org-bound reads (assert_rls_coverage).
-- The policy form is 0154's, verbatim, because it is the shape the neighbouring
-- plugin table ships and one plugin-state family should not carry two.

CREATE TABLE plugin_kv_entries (
  -- The tenant. Bound by the host from the invocation context, never by the
  -- guest, and checked by the policy below so a pair that disagrees cannot be
  -- written at all.
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The install that owns the row. `ON DELETE CASCADE` is the uninstall path's
  -- backstop: an uninstall that forgot to call `plugin_kv::erase` still leaves
  -- no state behind.
  install_id uuid NOT NULL REFERENCES plugin_installs(id) ON DELETE CASCADE,
  -- The guest-chosen label. The host validates `[a-z0-9_.-]{1,64}` before it
  -- reaches SQL; the CHECK is the backstop that keeps a hand-written query from
  -- inventing a label the ABI would refuse.
  namespace text NOT NULL,
  -- The guest's key. `text` rather than `bytea` so the export and the list page
  -- are ordered by a value a human can read.
  key text NOT NULL,
  -- Opaque bytes. The core stores them and returns them and never parses them:
  -- serialization is the plugin's business, and an unparseable value comes back
  -- as an unparseable value.
  value bytea NOT NULL,
  -- TTL. NULL means "no expiry". A row past this instant is invisible to
  -- `kv_get` and `kv_list` and is removed by `plugin_kv::sweep_expired`.
  expires_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (install_id, namespace, key),
  CONSTRAINT plugin_kv_entries_namespace_shape
    CHECK (namespace ~ '^[a-z0-9_.-]{1,64}$'),
  CONSTRAINT plugin_kv_entries_key_shape
    CHECK (key <> '' AND length(key) <= 512),
  -- Mirrors `abi::KV_VALUE_MAX_BYTES`. The host refuses an oversized value with
  -- `plugin_kv_value_too_large` before any statement runs; this is the backstop
  -- for a caller that bypasses the host.
  CONSTRAINT plugin_kv_entries_value_size
    CHECK (octet_length(value) <= 1048576)
);

-- The sweep's index: partial, because most rows have no TTL and the common case
-- is an empty result.
CREATE INDEX plugin_kv_entries_expiry_idx
  ON plugin_kv_entries (expires_at)
  WHERE expires_at IS NOT NULL;

ALTER TABLE plugin_kv_entries ENABLE ROW LEVEL SECURITY;
CREATE POLICY plugin_kv_entries_org ON plugin_kv_entries
  USING (org_id = current_setting('app.org_id', true)::uuid);

COMMENT ON TABLE plugin_kv_entries IS
  'The install-scoped byte store behind kv_get/kv_put/kv_delete/kv_list. '
  'Primary key (install_id, namespace, key): the guest names only the label, '
  'and the host fills org_id and install_id from the invocation context, so a '
  'plugin cannot address another install''s rows even if its own authorization '
  'is broken.';

CREATE TABLE plugin_kv_usage (
  -- One row per install: the unit the quota applies to.
  install_id uuid PRIMARY KEY REFERENCES plugin_installs(id) ON DELETE CASCADE,
  -- Carried for the RLS policy and the operator's per-org view.
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Bytes currently held by this install's entries.
  bytes bigint NOT NULL DEFAULT 0,
  -- Keys currently held by this install's entries.
  keys integer NOT NULL DEFAULT 0,
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT plugin_kv_usage_bytes_nonnegative CHECK (bytes >= 0),
  CONSTRAINT plugin_kv_usage_keys_nonnegative CHECK (keys >= 0)
);

CREATE INDEX plugin_kv_usage_org_idx ON plugin_kv_usage (org_id);

ALTER TABLE plugin_kv_usage ENABLE ROW LEVEL SECURITY;
CREATE POLICY plugin_kv_usage_org ON plugin_kv_usage
  USING (org_id = current_setting('app.org_id', true)::uuid);

COMMENT ON TABLE plugin_kv_usage IS
  'The per-install KV quota counter, written in the same transaction as the '
  'entry it accounts for. A counter rather than SUM(octet_length(value)) '
  'because the sum is a scan of up to 100,000 rows on the put path. '
  'plugin_kv::reconcile recomputes it from plugin_kv_entries and logs a '
  'mismatch, so a lost update is a repair rather than a silent overrun.';

CREATE TABLE plugin_kv_erasure_receipts (
  receipt_id uuid PRIMARY KEY,
  -- The org cascades: a purged org takes its proof with it, exactly as
  -- `org_erasure_receipts` does for the org-level erasure.
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- NO foreign key, deliberately. See the header: a cascade from
  -- `plugin_installs` would delete the proof of the erase that removed the
  -- install.
  install_id uuid NOT NULL,
  -- `'*'` for a whole-install erase; a label when a future erase is narrower.
  namespace text NOT NULL,
  -- `'plugin_kv'`, the same role `rag_deletion_receipts.source_kind` plays.
  source_kind text NOT NULL,
  -- The install id as text, the same role `source_id` plays there.
  source_id text NOT NULL,
  deleted_entries integer NOT NULL,
  deleted_bytes bigint NOT NULL,
  -- One sha256 per removed value, ordered by (namespace, key): enough to prove
  -- which values went without keeping any of them.
  content_hashes text[] NOT NULL DEFAULT '{}'::text[],
  deleted_by uuid REFERENCES users(id) ON DELETE SET NULL,
  deleted_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT plugin_kv_erasure_receipts_namespace_shape
    CHECK (namespace = '*' OR namespace ~ '^[a-z0-9_.-]{1,64}$'),
  CONSTRAINT plugin_kv_erasure_receipts_entries_nonnegative
    CHECK (deleted_entries >= 0),
  CONSTRAINT plugin_kv_erasure_receipts_bytes_nonnegative
    CHECK (deleted_bytes >= 0)
);

CREATE INDEX plugin_kv_erasure_receipts_org_install_idx
  ON plugin_kv_erasure_receipts (org_id, install_id, deleted_at DESC);
CREATE INDEX plugin_kv_erasure_receipts_org_time_idx
  ON plugin_kv_erasure_receipts (org_id, deleted_at DESC, receipt_id DESC);

ALTER TABLE plugin_kv_erasure_receipts ENABLE ROW LEVEL SECURITY;
CREATE POLICY plugin_kv_erasure_receipts_org ON plugin_kv_erasure_receipts
  USING (org_id = current_setting('app.org_id', true)::uuid);

COMMENT ON TABLE plugin_kv_erasure_receipts IS
  'Append-only proof of a plugin KV erase: how many entries and bytes went, a '
  'sha256 per removed value, who asked and when. Same vocabulary as '
  'rag_deletion_receipts (0134) rather than a second receipt shape.';
