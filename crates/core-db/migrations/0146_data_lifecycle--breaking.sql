-- Data lifecycle: one retention matrix, an object-integrity ledger, and an
-- erasure receipt (Wave 52 lane R6, `p52-data-lifecycle-dr`).
--
-- BREAKING flag rationale (scripts/check-migrations.sh): `auth_attempt_log` is
-- rebuilt as a range-partitioned table, so the legacy shell is dropped. No row
-- is lost: every row is copied into the new parent inside this migration's
-- transaction. The table is write-locked for the copy because a partition key
-- cannot be added in place — an upgrade window, not a data-loss step. A dump
-- taken before this migration restores normally: the rebuild runs against the
-- restored database, not against the dump.
--
-- Why partition `auth_attempt_log` and patch-delete the other eight tables in
-- the matrix:
--
--   * `auth_attempt_log` is append-only, ordered by one timestamp, has no
--     foreign key pointing at it, no view or materialised view reading it, and
--     its window is short. That combination is what a range partition earns
--     expiry by `DROP TABLE`: the delete is metadata work instead of a scan
--     that leaves dead tuples for autovacuum.
--   * Every other candidate has at least one property that makes partitioning
--     the wrong tool. `jobs`, `email_outbound` and `webhook_deliveries` are
--     UPDATE-heavy state machines whose rows are claimed and re-armed; a
--     partition key of `created_at` would put a row's lifecycle in one
--     partition but its eligibility in another. `email_messages` and
--     `notifications` are read by ticket/user and pruned by an age predicate
--     that is not the leading column of any of their indexes.
--     `consumer_deliveries` has a composite primary key on identity columns.
--     `dead_events` carries a unique index on `(consumer_group, entry_id)` that
--     would have to include the partition key to stay enforceable, and R3
--     deliberately made that table's shape a contract. `audit_events` is the
--     one table where partitioning would genuinely pay eventually, and the
--     verdict is written down in `docs/reference/data-lifecycle.md`: it is
--     deferred because the conversion costs an exclusive rewrite AND a
--     recreate of the `org_usage` read model that reads it, at a row count
--     where a batched delete on a fresh timestamp index is still measured in
--     seconds. The trigger to revisit is written beside it.
--
-- Non-purpose: no per-tenant windows (retention is a platform policy; the
-- windows live in `core_db::retention` and are overridden by environment, not
-- by a settings row), no export-before-purge path (the audit export belongs to
-- the compliance surface), and no change to how rows are written apart from
-- removing one global delete from the login-failure path, described in §5.

-- ---------------------------------------------------------------------------
-- 1. `auth_attempt_log` becomes a monthly range partition
-- ---------------------------------------------------------------------------
--
-- Shape copied from 0088 verbatim except for the primary key, which has to
-- carry the partition key: Postgres requires every unique constraint on a
-- partitioned table to include the partition columns. `id` is a uuid and stays
-- the identity; `(id, occurred_at)` is the same guarantee with the partition
-- column appended, and no application query joins on the primary key anyway.

ALTER TABLE auth_attempt_log RENAME TO auth_attempt_log_legacy;

-- The legacy shell keeps its relation, its constraint and its index NAMES when it
-- is renamed, and those names are schema-scoped while the table that replaces it
-- must reuse them. Rename the primary-key constraint and drop the three indexes
-- (which this migration recreates on the new parent) so the new table can claim
-- the names. No data is touched by any of this.
ALTER TABLE auth_attempt_log_legacy
  RENAME CONSTRAINT auth_attempt_log_pkey TO auth_attempt_log_legacy_pkey;
DROP INDEX IF EXISTS idx_auth_attempts_subject;
DROP INDEX IF EXISTS idx_auth_attempts_ip;
DROP INDEX IF EXISTS idx_auth_attempts_retention;

