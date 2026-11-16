-- Migration 0229: the remaining shared-row read policies read only.
--
-- 0224 narrowed `contract_clauses` to `FOR SELECT`. Four more policy families
-- carried no command scope, so their `org_id IS NULL` branch also admitted
-- UPDATE and DELETE on shared or pre-auth rows for every tenant:
--
--   * `kb_embed_providers` and `ai_surfaces` ship global rows. Neither had a
--     separate write policy, so the fix splits the old policy instead of
--     narrowing it: read stays shared, write is org-scoped, which is the write
--     surface the query modules already use.
--   * `auth_attempt_log` (parent, every month child and the DEFAULT child) and
--     `auth_lockouts` carry pre-auth rows that are instance state, not tenant
--     state. Reads keep the pre-auth branch because the counting path needs it.
--     `auth_lockouts` is written only through the 0088 SECURITY DEFINER
--     bridges, so it needs no write policy at all. `auth_attempt_log` keeps an
--     INSERT policy with the old predicate so a non-owner writer behaves
--     exactly as before. UPDATE and DELETE, which nothing in the product
--     performs, are no longer granted to every tenant.
--
-- No row is read, written or removed here: only policy scope changes. The file
-- is flagged `--breaking` because the migration gate classifies `DROP POLICY`
-- as destructive; the drops are required (a policy's command scope cannot be
-- altered in place) and each is followed by the narrower replacement.

-- ---------------------------------------------------------------------------
-- kb_embed_providers: shared registry, org-owned rows.
-- ---------------------------------------------------------------------------
DROP POLICY IF EXISTS kb_embed_providers_org ON kb_embed_providers;
CREATE POLICY kb_embed_providers_read ON kb_embed_providers
  FOR SELECT
  USING (org_id IS NULL OR org_id = current_setting('app.org_id', true)::uuid);
CREATE POLICY kb_embed_providers_write ON kb_embed_providers
  FOR ALL
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- ai_surfaces: the shipped surface list, org-owned plugin registrations.
-- ---------------------------------------------------------------------------
DROP POLICY IF EXISTS ai_surfaces_org ON ai_surfaces;
CREATE POLICY ai_surfaces_read ON ai_surfaces
  FOR SELECT
  USING (org_id IS NULL OR org_id = current_setting('app.org_id', true)::uuid);
CREATE POLICY ai_surfaces_write ON ai_surfaces
  FOR ALL
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- auth_attempt_log: read scoped, inserts unchanged, updates and deletes gone.
-- ---------------------------------------------------------------------------
DROP POLICY IF EXISTS auth_attempt_log_org ON auth_attempt_log;
CREATE POLICY auth_attempt_log_read ON auth_attempt_log
  FOR SELECT
  USING (
    org_id IS NULL
    OR org_id = NULLIF(current_setting('app.org_id', true), '')::uuid
  );
CREATE POLICY auth_attempt_log_insert ON auth_attempt_log
  FOR INSERT
  WITH CHECK (
    org_id IS NULL
    OR org_id = NULLIF(current_setting('app.org_id', true), '')::uuid
  );

-- The children carry their own policies (0146): a policy on a parent does not
-- answer a query that targets a child, and the RLS-coverage gate reads children.
DO $$
DECLARE
  part text;
BEGIN
  FOR part IN
    SELECT c.relname
      FROM pg_inherits i
      JOIN pg_class c ON c.oid = i.inhrelid
     WHERE i.inhparent = 'auth_attempt_log'::regclass
  LOOP
    EXECUTE format('DROP POLICY IF EXISTS %I ON %I', part || '_org', part);
    EXECUTE format('DROP POLICY IF EXISTS %I ON %I', part || '_read', part);
    EXECUTE format('DROP POLICY IF EXISTS %I ON %I', part || '_insert', part);
    EXECUTE format(
      'CREATE POLICY %I ON %I FOR SELECT USING (org_id IS NULL OR org_id = '
      'NULLIF(current_setting(''app.org_id'', true), '''')::uuid)',
      part || '_read', part
    );
    EXECUTE format(
      'CREATE POLICY %I ON %I FOR INSERT WITH CHECK (org_id IS NULL OR org_id = '
      'NULLIF(current_setting(''app.org_id'', true), '''')::uuid)',
      part || '_insert', part
    );
  END LOOP;
END
$$;

-- ---------------------------------------------------------------------------
-- auth_lockouts: written only through the 0088 SECURITY DEFINER bridges.
-- ---------------------------------------------------------------------------
DROP POLICY IF EXISTS auth_lockouts_org ON auth_lockouts;
CREATE POLICY auth_lockouts_read ON auth_lockouts
  FOR SELECT
  USING (
    org_id IS NULL
    OR org_id = NULLIF(current_setting('app.org_id', true), '')::uuid
  );
