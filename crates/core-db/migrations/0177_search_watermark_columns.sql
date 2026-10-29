-- 0176 — the two tables a source watermark needs now carry a trustworthy
-- `updated_at`, and nothing reads it as a cursor yet.
--
-- Why the columns. A per-source watermark for the search backfill needs to ask
-- "what changed since X" cheaply. `sites` had no `updated_at` at all, and
-- `ticket_comments.updated_at` was nullable, so a timestamp walk would have
-- silently skipped every site and every comment written before it was set. The
-- data-loss shape is the one the register already named: a cursor that skips is
-- worse than a scan (Wave 62 lane `p62-data-truth`, ruling 2026-10-27).
--
-- Why no cursor is wired here. The backfill's predicate is
-- `search_index LEFT JOIN search_embeddings` where the vector is missing OR the
-- content hash moved on (`crates/core-db/src/search.rs`, `rows_needing_embedding`).
-- It is self-draining: rows it embeds leave the predicate, so each pass
-- converges without a position, and a position would strand a row whose
-- embedding failed on the next pass. The event-driven indexer is what keeps
-- `search_index` current; these columns make a source-driven walk possible when
-- one exists, and are correct on their own (a site had no modification time at
-- all).
--
-- The trigger is the point of the column: a watermark misses a row exactly when
-- one write path forgets to set the timestamp, and a forgetful path is silent.

CREATE OR REPLACE FUNCTION marula_touch_updated_at() RETURNS trigger AS $$
BEGIN
  NEW.updated_at := now();
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

ALTER TABLE sites
  ADD COLUMN updated_at timestamptz NOT NULL DEFAULT now();

CREATE TRIGGER sites_touch_updated_at
  BEFORE UPDATE ON sites
  FOR EACH ROW EXECUTE FUNCTION marula_touch_updated_at();

-- Every pre-existing comment gets its creation time, not the migration's clock:
-- the value has to mean "when this text last moved".
UPDATE ticket_comments SET updated_at = created_at WHERE updated_at IS NULL;

ALTER TABLE ticket_comments
  ALTER COLUMN updated_at SET DEFAULT now(),
  ALTER COLUMN updated_at SET NOT NULL;

CREATE TRIGGER ticket_comments_touch_updated_at
  BEFORE UPDATE ON ticket_comments
  FOR EACH ROW EXECUTE FUNCTION marula_touch_updated_at();

-- The walk's own shape: org, then time, then id to break ties inside a
-- timestamp. Both are partial-free and cheap to maintain.
CREATE INDEX IF NOT EXISTS idx_sites_org_updated
  ON sites (org_id, updated_at, id);

CREATE INDEX IF NOT EXISTS idx_comments_org_updated
  ON ticket_comments (org_id, updated_at, id);
