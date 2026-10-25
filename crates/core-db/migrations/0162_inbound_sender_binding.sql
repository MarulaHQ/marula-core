-- Inbound sender binding (Wave 55 lane V7, `p55-inbound-sender-binding`).
--
-- Record 4 of the Wave 54 security decisions, and the user ruling of
-- 2026-10-25: hold and triage, at three levels the org chooses between.
--
-- The webhook HMAC proves a message came through the mail provider. It says
-- nothing about who wrote it, and the comment path never compared `From` to
-- the ticket it was about — so a stranger who mailed the board address with
-- `[MER-1042]` in the subject wrote a client-visible comment into that
-- ticket. From here an inbound message whose sender is not already a
-- participant on the ticket it names is HELD: recorded with its reason, off
-- the ticket, and waiting for a decision.
--
-- WHY THESE COLUMNS LIVE ON THE LEDGER. `email_messages` already carries the
-- (org_id, rfc_message_id) unique key that makes inbound processing
-- idempotent. Putting the hold anywhere else would need a second idempotency
-- key and would let one of the two rows land alone. The hold is the message,
-- so a replayed delivery is a `duplicate` and can never be held twice. It
-- also means the held set is a partial index away rather than a join.
--
-- `ticket_id` IS THE VISIBILITY SWITCH, NOT A HINT. The ticket timeline, the
-- portal read and attachment visibility all join through it, so a held row
-- (ticket_id NULL, `sender_binding = 'held'`) is invisible to the client by
-- construction. A decision sets ticket_id and comment_id together with the
-- state, in one statement, in one transaction.
--
-- THE DEFAULT IS `not_required`, NOT `bound`. Rows written before this
-- migration threaded without any sender check at all. Calling them bound
-- would report a rule that never ran on them.
--
-- `sender_binding_candidate_ticket_id` is the ticket routing had already
-- resolved from the thread or the subject token. It is recorded so a person
-- can accept the message with one click, and it is a SEPARATE column from
-- ticket_id so a held message is not attached merely because routing knew
-- where it belonged.
--
-- `sender_binding_proposal` is the decision engine's answer under `shadow`
-- and `autonomous`: the ticket it would attach to, the sender match it
-- found, or the refusal it would apply, with the signals behind it. It is
-- recorded, never inferred later, because shadow exists to measure agreement
-- against what a person then decided on the same row.

ALTER TABLE email_messages
  ADD COLUMN IF NOT EXISTS sender_binding text NOT NULL DEFAULT 'not_required'
    CHECK (sender_binding IN ('not_required', 'bound', 'held', 'accepted', 'refused')),
  ADD COLUMN IF NOT EXISTS sender_binding_reason text,
  ADD COLUMN IF NOT EXISTS sender_binding_candidate_ticket_id uuid
    REFERENCES tickets(id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS sender_binding_proposal jsonb,
  ADD COLUMN IF NOT EXISTS sender_binding_decided_at timestamptz,
  ADD COLUMN IF NOT EXISTS sender_binding_decided_by uuid
    REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS sender_binding_decided_source text
    CHECK (sender_binding_decided_source IN ('human', 'engine'));

-- The triage queue reads held messages newest first, per org. Partial, so
-- the index stays the size of the queue rather than the size of the ledger.
CREATE INDEX IF NOT EXISTS idx_email_messages_held
  ON email_messages (org_id, created_at DESC)
  WHERE sender_binding = 'held';

COMMENT ON COLUMN email_messages.sender_binding IS
    'Whether the From address was already a participant on the ticket this message names: not_required (no existing ticket), bound, held, accepted, refused.';
COMMENT ON COLUMN email_messages.sender_binding_reason IS
    'The typed reason a message is held (sender_not_participant) or the reason recorded with a decision.';
COMMENT ON COLUMN email_messages.sender_binding_candidate_ticket_id IS
    'The ticket routing resolved from the thread or the subject token. Separate from ticket_id so a held message is not attached by routing alone.';
COMMENT ON COLUMN email_messages.sender_binding_proposal IS
    'The decision engine proposal under shadow and autonomous: proposed_action, ticket_id, sender_match, signals, rationale.';
COMMENT ON COLUMN email_messages.sender_binding_decided_source IS
    'human or engine. An engine decision is never recorded as a person''s.';
