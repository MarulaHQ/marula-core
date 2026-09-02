-- Undo read model v1 (TASKLIST IF7, Wave 9 lane 5).
--
-- Purpose: the per-field / per-record applied-state history the undo
-- contract names, exposed as TWO versioned `ops.v1_*` views over the 0028
-- write trail (`entity_writes`). The trail already carries everything the
-- read model needs — before/after entity snapshots, op, actor, seq,
-- timestamps — so this migration is VIEWS ONLY: no new physical table, no
-- second copy of the history, nothing to reconcile. The append-only trail
-- stays the single storage of record; undo reads it, never rewrites it
-- (the revert path in core-api appends a NEW trail row like any write).
--
--   * ops.v1_record_undo_stack — one row per applied write on a record,
--     the raw undo stack a client walks newest-first. `restorable` marks
--     steps the value-restore revert path can step back over: updates and
--     status changes carry a prior snapshot; creates do not (a create has
--     no "before", so undoing one is a withdrawal, not a field restore —
--     deliberately out of the v1 revert scope, still fully visible here).
--
--   * ops.v1_field_undo_stack — the same trail exploded to per-field
--     granularity: one row per (write × field) with the prior and new
--     value side by side (`jsonb_each` over both snapshots). `changed`
--     distinguishes real field moves from snapshot identity; clients use
--     it to render "who changed field X, from what, to what, when" and to
--     offer Ctrl/Cmd+Z per field. Fields absent from a snapshot read as
--     SQL NULL (JSON null values read as jsonb 'null' — both are lossless
--     round-trips through the client's JSON view).
--
-- BINDING POLICY (same as 0026, upgrade-compatibility-policy §2/§5):
--   * Everything downstream queries ONLY `ops.v1_*` — the view list +
--     columns below are the v1 contract; additions are safe, renames/
--     removals require a new versioned namespace.
--   * Both views are `security_invoker = true`: the caller's RLS scope on
--     `entity_writes` applies exactly as if querying the base table, so a
--     session bound via set_config('app.org_id', …) sees only its own
--     org's trail and an unbound session sees nothing (default deny).
--
-- Non-purpose: no revert EXECUTION here (that is the core-api IF1 write
-- path), no provenance aggregation (IF9 extends the audit API), no
-- scheduled pruning — the trail is append-only by design.

-- ---------------------------------------------------------------------------
-- ops.v1_record_undo_stack — one row per applied write (the undo stack).
-- Columns (v1):
--   id uuid              trail row id (PK of this view)
--   org_id uuid          owning org (RLS seam)
--   entity text          entity family ('ticket', ...)
--   entity_id uuid       record the write applied to
--   seq bigint           per-entity monotonic write number (the version)
--   op text              machine op name ('create', 'update', 'status_change',
--                        'revert', ...)
--   actor_id uuid        acting principal (user id today; actor_kind
--                        widens this per the 0028 vocabulary)
--   actor_kind text      'human' | 'agent' | ... (audit_events vocabulary)
--   idempotency_key text the IF1 key that claimed this write (nullable)
--   before jsonb         prior entity snapshot (NULL on creates)
--   after jsonb          applied entity snapshot
--   restorable boolean   the revert path can restore `before` values
--                        (op <> 'create' AND before IS NOT NULL)
--   created_at timestamptz  when the write committed
CREATE VIEW ops.v1_record_undo_stack WITH (security_invoker = true) AS
SELECT w.id,
       w.org_id,
       w.entity,
       w.entity_id,
       w.seq,
       w.op,
       w.actor_id,
       w.actor_kind,
       w.idempotency_key,
       w.before,
       w.after,
       (w.op <> 'create' AND w.before IS NOT NULL) AS restorable,
       w.created_at
FROM entity_writes w;

-- ---------------------------------------------------------------------------
-- ops.v1_field_undo_stack — per-field applied-state history.
-- Columns (v1):
--   id uuid              trail row id (NOT unique in this view — one row per
--                        field per write; key on (id, field))
--   org_id uuid          owning org (RLS seam)
--   entity text          entity family
--   entity_id uuid       record
--   seq bigint           per-entity monotonic write number
--   op text              machine op name
--   field text           snapshot key the write carried
--   prior jsonb          value BEFORE the write (NULL when absent before —
--                        e.g. every field of a create)
--   new_value jsonb      value AFTER the write (NULL when absent after)
--   changed boolean      prior IS DISTINCT FROM new (NULL-safe)
--   actor_id uuid        acting principal
--   actor_kind text      actor vocabulary (0028)
--   created_at timestamptz  when the write committed
--
-- Shape: driven by the AFTER snapshot (always present), LEFT JOINed to the
-- BEFORE snapshot on the key. Creates (before IS NULL) therefore still
-- produce one row per field with prior = NULL and changed = true. A key
-- that existed only in the before snapshot (dropped from the entity
-- shape) yields no row — the current state's fields are the contract.
-- Non-object snapshots (defensive): jsonb_each over a scalar yields no
-- rows, so malformed payloads cannot poison the field view.
CREATE VIEW ops.v1_field_undo_stack WITH (security_invoker = true) AS
SELECT w.id,
       w.org_id,
       w.entity,
       w.entity_id,
       w.seq,
       w.op,
       a.key AS field,
       b.value AS prior,
       a.value AS new_value,
       (b.value IS DISTINCT FROM a.value) AS changed,
       w.actor_id,
       w.actor_kind,
       w.created_at
FROM entity_writes w
CROSS JOIN LATERAL jsonb_each(w.after) a
LEFT JOIN LATERAL (
    SELECT key, value FROM jsonb_each(w.before)
) b ON b.key = a.key;

-- Same grant posture as 0026: the app role reads the views (invoker RLS
-- applies per-query); default privileges keep future ops views readable.
GRANT SELECT ON ALL TABLES IN SCHEMA ops TO meridian_app;
