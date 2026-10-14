-- Org-level resource usage for hosted-tier monitoring (TASKLIST Wave 47
-- lane A6, `p47-org-resource-quota-api`).
--
-- NUMBERING: this is 0131, not 0130. The lane claimed 0130 at start, when
-- main's head was 0129 (the workflow run ledger). Sibling lane A9
-- (`p47-client-cert-auth`) claimed the SAME 0130 and landed first, so main's
-- head became `0130_service_accounts.sql`. Two migrations sharing a version
-- number breaks sqlx's ordering, so this file was renumbered to the next
-- free slot as soon as the collision was visible. Any lane claiming a
-- migration number must re-check it against main's head immediately before
-- merge — the board claim is a reservation at claim time, not a guarantee
-- at merge time.
--
-- TWO objects, and the split between them is the lane's load-bearing
-- decision:
--
-- (A) `ops.v1_org_usage` — the READ-MODEL VIEW that DEFINES each metric:
--     one row per org, every column an exact aggregate over core tables.
--     `security_invoker = true` (the 0026/ADR 0001 convention) so row-level
--     security of the underlying tables applies exactly as if the caller
--     had queried them: an app session bound to `app.org_id` sees its own
--     org's row and nothing else; an unbound session sees nothing.
--
-- (B) `org_usage` — the MATERIALIZED TABLE the API reads. One row per org
--     (org_id is the primary key, so a refresh is an UPSERT and can never
--     fork a row), plus `refreshed_at` so staleness is visible to the
--     consumer instead of being inferred.
--
-- Why a table and not a real MATERIALIZED VIEW: `REFRESH MATERIALIZED
-- VIEW CONCURRENTLY` needs a unique index and refreshes EVERY org on
-- every call, and the metrics an org cares about move on different clocks.
-- Refreshing through the view into the table lets the worker refresh only
-- orgs whose row is STALE, keeps the row's own timestamp, and makes a
-- refresh idempotent by construction (same org_id ⇒ same row, updated).
-- The view stays the single definition of every metric, so the table can
-- never drift into a second, quieter definition.
--
-- THE POSTURE — REPORT, NEVER GATE (binding on this lane):
--   Nothing in this schema enforces anything. There is no CHECK that fails
--   a write because a count is over a limit, no trigger that throttles, and
--   no quota column that gates behaviour. `org_usage` is a READ MODEL for
--   monitoring: it answers "how much is this org using" and nothing else.
--   Configured limits are ordinary org-scoped `settings` rows (see the
--   `usage_quota.` prefix documented at the bottom of this file) — data,
--   not schema — because the hosted tier configures them per org and the
--   core does not act on them.
--
-- HONESTY (the other binding rule): every metric below is computed from a
-- real count over real columns. Two metrics that CANNOT be computed
-- honestly in a shared database are deliberately ABSENT rather than
-- faked, and the API names them as unavailable with the reason:
--   * API calls in the last 24h — there is no per-org request counter
--     surface anywhere in the core (telemetry leaves through OTLP metrics,
--     never into a queryable per-org table). Nothing to count.
--   * Database bytes for one org — Postgres has no per-row-set size; a
--     shared table's pages belong to every org that shares it. An
--     attribution would be invented.
-- The object-store figure below counts only payload bytes the core
-- RECORDS in the same transaction as the upload, and says so.
--
-- RUNTIME COST (the third binding rule — a cheap refresh is part of the
-- contract): every subquery below is written so an existing `org_id`-leading
-- index serves it. The two indexes the view needs but the core did not
-- yet have are created in the "indexes the view needs" block below, with a
-- comment naming the predicate each one exists for. A metric whose
-- predicate cannot be indexed is either restructured into a sargable shape
-- or reported as a floor — never left to seq-scan once per org per refresh.

-- ============================================================================
-- (A) the read-model view: the definition of every metric
-- ============================================================================

