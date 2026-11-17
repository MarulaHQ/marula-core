---
description: "Connectors are installed per workspace and configured per connection: credentials, settings, destinations, health, and who may use it."
---

# How to connect a tool your team already uses

> **Audience:** admin — installing and changing a connection needs settings
> access and the integration grant.

A connector is the link between Marula and a system your team already works
in: chat, telephony, automation. It contributes data and destinations, never a
new page of its own. **Settings → Connectors** is the roster. Each card is one
installed connector, and each connection inside it is one use of that connector
with its own credentials, settings and audience.

Connectors are not webhooks. Signed delivery logs and inbound endpoints live
under **Settings → Integrations**. A connection is the vendor relationship
itself.

## Prerequisites

- **Install and change** needs `special:manage_integrations`. An admin who
  configured a connection can always reopen it to repair it.
- The connector must be installed in the workspace. The roster lists what is
  installed, and the plugin marketplace is where a new connector comes from.
- The vendor's own identifiers for the destinations you want to use. Marula
  does not list a vendor's channels yet, so bring the channel id, conversation
  id, route id or workflow id with you.

## 1. Read the roster before you change anything

Open **Settings → Connectors**. Each card shows the connector's name and
purpose, one status pill, one action, and a line naming how many connections
it has and which clients they serve.

The pill is the worst state among the connector's connections:

| Pill | What it means |
|---|---|
| **Connected** | Every connection is healthy |
| **Needs attention** | At least one connection is degraded, unauthenticated or half-configured. The meta line says how many of how many |
| **Not connected** | The connector is installed but has no usable connection yet |
| **Disabled** | The connection exists but is turned off |

The action follows the state: **Connect** when there is nothing yet, **Fix**
when something needs attention (it opens the failing connection, where the
health section explains why), and **Manage** otherwise. Search filters by name
and purpose.

An empty roster is not an error. It means no connector is installed, and the
empty state names the standard set and where the rest come from.

## 2. Create one connection

Press **Connect** on the card. A connection is one use of the connector, so it
has its own name, scope and credentials.

1. **Name it so you will recognise it.** The card lists connections by this
   name, so "Slack — Acme" survives a second workspace better than "Primary".
2. **Choose the scope.** An organisation connection serves every client a
   caller can see. A client-scoped connection serves one client, which is how
   two customers with two credentials stay separate under one connector card.
3. **Fill the settings.** Fields come from the connector's own schema. A
   credential field is write-only: once saved it reads **Configured** and can
   only be replaced, never read back.
4. **Press Connect.** Validation errors land on the field that caused them.

The server's answer replaces the card. Nothing is painted before the API
confirms it.

## 3. Add the destinations work goes to

Open the connection and use **Add destination**. A destination is one named
target inside the vendor: a Slack channel, a Teams conversation, a phone route
or an automation workflow. Give it a label a person would say out loud, such as "#tier-2
cover" rather than "C01234567", then the vendor's own identifier:

- Slack wants the channel id, which starts with `C`, not the `#name`.
- Teams wants the conversation id with the tenant id as its parent.
- A phone route wants the id the PBX assigned.
- A Rewst workflow wants the workflow id Rewst assigned.

A duplicate destination answers a conflict on the field that caused it. A
disabled destination stays in the list. Work sent to it is refused with the
reason rather than dropped silently. Remove a destination only when it should
stop existing.

## 4. Prove the connection works

The **Health** section carries the status, the last success, the last failure
with the vendor's own message, and **Test connection**. The test runs the
connector's canary and writes the result into the section — it does not
disappear as a toast before you have read it.

A failed canary is the vendor's answer, not a Marula guess. Read the vendor
message first. It usually names the expired token, the wrong workspace or the
missing scope. Fix that, then test again.

## 5. Decide who may use the connection

**Access** is enforced, not advisory. The default is **All staff**, meaning
everyone the module permission already allows. Add named roles to narrow it.

The server enforces three rules, so the panel cannot soften them:

- Outside the audience, the connection does not exist. It is absent from the
  roster, from record surfaces and from destination pickers — never a disabled
  control for a door the person cannot open.
- The audience narrows use, never client access. A client-scoped connection is
  still refused to a caller who cannot see that client, whatever the audience
  says.
- Owner and admin bypass the audience, so the person who installed a
  connection can always return to repair it.

An empty audience is the shipped default and is stated as **All staff**, not
as an absence.

## 6. Repair, disable or remove

- **Fix** from the roster, or **Test connection** in Health, is the repair
  path. Start with the vendor message.
- **Disable a destination** when it should stop receiving work but stay on
  record.
- **Remove connection** when the relationship is over. It is destructive and
  confirmed. The audit trail keeps the decision, not the credential.

Credentials are never displayed after they are saved. Replacing a credential
is the only edit. Reading it back is not offered, because the value was sealed
on write.

## What you learned

- The roster is one card per installed connector, and the card's pill is the worst
  state among its connections.
- A connection has a name, a scope and its own credentials, so two clients can
  share a connector without sharing a secret.
- Destinations use the vendor's own identifiers, and a disabled one refuses
  work with a reason.
- Health shows the vendor's message and a canary that stays on screen.
- Access narrows who may use a connection. It never widens what they can see.
- Connectors are not webhooks. Delivery logs live under Integrations.

## Where to go next

- [Manage AI access](admin-ai-access.md) — the same grant discipline for AI
  surfaces.
- [Configure the plugin consent flow](admin-plugin-consent.md) — how a
  connector is installed in the first place, and what the capability list
  means before you enable it.
- [Author automation rules](admin-automation-rules.md) — where a destination is
  most often used.
