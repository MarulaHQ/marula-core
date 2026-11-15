-- Migration 0199: indexes behind contact activity.
--
-- The client record now answers "when did we last talk to this person" from
-- the event ledger it already keeps: email on the client's tickets, portal
-- replies and calls. Those reads narrow by org and then match an address or a
-- number, and none of the three tables had an index for that shape.
--
-- Additive only. Each index is partial or expression-based so it stays small
-- and serves exactly the derived read.

-- Inbound mail is matched on the sender address.
CREATE INDEX IF NOT EXISTS idx_email_messages_org_from_lower
  ON email_messages (org_id, lower(from_address));

-- A call is matched on the digits of the external number, whatever the
-- provider's formatting.
CREATE INDEX IF NOT EXISTS idx_voip_call_logs_org_number
  ON voip_call_logs (org_id, regexp_replace(COALESCE(external_number, ''), '\D', '', 'g'));

-- Portal replies are looked up by the portal identity that wrote them.
CREATE INDEX IF NOT EXISTS idx_audit_events_portal_reply
  ON audit_events (org_id, actor_id, occurred_at DESC)
  WHERE action = 'portal.ticket_reply';
