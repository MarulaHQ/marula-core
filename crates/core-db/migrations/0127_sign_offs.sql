-- Wave 43 track 2 lane 12 (`p43-signature-pad`): customer sign-off capture.
--
-- The row is a first-class `sign_offs` entity; the BYTES ride the existing
-- content-addressed attachment store as its payload. A signature is EVIDENCE
-- (signer identity, captured-at, who witnessed it, the consent record), so it
-- is not stored as "an attachment that happens to be a scribble" — the
-- attachment is the payload, the sign_offs row is the fact.
--
-- Shape decisions, all recorded in design/qa-p43-signature-pad/LANDING.md:
--
-- (1) ATTACHMENT vs ENTITY. `attachment_kind` gains `sign_off`. A sign-off
--     attachment always carries ticket_id (the subject being signed off) and
--     never an email/comment owner, which the pre-existing owner-shape CHECKs
--     in 0024 already permit — so this migration changes NO existing
--     constraint. `sign_offs.attachment_id` names the precise owner, so a
--     drawn payload can never dangle from the ticket without its fact.
--     A typed agreement has no payload, and the capture-shape CHECK keeps
--     that the only reason the column may be NULL.
--
-- (2) IMMUTABLE, NOT EDITABLE. `sign_offs` is INSERT-ONLY: there is no UPDATE
--     or DELETE path, and no UPDATE-triggering column. Revocation is a NEW
--     append-only row in `sign_off_revocations` (the `time_entry_approvals`
--     seq pattern from approvals, 0152-era) — the original row and its bytes
--     survive untouched, and "is this revoked" is a question answered by a
--     JOIN, not by a mutable flag. A correction is a SECOND sign_off linked by
--     `supersedes_sign_off_id` pointing BACKWARD at the original; the original
--     is never written, so the "what superseded this" relation is derived by
--     reading WHERE supersedes_sign_off_id = :original.
--
-- (3) CONSENT IS VERSIONED AND ORG-CONTROLLABLE. `sign_off_consents` holds
--     append-only versions per org. A sign_off stores BOTH the version AND a
--     verbatim `consent_text_snapshot`, so an org rewriting its consent
--     wording can never retroactively change what an earlier signer agreed
--     to. Seeding version 1 for existing orgs keeps capture working with no
--     admin setup step.
--
-- (4) RETENTION. No purge window. The repo's only retention constant is
--     DRAFT_RETENTION (30 days, note_drafts) for TRANSIENT work product that
--     never renders on a ledger. A sign-off is durable evidence and rides the
--     same posture as tickets/attachments: kept for the life of the org,
--     removed only by the org-delete cascade. This lane invents no second
--     retention posture (docs/theme.md is the UI side; the vault's ADR 0006
--     states no window either).
--
-- (5) PORTAL SIGNING IS DEFERRED (recorded, not implicit). `signer_kind`
--     carries a documented RESERVED value `portal_client` that the API
--     rejects today — the seam exists so a portal lane needs no schema change,
--     exactly like the reserved `attachments.uploaded_by_portal_user_id`
--     column in 0024. `witnessed_by_user_id` is nullable for the same reason.
--
-- RLS: org-scoped policies as everywhere — the CI gate parses this file.

-- (1) Payload taxonomy. ADD VALUE is safe here because this migration never
-- references the new label outside its own transactional block.
ALTER TYPE attachment_kind ADD VALUE IF NOT EXISTS 'sign_off';

CREATE TYPE sign_off_signer_kind AS ENUM ('customer', 'portal_client');

-- Append-only versioned consent wording. UNIQUE (org_id, version) makes
-- "create the next version" idempotent per org; revision N+1 never rewrites
-- revision N.
CREATE TABLE sign_off_consents (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  version integer NOT NULL CHECK (version > 0),
  body text NOT NULL CHECK (length(btrim(body)) > 0),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, version)
);

CREATE INDEX idx_sign_off_consents_org
  ON sign_off_consents (org_id, version DESC);

