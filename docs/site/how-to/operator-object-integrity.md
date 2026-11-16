---
description: "The object-integrity pass compares rows with stored bytes. Here is how to read its findings, what each direction means, and why there is no repair button."
---

# How to check that records and stored files still agree

> **Audience:** admin — the reader is limited to roles that can read the admin
> module.

**Settings → Object integrity** shows what the periodic object-integrity pass
found between the database and the object store. It answers one question in two
clauses: do the rows and the bytes still agree, and if not, which direction is
wrong.

## Prerequisites

- The admin module's read permission.
- The pass has run at least once. A fresh install can show nothing because the
  pass has not run yet, which is a different state from a clean result.
- For a missing-bytes finding, the source record the finding names. Repair is
  an upload to that record, not an action on this page.

## 1. Read the three counts

The top row gives you the shape before you open a row:

| Count | What it means |
|---|---|
| **Open findings** | Everything still to be explained or repaired. A zero reads **rows and bytes agree** |
| **Orphans reclaimed** | Bytes that nothing referenced, which the pass deleted after the grace window |
| **Missing bytes** | Rows that claim bytes the store does not have |

An orphan is usually the less urgent direction: it means a delete removed the
row and the grace window has passed. Missing bytes are the direction that
loses something a person can see, such as an attachment or a document.

## 2. Understand the two directions

Every finding is one of two kinds, and the page says which in words:

- **orphan deleted** — bytes nothing referenced, which the pass reclaimed. The
  finding records that it happened, with the storage key and size, so the
  history is auditable. It is not asking you to act.
- **bytes missing** — a row claims bytes the store does not have. This is the
  one to investigate: the row is real, a person can open it, and the file is
  gone.

A finding carries the pass id, when it was found, the storage key, the size if
known, the source table and a note. That is enough to match it to a restore or
an export without querying the database.

## 3. Filter, and include what is resolved

Filters keep the list readable:

- **Restrict to one finding direction** — orphans or missing bytes. During a
  restore triage this is the useful view: the same restore can produce one of
  each.
- **Include resolved** — off by default. Turn it on when you are checking
  whether a repair closed the finding.

Resolved rows keep their resolution time, so "we fixed it" is a fact on the
record rather than a memory.

## 4. Repair from the record, not from this page

There is no repair button, on purpose. The pass is the writer, and repairing a
finding is not a click:

- A **missing** object clears when the bytes are stored again under the same
  content address. The attachment's `missing_since` clears, and the finding's
  resolution follows.
- An **orphan** is already gone. The finding is the receipt, not a task.

So the path is: open the source record the finding names, upload the file
again, then return here with **Include resolved** on to confirm it closed. If
the bytes came from a backup, restore them first. The page will not pretend an
upload happened.

## 5. Pair the page with backup and restore

Object integrity is the check that tells you whether a restore was complete.
Run it after a restore, not only on a schedule:

1. Restore the database and the object store from the same point in time.
2. Let the pass run, or open the page after it has.
3. Read the missing-bytes count. A non-zero count means the two halves of the
   restore do not match. Find the rows named and re-upload from the backup.
4. Read the orphans. A burst of orphans after a restore usually means the
   object store and the database are from different points in time.

## What you learned

- The page reads the pass's findings. It does not run the pass.
- Orphans are reclaimed bytes with a receipt. Missing bytes are rows whose file
  is gone.
- Findings name the storage key, source table and pass, so they can be matched
  to a restore.
- Repair is an upload to the source record, and the finding resolves when the
  object exists again.
- A restore is not complete until the missing-bytes count is zero.

## Where to go next

- [Back up and restore](../reference/backup-restore.md) — the procedure this
  page verifies.
- [Recover work that gave up](operator-dead-work.md) — the other recovery
  reader.
- [Database observability](../reference/db-observability.md) — the query-side
  view of the same instance.
- [Watch your self-hosted services](admin-self-host-services.md) — the worker
  that runs the pass.
