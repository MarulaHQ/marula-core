-- Provenance diff hardening (the private build tracker backend-gaps lane, 2026-09-10).
--
-- Bug: a delete via the instant-first write path records `after: 'null'`
-- (the jsonb NULL SCALAR, not SQL NULL) into entity_writes — e.g. BYOK
-- key removal (routes/byok.rs binds serde_json Value::Null). The 0043
-- `provenance_diff` fed that scalar straight into `jsonb_object_keys`,
-- which errors on non-object input; COALESCE did not catch it because a
-- jsonb scalar is not SQL NULL. Every delete-shaped write with a null
-- scalar `after` then failed the whole insert (HTTP 500).
--
-- Fix: guard BOTH parameters with `jsonb_typeof(x) = 'object'` — any
-- non-object input (scalar, array, SQL NULL) reads as the empty object,
-- so the diff projects nothing for a delete while the write-trail row
-- itself (before snapshot, op=delete) still lands untouched.
--
-- CREATE OR REPLACE only: same signature, no data rewrite needed — the
-- trigger (provenance_project_write) and the 0043 backfill pick the new
-- body up on the next insert; previously projected rows were either
-- correct (object after/before) or never inserted (they failed).

CREATE OR REPLACE FUNCTION provenance_diff(p_before jsonb, p_after jsonb)
RETURNS TABLE (field text, prior jsonb, applied jsonb)
LANGUAGE sql STABLE
AS $fn$
  SELECT changed.key,
         CASE WHEN jsonb_typeof(p_before) = 'object' THEN p_before -> changed.key END,
         CASE WHEN jsonb_typeof(p_after) = 'object' THEN p_after -> changed.key END
  FROM (
    SELECT key FROM jsonb_object_keys(CASE WHEN jsonb_typeof(p_before) = 'object' THEN p_before ELSE '{}'::jsonb END) AS k(key)
    UNION
    SELECT key FROM jsonb_object_keys(CASE WHEN jsonb_typeof(p_after) = 'object' THEN p_after ELSE '{}'::jsonb END) AS k(key)
  ) changed
  WHERE (CASE WHEN jsonb_typeof(p_before) = 'object' THEN p_before -> changed.key END)
        IS DISTINCT FROM
        (CASE WHEN jsonb_typeof(p_after) = 'object' THEN p_after -> changed.key END)
  ORDER BY changed.key;
$fn$;
