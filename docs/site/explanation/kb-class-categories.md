# Knowledge-base categories and the secret vault share one grant model

> **Audience:** both — admins administer grants; every staff member
> experiences the hidden-vs-dead distinction as security, not cosmetics.

Your runbooks in the **Knowledge Base** (`/kb`) and your stored credentials in the **Secret Vault** (`/passwords`) use the exact same folder permission model ([ADR 0006](../../adr/0006-native-secrets-vault-and-request-portal-plugin-boundary.md)). Once you understand how access works on a Knowledge Base folder tree, you already know how it works in the password vault.

## 1. Access is granted on the category folder tree

Both documentation and credentials vary in sensitivity depending on where they sit: a general printer setup guide is fine for every technician to read, while a client's root domain controller credentials or an internal HR policy folder should be restricted to senior leads.

In Marula, you organize both `/kb` and `/passwords` into category trees (either organization-wide or scoped to a specific client) and attach access grants directly to the category chain:

- **Knowledge Base (`/kb`):** Categories hold articles. In addition to staff access grants, articles inside a client-facing category can be marked **client-facing** so they appear in that customer's [Client Portal](portal-isolation.md).
- **Secret Vault (`/passwords`):** Categories hold credentials at the organization level or per client, with restricted category chains gated by role or user grants. Every time an authorized user reveals a secret, Marula writes an entry to the audit log.

## 2. Restricted items are completely hidden, never shown as locked rows

The vault's permission rule is simple:

> "you cannot access are hidden entirely, never rendered as dead entries"

This protects your security posture in two ways:

- **No "Access Denied" teasers for ungranted staff:** If a technician does not have access to a restricted folder chain, those passwords or articles do not appear in their list or search results at all. There are no greyed-out rows or item counts hinting that a secret exists.
- **Every password reveal is logged:** When a user *does* hold access to a vault category, viewing a secret writes a signed `secret.*` event to the [Audit ledger](provenance-vs-audit.md) recording who revealed it and when.

## 3. One permission governs reading and editing in a folder

Because permissions attach to the category chain itself, you never have to juggle mismatched per-document read/write overrides. If your role has access to a category chain, you can read and maintain records in that folder; if an article should also be visible to customers in the portal, you flip the explicit **client-facing** opt-in on that article.

## Related guides

- How ticket categories and workflow stages work: [How to set workflow categories](../how-to/admin-workflow-categories.md).
- Reviewing secret reveal events in the security log: [Provenance vs audit](provenance-vs-audit.md).
