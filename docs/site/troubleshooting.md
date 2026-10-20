# Troubleshooting

Start from what you see. Each entry below names a symptom in plain words, the
checks in the order they usually find it, and the page that explains the part
that turns out to be broken. Nothing here restates a feature. It points at the
page that owns it.

If your symptom is not here, two other places help:

- [Get help inside Marula](how-to/get-help.md) opens the `?` panel on the surface
  you are standing on.
- [Start here](start/index.md) routes by who you are, if the problem is "I do not
  know where this lives" rather than "this is broken".

## "A customer cannot log in"

A customer here means a contact at a client company with a portal account. It is
a different login system from staff, and that is deliberate.

1. Does the client have a portal account at all, and is the contact's email the
   one that was provisioned? See [Set up the portal](how-to/admin-portal-setup.md).
2. Portal accounts do not use the staff password-reset flow. Your service desk
   resets the password or sends a sign-in link from **Settings → Portal**
   ([Manage users and invites](how-to/admin-users-invites.md) §3).
3. If they sign in but see an empty workspace, the problem is exposure, not auth:
   [the client sees no tickets](#the-client-signs-in-and-sees-nothing).

## "I cannot sign in, or I was signed out"

1. An email verification link is single-use and valid for 24 hours. A password
   reset link is single-use too. Ask for a new one rather than reusing the old
   one ([Manage users and invites](how-to/admin-users-invites.md) §2).
2. If your workspace uses SSO, sign-in happens at your identity provider, and the
   provider's own lockout applies first. [Connect an IdP](how-to/admin-sso.md)
   describes the round trip and what SSO does not change.
3. Locked out after repeated attempts, or revoked sessions: an admin can review
   signed-in devices and has to be involved. Nothing on the site states a
   self-service way back in.

## "The two-factor prompt will not accept my code"

- Check the device clock. A TOTP code is time-based, and a phone minutes out of
  step produces codes that are refused.
- Use a saved recovery code, then register the authenticator again.
- Turning 2FA off, or regenerating recovery codes, needs your password **and** a
  live code. An admin cannot do it for you from the outside
  ([Manage users and invites](how-to/admin-users-invites.md) §4).

## "A ticket is on the wrong board, or a status disappeared"

Columns and categories are yours to rename. The **stage** underneath is what SLA
and billing bind to, and what automation and reporting read. A column that was deleted did not take
its tickets' stage with it. Read [Set workflow categories](how-to/admin-workflow-categories.md),
then [Customizations are data](explanation/customizations-are-data.md) for why a
rename is safe and a stage change is not.

## "The SLA clock says something I did not expect"

The clock reads the business-hours calendar attached to the policy, not wall
time. A holiday, a different start time, or a priority that has no ladder entry
all change what the same elapsed time means. See
[Configure SLA calendars and escalation ladders](how-to/admin-sla-calendars.md).

## "My timer kept running, or my hours never reached billing"

1. Hours reach billing only after they leave the timer and are sent for approval.
   [Work a ticket](how-to/staff-work-a-ticket.md) shows the step.
2. Hours marked non-billable need a recorded reason. Without it the entry is not
   complete ([Log billable time with categories](how-to/staff-time-categories.md)).
3. If the hours exist but the invoice does not include them, this is a billing
   window question, not a timer question:
   [Verify a billing period](how-to/admin-billing-verification.md).

## "A customer's email never created a ticket"

1. Is the destination address routed to a board? An unrouted address has nowhere
   to put the message ([Configure email](how-to/admin-email.md) §1).
2. Is the inbound webhook signed correctly? An unsigned or mis-signed delivery is
   refused, which is the intended behaviour (§1, "Sign the webhook").
3. Did the message get scanned and held? See
   [the quarantined message](#a-message-was-held-or-quarantined).

## "Our replies are not arriving"

1. SPF, DKIM and DMARC first. A domain that has not been proven sends mail that
   receivers discard quietly (§2).
2. Then the transport: the test send in §3 is the fastest way to separate a
   transport problem from a domain problem.
3. Then the delivery ledger, which records the state each send reached.

## "A message was held or quarantined"

The malware scanner is optional and off unless configured. When it is on, a held
message appears in the quarantine view rather than being delivered.
See [Configure email](how-to/admin-email.md) §7 and
[Watch your self-hosted services](how-to/admin-self-host-services.md).

## "The client signs in and sees nothing"

Exposure, not permissions. A portal account only sees the boards an admin exposed
to that client company. [Set up the portal](how-to/admin-portal-setup.md) §1, and
[The portal isolation model](explanation/portal-isolation.md) for why the fencing
works this way.

## "The invoice total does not match what I expected"

Work through it in this order, because each step changes the next number:

1. Were the hours verified in that window? [Verify a billing period](how-to/admin-billing-verification.md).
2. Is a contract cap, minimum, escalator or prepaid block involved?
   [Price a service contract](how-to/admin-contract-types.md).
3. Is more than one currency involved? Reconciliation totals are grouped by
   document type and are never summed across types, so a total that looks low is
   often two totals that were never meant to add
   ([FX additivity](explanation/fx-additivity.md)).
4. Was the invoice already issued? Then it is locked, and the correction is a
   credit note, not an edit ([Billing corrections](explanation/billing-corrections.md)).

## "AI does nothing"

Two switches, in order. The organization-wide master switch ships **off**, and
each helper still starts disabled until it is granted to a role.
[Manage AI access](how-to/admin-ai-access.md). If the master switch is on and the
grant exists, check the provider credential path:
[Bring up AI with your own API keys](how-to/admin-ai-bringup.md) lists the six
settings decisions in the order that makes the first live call work, and the
refusal names the step you skipped.

## "An automation rule did not fire"

Run the rule's dry run against sample tickets before changing anything. It shows
what the rule would have done with the event you expected
([Author automation rules](how-to/admin-automation-rules.md)). Day packs and
recommendations depend on the rules and on stock existing for the items, so check
both.

## "A plugin will not install"

The artifact digest must match what the plugin was signed as, and the declared
capability list is what you consent to at install time. A digest mismatch stops
the install rather than warning
([Configure the plugin consent flow](how-to/admin-plugin-consent.md)). For a
key you are rotating rather than installing:
the plugin signing key rotation runbook (internal documentation).

## "Uploads fail above a few megabytes"

This is usually the proxy, not Marula. The API's own limit defaults to 25 MB
(`MARULA_MAX_UPLOAD_MB`), and the proxy in front of it must carry the same or a
larger value — a proxy set to `25MiB` while the app allows slightly more refuses
files the app would have accepted, and the error names the wrong layer.
[Reverse proxy and TLS](reference/reverse-proxy-tls.md) has the exact numbers for
nginx, for Caddy, and for a Kubernetes ingress.

## "Live updates stopped"

The live leg is a streamed connection. A proxy that buffers it turns live updates
into no updates, and the page still looks healthy. Check the buffering settings
and the route listed in [Reverse proxy and TLS](reference/reverse-proxy-tls.md).

## "The site is down, or returns 502/504"

1. Which layer answers? A proxy error page means the proxy is up and the app is
   not. A connection refused means neither is.
2. Check the container health probes on
   [Watch your self-hosted services](how-to/admin-self-host-services.md).
3. If the database is the part that is gone, go to
   [Backup and restore](reference/backup-restore.md) rather than restarting
   containers repeatedly.
4. A database that will not start after an upgrade, or a migration that stops
   halfway, is a restore decision, not a debugging session: see
   [Upgrade to the latest release](how-to/upgrade.md) and the N−2 window in
   [the upgrade policy](explanation/upgrade-policy.md).

## "A report disagrees with the list I am looking at"

Reports read the stable views. The list reads the write tables. They are allowed
to differ in shape, and the view is the contract
([The read model view catalogue](reference/read-model.md)). If the underlying
value is in dispute, use provenance to see who changed what:
[Provenance vs audit](explanation/provenance-vs-audit.md).

## Not covered yet

These symptoms are real and have no page. They are listed here so the gap stays
visible instead of being answered with a guess.

- **Search returns nothing, or stale results.** No page describes what the index
  contains or how quickly it catches up.
- **Notifications arrive late, or not at all.** Quiet hours and urgent bypass are
  documented. The delivery path behind them is not.
- **An import wrote the wrong rows.** The dry run is documented, the recovery path
  is not.
- **A sign-off cannot be undone.** The billing pages say issued invoices lock. No
  page walks the escalation when a sign-off was a mistake.
- **Everything is slow.** The perf program records baselines for the maintainers.
  Nothing tells an operator which number to look at first on their own install.
