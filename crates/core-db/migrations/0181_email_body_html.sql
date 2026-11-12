-- 0181 — outbound mail gains an optional HTML part (`email_messages.body_html`).
--
-- Why. The first branded outbound is the user invitation (0181's companion
-- code): a plain-text invite is the difference between a link an admin
-- copy-pastes and an email that looks like the product the invitee is joining.
-- Every transport already has an HTML path — Postmark's `HtmlBody` field, and
-- the SMTP/SES renderer, which can emit `multipart/alternative` — so the
-- schema change is the only structural piece. HTML stays NULL on every
-- message that has no reason to carry it; the text part remains the
-- fallback that always exists.
--
-- The claim function is redefined in the same migration: the worker reads
-- claimed rows through `claim_due_email_outbound` (SECURITY DEFINER), so a
-- column the Rust row-mapping expects but the function never returns is a
-- guaranteed None — and a branded email that renders as plain text with no
-- error anywhere to point at.

ALTER TABLE email_messages
  ADD COLUMN body_html TEXT;

-- 0014 created this claim function with the pre-HTML return shape, and
-- PostgreSQL refuses a return-type change through CREATE OR REPLACE —
-- a fresh database must DROP the old signature first. Dropping it drops
-- 0014's EXECUTE grant with it, so the grant is restated below.
DROP FUNCTION IF EXISTS claim_due_email_outbound(integer, interval);

CREATE OR REPLACE FUNCTION claim_due_email_outbound(p_limit integer, p_lease interval)
RETURNS TABLE (
  r_ledger_id uuid, r_org_id uuid, r_attempts integer, r_max_attempts integer,
  r_rfc_message_id text, r_from_address text, r_from_name text,
  r_to_addresses text[], r_subject text, r_body_text text, r_body_html text,
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
         m.subject, m.body_text, m.body_html, m.in_reply_to, m.email_refs
    FROM claimed cl
    JOIN email_messages m ON m.id = cl.email_message_id
   ORDER BY cl.id
$fn$;

GRANT EXECUTE ON FUNCTION claim_due_email_outbound(integer, interval) TO meridian_app;