-- The evidence row. INSERT-ONLY by design (see header note 2).
CREATE TABLE sign_offs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  -- The payload: one PNG in the content-addressed store, kind 'sign_off'.
  -- NULL exactly when the signer agreed by typing their name instead of
  -- drawing, so a pad that only accepts ink would be an accessibility
  -- failure. The capture-shape CHECK makes that the ONLY way this column is
  -- NULL, and no row has both.
  attachment_id uuid REFERENCES attachments(id) ON DELETE RESTRICT,
  -- Who signed, as they typed it. The typed name is part of the record
  -- because a drawn mark alone identifies nobody (the same reason Adobe's
  -- own docs require a name beside a drawn signature).
  signer_name text NOT NULL CHECK (length(btrim(signer_name)) > 0),
  signer_kind sign_off_signer_kind NOT NULL DEFAULT 'customer',
  -- The staff member who presented the pad and watched the signature. NULL
  -- is permitted so the reserved portal path needs no schema change; today
  -- every shipped row has a witness.
  witnessed_by_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
  -- Client-side clock reading when the stroke was completed. Recorded
  -- verbatim (not trusted as the server time) because offline capture is
  -- the normal case: a signature taken at the door may queue for hours.
  captured_at timestamptz NOT NULL,
  -- True when the signer agreed by typing their name rather than drawing.
  -- An auditor reading the ledger needs to know which captures carried ink,
  -- because the two are not equally strong evidence and only one of them can
  -- ever be compared against a handwriting sample.
  typed_agreement boolean NOT NULL DEFAULT false,
  -- Consent the signer actually saw: the version PLUS a verbatim snapshot.
  consent_version integer NOT NULL CHECK (consent_version > 0),
  consent_text_snapshot text NOT NULL,
  -- Set only on a correction row; NULL for every first-party capture.
  -- Points BACKWARD at the original it replaces.
  supersedes_sign_off_id uuid REFERENCES sign_offs(id) ON DELETE RESTRICT,
  -- Offline-replay anchor: the client mints it, the server enforces
  -- uniqueness per org so a replayed queue op lands exactly once.
  idempotency_key text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, idempotency_key),
  -- The portal path is reserved, not shipped: a portal sign-off must not
  -- arrive with a staff witness attached, because a signature nobody
  -- watched is a different audit story. POST /api/tickets/{id}/sign-offs
  -- rejects that signer kind outright today, so this is the backstop for a
  -- later lane opening the path.
  CONSTRAINT sign_offs_signer_shape CHECK (
    signer_kind = 'customer' OR witnessed_by_user_id IS NULL
  ),
  -- Exactly one capture path: drawn ink with a payload, or a typed
  -- agreement with none. Never neither, never both.
  CONSTRAINT sign_offs_capture_shape CHECK (
    (attachment_id IS NOT NULL AND typed_agreement = false)
    OR (attachment_id IS NULL AND typed_agreement = true)
  )
  -- The "a correction must target an earlier sign-off on the SAME ticket"
  -- rule compares two rows, which SQL cannot express in a table constraint.
  -- It is enforced in the route (sign_offs.rs) where both rows are in
  -- scope. No vacuous CHECK stands in for it here.
);

CREATE INDEX idx_sign_offs_ticket
  ON sign_offs (org_id, ticket_id, created_at DESC);
-- Partial: only drawn marks carry a payload, so an index over every row
-- would spend half its entries on typed agreements.
CREATE INDEX idx_sign_offs_attachment ON sign_offs (attachment_id)
  WHERE attachment_id IS NOT NULL;
-- "What superseded this sign-off?" is a read, never a write on the original.
CREATE INDEX idx_sign_offs_supersedes
  ON sign_offs (supersedes_sign_off_id) WHERE supersedes_sign_off_id IS NOT NULL;

-- Append-only revocation trail. Mirror of time_entry_approvals' seq: one row
-- per revocation attempt, MAX(seq)+1 per sign-off, so a second revocation is a
-- data error rather than a silent overwrite.
CREATE TABLE sign_off_revocations (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  sign_off_id uuid NOT NULL REFERENCES sign_offs(id) ON DELETE CASCADE,
  seq integer NOT NULL CHECK (seq > 0),
  reason text NOT NULL CHECK (length(btrim(reason)) > 0),
  revoked_by_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (sign_off_id, seq)
);

CREATE INDEX idx_sign_off_revocations_lookup
  ON sign_off_revocations (org_id, sign_off_id, seq DESC);

ALTER TABLE sign_off_consents ENABLE ROW LEVEL SECURITY;
CREATE POLICY sign_off_consents_org ON sign_off_consents
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE sign_offs ENABLE ROW LEVEL SECURITY;
CREATE POLICY sign_offs_org ON sign_offs
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE sign_off_revocations ENABLE ROW LEVEL SECURITY;
CREATE POLICY sign_off_revocations_org ON sign_off_revocations
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Seed consent version 1 for every org that already exists, so capture works
-- with no admin setup. Wording is deliberately conservative and matches the
-- rendered pack's own "Client sign-off" label (mobile pack states 08/11).
INSERT INTO sign_off_consents (id, org_id, version, body, created_by)
SELECT gen_random_uuid(), o.id, 1,
  'I confirm the work described above was completed to my satisfaction and I am authorised to sign off on it.',
  NULL
FROM orgs o
ON CONFLICT (org_id, version) DO NOTHING;

-- Platform event types: a webhook subscription naming these needs no org grant
-- (check_event_types exempts source='platform').
INSERT INTO event_types (event_type, source, label, description) VALUES
  ('sign_off.created',   'platform', 'Sign-off captured',  'A customer signed off on a ticket'),
  ('sign_off.revoked',   'platform', 'Sign-off revoked',   'A captured sign-off was revoked'),
  ('sign_off.superseded','platform', 'Sign-off superseded','A corrected sign-off replaced an earlier one'),
  ('sign_off_consent.created','platform','Sign-off consent created','A new consent wording version was added')
ON CONFLICT (event_type) DO NOTHING;
