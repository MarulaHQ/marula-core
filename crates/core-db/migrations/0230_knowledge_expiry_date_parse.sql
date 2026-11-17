-- Migration 0221: a date parse that cannot abort the expiry rollup.
--
-- `knowledge_home::expirations` reads the warranty end a person typed into an
-- asset's custom-field JSON. The column is `jsonb`, so there is no type to lean
-- on: `(fields->>'warranty_end')::date` raises on `2026-02-30`, and because the
-- whole rollup is one statement, one mistyped asset makes the knowledge hub
-- fail for the organisation.
--
-- The shape check `~ '^\d{4}-\d{2}-\d{2}$'` is not a calendar check, so it does
-- not help. `app_parse_date_or_null` is the calendar check: it returns the date
-- when PostgreSQL accepts it and NULL when it refuses, so the read can skip the
-- row instead of raising. STRICT makes a missing field a NULL without entering
-- the body.
--
-- Additive only: one function. The read predicate that calls it is in
-- `crates/core-db/src/knowledge_home.rs`.

CREATE OR REPLACE FUNCTION app_parse_date_or_null(p_value text)
RETURNS date
LANGUAGE plpgsql
IMMUTABLE
STRICT
PARALLEL SAFE
AS $$
BEGIN
    RETURN p_value::date;
EXCEPTION
    WHEN others THEN
        RETURN NULL;
END;
$$;