CREATE TABLE auth_attempt_log (
  id uuid NOT NULL,
  -- NULL for pre-auth rows (org not yet resolved).
  org_id uuid REFERENCES orgs(id) ON DELETE CASCADE,
  -- Throttled surface: 'password_login' | 'mfa_verify' | 'webauthn_login'
  -- | 'webauthn_begin' | 'refresh' | 'password_reset' | 'email_verify'
  -- | 'magic_link'.
  kind text NOT NULL,
  -- Per-actor key: 'email:<lowered email>' when known.
  subject_key text,
  -- Per-IP key: 'ip:<sha256 hex>' (server-salt).
  ip_key text,
  occurred_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (id, occurred_at)
) PARTITION BY RANGE (occurred_at);

-- Indexes on the parent are cloned onto every partition, including the ones
-- the maintainer creates later. That is why the parent declares all three
-- rather than the migration creating them per child.
CREATE INDEX idx_auth_attempts_subject
  ON auth_attempt_log (kind, subject_key, occurred_at);
CREATE INDEX idx_auth_attempts_ip
  ON auth_attempt_log (kind, ip_key, occurred_at);
-- The retention/cutoff index. On the parent it also serves a fallback delete
-- against the DEFAULT partition, which is the one partition no future
-- `PARTITION OF` statement will ever clone these onto.
CREATE INDEX idx_auth_attempts_retention
  ON auth_attempt_log (occurred_at);

ALTER TABLE auth_attempt_log ENABLE ROW LEVEL SECURITY;
-- Same predicate as 0088: a pre-auth row (org NULL) is visible for the
-- counting path, everything else is scoped to the bound org.
CREATE POLICY auth_attempt_log_org ON auth_attempt_log
  USING (
    org_id IS NULL
    OR org_id = NULLIF(current_setting('app.org_id', true), '')::uuid
  );

-- Partitions cover the legacy data's whole span plus the current month and
-- three ahead, so the application works before the first housekeeping pass
-- runs. Empty partitions cost metadata only; a five-year history is sixty
-- small tables, not sixty scans.
DO $$
DECLARE
  first_month date;
  horizon date;
  m date;
  part text;