CREATE OR REPLACE VIEW ops.v1_org_usage WITH (security_invoker = true) AS
SELECT
  o.id AS org_id,

  -- Active users: rows the org can still log in with. NOT "accounts ever
  -- created" — a disabled account is not using anything. Served by the
  -- `users (org_id, email)` unique index.
  (SELECT count(*) FROM users u
    WHERE u.org_id = o.id AND u.is_active) AS active_users,

  -- Tickets by status CATEGORY, the closed workflow vocabulary
  -- ('open'|'in_progress'|'waiting'|'closed' — board_statuses' CHECK in
  -- 0003). Category, not status NAME: names are per-board and repeat
  -- across boards, so a name-keyed map would collapse two different
  -- statuses into one number and double-count an org's boards.
  --
  -- Four scalar subqueries, deliberately. Postgres plans each target-list
  -- subquery independently, so this is four index range scans and four
  -- nested-loop probe chains per org per refresh instead of one pass with
  -- `count(*) FILTER (…)`. Collapsing them was deferred rather than taken:
  -- the single-pass forms either change the column type (a jsonb object,
  -- which breaks the column contract `OrgUsageRow` and the test pin) or put
  -- a join in a target list, which is not valid SQL. The edit that would
  -- have shipped that is reverted — this is a KNOWN, MEASURED cost, not an
  -- accident, and closing it wants a `jsonb` column contract change (and an
  -- SDK shape change) rather than a query edit. Until then the four counts
  -- are each individually index-served, which is the binding rule.
  -- Served by the table's `UNIQUE (org_id, number)` btree; the other
  -- `org_id`-leading indexes on `tickets` are PARTIAL on non-null columns
  -- this predicate never implies, so none could serve an unrestricted
  -- per-org count.
  (SELECT count(*) FROM tickets t
     JOIN board_statuses s ON s.id = t.status_id
    WHERE t.org_id = o.id AND s.category = 'open') AS tickets_open,
  (SELECT count(*) FROM tickets t
     JOIN board_statuses s ON s.id = t.status_id
    WHERE t.org_id = o.id AND s.category = 'in_progress') AS tickets_in_progress,
  (SELECT count(*) FROM tickets t
     JOIN board_statuses s ON s.id = t.status_id
    WHERE t.org_id = o.id AND s.category = 'waiting') AS tickets_waiting,
  (SELECT count(*) FROM tickets t
     JOIN board_statuses s ON s.id = t.status_id
    WHERE t.org_id = o.id AND s.category = 'closed') AS tickets_closed,

  -- Managed devices: the org's ACTIVE client assets. Retired/disposed
  -- assets are not connected, and an inventory count would overstate
  -- what the hosted tier is supporting.
  --
  -- `device_assets.status` is deliberately FREE TEXT: migration 0054 kept
  -- `kind`/`status` "for connector compatibility", and the API accepts it
  -- as an unvalidated `String` defaulting to `'active'`. A CHECK here
  -- would break those connectors, so the match is CASE-INSENSITIVE
  -- instead (the realistic drift for a caller-supplied default is a casing
  -- variant, not a different vocabulary). Anything outside the active
  -- vocabulary still falls out of the count, so this is a FLOOR for orgs
  -- whose connectors write another status string — which is why the
  -- metric is named `managed_devices` (active inventory) rather than
  -- "connected devices": `device_assets` carries no connectivity or
  -- last-seen signal at all. Served by `idx_assets_org_scope (org_id, …)`.
  (SELECT count(*) FROM device_assets d
    WHERE d.org_id = o.id AND lower(d.status) = 'active') AS managed_devices,

  -- Object-store bytes: the SUM OF RECORDED PAYLOAD SIZES for every
  -- org-keyed object family the core tracks a size for, in the same
  -- transaction as the upload —
  --   attachments.size_bytes       (ticket/comment/email/sign-off objects)
  --   secret_records.file_bytes    (vault document ciphertext)
  --   org_exports.size_bytes       (data-export bundles)
  -- A record is the upload's other half: the row is written with the
  -- object in one transaction, so a recorded size is a real stored size.
  -- KNOWN GAP: staged import chunks (migration_push_chunks) live in the
  -- object store under an org key but record NO byte size, so they cannot
  -- be counted without inventing a number; they are excluded and the gap
  -- is tracked as a follow-up. That is the error bar: the figure is a
  -- floor for orgs that ran staged migrations, exact otherwise.
  --
   -- Each term is clamped per ROW with GREATEST(..., 0), not on the net
   -- sum. Only the vault term needs it: `secret_records.file_bytes` is a
   -- caller-supplied `i64` with no non-negative constraint on either write
   -- path, and `object_store_bytes` carries a `>= 0` CHECK. Clamping per
   -- row is the load-bearing distinction: clamping the NET sum would let
   -- one malformed negative row cancel its siblings' real bytes (an org
   -- with -1e12 and +1e9 would report 0 while ~1 GB is really stored) and
   -- still pass the CHECK — a plausible-looking number that is silently
   -- wrong by an unbounded amount. A malformed row contributing 0 keeps the
   -- figure a floor instead of a lie. The other two terms already carry
   -- `CHECK (size_bytes > 0)` (0024/0089), so their clamps are
   -- belt-and-braces against a future constraint change.
  (SELECT GREATEST(COALESCE(sum(GREATEST(a.size_bytes, 0)), 0), 0)::bigint
     FROM attachments a
    WHERE a.org_id = o.id)
  + (SELECT GREATEST(COALESCE(sum(GREATEST(v.file_bytes, 0)), 0), 0)::bigint
       FROM (
         -- Org-scope vault records: `idx_secret_records_scope` leads with
         -- org_id, so this arm is an index range scan.
         SELECT sr.file_bytes
           FROM secret_records sr
          WHERE sr.org_id = o.id
            AND sr.file_bytes IS NOT NULL
         UNION ALL
         -- Client-scope vault records: `org_id` is NULL by the exactly-one
         -- scope pair (0078), so they are attributed through their client.
         -- Driven from `idx_clients_org (org_id, name)` and served by the
         -- `idx_secret_records_client_scope` partial index below. A
         -- `COALESCE(sr.org_id, c.org_id) = o.id` predicate would mix a
         -- joined column and defeat every index, so the scope pair is
         -- split into two sargable arms instead.
         SELECT sr2.file_bytes
           FROM clients c
           JOIN secret_records sr2
             ON sr2.client_id = c.id
            AND sr2.org_id IS NULL
            AND sr2.file_bytes IS NOT NULL
          WHERE c.org_id = o.id
       ) v)
  + (SELECT GREATEST(COALESCE(sum(GREATEST(e.size_bytes, 0)), 0), 0)::bigint
     FROM org_exports e
    WHERE e.org_id = o.id) AS object_store_bytes,

  -- Automation runs: every execution attempt the run ledger recorded
  -- (0129). A run that is parked and later resumed counts twice, because
  -- it IS two ledger rows — the ledger counts work performed, not rule
  -- definitions. The all-time arm is served by
  -- `idx_workflow_runs_ledger (org_id, created_at DESC, id DESC)`; the 24h
  -- arm needs `started_at` specifically, so it is served by
  -- `idx_workflow_runs_org_started` below.
  (SELECT count(*) FROM workflow_runs r
    WHERE r.org_id = o.id) AS automation_runs,
  (SELECT count(*) FROM workflow_runs r
    WHERE r.org_id = o.id AND r.started_at >= now() - interval '24 hours')
    AS automation_runs_24h,

  -- Webhook delivery attempts: one row per ATTEMPT, not per event, so a
  -- retried delivery is three rows and three deliveries' worth of work.
  -- Served by `idx_webhook_deliveries_org_time` below — the only index the
  -- table had led with `webhook_id`, so both counts were full scans.
  (SELECT count(*) FROM webhook_deliveries w
    WHERE w.org_id = o.id) AS webhook_deliveries,
  (SELECT count(*) FROM webhook_deliveries w
    WHERE w.org_id = o.id AND w.created_at >= now() - interval '24 hours')
    AS webhook_deliveries_24h,

  -- Audit events in the last 24h: the closest HONEST proxy for API
  -- activity the core records per org. It is NOT an API-call count (see
  -- the header) — reads are not audited and writes are audited
  -- best-effort — so the API reports it under its own name and never
  -- under "api calls". Served by `idx_audit_org_time (org_id, …)`.
  --
  -- WINDOW SEMANTICS: `now()` is evaluated when the row is REFRESHED, and
  -- the resulting closed window `[refreshed_at - 24h, refreshed_at]` is
  -- stored. The API state this on each `_24h` field's schema description so
  -- a consumer never reads a window relative to the request time. As
  -- `refreshed_at` ages the window ages with it, which is exactly why the
  -- payload surfaces that timestamp beside the numbers.
  (SELECT count(*) FROM audit_events ae
    WHERE ae.org_id = o.id AND ae.occurred_at >= now() - interval '24 hours')
    AS audit_events_24h

