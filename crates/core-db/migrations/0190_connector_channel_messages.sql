-- 0190 — the messaging surface: a posted card, and the ledger of what a press did.
--
-- ADR 0032 makes a binding "the audit join: a message id, a channel, a
-- connection and a ticket id". The bindings themselves landed in 0185
-- (`connector_bindings`), so this migration adds the two rows that join
-- actually needs, and nothing else:
--
--   * `connector_channel_messages` — the message registry. One row per card
--     Core posted into a channel. It is what turns an inbound vendor callback
--     ("somebody pressed a button on message `1699…` in `C123`") into a
--     ticket and a connection without trusting a single field of the callback
--     body. It is also the handle a later edit needs: the vendor's message id
--     is stored, so updating the card is an update, not a re-post.
--
--   * `connector_action_audit` — one row per inbound callback, applied or
--     refused. It carries the connection, the vendor's channel and message,
--     the vendor user id AND the Marula user it resolved to, the action and
--     its result. `callback_key` is the ledger's identity: it is the digest of
--     the callback's own bytes, so a Slack retry (Slack re-delivers a slow
--     callback up to three times) collides on the unique index and is a
--     replay, not a second press.
--
-- Why a second audit table rather than `audit_events`. `audit_events` is the
-- org-wide stream: it answers "who changed what" with a typed actor and a
-- record. This table answers a narrower question the channel surface needs —
-- "which message in which channel, pressed by which vendor identity, resolved
-- to whom, and what did it do or refuse" — and it is keyed by the callback so
-- idempotency and evidence are the same row. The route also writes one
-- `audit_events` row per press so the act appears where operators look; the
-- two are different ledgers, not two copies of one answer.
--
-- Append-only posture: the application role keeps SELECT and INSERT on the
-- audit table and loses UPDATE and DELETE (0168's reasoning, applied here).
-- The org cascade still works — Postgres performs a cascading delete with the
-- owner's privileges, not the app role's. There is no retention function and
-- no trigger: this ledger is small (one row per press), and a guard that
-- needed a demo-reset escape would be more machinery than the guarantee is
-- worth. `assert_rls_coverage` and the append-only grant are the gate.
--
-- RLS: both tables carry `org_id` and an org policy. Client scoping is not an
-- RLS concern in this product (0185's header states why), and neither table
-- holds a secret or a token.

-- 1. The message registry ---------------------------------------------------

CREATE TABLE connector_channel_messages (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  connection_id uuid NOT NULL REFERENCES connector_connections(id) ON DELETE CASCADE,
  -- Which destination it was posted to. The binding can be re-pointed or
  -- disabled later; the message stays where it was posted, which is why the
  -- channel id below is recorded rather than joined at press time.
  binding_id uuid NOT NULL REFERENCES connector_bindings(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  -- The vendor's channel id at post time (Slack `C…`, Teams conversation id).
  channel_id text NOT NULL,
  -- The vendor's id for the message this card lives in: Slack `ts`
  -- ("1699999999.000100"), Teams activity id. This is the join a press and a
  -- later card edit both use.
  message_id text NOT NULL,
  -- Who shared it. NULL when the sharer left: the message outlives them and
  -- the card must still resolve to its ticket.
  shared_by uuid REFERENCES users(id) ON DELETE SET NULL,
  -- The client the card was scoped to at share time. Kept so a later reader
  -- can tell a card that was posted before a client became restricted, and a
  -- re-render can refuse to widen what a channel already saw.
  shared_client_id uuid REFERENCES clients(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- One vendor message is one registry row. This is the uniqueness a press
-- depends on: two rows for one message would make attribution a coin flip.
CREATE UNIQUE INDEX connector_channel_messages_vendor
  ON connector_channel_messages (org_id, connection_id, channel_id, message_id);
-- Case: "which copy of this ticket is in a channel" — the dialog and a future
-- edit both read by ticket.
CREATE INDEX connector_channel_messages_ticket
  ON connector_channel_messages (org_id, ticket_id);
CREATE INDEX connector_channel_messages_binding
  ON connector_channel_messages (org_id, binding_id);

-- 2. The action ledger ------------------------------------------------------

CREATE TABLE connector_action_audit (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  connection_id uuid NOT NULL REFERENCES connector_connections(id) ON DELETE CASCADE,
  -- The callback's own identity: hex SHA-256 of the raw request bytes. Slack's
  -- retry re-sends the same bytes, so the retry is a replay of this row.
  callback_key text NOT NULL CHECK (length(callback_key) = 64),
  -- The registry entry of the pressed action ("take", "status:in_progress").
  -- Text, not an enum: a new action is a registry entry, never a migration.
  action text NOT NULL,
  -- `applied` performed the effect; `refused` names why in refusal_code.
  outcome text NOT NULL CHECK (outcome IN ('applied', 'refused')),
  -- Typed refusal: `identity_unmapped`, `permission_denied`,
  -- `unknown_action`, `message_unmapped`, `ticket_unreadable`,
  -- `transition_refused`, `actor_inactive`.
  refusal_code text,
  -- What the press was about. Both ids are nullable-on-delete on purpose:
  -- the ledger is evidence and must survive the record it names, which is why
  -- the human-readable numbers are copied in below.
  ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  ticket_number text,
  message_id uuid REFERENCES connector_channel_messages(id) ON DELETE SET NULL,
  channel_id text,
  -- The vendor's user id exactly as the callback carried it (U…, AAD object id)
  -- — stored even when no Marula user could be resolved, because that is the
  -- fact an operator needs to fix the mapping.
  vendor_user_id text,
  -- The Marula user the vendor identity resolved to, when it did.
  actor_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
  -- What the action did, in the vendor's own vocabulary: the assignee it set,
  -- the status it moved to, the reason a transition was refused.
  detail jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now()
);

-- The idempotency gate. A retry inserts nothing and reads this row instead.
CREATE UNIQUE INDEX connector_action_audit_callback
  ON connector_action_audit (org_id, connection_id, callback_key);
-- The ticket's press history, newest first — the ticket surface's read.
CREATE INDEX connector_action_audit_ticket
  ON connector_action_audit (org_id, ticket_id, created_at DESC)
  WHERE ticket_id IS NOT NULL;
-- The connection's press history, and what was refused most recently.
CREATE INDEX connector_action_audit_connection
  ON connector_action_audit (org_id, connection_id, created_at DESC);

-- 3. The unauthenticated door's narrow lookup -------------------------------

-- The four callback endpoints in `routes::connector_inbound` have no session:
-- the vendor signature IS the authentication. That creates one ordering
-- problem — the connection's signing secret has to be found before the
-- signature can be checked, and the only thing in the request that could name
-- the connection is a body field (Slack `team.id`, Teams `recipient.id`).
--
-- This function is the answer, and it is deliberately the whole of it: it
-- returns CANDIDATES for a vendor selector, never an authenticated answer.
-- The caller tries the signature against each candidate's sealed secret and
-- proceeds only on a constant-time match. A forged selector therefore selects
-- a different secret and fails verification; it can never select an org.
--
-- The same narrow SECURITY DEFINER bridge the alert ingestor uses
-- (`alert_source_auth_lookup`, 0037): the app role cannot read across orgs
-- under RLS, and this function reads exactly one indexed shape.

-- The lookup's own index. The two selector keys are expressions over
-- `settings`, so a jsonb containment index would not serve them; these do, and
-- they keep the unauthenticated door from scanning every connection in the
-- product on each callback.
CREATE INDEX connector_connections_slack_team_idx
  ON connector_connections ((settings->>'slack_team_id'))
  WHERE status <> 'disabled' AND settings ? 'slack_team_id';
CREATE INDEX connector_connections_teams_app_idx
  ON connector_connections ((settings->>'teams_bot_app_id'))
  WHERE status <> 'disabled' AND settings ? 'teams_bot_app_id';

CREATE FUNCTION connector_callback_candidates(p_vendor text, p_selector text)
RETURNS TABLE (
  r_id uuid,
  r_org_id uuid,
  r_installation_id uuid,
  r_label text,
  r_settings jsonb,
  r_secrets_encrypted bytea,
  r_status text
)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS $$
  SELECT c.id, c.org_id, c.installation_id, c.label, c.settings,
         c.secrets_encrypted, c.status
  FROM connector_connections c
  WHERE c.status <> 'disabled'
    AND length(p_selector) BETWEEN 1 AND 200
    AND (
      -- One key per vendor, not a shared `team_id`: two connectors' settings
      -- schemas are independent, so a shared key would let a Teams connection
      -- answer a Slack lookup (and the reverse) whenever both recorded the
      -- same string.
      (p_vendor = 'slack' AND c.settings->>'slack_team_id' = p_selector)
      OR (p_vendor = 'teams' AND c.settings->>'teams_bot_app_id' = p_selector)
    )
  ORDER BY c.created_at
  LIMIT 16
$$;

-- Both role names, the legacy one only where it still exists (0168's posture:
-- a deployment that dropped `meridian_app` must not fail here).
DO $$
BEGIN
  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'meridian_app') THEN
    EXECUTE 'GRANT EXECUTE ON FUNCTION connector_callback_candidates(text, text) TO meridian_app';
  END IF;
END
$$;

GRANT EXECUTE ON FUNCTION connector_callback_candidates(text, text) TO marula_app;

-- 4. RLS --------------------------------------------------------------------

ALTER TABLE connector_channel_messages ENABLE ROW LEVEL SECURITY;
CREATE POLICY connector_channel_messages_org ON connector_channel_messages
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE connector_action_audit ENABLE ROW LEVEL SECURITY;
CREATE POLICY connector_action_audit_org ON connector_action_audit
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- 5. Append-only, by privilege ----------------------------------------------

DO $$
BEGIN
  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'meridian_app') THEN
    EXECUTE 'REVOKE UPDATE, DELETE ON connector_action_audit FROM meridian_app';
  END IF;
END
$$;

REVOKE UPDATE, DELETE ON connector_action_audit FROM marula_app;

-- 6. What each table is for, in the catalogue -------------------------------

COMMENT ON TABLE connector_channel_messages IS
  'One Core-rendered ticket card posted into a bound channel: the ticket, the '
  'connection, the binding, and the vendor channel + message ids a press or a '
  'later edit resolves through.';
COMMENT ON TABLE connector_action_audit IS
  'One row per inbound connector callback (Slack block_actions, Slack slash '
  'command, Teams activity, Teams messaging extension query), applied or '
  'refused, keyed by the digest of the callback body so a vendor retry is a '
  'replay rather than a second press. Append-only for the application role.';
