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

-- Cross-org seams for the send pipeline and inbound routing, WITHOUT
-- weakening RLS: a role membership grant to `meridian_app` would exempt it
-- from row security everywhere (Postgres exempts roles that have the table
-- owner's privileges), so instead these three narrow SECURITY DEFINER
-- helpers run AS the table owner internally and nothing else does. All
-- other statements stay fully RLS-constrained.

-- Recipient address → (org, board): board-level exact addresses win over
-- the org catch-all; both are globally unique by index.
CREATE FUNCTION resolve_inbound_email_route(p_addresses text[])
RETURNS TABLE (r_org_id uuid, r_board_id uuid)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS $fn$
  SELECT x.r_org_id, x.r_board_id FROM (
    SELECT b.org_id AS r_org_id, b.id AS r_board_id, 1 AS prio
      FROM boards b
     WHERE lower(b.inbound_address) = ANY (p_addresses)
    UNION ALL
    SELECT s.org_id, s.catchall_board_id, 2
      FROM email_inbound_settings s
     WHERE s.enabled AND lower(s.catchall_address) = ANY (p_addresses)
  ) x
  ORDER BY x.prio
  LIMIT 1
$fn$;

-- Atomically claims due sends (queued past backoff, plus abandoned leases)
-- across ALL orgs; FOR UPDATE SKIP LOCKED keeps parallel workers honest.
CREATE FUNCTION claim_due_email_outbound(p_limit integer, p_lease interval)
RETURNS TABLE (
  r_ledger_id uuid, r_org_id uuid, r_attempts integer, r_max_attempts integer,
  r_rfc_message_id text, r_from_address text, r_from_name text,
  r_to_addresses text[], r_subject text, r_body_text text,
  r_in_reply_to text, r_refs text[]
)
LANGUAGE sql SECURITY DEFINER SET search_path = public AS $fn$
  WITH claimed AS (
    UPDATE email_outbound AS q
       SET status = 'sending', claimed_at = now(), updated_at = now(),
           attempts = q.attempts + 1
     WHERE q.id IN (
           SELECT c.id FROM email_outbound c
            WHERE (c.status = 'queued' AND c.next_attempt_at <= now())
               OR (c.status = 'sending' AND c.claimed_at < now() - p_lease)
            ORDER BY c.next_attempt_at
            LIMIT LEAST(GREATEST(p_limit, 1), 1000)
            FOR UPDATE SKIP LOCKED
     )
     RETURNING q.id, q.org_id, q.attempts, q.max_attempts, q.email_message_id
  )
  SELECT cl.id, cl.org_id, cl.attempts, cl.max_attempts,
         m.rfc_message_id, m.from_address, m.from_name, m.to_addresses,
         m.subject, m.body_text, m.in_reply_to, m.email_refs
    FROM claimed cl
    JOIN email_messages m ON m.id = cl.email_message_id
   ORDER BY cl.id
$fn$;

-- Applies one worker report; guards on status='sending' so late/duplicate
-- reports after lease recovery are inert. Returns whether a row changed.
CREATE FUNCTION apply_email_send_result(
  p_id uuid, p_ok boolean, p_provider_id text, p_error text, p_delay interval
)
RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public AS $fn$
BEGIN
  IF p_ok THEN
    UPDATE email_outbound
       SET status = 'sent', provider_message_id = p_provider_id,
           last_error = NULL, claimed_at = NULL, updated_at = now()
     WHERE id = p_id AND status = 'sending';
  ELSE
    UPDATE email_outbound
       SET status = CASE WHEN attempts >= max_attempts THEN 'dead' ELSE 'queued' END,
           next_attempt_at = CASE WHEN attempts >= max_attempts THEN next_attempt_at
                                  ELSE now() + p_delay END,
           last_error = p_error, provider_message_id = NULL,
           claimed_at = NULL, updated_at = now()
     WHERE id = p_id AND status = 'sending';
  END IF;
  RETURN FOUND;
END
$fn$;

GRANT EXECUTE ON FUNCTION resolve_inbound_email_route(text[]) TO meridian_app;
GRANT EXECUTE ON FUNCTION claim_due_email_outbound(integer, interval) TO meridian_app;
GRANT EXECUTE ON FUNCTION apply_email_send_result(uuid, boolean, text, text, interval) TO meridian_app;
