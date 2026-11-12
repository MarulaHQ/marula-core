-- 0182 — every system-sent email becomes template-backed (stored override,
-- hardcoded fallback).
--
-- Why. Head 0181 gave outbound mail an optional HTML part; the invitation
-- renders its branded HTML from hardcoded Rust. This migration completes
-- the same shape for CONTENT: the copy an org's system mail carries moves
-- into the templates registry, where orgs can edit it directly (the
-- enqueue lookup reads the row by the outbound kind's fixed slug, so
-- these seed as editable org rows, not shipped defaults), and the
-- hardcoded rendering in `core_mail::outbound::OutboundKind` stays as the
-- fallback that applies when a row is absent or unreadable. Templates are
-- read at enqueue time (one indexed read per outbound on an already-cold
-- path) — no claim function touches them, so this is columns + backfill
-- only.
--
-- Two columns:
--
-- * `templates.html` — the optional branded HTML part of an email-kind
--   template, riding beside `subject`/`body` exactly the way
--   `email_messages.body_html` rides beside `body_text`. Nullable because
--   every text-only template keeps no HTML column content at all.
-- * `email_messages.template_source` — which content source applied,
--   `stored` (the org's template row) or `builtin` (the hardcoded
--   fallback, when the row was missing or failed to render). Same ledger
--   contract as `signature_source`/`greeting_source` (Wave 42): the
--   column says what was recorded, and the delivery log can answer
--   "which copy went out" without replaying the enqueue.

ALTER TABLE templates
  ADD COLUMN html TEXT;

ALTER TABLE email_messages
  ADD COLUMN template_source TEXT;

