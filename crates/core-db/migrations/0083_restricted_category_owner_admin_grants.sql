-- Restricted-category owner/admin grants (Wave 16 lane 10, ADR 0006
-- coordinator-resolved product decision).
--
-- Purpose: the ONE permitted schema-adjacent change for this lane and, by
-- the lane contract, a DATA-ONLY migration — no new tables, no column
-- changes, no RLS pollution. The 0078 vault trees shipped deny-by-default
-- with ZERO seeded `secret_category_grants` rows (the lane-9
-- registrations sweep deliberately filed that posture back for a product
-- decision). Directive (2026-09-17): deny-by-default stays for members,
-- but the org owner/admin must never be locked out of their own vault.
-- This migration backfills, for EVERY existing org, `can_read`+`can_edit`
-- grant rows on every RESTRICTED category chain for the org's Owner and
-- Admin builtin role rows.
--
-- Invariants:
--   * idempotent backfill — the unique `secret_grants_role_folder_key`
--     cover makes reruns exact no-ops (ON CONFLICT DO NOTHING); a rerun
--     must insert zero rows. The org creation path seeds the same rows at
--     runtime (core-db seed path post-0078), and the backfill must agree
--     with it (same action set: read + edit).
--   * never widens member roles: only `roles.builtin = 'owner'|'admin'`
--     rows are touched; the deny-by-default posture for every other role
--     is untouched (ADR 0006).
--   * non-restricted folders are untouched — the matrix already gates
--     them; grants are meaningful only on restricted chains.
--   * the 0078 immutable-seed + fork discipline is untouched: no folder
--     row is modified; grants are companion rows.

INSERT INTO secret_category_grants
  (id, org_id, role_id, folder_id, can_read, can_edit, created_by, updated_by)
SELECT gen_random_uuid(),
       f.org_id,
       r.id,
       f.id,
       true,
       true,
       f.created_by,
       f.created_by
FROM secret_folders f
JOIN roles r
  ON r.org_id = f.org_id
 AND r.builtin IN ('owner', 'admin')
WHERE f.restricted
ON CONFLICT (role_id, folder_id) DO NOTHING;
