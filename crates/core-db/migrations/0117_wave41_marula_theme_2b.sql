-- Wave 41 lane 2b (p41-marula-theme) — the D7 density key and the D12
-- data-surface persistence.
--
-- Both keys are NEW rather than renamed, so unlike migration 0115 (the
-- `tangerine` -> `marula` preset id) there is no old value here to
-- supersede and no row to rewrite. The migration is the storage half of the
-- two keys, and it does three things that the typed registry alone cannot:
--
-- 1. A CHECK constraint that holds the D7 step at the DATABASE level. The
--    registry validates on the write path, and `core-domain`'s test asserts
--    the option list, but a settings row can also arrive by a restore, a
--    support fix, or a future code path that writes the table directly. An
--    illegal step in the database is not a cosmetic problem: the attribute
--    writer has no token block for a step the ladder does not define, so
--    every control would silently fall back while the UI claimed a step the
--    user did not pick. The constraint is a no-op for every other key
--    (`key <> ... OR ...`), so it constrains exactly one key and only the
--    user-scope rows that may hold it.
--
-- 2. Normalization of any row that predates the constraint. This matches
--    ZERO rows on a fresh install, because `appearance.density` did not
--    exist before this migration. It is written anyway, on the same reasoning
--    migration 0097 used for the `appearance.theme` -> `appearance.mode`
--    rename: a row that cannot be valid must not be left in place for a later
--    migration to argue about, and a restore from before the constraint is
--    the one path that can produce one.
--
--    The normalization deliberately does NOT back-fill `dense`. A pre-Marula
--    install has no density row at all, so it lands on `comfortable` — which
--    is the decision: D7 makes comfortable the product's default for every
--    account, and a self-hoster who wants the old measurements picks `dense`
--    once. Writing `dense` here would preserve old pixels at the cost of
--    shipping the wrong default to everyone who never had a preference, and
--    those two mistakes are not symmetric.
--
-- 3. The same guard for the D12 document's SHAPE. `appearance.data_surfaces`
--    is a JSON document stored as a string, because `settings.value` carries
--    the check `jsonb_typeof(value) IN ('string', 'number', 'boolean')`
--    (migration 0018) and an object is refused by the database before any
--    application code sees it. Every literal below is therefore a jsonb
--    string — `'…'::jsonb`, never a bare text literal, which would not compare
--    equal and would match zero rows. The document's CONTENTS (closed operator
--    and direction vocabularies, column bounds, the per-surface caps) are
--    enforced by `core_domain::data_surface`, which is where the vocabularies
--    live; only the outer object is a database-level concern.
--
-- RLS: neither key adds a table. Both land in `settings`, whose policy
-- `settings_org` (migration 0018) admits org-scope rows org-wide and
-- user-scope rows only to the owning `app.user_id`, so per-user state needs
-- no new policy and cannot leak sideways inside an org. The `rls_coverage`
-- suite asserts that coverage, and the wave-41 registry test in
-- `core-domain` asserts both keys are user scope.

-- 1. Hold the D7 step at the database level. NULLB values are not a case the
--    column allows (`value` is NOT NULL), so the three literals are the whole
--    legal set for this key.
ALTER TABLE settings
  ADD CONSTRAINT settings_appearance_density_step CHECK (
    key <> 'appearance.density'
    OR (
      scope = 'user'
      AND value IN ('"comfortable"'::jsonb, '"compact"'::jsonb, '"dense"'::jsonb)
    )
  );

-- 2. Normalize a row the constraint above would refuse. Zero rows on a fresh
--    install; the restore path is why it is here.
UPDATE settings
   SET value = '"comfortable"'::jsonb
 WHERE key = 'appearance.density'
   AND NOT (
     scope = 'user'
     AND value IN ('"comfortable"'::jsonb, '"compact"'::jsonb, '"dense"'::jsonb)
   );

-- 3. The D12 document must be a JSON string whose contents are an object.
--
--    This one needs an exception handler, and the reason is worth writing
--    down because the obvious formulation is wrong. `jsonb_typeof()` takes
--    jsonb, not text, so a guard written as `jsonb_typeof(value #>> '{}')` is a
--    type error at migration time (42883). The fix is not to cast instead —
--    `jsonb_typeof((value #>> '{}')::jsonb)` raises on a string that is not
--    valid JSON, which is precisely the malformed case this statement exists
--    to repair, and a migration that fails halfway leaves the worse value in
--    place. Postgres has no TRY_CAST, so the safe form is an EXCEPTION block:
--    attempt the cast, and treat a failure as "not an object".
DO
$$
DECLARE
    v_key constant text := 'appearance.data_surfaces';
    v_scope constant text := 'user';
    v_row record;
BEGIN
    FOR v_row IN
        SELECT ctid, value
          FROM settings
         WHERE key = v_key
           AND scope = v_scope
    LOOP
        IF jsonb_typeof(v_row.value) <> 'string' THEN
            UPDATE settings SET value = '"{}"'::jsonb WHERE ctid = v_row.ctid;
            CONTINUE;
        END IF;

        BEGIN
            IF jsonb_typeof((v_row.value #>> '{}')::jsonb) = 'object' THEN
                CONTINUE;
            END IF;
            UPDATE settings SET value = '"{}"'::jsonb WHERE ctid = v_row.ctid;
        EXCEPTION WHEN others THEN
            -- The stored text is not JSON at all. Repair it the same way.
            UPDATE settings SET value = '"{}"'::jsonb WHERE ctid = v_row.ctid;
        END;
    END LOOP;
END
$$;
