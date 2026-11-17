---
description: "Outbound mail is a template, not code. Here is which rows you can edit, which are shipped defaults, and how placeholders become values at send time."
---

# How to edit the emails Marula sends

> **Audience:** admin — the template list is gated by the admin module, and a
> send is a workspace-wide decision.

**Settings → Email → Email templates** lists every template the workspace can
render: the mail Marula sends, and the starters it offers for tickets, notes,
quotes, projects, workflows — and knowledge articles. Each row shows its kind, its
provenance and the three actions the API supports — edit, duplicate, reset.

The list groups by kind, email first. The outbound mail you are most likely to
change is there: the workspace invitation, ticket-created confirmation, comment
notification, invoice statement, magic-link sign-in, password reset and email
verification.

## Prerequisites

- Admin access to **Settings → Email**. Reading the list needs `admin:view`.
  Creating, editing and deleting need the matching admin grant.
- A transport that can send. A template renders whether or not mail is
  configured. The delivery ledger and the test send are covered in
  [Configure email](admin-email.md).

## 1. Know which rows you may edit

Provenance has two states, and the list marks them:

- **Built-in** — a shipped starter. It is read-only at the API, so the row
  offers **Duplicate** instead of a dead editor. Duplicate it, edit the copy,
  and keep the original as the reference.
- **Workspace row** — an ordinary row in your organisation. It is editable in
  place.

The outbound emails seed as workspace rows with fixed slugs on purpose: the
sender looks the row up by the kind's slug, so editing the row changes what sends. A duplicate gets its own slug and does **not** replace the
message that sends. If your goal is to change the invitation email, edit the
invitation row. Duplicate it only if you want a spare.

**Reset to default** appears on shipped defaults that carry a built-in tag. It
restores the canonical content for that slug. A fork is never restorable,
because the fork is yours.

## 2. Edit one message

Open a row. The editor has:

- **Name and description** — what the template is for. The description is the
  line that helps the next admin pick the right row.
- **Subject** — optional. A body-only template says so instead of showing an
  empty field that looks broken.
- **Body** — rich text for HTML mail, with the same editor used elsewhere in
  the workspace.
- **Placeholders** — a dropdown of the keys that kind may use. Inserting one
  puts `{{client.name}}` (or the kind's equivalent) at the cursor.

Saving is automatic. A draft that would fail the server's write-time checks (an empty name or body,
or a placeholder outside the kind's closed vocabulary) is blocked locally with
the reason, so autosave does not loop on a refusal.

The placeholder list is closed on purpose. The server validates the same list
on write, which means a typo cannot become an empty string in a customer's
inbox. It is refused while you are editing.

## 3. Use the preview, not a test send, for wording

The preview renders the template with sample values and labels itself as such:
**Your draft with sample values. Nothing here is saved or sent.** Use it to
check that a sentence still reads when a client name is long, or when a ticket
number replaces the placeholder.

Below the editor, the plain-text fallback is derived from the HTML body and is
read-only. It is what a text-only mail client receives, so read it once before
you save a heavy redesign.

When you do want a real delivery, use the test send on the transport page. The
preview never sends.

## 4. Match the words to the moment

A few habits keep transactional mail from reading like marketing:

- **Invitation** — say who invited the person, what the workspace is, and when
  the link expires. The default already carries `{{invite.inviter_name}}`,
  `{{invite.url}}` and `{{invite.expires_at}}`. Keep those.
- **Password reset and magic link** — one action, one link, one expiry. Do not
  add a second call to action.
- **Ticket confirmation and comment notification** — name the ticket by number
  and summary so the recipient can match it to their own records.
- **Invoice statement** — the amount and the period, not a pitch.

If a message needs a person's name, use the contact or user placeholder rather
than a generic "Hi there". The placeholder renders empty when the record has no
value, so the sentence must still work without it.

## 5. Keep the set coherent

Templates are looked up by kind and slug. Renaming a template does not change
its slug, so the send path keeps working. Changing a slug on a workspace row
would point the sender at a different row. Treat a slug as an identifier, not
as copy.

Two habits keep the set maintainable:

- Before editing a shipped default, duplicate it if you may want the original
  wording back. A built-in can be reset. A fork cannot.
- After a change, read the preview of each message in the same flow: invite,
  accept, first ticket, close. A phrase that works alone can repeat awkwardly
  when the messages arrive together.

## What you learned

- The list holds both shipped starters and workspace rows, and the badge tells
  you which.
- Outbound email rows are editable in place because the sender looks them up by
  a fixed slug. A duplicate does not replace them.
- Autosave blocks an empty body or an out-of-vocabulary placeholder locally.
- The preview uses sample values and never sends. The plain-text fallback is
  derived and read-only.
- Built-in rows can be reset. Forks cannot.

## Where to go next

- [Configure email](admin-email.md) — transport, DNS, the delivery ledger and
  the test send.
- [Manage users and invites](admin-users-invites.md) — the flow the invitation
  template belongs to.
- [Brand the workspace](customize.md) — where the logo and footer come from.
- [Manage AI access](admin-ai-access.md) — the same grant discipline for the
  assistant's mail drafts, if you grant it.
