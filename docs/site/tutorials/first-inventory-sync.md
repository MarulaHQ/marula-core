# Tutorial: your first inventory sync

> **Audience:** both — admins configure the connector; staff and admins
> both read the device inventory.

When you pull computers and network gear from an RMM tool, an MDM, and a spreadsheet, most PSAs dump duplicate rows into your asset list and leave techs guessing which "NYC-FW-01" is real. Marula routes incoming hardware through a **merge queue** first. It matches serial numbers, MAC addresses, and hostnames so overlapping sources combine into one clean record while keeping a full history of which tool reported what.

This tutorial walks one synced device from a connector through the merge queue to a client-linked asset record.

## Prerequisites

- A running workspace signed in to the **seeded demo org** ([Your first ticket](first-ticket.md) covers sign-in).
- At least one connector configured under **Settings → Integrations** (if no connector is installed yet, the asset page's right column tells you clearly how to add one).

## 1. Connect your hardware source

Sign in as an admin and open **Settings → Integrations**. Here you manage RMM and documentation connectors alongside outbound webhooks (machine-to-machine tokens live on the [API keys page](../how-to/admin-api-keys.md)).

Once enabled, a connector pushes discovered devices into Marula's sync queue in the background.

## 2. Review incoming devices in the merge queue

Open **Inventory** (`/devices`). The page is split into two parts:

- **Top — Merge Queue:** Incoming device records that need a quick human check before merging, along with a summary of why the identity matcher flagged them (for example, matching serial numbers with different hostnames).
- **Bottom — Configuration Items Table:** Your active hardware list showing tags, status badges, the primary identity key on each row, and a `merged` badge when multiple sources feed the same machine.

## 3. Promote a device to a managed asset

Click any row on `/devices` to open its detail view and manage it as a tracked asset:

- **Fields panel:** Set the asset name, operational status, client site, and hardware type.
- **Lifecycle panel:** Track ownership model (client-owned, leased, or MSP-loaned), purchase cost and currency, depreciation window, salvage value, and warranty or rental dates.

## 4. Check what the connector last reported

Look at the **Integrations** card on the right side of the asset page. It lists every external connector linked to this machine and the exact values that connector sent on its most recent sync.

Notice the note on the card: *"Link state is shown as data; no live sync runs from this page."* Opening an asset page never pauses your browser to poll a third-party RMM API. You always see a fast, reliable snapshot of the last sync timestamp.

## 5. Link the asset to its client and tickets

Under **Related items** on the right rail, you can link this asset to its owning client, active support tickets, runbook articles, stored credentials, or other hardware (such as connecting a server to the rack switch it plugs into).

Add a link to the owning client. Now whenever a technician opens a ticket for that client (or scans the asset's barcode in the field), its warranty status, sync lineage, and repair history are one click away.

## What you learned

- External connectors feed a deduplicating **merge queue** rather than creating duplicate device rows.
- Asset pages show the last recorded sync snapshot immediately without waiting on live external API calls.
- For one-time CSV or JSON hardware imports without a live connector, use the same deduplication workflow in [Import data from CSV or JSON](../how-to/admin-imports.md).
