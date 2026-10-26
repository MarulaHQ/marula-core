---
description: "Every message you send to a client can close with your own details: your name, your address, the workspace name."
---

# How to sign your own email

> **Audience:** staff — anyone who writes to a client from a ticket. Admins
> can read this too, but the policy that governs it is on
> [How to configure email](admin-email.md).

Every message you send to a client can close with your own details: your
name, your address, the workspace name. You write it once, and it goes out
on the messages you send from then on.

## Write your signature

1. Open **Settings**, then **Preferences**.
2. In the **Email signature** card, write your greeting (the line that opens
   the message) and your signature (the block that closes it).
3. Saving is automatic — there is no Save button. The card says so under the
   field once it lands.

**Placeholders fill in when a message sends, not when you save.** Insert
`{{user.name}}`, `{{user.email}}` or `{{org.name}}` with the Insert a field
buttons, and the picker shows what each one expands to for you right now. If
you change your display name or your email address later, the next message
picks that up — you do not touch the signature again.

The preview under the editor shows the saved block as the next message will
carry it, along with which of the two is answering: your default, or the
workspace's.

## Change it for one message only

The close-flow's email step has the same two fields, and the same preview.

- **Empty** the field to send your own default, and the workspace's if you
  have none.
- **Write** something to send it for this message only. Nothing is stored.
- **Reset to my default** puts your own default back after an edit.
- **Save as my default** makes the edit permanent. It is the same write as
  the preferences page, so the two never disagree.

The note you post stays a note. The requester is emailed by the backend,
which puts the greeting above and the signature below the reply it renders —
so what the preview shows is what arrives, and nothing is welded into the
note the rest of the team reads.

## If your workspace locks signatures

Some workspaces send one signature of their own on every message. When that
is on:

- Your signature field is closed, and the card shows the workspace
  signature your messages will carry.
- The Insert a field buttons and the per-message fields in the close-flow
  are gone rather than silently ignored — the close-flow says the workspace
  signature is locked to the organization, and a message that tried to
  override it was refused rather than sent with your own block.
- Your greeting still works. The lock covers the signature only, because a
  signature is a claim about who wrote the message and a greeting is just
  the house opening line.

An admin changes this under **Settings → Email → Sending**. If you expected
a personal signature and cannot find it, check that setting first.

## What the delivery log shows

Admins see, per outbound message, which signature went out and where it came
from: **the sender**, **the board**, **the organization**, **this message**,
or **none**. If you send a one-off signature to a client and the log says
**this message**, that was the override, and your default is unchanged.