BEGIN
  SELECT date_trunc('month', min(occurred_at))::date
    INTO first_month
    FROM auth_attempt_log_legacy;
  IF first_month IS NULL THEN
    first_month := date_trunc('month', now())::date;
  END IF;
  horizon := (date_trunc('month', now()) + interval '4 months')::date;
  m := first_month;
  WHILE m < horizon LOOP
    part := 'auth_attempt_log_p' || to_char(m, 'YYYY_MM');
    -- Bound literals carry an explicit +00 offset. A bare date is coerced to
    -- timestamptz through the session's TimeZone, which on a non-UTC session
    -- gives a month boundary that is not the month boundary — a gap or an
    -- overlap at midnight, and an INSERT that lands in the wrong partition.
    EXECUTE format(
      'CREATE TABLE IF NOT EXISTS %I PARTITION OF auth_attempt_log '
      'FOR VALUES FROM (%L) TO (%L)',
      part,
      to_char(m, 'YYYY-MM-DD') || ' 00:00:00+00',
      to_char(m + interval '1 month', 'YYYY-MM-DD') || ' 00:00:00+00'
    );
    -- RLS on the child as well as the parent: a direct query against a
    -- partition is not covered by the parent's policy, the RLS-coverage gate
    -- reads every relation of kind 'r', and the restore verify leg does the
    -- same. Both predicates are identical, so access through the parent is
    -- unchanged.
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', part);
    EXECUTE format(
      'CREATE POLICY %I ON %I USING (org_id IS NULL OR org_id = '
      'NULLIF(current_setting(''app.org_id'', true), '''')::uuid)',
      part || '_org', part
    );
    m := (m + interval '1 month')::date;
  END LOOP;
END
$$;

-- The safety net for the failure this table must not have: an insert outside
-- every partition must not be refused, because that insert is a failed login
-- and refusing it would be a lockout the operator never asked for. Rows that
-- land here are moved by the housekeeping pass as soon as the month they
-- belong to is created.
CREATE TABLE auth_attempt_log_default PARTITION OF auth_attempt_log DEFAULT;
ALTER TABLE auth_attempt_log_default ENABLE ROW LEVEL SECURITY;
CREATE POLICY auth_attempt_log_default_org ON auth_attempt_log_default
  USING (
    org_id IS NULL
    OR org_id = NULLIF(current_setting('app.org_id', true), '')::uuid
  );

INSERT INTO auth_attempt_log (id, org_id, kind, subject_key, ip_key, occurred_at)
SELECT id, org_id, kind, subject_key, ip_key, occurred_at
  FROM auth_attempt_log_legacy;

DROP TABLE auth_attempt_log_legacy;

-- Explicit rather than relying on the schema's ALTER DEFAULT PRIVILEGES, for
-- the reason 0144 gives: a permission error on an insert that looks correct is
-- a bad hour to spend. A partition created later inherits the same defaults.
GRANT SELECT, INSERT, UPDATE, DELETE ON auth_attempt_log TO marula_app;

-- ---------------------------------------------------------------------------
-- 2. `audit_events` gets the cutoff index a cross-tenant delete needs
-- ---------------------------------------------------------------------------
--
-- 0001's `idx_audit_org_time` is `(org_id, occurred_at DESC)`: perfect for the
-- audit page, useless for "delete everything older than N days across every
-- org", which cannot use a leading-column predicate. The retention sweep
-- batches on this index; without it the first pass after an upgrade is a
-- sequential scan of the largest table in the database.

CREATE INDEX IF NOT EXISTS idx_audit_occurred
  ON audit_events (occurred_at);

-- ---------------------------------------------------------------------------
-- 3. An attachment row can say its bytes are gone
-- ---------------------------------------------------------------------------
--
-- The object store and the database are two systems with no shared
-- transaction, and the failure mode that actually happens is silent: a restore
-- from a dump older than the object snapshot, a bucket rule, an operator
-- emptying a volume. Today the row keeps advertising a download that 500s. A
-- timestamp turns that into a state a surface can report ("no longer
-- available") and an operator can list. It is set by the object-integrity pass
-- and cleared by a re-upload that stores the same content address — the key is
-- a digest, so a matching re-upload IS the repair.

ALTER TABLE attachments ADD COLUMN IF NOT EXISTS missing_since timestamptz;
ALTER TABLE documents_attachments ADD COLUMN IF NOT EXISTS missing_since timestamptz;

-- Listing what is broken must not scan the healthy majority.
CREATE INDEX IF NOT EXISTS idx_attachments_missing
  ON attachments (org_id, missing_since)
  WHERE missing_since IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_documents_attachments_missing
  ON documents_attachments (org_id, missing_since)
  WHERE missing_since IS NOT NULL;

-- ---------------------------------------------------------------------------
-- 4. What the object-integrity pass saw, and what it did
-- ---------------------------------------------------------------------------
--
-- Two directions, one ledger:
--   * `orphan_deleted` — bytes with no row referencing them. Uploaded and never
--     committed, a row deleted while the store write failed, a restore that
--     brought back an older row set.
--   * `bytes_missing` — a row whose bytes are gone. The finding is what an
--     operator works from; `attachments.missing_since` is what the download
--     surface reads.
--
-- One row per finding per pass, keyed by `pass_id` so a pass is reviewable as
-- a unit and two passes cannot overwrite each other's evidence.

CREATE TABLE object_integrity_findings (
  id uuid PRIMARY KEY,
  -- Groups every finding of one integrity pass.
  pass_id uuid NOT NULL,
  found_at timestamptz NOT NULL DEFAULT now(),
  kind text NOT NULL CHECK (kind IN ('orphan_deleted', 'bytes_missing')),
  -- The key's own namespace carries the org (`org/<uuid>/...`), so this is
  -- derived rather than invented. NULL only for a key that predates the
  -- convention, which the pass reports instead of guessing.
  org_id uuid REFERENCES orgs(id) ON DELETE CASCADE,
  storage_key text NOT NULL,
  -- Bytes reclaimed (orphan) or bytes that should have been there (missing).
  size_bytes bigint,
  -- Which table referenced the key, for a missing-bytes finding.
  source_table text,
  -- Set when a later pass observes the same key healthy again.
  resolved_at timestamptz,
  note text
);

CREATE INDEX idx_object_findings_pass
  ON object_integrity_findings (pass_id, kind);
CREATE INDEX idx_object_findings_key
  ON object_integrity_findings (storage_key);
CREATE INDEX idx_object_findings_open
  ON object_integrity_findings (org_id, found_at DESC)
  WHERE resolved_at IS NULL;

ALTER TABLE object_integrity_findings ENABLE ROW LEVEL SECURITY;
CREATE POLICY object_integrity_findings_org ON object_integrity_findings
  USING (
    org_id IS NULL
    OR org_id = NULLIF(current_setting('app.org_id', true), '')::uuid
  );

GRANT SELECT, INSERT, UPDATE, DELETE ON object_integrity_findings TO marula_app;

-- ---------------------------------------------------------------------------
-- 5. The login-failure path stops pruning
-- ---------------------------------------------------------------------------
--
-- 0088's `app_auth_gate_fail` runs `DELETE FROM auth_attempt_log WHERE
-- occurred_at < now() - interval '1 day'` on EVERY failed attempt. That is a
-- global write on the unauthenticated path, executed once per failed password:
-- under a brute-force run it is the most expensive statement an anonymous
-- caller can trigger, and its cost grows with the table's dead-tuple debt. It
-- also made the window unenforceable — one day, decided by a comment, on a
-- table the partition policy now owns.
--
-- The function keeps its contract exactly (same signature, same return, same
-- counters) and loses the delete. `core_db::retention` owns the window from
-- here, and the partition drop is free.

CREATE OR REPLACE FUNCTION app_auth_gate_fail(
  p_kind text, p_lock_keys text[], p_org_id uuid, p_subject text, p_ip text
)
RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
  v_now timestamptz := clock_timestamp();
BEGIN
  -- Durable evidence row (subject_key/ip_key pre-shaped by the caller).
  INSERT INTO auth_attempt_log (id, org_id, kind, subject_key, ip_key)
  VALUES (
    gen_random_uuid(), p_org_id, p_kind,
    p_subject, p_ip
  );

  -- Retention deliberately absent here: the housekeeping pass owns it, and the
  -- attempt log is partitioned so its expiry is a DROP rather than a delete
  -- this path would pay for on behalf of every anonymous caller.

  -- Bump EVERY key (per-actor AND per-IP lock independently of each other).
  INSERT INTO auth_lockouts (kind, lock_key, org_id, attempts, locked_until, last_failure_at)
  SELECT p_kind, k, p_org_id, 1, v_now, v_now FROM unnest(p_lock_keys) AS k
  ON CONFLICT (kind, lock_key) DO UPDATE
    SET attempts = auth_lockouts.attempts + 1,
        last_failure_at = v_now,
        org_id = COALESCE(auth_lockouts.org_id, EXCLUDED.org_id);

  -- Recompute locked_until from the NEW counter so backoff doubles; the
  -- last_failure_at = v_now predicate limits the update to THIS call's keys so
  -- a concurrent failure is not clobbered mid-window.
  UPDATE auth_lockouts
    SET locked_until = v_now + make_interval(secs => app_auth_backoff_secs(attempts))
    WHERE kind = p_kind AND lock_key = ANY (p_lock_keys) AND last_failure_at = v_now;

  RETURN app_auth_gate_check(p_kind, p_lock_keys);
END;
$$;

REVOKE ALL ON FUNCTION app_auth_gate_fail(text, text[], uuid, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_auth_gate_fail(text, text[], uuid, text, text)
  TO marula_app;

-- ---------------------------------------------------------------------------
-- 6. A tenant erasure leaves a receipt, not a rumour
-- ---------------------------------------------------------------------------
--
-- Deleting an org cascades through every table that names it, so the rows are
-- gone and nothing is left to say what was deleted. But a tenant erasure is a
-- decision someone made for a reason, and the evidence that it covered every
-- surface — rows, objects, index, backups — is the whole point of the
-- contract in `docs/reference/data-lifecycle.md`. This table is that evidence.
--
-- The tenant column is `purged_org_id`, deliberately: it is not a tenancy
-- column (the tenant it names no longer exists), and the RLS-coverage gate
-- keys off a column called `org_id` for the good reason that such a table is
-- user data. This one is a platform record with no tenant-readable surface and
-- no foreign key — the parent row is gone by definition.

CREATE TABLE org_erasure_receipts (
  id uuid PRIMARY KEY,
  -- No FK on purpose: the org row is deleted in the same operation.
  purged_org_id uuid NOT NULL,
  -- Slug or name captured BEFORE the delete; the only human-readable handle
  -- left once the row is gone.
  org_label text NOT NULL,
  -- Who asked: an operator identity, a ticket reference, a DSAR case id.
  requested_by text NOT NULL,
  started_at timestamptz NOT NULL,
  finished_at timestamptz,
  -- Per-surface row counts, e.g. {"tickets": 412, "time_entries": 2881, ...}.
  rows_deleted jsonb NOT NULL DEFAULT '{}'::jsonb,
  objects_deleted integer NOT NULL DEFAULT 0 CHECK (objects_deleted >= 0),
  objects_failed integer NOT NULL DEFAULT 0 CHECK (objects_failed >= 0),
  -- `completed` = rows and objects both gone. `objects_incomplete` = the rows
  -- are gone and some bytes are not; the receipt says how many, because an
  -- erasure that silently left bytes behind is the failure this table exists
  -- to expose. Unclaimed bytes stay findable by the orphan pass.
  outcome text NOT NULL CHECK (outcome IN ('completed', 'objects_incomplete')),
  note text
);

CREATE INDEX idx_erasure_receipts_time
  ON org_erasure_receipts (started_at DESC);
CREATE INDEX idx_erasure_receipts_org
  ON org_erasure_receipts (purged_org_id);

-- ---------------------------------------------------------------------------
-- 7. The restore drill keeps its own ledger
-- ---------------------------------------------------------------------------
--
-- A backup nobody restores is a hope, and a drill whose only output is a line in
-- a container log is the same hope with extra steps. This table is where
-- `deploy/backup/drill.sh` records what it restored, what it found, and how long
-- the restore took — and the duration is the number the written RTO is checked
-- against (`docs/reference/rpo-rto.md`).
--
-- It lives in `ops` rather than `public` on purpose: it is a platform record with
-- no tenant column, and `public` is the schema the RLS-coverage gate and the
-- restore verify leg both reason about. Nothing tenant-scoped belongs here.
--
-- SECURITY: the drill connects as the table owner to restore, so no grant to
-- `marula_app` is needed or given — the application has no reason to read it.

CREATE TABLE ops.backup_drill_runs (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  started_at timestamptz NOT NULL,
  finished_at timestamptz,
  -- 'passed' means every assertion held: schema landed, RLS on, the core tables
  -- carried data. 'failed' means at least one did not, and `detail` says which.
  outcome text NOT NULL CHECK (outcome IN ('passed', 'failed')),
  -- Which backup path was rehearsed: the nightly logical dump, or a WAL-G base
  -- backup (where the drill proves retrievability, not that Postgres starts).
  source text NOT NULL CHECK (source IN ('logical_dump', 'walg_base')),
  -- Filename or WAL-G backup name, so a failure names the artifact that failed.
  artifact text NOT NULL,
  -- How long the restore took. The RTO claim in the docs is a model; this is the
  -- measurement that replaces it.
  duration_ms bigint CHECK (duration_ms IS NULL OR duration_ms >= 0),
  schema_tables integer,
  -- Row counts of the tables the drill asserts on, e.g. {"orgs": 2, ...}.
  restored_rows jsonb NOT NULL DEFAULT '{}'::jsonb,
  detail text
);

CREATE INDEX idx_backup_drill_runs_time
  ON ops.backup_drill_runs (started_at DESC);
