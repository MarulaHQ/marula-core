# How to configure email

> **Audience:** admin — mail wiring decides who hears from you and whether
> client replies reach the ticket. Every staff and portal account lives
> with the addresses and transport you set here.

## Prerequisites

- Admin access to Settings → Email.
- Your own domain, and the ability to add DNS records where you registered it.
- One mail provider decision: a hosted API (Postmark) or your own SMTP
  relay. You can start on **None** and keep working. Mail queues.

This page covers three jobs in order: routing inbound mail to boards,
proving your domain with DNS, and choosing how outbound mail leaves.

## 1. Route inbound mail to boards

Meridian receives mail through a signed webhook, matches the recipient
address against your routing table, and lands the message on the target
board. Two kinds of addresses exist:

- a **catch-all** you set on Settings → Email (one per workspace), and
- a **per-board address**, set in the same page's board table.

Routing precedence is fixed. A board's own address wins. When no board
matches, an enabled catch-all takes the message. Both kinds of addresses
must be unique inside your workspace. Saving a duplicate returns a typed
conflict that names the owner, rather than resolving silently.

Steps:

1. Open Settings → Email and enter the catch-all address, pick the board it
   targets, and leave **Routing enabled** checked.
2. Add per-board addresses where clients or suppliers expect a specific
   inbox (for example `projects@yourdomain.com`). Leave a board's address
   empty to keep it on the catch-all.
3. Point your provider at `POST https://<your-host>/api/webhooks/email-inbound`
   and send one real message to each address.

Accepted messages answer `202` with one of these outcomes:

| Outcome | Meaning |
| --- | --- |
| `ticket_created` | New ticket on the target board. |
| `comment` | Threaded reply added to an existing ticket. |
| `duplicate` | Same Message-ID already processed. Nothing changed. |
| `quarantined` | Infected or unscannable attachment. Stored, not delivered. |
| `unrouted` | No board or catch-all matched. Stored, nothing created. |

`unrouted` and `quarantined` are records, not errors. The message is
visible, and you can fix routing and re-send from your provider.

### Sign the webhook

The endpoint is disabled until you set `EMAIL_INBOUND_HMAC_SECRET` on the
server. While it is unset every request answers `503`. When it is set,
each request must carry:

```text
X-Meridian-Signature: <hex HMAC-SHA256 of the exact request body>
```

Compute the HMAC over the raw body bytes with the shared secret. There is
no timestamp and no prefix. A mismatch answers `401`. The body is one
normalized JSON message:

```json
{
  "message_id": "<abc@example.com>",
  "from": "client@example.com",
  "from_name": "Dana at Example",
  "to": ["projects@yourdomain.com"],
  "cc": [],
  "subject": "Printer down",
  "text": "It is flashing red.",
  "html": null,
  "in_reply_to": null,
  "references": [],
  "received_at": "2026-09-26T09:00:00Z",
  "attachments": [
    { "filename": "photo.jpg", "content_type": "image/jpeg", "content_base64": "..." }
  ]
}
```

Your provider's inbound parser writes this shape. Meridian does no MIME
parsing itself. Message-ID is the idempotency key, so a provider retry
cannot create a second ticket.

## 2. Prove your domain with DNS

Deliverability is DNS work, not a Meridian setting. Add these records at
your DNS host, then give them an hour to propagate before you test.

| Record | Host | Value | Why |
| --- | --- | --- | --- |
| SPF | `@` or the sending subdomain | `v=spf1 include:<your provider's SPF host> -all` | Names the servers allowed to send for your domain. Keep one `v=spf1` record per name, because a second one voids both. |
| DKIM | the provider's selector name | the `TXT` or `CNAME` value your provider issues | Signs each message so receivers can verify it left with your consent. Postmark issues a CNAME pair. Keep both in place. |
| DMARC | `_dmarc` | `v=DMARC1; p=none; rua=mailto:dmarc@yourdomain.com` to start | Tells receivers what to do with mail that fails SPF or DKIM, and sends you reports. Move to `p=quarantine` once reports look clean. |

Two rules keep this honest:

- **Align the domains.** The From address you set on Settings → Email,
  and every per-board From address, must sit on a domain you have signed.
  A board sending as `projects@yourdomain.com` while only `mail.yourdomain.com`
  is signed fails DMARC alignment.
- **One sender per record set.** When part of your mail leaves through
  Postmark and part through your own relay, both include themselves in SPF
  and both need their own DKIM keys. Skipping this is the usual cause of
  "some replies go to spam".

## 3. Choose an outbound transport

Settings → Email → Sending holds one transport per workspace:

- **None** keeps every message queued in the delivery log. Nothing is
  sent and nothing is lost. This is a supported resting state, not an error.
- **Postmark** sends through the Postmark API. Paste the server token into
  the sealed field. It is encrypted at rest and never shown again.
- **SMTP relay** connects to your own host and port. The password goes
  through the same sealed field. Leave the username empty only when the
  relay trusts your network without authentication.

The page shows whether each credential is sealed and when it was last
replaced. Replacing a credential means pasting a new value, and there is no
way to read the old one back. Keep the original in your password manager.

