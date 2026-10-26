---
description: "When an external script, monitoring tool, or custom integration needs to call Marula's API without a human signing in, you issue a scoped API key under…"
---

# How to manage API keys

> **Audience:** admin — machine credentials are settings-owner work.

When an external script, monitoring tool, or custom integration needs to call Marula's API without a human signing in, you issue a scoped **API key** under **Settings → API keys**. Every key is shown once when you create it, limited to only the permissions you select, and rotated by creating a replacement key before revoking the old one.

## Prerequisites

- Admin access to **Settings → API keys**.
- A list of which resources the integration actually needs to read or write (for example, reading tickets vs. creating tickets).

## The API key lifecycle

**Settings → API keys** summarizes the security rule:

> "Bearer credentials for machine integrations. Secrets are shown once at
> creation; to rotate, create a replacement key then revoke this one."

There is no "view secret again" button and no editing scopes on a live key. The lifecycle is always: **create and copy once → rotate by creating a replacement → revoke the old key**.

## 1. Create a key with the narrowest permissions

Click to create a new key and select only the `resource:verb` scopes the integration requires (such as `tickets:read`, `tickets:write`, `clients:read`, or `timers:read`).

If you run two separate scripts (one that reads ticket queues for a wallboard and another that creates tickets from an alert script), create two separate keys so the wallboard key can never modify data.

## 2. Copy and store the secret immediately

Marula displays the full bearer token **exactly once** right after creation. Copy it directly into your integration's secret manager or environment file.

After you close the creation dialog, the table shows only the key's short **prefix** (such as `mrp_dem…`), its granted scopes, and its **last-used** timestamp. Because the full token is never shown again, taking a screenshot of the API keys table for a security review is always safe.

## 3. Rotate keys without downtime

When it is time to rotate a key:

1. Create a **new replacement key** with the needed scopes.
2. Update your external integration with the new token and confirm its requests succeed.
3. Click **Revoke** on the old key row.

Creating the replacement before revoking the old key keeps your integration running without a single dropped request.

## 4. Revoke unused or compromised keys

Click **Revoke** on any row whenever an integration is retired, a staff member who handled the secret leaves, or a token may have been exposed in logs. Check the **Last used** column to spot forgotten keys that are no longer active.

## 5. Audit log visibility

Every key creation and revocation is recorded in the [audit trail](../explanation/provenance-vs-audit.md) with the administrator's name and timestamp.

## Related machine integration guides

- **Webhooks:** Configure HMAC-signed outbound webhooks and inbound event receivers under **Settings → Integrations** (see [Author automation rules](admin-automation-rules.md)).
- **Plugins and connectors:** Manage capability consent for installed plugins in [Configure the plugin consent flow](admin-plugin-consent.md).
