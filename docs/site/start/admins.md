# Admin tasks

> **Audience:** admin — you hold settings access and you are the person the team
> asks when something is set up wrong.

Most of this work happens once, at the start, and then twice a year. The list is
grouped by the job rather than by the settings menu, because the menu moves and
the job does not. When something is broken rather than unowned,
[Troubleshooting](../troubleshooting.md) starts from the symptom.

## "We are new. Where do I start?"

1. [Bootstrap your first organization](../how-to/admin-org-bootstrap.md) — create
   the org, then set the workspace name, the ticket prefix and the base currency,
   and the timezone the workspace runs on.
2. [Manage users and invites](../how-to/admin-users-invites.md) — invite the
   team, turn on two-factor and passkeys, set quiet hours.
3. [Connect an IdP (SSO)](../how-to/admin-sso.md) — if you would rather people
   signed in through Google Workspace, Entra ID, Okta, Keycloak or any OIDC
   provider.
4. [Set workflow categories](../how-to/admin-workflow-categories.md) — columns
   and categories, while SLA timers and billing stay tied to the stable stages
   underneath.
5. [Configure SLA calendars and escalation ladders](../how-to/admin-sla-calendars.md) —
   define business hours and holidays, then what happens when a priority clock
   runs out.
6. [Run the local demo stack](../how-to/local-demo-stack.md) — if you want a
   seeded copy to try all of this against before you touch the real workspace.

## "Email is not working"

[Configure email](../how-to/admin-email.md) covers the whole path: inbound routing
to boards, the signed webhook, SPF/DKIM/DMARC, transport choice and its test send,
the delivery ledger, notification email and the digest, the signature policy, reply
by email, and the optional malware scanner. Read it top to bottom once; after that
jump to the section that matches the failure.

## "Clients should be able to help themselves"

- [Set up the portal](../how-to/admin-portal-setup.md) — which boards each client
  sees, portal accounts for their contacts, and what portal accounts can never
  reach.
- [Portal onboarding](../tutorials/portal-onboarding.md) — the same setup as a
  worked walkthrough, ending with what the client sees on first sign-in.
- [The portal isolation model](../explanation/portal-isolation.md) — the fencing
  rule behind the exposure setting.

## "The queue needs to sort itself"

- [Author automation rules](../how-to/admin-automation-rules.md) — event-driven
  rules, a dry run against sample tickets before you trust one, and alert routing.
- [Turn on travel time and mileage reconciliation](../how-to/admin-routing-travel.md) —
  drive-time estimates, technician start points, mileage reimbursement.
- [Set up CSAT surveys on resolution](../how-to/admin-csat-surveys.md) — one-click
  surveys on close, with quiet hours and a minimum interval.
- [Set up toolboxes and dispatch defaults](../how-to/admin-toolboxes.md) — group
  serialized tools into kits and recommend parts on the day pack.
- [Set up labels](../how-to/admin-labels.md) — QR and Code128 labels for Zebra and
  DYMO printers.

## "I need numbers"

- [Build dashboards and report schedules](../how-to/admin-dashboards-reports.md) —
  widgets over the read model and recurring email digests.
- [Report against the read model](../how-to/read-model.md) — the stable `ops.v1_*`
  views for a BI tool or SQL client.
- [Billing schema reference](../reference/billing-schemas.md) — the tables behind
  work categories, billing verification, contract pricing and project milestones.

## "We are moving data in, or out"

- [Run a dry-run import](../how-to/admin-imports.md) — preview a CSV or JSON
  import, save the column mapping, then run it for real.
- [Customize without code](../how-to/customize.md) — typed custom fields and board
  changes that survive an upgrade.
- [Export your organization's data](../how-to/export-data.md) — a versioned bundle
  and a record of past bundles.

## "Something is extended, connected or automated"

- [Manage AI access](../how-to/admin-ai-access.md) — master switch, per-role
  grants, usage caps.
- [Bring up AI with your own API keys](../how-to/admin-ai-bringup.md) — the six
  settings decisions in the order that makes the first live call work.
- [Run transcription and embeddings on your own hardware](../how-to/admin-local-ai-models.md) —
  keeps model weights and audio inside your network.
- [Configure the plugin consent flow](../how-to/admin-plugin-consent.md) — install
  digest-verified plugins and read the capability list before enabling one.
- [Extend Marula with plugin surfaces](../how-to/admin-plugin-surfaces.md) —
  declarative widgets, triggers, read nodes, action steps and field validators.
- [Manage API keys](../how-to/admin-api-keys.md) — narrow bearer tokens and
  rotation without downtime.

## "Keep it healthy"

- [Watch your self-hosted services](../how-to/admin-self-host-services.md) —
  container health, ClamAV quarantine, and links out to Grafana or Uptime Kuma.
- [Upgrade to the latest release](../how-to/upgrade.md) — preflight, backup,
  migrations and verification, plus what to do when you are stuck.
- [Backup and restore](../reference/backup-restore.md) — the runbook you want to
  have rehearsed before you need it.

Your instance's operator owns most of this last group. If that is a different
person, send them [Operator tasks](operators.md).

## Not documented yet

- A map of the settings surface: which left-rail group holds which control. The
  how-to pages name the paths they need; no page lists them all.
- The permission model: role names, what each grants, and how a custom role
  combines with grants elsewhere. Nothing on the site defines the matrix.
- The connector catalogue: which RMM, PSA, DNS and accounting sources exist, and
  what each one syncs. [Your first inventory sync](../tutorials/first-inventory-sync.md)
  walks one connector; the set is undocumented.
- Audit export: no page states how to take an audit trail out of Marula for a
  compliance request.
