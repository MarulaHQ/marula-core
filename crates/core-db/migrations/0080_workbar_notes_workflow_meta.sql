-- Wave 17 lane 11 — p17-workbar-notes-tasks (Notes & Tasks WorkBar notepad,
-- docs/notes-tasks-workbar.md §5). Lane-exclusive migration pair 0080 (PG
-- port 54380 is the branch-local test port). ADDITIVE ONLY: no existing
-- column, table, index, or policy changes shape; RLS posture unchanged
-- (the single policy on ticket_comments stays, the new table gets the
-- same one-policy-per-org shape). U9-customization cross-check: none of
-- the constructs referenced by customization rows are touched.
--
-- (1) ticket_comments.workflow_meta — machine-provenance metadata for
--     server-composed workflow notes (docs/notes-tasks-workbar.md §5.3).
--     NULL for ordinary human notes. Written ONLY by the in-tx compose
--     path (Completed / N-A / Reopened / close-gate auto-close appends);
--     never a body-text sentinel. Queryable for audit:
--     `WHERE workflow_meta ? 'kind'` selects exactly the machine notes,
--     mute-user-date-spoofing. Shape ≈
--       { "kind": "task_complete" | "task_na" | "task_reopen" | "task_closeall",
--         "auto": true,
--         "item_ids": [uuid...] }
--     (spec §6 keeps AI provenance OUT of this column — it rides the
--     approval pipeline per docs/10-agent-platform.md).
--
-- (2) note_drafts — IF2 autosave store for the workbar notepad composer
--     (docs/notes-tasks-workbar.md §5.2): one draft per (org, ticket,
--     user). NOT a comment: the row never renders on any ledger or
--     timeline. `idempotency_key` links the draft to its submit — when a
--     comment carrying that key commits, the API deletes the row in the
--     same transaction (delete-on-submit), so an interrupted session
--     restores the draft and a submitted one never resurrects. 30-day
--     retention: stale drafts are purged opportunistically by the autosave
--     route (write-path housekeeping, deterministic — see the API
--     comments; no cron dependency).
--
-- Rollback-safety: both artifacts can be dropped without data semantics
-- depending on them (column is NULL for all existing rows; the draft
-- store is transient work product).

-- (1)
ALTER TABLE ticket_comments
  ADD COLUMN workflow_meta jsonb NULL;

COMMENT ON COLUMN ticket_comments.workflow_meta IS
  'Server-composed workflow-note provenance: {kind, auto, item_ids}. NULL for user-written notes; never body text.';

-- Queryable machine-note surface (audit analytics read the rows through
-- this index instead of filtering payloads ad hoc).
CREATE INDEX idx_comments_workflow_meta
  ON ticket_comments (org_id, ticket_id)
  WHERE workflow_meta IS NOT NULL;

-- (2) Envelope IF-keying: (org, ticket, user) is ONE draft; the key owns
-- the delete-on-submit claim.
CREATE TABLE note_drafts (
  id uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000'::uuid,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  body text NOT NULL,
  idempotency_key text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- §5.2: exactly ONE draft per (org, ticket, user).
  PRIMARY KEY (org_id, ticket_id, user_id)
);

ALTER TABLE note_drafts ENABLE ROW LEVEL SECURITY;
CREATE POLICY note_drafts_org
  ON note_drafts USING (org_id = current_setting('app.org_id', true)::uuid);
CREATE INDEX idx_note_drafts_created_at ON note_drafts (created_at);
