---
description: "Marula keeps your organization's records in its own database. When you need a copy outside the app, for a migration, a compliance request, or a look at the…"
---

# How to export your data

> **Audience:** admin — exports are owner/admin-only, and every run is audited.

Marula keeps your organization's records in its own database. When you need a copy outside the app, for a migration, a compliance request, or a look at the raw rows, the export surface builds one and hands you a link.

## Build an export

1. Open **Settings → Export**.
2. Press **Generate export**.
3. Marula gathers the org's records into one versioned JSON bundle, stores it, and returns a signed download link.
4. Press **Download bundle** to save the file.

The export runs on the server for the whole org and writes an entry to the audit log.

## The signed link

> [!IMPORTANT]
> A live download link is a bearer secret: anyone holding it can fetch the
> bundle for 15 minutes. Do not paste it into a ticket or a chat.

The download link is self-contained. Anyone holding it can fetch the bundle for 15 minutes without signing in, and then it expires. Treat a live link like a secret, because that is the whole authorization story.

The org id, the export id, and the expiry are all inside the signature, so a link can only ever reach the one bundle it was minted for. An expired or edited link is refused, and the refusal says nothing about which of the two it was.

## The ledger and fresh links

Every bundle you build lands in the **History** list with its size, SHA-256 digest, and bundle version. That list keeps facts, never links. Press **Download** on a row and Marula mints a fresh 15-minute link for the bundle behind it. **Archive** hides a row from the list without deleting the stored bundle.

Check the SHA-256 after a download when the copy matters. The digest is how you know the file arrived the way it left.

## What lands in the bundle

The export is one JSON document with a `bundle_version` field at the top, so the shape of a later release stays readable to a tool that was written against an earlier one. It covers the org's tickets and their comments, time entries, invoices and payments, clients and contacts, the taxonomy (boards, categories, tags), and the org's own settings.

Document attachments point at object storage, not at their bytes. If you also need the uploaded files, take them from the bucket, and read [Backup and restore](../reference/backup-restore.md) for how the two stores fit together.

## When the export button is missing

Export is limited to owners and admins. If you are signed in as a technician or another member, the panel explains that the surface is admin-only and points you at an admin who can run it.

If your org is showing a suspension notice in the app, that notice is information, not a lock: the notice tells you what is happening and why, while exports, tickets, and the rest of Marula stay open. Pull your data out whenever you want it.

## Related

- [Backup and restore](../reference/backup-restore.md) covers the operator job of backing up Postgres and object storage together. An export is a copy of your records, not a substitute for a server backup.
- [Call the API](call-the-api.md) shows the direct path: `POST /api/v1/org/exports`, then the minted link.
