-- A pin move may come from a declarative install (Wave 53 lane 4,
-- `p53-lifecycle-distribution`).
--
-- `plugin_installs.module_digest` is `''` when a version ships no module: the
-- declarative-only case that every install starts from. `commit_pending_version`
-- parks the outgoing state by copying that column into `previous_module_digest`,
-- so consenting to a module on a declarative install parks `''` — the honest
-- answer to "where did this come from".
--
-- 0156 allowed the empty digest on the live column and not on the parked one, so
-- that move failed with `23514` instead of recording the move. Only the check is
-- wrong: a parked empty digest means the install was declarative before, which is
-- exactly what a rollback needs to restore.
ALTER TABLE plugin_installs
  DROP CONSTRAINT plugin_installs_previous_module_digest_sha256,
  ADD CONSTRAINT plugin_installs_previous_module_digest_sha256
    CHECK (previous_module_digest IS NULL
           OR previous_module_digest = ''
           OR previous_module_digest ~ '^[0-9a-f]{64}$');
