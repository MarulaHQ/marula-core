---
description: "Installing a plugin or external connector in an MSP platform is a high-trust security decision."
---

# How to configure the plugin consent flow

> **Audience:** admin — plugin installation, consent, and cap grants are
> settings-owner security decisions.

Installing a plugin or external connector in an MSP platform is a high-trust security decision. In Marula, plugins never run code on arrival: every package must pass a cryptographic digest check, land in a **Disabled** state first, and wait for an administrator to review and approve its exact list of requested permissions under **Settings → Plugins**.

## Prerequisites

- Admin access to **Settings → Plugins** (*"Plugins & agent skills"*).

## How the plugin safety gate works

> "Nothing executes that did not arrive digest-verified with provenance
> and pass admin consent. Installs start DISABLED; updates pin the old
> version until the consent diff is approved."

Every plugin or connector follows the same path: **digest-verified import → installed as Disabled → admin reviews permission diff → enabled**. When a plugin releases a new version later, Marula keeps running your current approved version until an admin inspects and approves the update's permission diff.

## 1. Choose a verified plugin source

Open **Settings → Plugins**:

- **Registry tab:** Browse and install signed plugins from the marketplace catalog.
- **GitHub release tag or local `.zip` upload:** Import audited internal plugins. Every source records an append-only provenance chain (`source_kind`, `source_url`, `fetched_ref`, `artifact_digest`, and `uploader`).

A GitHub import always shows **manifest unsigned** on its consent screen. That is deliberate: the pinned tag and the SHA-256 digest record which bytes were fetched and that they have not changed, but a repository carries no publisher signature over its manifest, so there is no publisher claim to show. The [plugin runtime reference](../../reference/plugin-runtime.md#manifest-authentication-by-source) lists which sources can show **manifest signed**.

## 2. Newly installed plugins start Disabled

When you import or install a plugin, it lands in a **Disabled** state (or **Staged** if uploaded by a non-admin team member). A disabled plugin cannot run background jobs, receive webhooks, or display widgets until an administrator explicitly consents and enables it.

(In the preloaded demo org, `sample-stock-ticker@1.0.0` is pre-enabled so you can inspect a live consent record right away.)

## 3. Review the permission diff before enabling

Open the **Approvals** tab (or click the pending install) to inspect the **consent screen**. It lists everything the plugin asks for:

- Requested capabilities (such as `ui.widgets`, `automation.actions`, `automation.triggers`, or `automation.reads`)
- Outbound webhook endpoints that will be seeded on enable
- Declared UI widgets, automation nodes, or custom field validators
- Configuration settings schema

Approve the consent diff to enable that exact version digest. If an update arrives later requesting new capabilities or endpoints, your workspace stays pinned to the previously approved version until you review and approve the new diff.

## 4. Manage installed tabs

**Settings → Plugins** organizes extensions into clear tabs:

- **WASM** (sandboxed in-process plugins)
- **HTTP connectors** (external RMM, documentation, and accounting integrations)
- **Agent skills** (versioned prompt and rule playbooks)
- **Registry** (marketplace catalog)
- **Approvals** (staged installs and updates waiting for admin consent)

## 5. Disable, revoke, or uninstall at any time

Disabling or uninstalling a plugin immediately revokes its capability grants and tears down any outbound webhook subscriptions it seeded, while leaving your organization's core tickets and assets intact. To see what UI and automation surfaces a plugin can register, read [How to extend Marula with plugin surfaces](admin-plugin-surfaces.md).

## What you learned

- Every plugin starts **Disabled** and requires explicit admin consent over its capability list and SHA-256 digest before running.
- Plugin updates never auto-escalate permissions; the previous version stays pinned until you approve the update diff.
- For managing AI assistant permissions across roles, see [How to manage AI access](admin-ai-access.md).