### Test, then verify

Use **Send a test email** with your own address. It renders a real message
through the configured transport and logs the outcome. Then check:

1. the message arrived, and its headers show the From and Reply-To you expect,
2. the delivery log row turned `sent`,
3. a reply to it lands on the right board.

The delivery log lists every outbound message with its state. Ticket
replies, notification email, digests, test sends: all of them appear.

| State | Meaning |
| --- | --- |
| `queued` | Waiting for the next send cycle, or for a transport to be configured. |
| `sending` | A worker holds a lease on it right now. |
| `sent` | The transport accepted it. The provider's own log owns final delivery. |
| `dead` | Eight attempts failed. The last error is on the row. Fix the cause and re-send from the source. |

Retries back off from 30 seconds, doubling each attempt and capping at one
hour, up to eight attempts. Bounces are not fed back into the ledger yet.
That arrives with provider bounce webhooks. Until then, watch your
provider's log for them.

### SMTP in practice

The built-in SMTP client speaks plain SMTP with optional `AUTH PLAIN`. It
does **not** implement STARTTLS or implicit TLS, so use it only against a
relay on a trusted network: a local smarthost that forwards onward, or a
development mail catcher such as Mailpit. For internet-facing delivery, use
Postmark. A relay that requires STARTTLS will refuse the connection, and the
delivery log will name the reason on the row.

## 4. Notification email and the daily digest

Notification email uses the same transport and the same sealed credentials,
with a branded wrapper around it:

- **Brand name and logo URL** on Settings → Email → Notification email
  head the message. The logo must be an `https` URL. Empty renders the
  name alone.
- **Org signature** (Sending section) closes it, and the recipient's
  first name opens it.
- **Workspace defaults per event** decide which kinds send email at all
  until a member overrides them on their own Notifications page. The
  chain is member override, then workspace default, then built-in. Quiet
  hours hold non-critical email during the member's window.
- **Daily digest** is a member choice on the Notifications page: one email
  a day summarizing unread notifications, sent at the hour they pick in
  their account timezone. It is scheduled rather than interruptive, so
  quiet hours do not hold it. The workspace policy can disable digests
  entirely. It cannot turn them on for someone who has not opted in.

## 5. Reply by email

When a client replies to a message from a ticket, Meridian matches the
`In-Reply-To` and `References` headers against the message ledger and
threads the reply onto the ticket. For that to work:

- the Reply-To on the message must route inbound (a board address or the
  catch-all),
- inbound routing must be enabled, and
- the provider must preserve `Message-ID`, `In-Reply-To`, and
  `References`. Postmark and the common parsers do.

The Email page shows reply routing as on or off. If it reads off, no client
reply can reach a ticket until routing is enabled.

## 6. Malware scanning (optional, recommended)

Attachments are scanned before anything is written when a scanner is
configured with `EMAIL_SCAN_CLAMD_ADDR`, the host and port of a clamd
daemon. The pipeline fails closed. If the scanner is unreachable or errors,
the message is quarantined rather than admitted.

Without a scanner the message is stored with scan status `skipped`, which
means the bytes are kept unscanned. That is a deliberate standalone-first
default, not a promise of safety. Run clamd when you accept attachments
from outside your organization.

## 7. Self-host runbook

For deployments configured by environment (the path older installs use),
the worker still honors these variables:

```bash
# Inbound webhook (required before the endpoint accepts anything)
EMAIL_INBOUND_HMAC_SECRET=<64+ random chars>

# Outbound transport used when a workspace has not chosen one
EMAIL_TRANSPORT=postmark            # or: smtp, none
POSTMARK_SERVER_TOKEN=<token>
POSTMARK_BASE_URL=https://api.postmarkapp.com
# or, for SMTP:
SMTP_HOST=smtp.internal.example
SMTP_PORT=587
SMTP_USERNAME=meridian
SMTP_PASSWORD=<password>

# Optional malware scanning
EMAIL_SCAN_CLAMD_ADDR=clamd:3310

# Worker to core-api send pipeline (the worker claims and reports sends)
MERIDIAN_INTERNAL_TOKEN=<shared random token>
```

Notes:

- A transport chosen on Settings → Email wins over `EMAIL_TRANSPORT`. The
  environment value is the fallback for workspaces with no selection.
- Workspace credentials stored through Settings → Email are sealed with
  `MERIDIAN_SECRET_KEY`. core-api and the worker must share that key. If
  they do not, the worker cannot open what the API sealed, and sends fail
  with a decrypt error on the row.
- With `EMAIL_TRANSPORT=none`, the worker reports send failures as
  retryable ("no outbound transport is configured"). Messages stay queued
  instead of burning their attempts.

## What you learned

- Inbound mail is addresses plus a signed webhook. Routing is
  board-first, then catch-all, and both kinds of addresses must be unique.
- Deliverability is SPF, DKIM, and DMARC on the domain your From addresses
  use, including every per-board address.
- One transport per workspace, credentials sealed and write-only, and a
  delivery log that shows each message's state and last error.
- The built-in SMTP client has no TLS. Keep it on trusted networks, and use
  Postmark for the public internet.
