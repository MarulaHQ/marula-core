-- Attachment scan gate (Wave 52 lane P6a, `p52-scan-gate`).
--
-- ClamAV has been deployed and wired into `core-mail` since the email path
-- existed, but nothing on the upload path ever called it: `attachments.rs`
-- validated magic bytes, hashed, and stored. This migration creates the
-- record the gate writes, so the verdict and the bytes can no longer
-- disagree.
--
-- NUMBERING: 0139 is the next free slot (main's head was
-- 0138_outbox_bounds.sql at lane start). The board claim is a reservation at
-- claim time, not a guarantee at merge time — re-check main's head
-- immediately before merge and renumber on a collision (the 0131 header
-- records the precedent).
--
-- ONE TABLE, KEYED BY THE OBJECT, NOT THE UPLOAD. `attachments` rows are
-- per-OWNER (a ticket may hold two rows for one file; an email and a ticket
-- can share one); the scan verdict belongs to the BYTES. Marula storage keys
-- are content-addressed per org (`org/<org>/attachments/<sha256>`), so
-- (org_id, sha256) is the object's identity and its primary key here. That
-- is what makes the verdict "an object's metadata": one row per stored
-- object, whatever references it, and a future re-scan can be skipped by
-- lookup (`status = 'clean'` on that digest) instead of by paying clamd
-- again for bytes the org already holds.
--
-- THE QUARANTINED SET IS `status = 'infected'`. A rejected object gets no
-- `attachments` row at all — that table means "admitted" — so its record
-- lives here, and `storage_key` points into `org/<org>/quarantine/`.
-- Keeping one table for both outcomes is deliberate: the verdict, the
-- location of the bytes, and the release trail are the same fact about the
-- same object, and splitting them would let a release half-land.
--
-- FAIL-CLOSED IS ABSENT HERE BY CONSTRUCTION. A scanner that cannot answer
-- produces NO row and NO stored bytes: the upload is refused with a typed
-- error before either write. There is deliberately no `unavailable` status —
-- a row for it would mean the refusal had already stored something to
-- describe, which is exactly what failing closed forbids.
--
-- RLS: org-scoped policy as everywhere — the CI gate parses this file.
--
-- Non-purpose: no byte movement (core-store), no admission policy
-- (core-api), no retention sweep (nothing deletes quarantine rows yet; the
-- P6c admin surface owns delete/release).

CREATE TABLE attachment_scans (
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  sha256 text NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
  -- clean   — a scanner examined these bytes and passed them;
  -- skipped — no scanner was configured (the documented none mode);
  -- infected— a scanner named a signature in them; bytes are quarantined.
  status text NOT NULL CHECK (status IN ('clean', 'skipped', 'infected')),
  -- Signature name for an infection, or the none-mode note for a skip.
  detail text,
  -- Adapter that produced the verdict (`clamd`, `none`) — the record has to
  -- survive a later scanner change without reading as if the new one had
  -- always been there.
  scanner text NOT NULL,
  size_bytes bigint NOT NULL CHECK (size_bytes > 0),
  content_type text NOT NULL,
  filename text NOT NULL,
  -- Where the bytes live: the admission key for clean/skipped objects, the
  -- quarantine key for infected ones.
  storage_key text NOT NULL,
  -- What the upload was aimed at, for the operator reading the quarantine
  -- list. SET NULL rather than CASCADE: deleting a ticket must never delete
  -- the record that something was found on it.
  ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  uploaded_by_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
  uploaded_by_portal_user_id uuid REFERENCES portal_users(id) ON DELETE SET NULL,
  scanned_at timestamptz NOT NULL DEFAULT now(),
  quarantined_at timestamptz,
  -- Release trail (the P6c admin surface writes these; the columns exist
  -- now so releasing is an UPDATE, not a second migration on a table whose
  -- rows are evidence). A released object keeps its row: the audit trail is
  -- the point, and a deleted quarantine row would erase the fact that
  -- malware was ever uploaded to this org.
  released_at timestamptz,
  released_by_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
  release_reason text,
  PRIMARY KEY (org_id, sha256),
  -- An infection and its quarantine timestamp are one fact; so are a
  -- release, its actor, and its reason. Both are unenforceable from the API
  -- layer alone, and a half-set row is worse than either extreme.
  CONSTRAINT attachment_scans_quarantine_coherent CHECK (
    (status = 'infected') = (quarantined_at IS NOT NULL)
  ),
  CONSTRAINT attachment_scans_release_coherent CHECK (
    (released_at IS NULL) = (released_by_user_id IS NULL)
    AND (released_at IS NULL) = (release_reason IS NULL)
  )
);

-- The admin quarantine list reads exactly this: newest infections first,
-- for one org. Partial so the index stays the size of the quarantined set
-- rather than of every object the org ever stored, which is the whole point
-- of putting the status in a partial predicate.
CREATE INDEX idx_attachment_scans_quarantine
  ON attachment_scans (org_id, scanned_at DESC) WHERE status = 'infected';

-- The admission-side lookup is by primary key (org_id, sha256) and needs no
-- second index.

ALTER TABLE attachment_scans ENABLE ROW LEVEL SECURITY;
CREATE POLICY attachment_scans_org ON attachment_scans
  USING (org_id = current_setting('app.org_id', true)::uuid);
