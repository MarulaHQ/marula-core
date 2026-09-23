# Tutorial: your first inventory sync

> **Audience:** both — admins configure the connector; staff and admins
> both read the device inventory.

## Prerequisites

- A running stack with a signed-in account
  ([first ticket lesson](first-ticket.md) covers sign-in) on the **seeded
  demo org**.
- At least one connector available to the org under Settings→Integrations
  (a connector without a deployment renders the honest "install one"
  state on the asset page's right rail).

Inventory in Meridian is a **merge queue**, not a dumb import: records
arrive from connectors, get deduplicated by identity matching, and merge
into single queryable config items. Merged records stay queryable and
carry their source lineage. This lesson walks one record from a connector
to a client-linked asset.

## 1. Connect the source

Sign in and open **Settings→Integrations**. Connector availability lives
here alongside the signed webhooks, while machine API keys live on the
[API keys page](../how-to/admin-api-keys.md). The connected connector
begins writing to sync queues, and the merge queue processes them.

## 2. Read the merge queue

Open **Inventory** (/devices). The top of the page is the merge queue —
a pending count with a note about the identity matcher's copy. Beneath
it sits the config-items table (tags, status pills, identity-key callout on the
rows, with a special "merged" badge). Nothing here is a live view of your
tools — connector link-state is discussed step 4.

## 3. From device to asset

The **/devices** page is the full inventory view. Open a device and
convert it to a managed asset: asset detail's Fields panel (name, status,
site, type) is the owning surface, with Lifecycle panel (ownership,
depreciation, purchase cost and currency, salvage, rental metadata).

## 4. Link state is shown as data

On the asset detail's right rail, the **Integrations** card lists every
connector that claims to know this asset, and it is exactly what the
connector last reported. Copy on that card states the doctrine: "Link
state is shown as data; no live sync runs from this page". If we told you
otherwise, we broke our promise — the state is a reflection of what the
connector handed over on its last sync, not a run-this-second itch.

## 5. Link the asset to its client

Related items (also on the right rail): links to documents, tickets,
credentials, clients, KB articles, other assets. Add a link to the owning
client. Broken targets stay visible, and removing a link never removes
the target. Now the asset's provenance is reachable from the client record,
and so is its sync history.

## What you learned

- Connectors push, Meridian merges — the identity matcher deduplicates
  overlapping sources rather than stacking packs of rows.
- Sync state is data, surfaced honestly, never live-synced from a detail
  page.
- The [imports how-to](../how-to/admin-imports.md) covers the same
  deduplication doctrine for bulk brings (CSV/JSON) without a connector.
