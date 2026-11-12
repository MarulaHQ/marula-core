-- 0183 — an ambiguous portal address resolves to nothing, not to a guess.
--
-- Why. `portal_users` is unique per org (`uq_portal_users_org_email`, 0021),
-- not globally: two tenants on one instance may each hold a contact with the
-- same mailbox. `portal_resolve_login` is the pre-tenant bridge, so it read
-- across every org and answered with `LIMIT 1` — an arbitrary row. With a
-- matching password that signed the visitor into whichever tenant sorted
-- first: another customer's tickets, under another customer's name.
--
-- The rule now: an address that names more than one live portal identity
-- resolves to NOTHING. Both callers already treat "nothing" as the uniform
-- answer, so the refusal is not an oracle for "this address exists somewhere":
-- `/api/portal/login` pays the dummy hash and returns the same 401 a wrong
-- password gets, and `/api/portal/magic-link/request` mints nothing and still
-- answers `{"ok":true}`. A disabled identity never counts toward ambiguity,
-- which is why disabling one of two restores the other.
--
-- The cost, stated plainly: a person whose mailbox is a portal identity in two
-- tenants cannot sign in by password or magic link until the tenants
-- disambiguate — an org-scoped portal host, or distinct addresses. That is the
-- honest trade against a cross-tenant login, and `portal_login_ambiguity_*` in
-- `db_suite` pins both arms (ambiguous refuses, disabling one resolves the
-- other).
--
-- The return shape is unchanged, so CREATE OR REPLACE is safe; the grants from
-- 0021 survive the replace.

CREATE OR REPLACE FUNCTION portal_resolve_login(p_email text)
RETURNS TABLE (portal_user_id uuid, org_id uuid, client_id uuid)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  WITH live AS (
    SELECT pu.id AS portal_user_id, pu.org_id, pu.client_id,
           count(*) OVER () AS matches
    FROM portal_users pu
    WHERE lower(pu.email) = lower(p_email)
      AND pu.status IN ('active', 'invited')
  )
  SELECT portal_user_id, org_id, client_id
  FROM live
  WHERE matches = 1
$$;
