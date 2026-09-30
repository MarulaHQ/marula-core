-- Wave 42 (p42-email-signature-preferences) — per-user email greeting and
-- signature, org-governed.
--
-- Four things live here, and none of them is a new table:
--
-- 1. THE ORG POLICY, held at the database level. `email.signature_policy`
--    is `allowed` or `org_locked`, and the constraint below is the whole
--    legal set. The registry validates the write path and a core-domain
--    test pins the option list, but a settings row can also arrive by a
--    restore, a support fix, or a future code path that writes the table
--    directly — and an illegal policy is not a cosmetic problem: the
--    resolver's two personal rungs are gated on it, so a value the ladder
--    does not define would fall through to the permissive default and an
--    org that meant to lock signatures would ship unlocked. The
--    constraint is a no-op for every other key (`key <> ... OR ...`), so
--    it constrains exactly this one key, and only at org scope, which is
--    the only scope the declaration admits.
--
-- 2. NORMALIZATION of rows that predate the constraints. A row that cannot
--    be valid must not be left in place for a later migration to argue
--    about, and a restore from before the constraints is the one path that
--    can produce one. The policy row is repaired to `allowed` — the same
--    answer the declaration's default gives, so the repair never changes
--    what the resolver would have done with an unreadable value anyway.
--
-- 3. LENGTH on the three signature/greeting text keys. These are editor
--    text with a registry cap, and the cap matters at the database for the
--    same reason the policy does: an over-long stored block is rendered
--    into every message it touches, and the honest repair is to clip it to
--    what the editor would have accepted rather than to keep sending
--    something the UI refuses to show. The counts are the declaration's
--    `max_len` values, quoted here because a CHECK cannot read them.
--
-- 4. THE LEDGER'S MEMORY OF WHAT APPLIED. `email_messages` gains
--    `signature_source` and `greeting_source`, naming the rung of the
--    resolution chain that supplied each block for THAT message. The
--    columns are nullable with a closed CHECK, because "this row predates
--    the lane" is a real and permanent state for every message already in
--    the ledger: NULL reads as `unset` in the delivery view, which is the
--    truth about a row nothing was recorded against, rather than a guess.
--
-- RLS: no new table, and no new policy. The three settings keys land in
-- `settings`, whose `settings_org` policy (migration 0018) admits
-- org-scope rows org-wide and user-scope rows only to the owning
-- `app.user_id` — so a personal signature cannot be read or written
-- sideways inside an org, and the org policy row is org-scoped by the same
-- policy. The two ledger columns ride `email_messages`, which already
-- carries its own org-scoped policy. The `rls_coverage` suite asserts
-- that coverage, and the wave-42 registry test in core-domain asserts the
-- scopes and the policy option list.

-- 1. The policy, closed at the database.
ALTER TABLE settings
  ADD CONSTRAINT settings_email_signature_policy CHECK (
    key <> 'email.signature_policy'
    OR (
      scope = 'org'
      AND value IN ('"allowed"'::jsonb, '"org_locked"'::jsonb)
    )
  );

-- 2. Repair a policy row the constraint above would refuse. Zero rows on a
--    fresh install; the restore path is why it is here.
UPDATE settings
   SET value = '"allowed"'::jsonb
 WHERE key = 'email.signature_policy'
   AND NOT (
     scope = 'org'
     AND value IN ('"allowed"'::jsonb, '"org_locked"'::jsonb)
   );

-- 3. The three editor text keys hold the declaration's `max_len`:
--      email.org_greeting 500 · email.signoff 1000 · email.signature 1000
--    A non-string value (`#>> '{}'` gives its text form) is over the cap
--    for a key whose whole declared kind is text, so one rule covers both.
ALTER TABLE settings
  ADD CONSTRAINT settings_email_signature_text_len CHECK (
    key NOT IN ('email.org_greeting', 'email.signoff', 'email.signature')
    OR length(value #>> '{}') <= CASE key
         WHEN 'email.org_greeting' THEN 500
         ELSE 1000
       END
  );

-- Clip a row the constraint above would refuse, per key, so each is
-- normalized to the length ITS OWN declaration allows rather than to one
-- shared number.
UPDATE settings
   SET value = to_jsonb(substr(value #>> '{}', 1, 500))::jsonb
 WHERE key = 'email.org_greeting'
   AND length(value #>> '{}') > 500;

UPDATE settings
   SET value = to_jsonb(substr(value #>> '{}', 1, 1000))::jsonb
 WHERE key IN ('email.signoff', 'email.signature')
   AND length(value #>> '{}') > 1000;

-- 4. Which rung of the chain supplied the greeting and the signature for
--    this message. Additive, nullable, closed: the delivery view renders a
--    NULL as `unset`, and a row from before the lane is never re-written
--    to pretend it knows. `product` is the one value the chain cannot
--    return — it labels a line the product writes itself and no setting
--    governs (the CSAT survey's neutral "Hi,"), so the ledger never has to
--    claim a rung supplied a block it did not.
ALTER TABLE email_messages
  ADD COLUMN signature_source text,
  ADD COLUMN greeting_source text;

ALTER TABLE email_messages
  ADD CONSTRAINT email_messages_signature_source CHECK (
    signature_source IS NULL
    OR signature_source IN ('message_override', 'user_default', 'board_identity', 'org_default', 'product', 'unset')
  );

ALTER TABLE email_messages
  ADD CONSTRAINT email_messages_greeting_source CHECK (
    greeting_source IS NULL
    OR greeting_source IN ('message_override', 'user_default', 'org_default', 'product', 'unset')
  );