FROM orgs o;

GRANT SELECT ON ops.v1_org_usage TO marula_app;

-- ============================================================================
-- Indexes the view needs (the two the core did not already have)
-- ============================================================================
--
-- Each one exists for exactly one predicate above. Plain `CREATE INDEX`
-- matches repo convention — no migration in 130 files uses CONCURRENTLY.
--
-- ROLLOUT COST, stated rather than glossed (this comment used to claim that
-- the migration "has not been applied to any environment yet", which
-- conflated unapplied-so-far with never-applied). 0131 unapplied *so far* is
-- true — the branch is unmerged and its SQL has never executed — but 0131
-- will be applied to every already-deployed environment, and all three
-- tables (`tickets`, `webhook_deliveries`, `secret_records`) already hold
-- rows there. A non-concurrent `CREATE INDEX` holds a SHARE lock that blocks
-- INSERT/UPDATE/DELETE on that table for the whole build, so the first
-- upgrade of a large hosted tenant write-blocks three existing tables in
-- sequence. `scripts/check-migrations.sh` gates only destructive DDL, so
-- nothing catches this — it must be in the release notes, and a tenant
-- whose table is too large for that window should split these into a
-- follow-up `CREATE INDEX CONCURRENTLY` migration (a deliberate new
-- precedent, since no migration in the repo does so today).

