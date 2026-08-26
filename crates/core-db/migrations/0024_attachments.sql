-- Attachments (TASKLIST 2.14): ticket/comment/email files.
--
-- Purpose: metadata rows for binary objects persisted OUTSIDE Postgres
-- via the ObjectStore trait (crates/core-store). One row binds exactly one
-- OWNER TARGET: an email message, or a ticket (optionally one specific
-- comment on it). The structural invariants live in CHECKs:
--   * email_message attachments point at nothing else;
--   * comment links require their ticket and exclude emails.
-- `kind` records intent; a ticket_comment row may legitimately start with
-- ticket_comment_id NULL because the web composer uploads files BEFORE the
-- comment itself is created, then links them in the same transaction that
-- inserts the comment.
--
-- Dedup doctrine: storage keys are CONTENT-ADDRESSED per org
-- (`org/<org_id>/attachments/<sha256>`); several rows may reference one
-- key, so attaching the same file twice stores bytes once. There is
-- deliberately NO unique index on (org_id, sha256) — reusing an object is
-- the point; deletion of a row only reclaims the object when no sibling
-- row still references its key (refcount checked in core-db).
--
-- RLS: org-scoped policy as everywhere — the CI gate parses this file.
--
-- Non-purpose: no virus-scan state here (email scan status lives on
-- email_messages; API uploads are magic-byte allowlisted at admission),
-- no versioning/document history (Stage 3), no portal upload columns
-- beyond the reserved actor reference.

CREATE TYPE attachment_kind AS ENUM ('ticket_comment', 'email_message', 'ticket');

CREATE TABLE attachments (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  kind attachment_kind NOT NULL,
  -- Owner targets. Exactly ONE must be non-null and it must agree with
  -- `kind`; tickets/comments cascade so attachments never outlive their
  -- owner, while email_messages keep SET NULL semantics for audit trails.
  ticket_id uuid REFERENCES tickets(id) ON DELETE CASCADE,
  ticket_comment_id uuid REFERENCES ticket_comments(id) ON DELETE CASCADE,
  email_message_id uuid REFERENCES email_messages(id) ON DELETE CASCADE,
  filename text NOT NULL,
  content_type text NOT NULL,
  size_bytes bigint NOT NULL CHECK (size_bytes > 0),
  sha256 text NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
  storage_key text NOT NULL,
  uploaded_by_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
  uploaded_by_portal_user_id uuid REFERENCES portal_users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT attachments_owner_shape CHECK (
    (
      email_message_id IS NOT NULL AND ticket_id IS NULL
        AND ticket_comment_id IS NULL
    )
    OR (email_message_id IS NULL AND ticket_id IS NOT NULL)
  ),
  CONSTRAINT attachments_link_shape CHECK (
    ticket_comment_id IS NULL
    OR (ticket_id IS NOT NULL AND email_message_id IS NULL)
  ),
  -- At-most-one actor: staff uploads set the user, portal uploads (later)
  -- would set the portal actor, INBOUND EMAIL attachments have neither —
  -- they are system-created on behalf of the sender.
  CONSTRAINT attachments_uploader_shape CHECK (
    uploaded_by_user_id IS NULL OR uploaded_by_portal_user_id IS NULL
  )
);

CREATE INDEX idx_attachments_ticket ON attachments (org_id, ticket_id);
CREATE INDEX idx_attachments_comment ON attachments (org_id, ticket_comment_id)
  WHERE ticket_comment_id IS NOT NULL;
CREATE INDEX idx_attachments_email ON attachments (org_id, email_message_id)
  WHERE email_message_id IS NOT NULL;
-- Dedup lookup: does THIS org already hold these exact bytes?
CREATE INDEX idx_attachments_dedupe ON attachments (org_id, sha256);
-- Refcount for safe object reclamation on delete.
CREATE INDEX idx_attachments_key_refs ON attachments (storage_key);

ALTER TABLE attachments ENABLE ROW LEVEL SECURITY;
CREATE POLICY attachments_org ON attachments
  USING (org_id = current_setting('app.org_id', true)::uuid);
