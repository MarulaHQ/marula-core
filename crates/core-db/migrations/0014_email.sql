-- Email-in/out v1 (TASKLIST 2.4).
--
-- email_messages is the message ledger for BOTH directions and the
-- idempotency key for inbound processing: (org_id, rfc_message_id) is
-- unique, so a provider replaying a webhook can never duplicate a comment
-- or ticket. `email_refs` holds the RFC 5322 References chain (the word
-- REFERENCES itself is reserved SQL, hence the column name); threading
-- matches inbound mail against it plus In-Reply-To.
--
-- email_outbound is the retry/backoff ledger: core-api enqueues rows in
-- the same transaction as the mutation that caused them, the worker
-- claims/transport-sends them, and results are reported back through
-- core-api (the only database writer, §4.1).
--
-- Inbound routing config is minimal by design: an org-level catch-all
-- (email_inbound_settings) plus an additive board.inbound_address column.
-- Sibling-owned board status tables are untouched.

CREATE TABLE email_messages (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  direction text NOT NULL CHECK (direction IN ('inbound','outbound')),
  -- RFC 5322 Message-ID, unique per org (idempotent replay handling).
  rfc_message_id text NOT NULL,
  in_reply_to text,
  email_refs text[] NOT NULL DEFAULT '{}',
  subject text,
  from_address text NOT NULL,
  from_name text,
  to_addresses text[] NOT NULL DEFAULT '{}',
  body_text text,
  ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  comment_id uuid REFERENCES ticket_comments(id) ON DELETE SET NULL,
  -- Lifecycle of the malware scan between webhook receipt and persistence.
  scan_status text NOT NULL CHECK (scan_status IN ('clean','infected','skipped')),
  transport text NOT NULL,
  -- Provider-specific pointer to the raw .eml payload (object storage ref);
  -- Meridian stores metadata only until Stage-3 attachments land.
  raw_ref text,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, rfc_message_id)
);
CREATE INDEX idx_email_messages_ticket ON email_messages (org_id, ticket_id);
CREATE INDEX idx_email_messages_in_reply_to ON email_messages (org_id, in_reply_to)
  WHERE in_reply_to IS NOT NULL;

CREATE TABLE email_outbound (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  email_message_id uuid NOT NULL REFERENCES email_messages(id) ON DELETE CASCADE,
  -- queued → sending → sent | dead; failures re-enter queued with
  -- exponential backoff via next_attempt_at.
  status text NOT NULL DEFAULT 'queued'
    CHECK (status IN ('queued','sending','sent','dead')),
  attempts integer NOT NULL DEFAULT 0,
  max_attempts integer NOT NULL DEFAULT 8,
  next_attempt_at timestamptz NOT NULL DEFAULT now(),
  last_error text,
  provider_message_id text,
  claimed_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_email_outbound_due ON email_outbound (status, next_attempt_at);

-- Org-level inbound routing catch-all: one mailbox address per org whose
-- mail lands on the chosen board when no board-level address matched.
CREATE TABLE email_inbound_settings (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL UNIQUE REFERENCES orgs(id) ON DELETE CASCADE,
  catchall_address text NOT NULL,
  catchall_board_id uuid NOT NULL REFERENCES boards(id) ON DELETE RESTRICT,
  enabled boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now()
);
-- Addresses are globally unique: they are how an inbound webhook resolves
-- WHICH org (and board) receives a message.
CREATE UNIQUE INDEX idx_email_inbound_catchall_addr ON email_inbound_settings (catchall_address);

-- Additive board-level inbound address (exact-match routing ahead of the
-- catch-all). Globally unique for the same org-resolution reason.
ALTER TABLE boards ADD COLUMN inbound_address text;
CREATE UNIQUE INDEX idx_boards_inbound_address ON boards (inbound_address)
  WHERE inbound_address IS NOT NULL;

ALTER TABLE email_messages ENABLE ROW LEVEL SECURITY;
CREATE POLICY email_messages_org ON email_messages
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE email_outbound ENABLE ROW LEVEL SECURITY;
CREATE POLICY email_outbound_org ON email_outbound
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE email_inbound_settings ENABLE ROW LEVEL SECURITY;
CREATE POLICY email_inbound_settings_org ON email_inbound_settings
  USING (org_id = current_setting('app.org_id', true)::uuid);