-- Serves the four ticket category counts. The table's `UNIQUE (org_id,
-- number)` (0004) is an unrestricted btree leading with `org_id` and already
-- serves `t.org_id = o.id`; the other `org_id`-leading indexes
-- (`idx_tickets_assignee`, `idx_tickets_type`, `idx_tickets_category`) are
-- PARTIAL on non-null columns the count never implies, so none of them
-- competes for this predicate. No new index is added — duplicating the
-- unique constraint would add write amplification to one of the hottest
-- tables in the schema for no read gain, since the `board_statuses` join
-- needs `tickets.status_id` from the heap either way.
--
-- Serves both webhook delivery counts (the 24h variant needs the time
-- column after org_id to be a range scan rather than a filter). The only
-- index the table had led with `webhook_id`, so both counts were full scans.
CREATE INDEX idx_webhook_deliveries_org_time ON webhook_deliveries (org_id, created_at DESC);

-- Serves the 24h arm of the automation-run count. Every `org_id`-leading
-- `workflow_runs` index (0129) carries `created_at`, and NO index on the
-- table contains `started_at` at all — so `started_at >= now() - interval
-- '24 hours'` was a full per-org scan of the org's entire all-time run
-- history plus a heap filter, while the all-time arm above was indexed.
-- The two columns differ on purpose (a run's ledger row can be written long
-- after the run it records started), so a key on `started_at` cannot reuse
-- the `created_at` indexes.
CREATE INDEX idx_workflow_runs_org_started ON workflow_runs (org_id, started_at DESC);

-- Serves the client-scope arm of the vault byte sum: the half of
-- `secret_records` whose `org_id` is NULL by the scope pair, looked up from
-- the client's org. Partial, so it stays small on an org that stores no
-- client-scope documents at all.
CREATE INDEX idx_secret_records_client_scope
  ON secret_records (client_id)
  WHERE org_id IS NULL AND file_bytes IS NOT NULL;

-- ============================================================================
-- (B) the materialized table the API reads
-- ============================================================================

CREATE TABLE org_usage (
  -- One row per org, ever. The primary key IS the idempotency guarantee:
  -- a refresh is an upsert keyed on it, so replaying a refresh cannot
  -- fork a second row for the same org.
  org_id uuid PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,

  active_users bigint NOT NULL DEFAULT 0 CHECK (active_users >= 0),

  tickets_open bigint NOT NULL DEFAULT 0 CHECK (tickets_open >= 0),
  tickets_in_progress bigint NOT NULL DEFAULT 0 CHECK (tickets_in_progress >= 0),
  tickets_waiting bigint NOT NULL DEFAULT 0 CHECK (tickets_waiting >= 0),
  tickets_closed bigint NOT NULL DEFAULT 0 CHECK (tickets_closed >= 0),

  managed_devices bigint NOT NULL DEFAULT 0 CHECK (managed_devices >= 0),
  object_store_bytes bigint NOT NULL DEFAULT 0 CHECK (object_store_bytes >= 0),

  automation_runs bigint NOT NULL DEFAULT 0 CHECK (automation_runs >= 0),
  automation_runs_24h bigint NOT NULL DEFAULT 0 CHECK (automation_runs_24h >= 0),
  webhook_deliveries bigint NOT NULL DEFAULT 0 CHECK (webhook_deliveries >= 0),
  webhook_deliveries_24h bigint NOT NULL DEFAULT 0 CHECK (webhook_deliveries_24h >= 0),
  audit_events_24h bigint NOT NULL DEFAULT 0 CHECK (audit_events_24h >= 0),

  -- When this row was last computed. The API surfaces it verbatim so a
  -- consumer can TELL that the numbers are stale instead of assuming they
  -- are fresh; the refresh worker only ever moves it forward.
  refreshed_at timestamptz NOT NULL DEFAULT now()
);

-- The worker's claim: which orgs are due. Leading with `org_id` so the
-- claim's `ORDER BY refreshed_at, org_id` is served by the index rather
-- than by a sort after a scan. (A `refreshed_at`-only index could serve
-- neither that ordering nor the claim's other arm.)
CREATE INDEX idx_org_usage_refreshed ON org_usage (refreshed_at, org_id);

ALTER TABLE org_usage ENABLE ROW LEVEL SECURITY;
CREATE POLICY org_usage_org ON org_usage
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- The app role READS the materialized row. Migration 0114's `ALTER DEFAULT
-- PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON
-- TABLES TO marula_app` applies to every table created afterwards in
-- `public` by the same migration-owner role, and a later `GRANT` is
-- ADDITIVE — it cannot subtract. Without the explicit revoke below the
-- app role would still hold INSERT/UPDATE/DELETE here, so an org-bound
-- session (every core-api request) could rewrite the very report the
-- hosted tier monitors. RLS would confine such a write to the caller's own
-- org, but least privilege is the point: revoke the whole surface, then
-- grant back exactly the one verb this table has a reader for.
REVOKE ALL ON org_usage FROM marula_app;
GRANT SELECT ON org_usage TO marula_app;

-- ============================================================================
-- Where configured LIMITS live: data, not schema
-- ============================================================================
--
-- A quota limit is an ordinary org-scoped `settings` row
-- (scope = 'org', owner = the org) whose key is one of —
--
--   usage_quota.active_users          usage_quota.managed_devices
--   usage_quota.tickets               usage_quota.object_store_bytes
--   usage_quota.automation_runs       usage_quota.webhook_deliveries
--
-- each holding a number. Rationale:
--   * The lane brief prefers an org-settings key over new columns, and the
--     settings table is the org-scoped key/value store that already owns
--     exactly this shape (owner = org, RLS-scoped, one row per key).
--   * One family, one prefix: an org that configures nothing reads back
--     "no limits configured" instead of a default that looks like a
--     limit. There are deliberately NO default limits — a limit the
--     hosted tier did not set does not exist, and a report that invents
--     one would be a fake gate.
--   * The `settings.value` CHECK only admits scalars (string/number/
--     boolean), which is why the family is one key per limit rather than
--     one JSON document: a document would be a different table.
--
-- The core READS these rows. It never writes them through an API surface
-- and never acts on them: read is a report (TASKLIST A6 (B)), and the
-- write belongs to whoever provisions the hosted tenant.