-- Backfill the system-outbound templates for every org (idempotent —
-- defends pre-lane orgs exactly like 0070's backfill). Subjects/bodies/
-- html mirror core_domain::templates::OUTBOUND_EMAIL_TEMPLATES; the
-- byte-drift test in core-db keeps the two copies honest.
--
-- These seed as EDITABLE ORG ROWS (`is_system = false`), not shipped
-- defaults — deliberately off the defaults-first doctrine: the enqueue
-- lookup reads the org's row by the outbound kind's FIXED slug, so
-- forking cannot rewire it, and an immutable system row would make the
-- stored copy uneditable and the builtin fallback unreachable. Orgs own
-- these rows exactly like any other template: edit them, delete them
-- (the hardcoded fallback then applies, and
-- `email_messages.template_source` records which copy went out). No
-- `builtin` tag: shipped-default rows (0070) are immutable and
-- restorable, and neither property is offered here.
DO $$
DECLARE
  org record;
  missing record;
BEGIN
  FOR org IN SELECT id FROM orgs LOOP
    FOR missing IN
      SELECT * FROM (VALUES
        ('email', 'user_invited', 'Workspace invitation',
         'The invitation email a new member receives when an admin adds them to the workspace.',
         'You''ve been invited to join {{org.name}}',
         E'Hi {{invite.display_name}},\n\n{{invite.inviter_name}} invited you to join {{org.name}} on Marula, the workspace your team runs service work in.\n\nAccept your invitation:\n{{invite.url}}\n\nThe link expires on {{invite.expires_at}}. If you weren''t expecting this invitation you can ignore this email — your address won''t be added to the workspace until you accept.\n',
         E'<!DOCTYPE html>\n<html lang="en">\n<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"></head>\n<body style="margin:0;padding:0;background:#F8F7F4;">\n<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#F8F7F4;padding:32px 16px;">\n<tr><td align="center">\n<table role="presentation" width="560" cellpadding="0" cellspacing="0" style="max-width:560px;width:100%;background:#FFFFFF;border:1px solid #E2E0DA;border-radius:16px;font-family:-apple-system,''Segoe UI'',Helvetica,Arial,sans-serif;">\n  <tr><td style="padding:32px 36px 0 36px;">\n    <p style="margin:0 0 4px 0;font-family:Georgia,''Times New Roman'',serif;font-size:26px;line-height:32px;color:#2B2A33;font-weight:500;">\n      Hi {{invite.display_name}}, you''ve been invited</p>\n    <p style="margin:0;font-size:15px;line-height:24px;color:#6F685D;">\n      {{invite.inviter_name}} invited you to join <strong style="color:#2B2A33;">{{org.name}}</strong> on Marula —\n      the workspace your team runs service work in.</p>\n  </td></tr>\n  <tr><td style="padding:28px 36px 0 36px;">\n    <table role="presentation" cellpadding="0" cellspacing="0" width="100%"><tr><td align="center">\n      <a href="{{invite.url}}" style="display:inline-block;background:#2B2A33;color:#E8B83A;font-size:15px;font-weight:600;line-height:20px;padding:12px 28px;border-radius:10px;text-decoration:none;">Accept your invitation</a>\n    </td></tr></table>\n    <p style="margin:14px 0 0 0;text-align:center;font-size:13px;line-height:20px;color:#6F685D;">\n      If the button doesn''t work, copy this link into your browser:<br>\n      <a href="{{invite.url}}" style="color:#9A5B0F;word-break:break-all;">{{invite.url}}</a></p>\n  </td></tr>\n  <tr><td style="padding:28px 36px 32px 36px;">\n    <p style="margin:0;padding-top:16px;border-top:1px solid #E2E0DA;font-size:13px;line-height:20px;color:#6F685D;">\n      This link expires on {{invite.expires_at}}. If you weren''t expecting this invitation,\n      you can ignore this email — your address won''t be added to the workspace\n      until you accept.</p>\n  </td></tr>\n</table>\n</td></tr>\n</table>\n</body>\n</html>'),
        ('email', 'ticket_created_confirmation', 'Ticket created confirmation',
         'Auto-responder acknowledging an email that opened a new ticket.',
         '[{{ticket.number}}] {{ticket.summary}} — we received your request',
         E'Your request "{{ticket.summary}}" was received and registered as ticket {{ticket.number}}.\nReply to this email to add information — keep the [{{ticket.number}}] tag in the subject.\n',
         NULL::text),
        ('email', 'comment_notification', 'Comment notification',
         'Requester notification that a client-visible reply landed on their ticket.',
         'Re: [{{ticket.number}}] {{ticket.summary}}',
         E'New reply on ticket {{ticket.number}}:\n\n{{comment.excerpt}}\n\nReply to this email to respond.\n',
         NULL),
        ('email', 'invoice_statement', 'Invoice statement',
         'Account statement on the dunning path: the client''s open invoices with outstanding balances.',
         'Account statement — {{client.name}}',
         E'Your current account statement:\n\n{{invoice.lines}}\n\nTotal outstanding: {{invoice.total_outstanding}} {{invoice.currency}}\n\nIf you have already sent payment, please disregard this notice.\n',
         NULL),
        ('email', 'magic_link', 'Magic-link sign-in',
         'The single-use sign-in link email for orgs with magic-link sign-in enabled.',
         'Your Marula sign-in link',
         E'Open this single-use link to sign in (valid 15 minutes):\n{{link.url}}\n\nIf it wasn''t you, ignore this mail.\n',
         NULL),
        ('email', 'password_reset', 'Password reset',
         'The single-use password reset link email.',
         'Reset your Marula password',
         E'A password reset was requested for your account.\n\nOpen this single-use link (valid 30 minutes):\n{{link.url}}\n\nIf it wasn''t you, ignore this mail.\n',
         NULL),
        ('email', 'email_verification', 'Email verification',
         'The single-use email-verification link email.',
         'Verify your Marula email',
         E'Confirm your Marula account email.\n\nOpen this single-use link (valid 24 hours):\n{{link.url}}\n',
         NULL)
      ) AS t(k, s, n, d, subj, b, h)
      WHERE NOT EXISTS (
        SELECT 1 FROM templates tpl
        WHERE tpl.org_id = org.id AND tpl.kind = missing.k AND tpl.slug = missing.s
      )
    LOOP
      INSERT INTO templates (id, org_id, kind, slug, name, description, subject, body, html, variables, builtin, is_system)
      VALUES (gen_random_uuid(), org.id, missing.k, missing.s, missing.n, missing.d, missing.subj, missing.b, missing.h, '[]'::jsonb, NULL, false);
    END LOOP;
  END LOOP;
END $$;
