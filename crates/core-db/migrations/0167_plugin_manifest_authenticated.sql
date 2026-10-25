-- Manifest authentication on a staged import (Wave 55 lane `p55-registry-signature-scope`).
--
-- Record `plugin-distribution.registry-signature-binds-digest-only`. The
-- registry signature used to cover the artifact digest and nothing else, so a
-- published (digest, signature) pair could be replayed beside a substituted
-- manifest and the approver read publisher-identity text the publisher never
-- signed. The signature path now verifies a v2 statement over the artifact
-- digest AND the canonical manifest digest (`crates/plugin-host/src/distribution.rs`),
-- and this column is how the answer travels from ingest to the person who
-- approves the row.
--
-- WHY IT IS STORED RATHER THAN RECOMPUTED. The staged row is read later, by a
-- different request, from the database. Neither the signature nor the statement
-- is persisted, so there is nothing to re-verify at read time; without the
-- column the consent screen could only show the manifest as unqualified text,
-- which is the defect this lane closes. It is written once, at staging, from the
-- bundle the adapter actually verified.
--
-- WHY THE DEFAULT IS false. Rows staged before this migration were verified
-- against the digest only — some of them were never cryptographically checked at
-- all, because the dev posture with no trusted key skips verification. `false`
-- is the weaker claim, and it is the true one for every existing row. No
-- backfill can honestly set it to true, and a legacy registry entry accepted
-- after this migration also lands here as `false` (accepted, labelled — see the
-- lane's REVIEW.md for why refusing it would strand every published plugin).
--
-- Additive and forward-only: one nullable-free boolean with a default, so the
-- backfill is a catalog write and no row is rewritten. The table already carries
-- the org isolation policy from 0079; no new table, no new policy.

ALTER TABLE plugin_staged_items
  -- TRUE only when a trusted signing key verified a statement that covers THIS
  -- manifest (the registry v2 statement). FALSE for a digest-only legacy entry,
  -- for a zip/skill/GitHub import that carries no such statement, and for any
  -- ingest run with no trusted key configured.
  ADD COLUMN manifest_authenticated boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN plugin_staged_items.manifest_authenticated IS
  'Whether a signature the operator trust set verified covers this manifest, '
  'not just the artifact bytes. Written at staging from the adapter verdict; '
  'FALSE is the weaker claim and the honest default for rows staged before '
  'migration 0167.';
