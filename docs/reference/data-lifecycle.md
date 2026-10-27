# Data lifecycle: retention, object integrity and tenant erasure

> **Audience:** whoever operates or modifies a Marula instance. Three questions
> live here: what this instance keeps and for how long, what happens to bytes
> that stop matching their rows, and what "delete this tenant" means for each
> surface.

The short answer to "how long is data kept" used to be a comment in a migration
and a promise in two modules. `outbox_events` and `workflow_runs` had real
sweeps. `audit_events`, `notifications`, the email and webhook ledgers, the auth
attempt log and the two dead-work ledgers had none. One of those promises was
already false: migration 0088 said the attempt log was "purged opportunistically",
and it was, by a global delete executed inside the login-failure function — a
write on the unauthenticated path that an attacker controls the rate of.

The matrix below is the answer now. `crates/core-db/src/retention.rs` is its
source of truth. This page explains the numbers, the mechanisms and how to change
them.

## 1. The matrix

Windows are days. `compact` is the smallest install, `standard` ships,
`extended` is for a regulated MSP (a seven-year audit obligation is normal in
that trade).

| Table | compact | standard | extended | Floor | Mechanism |
|---|---|---|---|---|---|
| `jobs` | 7 | 30 | 90 | 7 | batched delete |
| `dead_events` | 30 | 90 | 180 | 30 | batched delete |
| `consumer_deliveries` | 14 | 30 | 60 | 7 | batched delete |
| `notifications` | 60 | 180 | 365 | 30 | batched delete |
| `webhook_deliveries` | 14 | 30 | 90 | 7 | batched delete |
| `email_outbound` | 30 | 60 | 180 | 14 | batched delete |
| `email_messages` | 180 | 365 | 730 | 90 | batched delete |
| `auth_attempt_log` | 14 | 30 | 90 | 7 | partition drop |
| `audit_events` | 400 | 730 | 2555 | 365 | batched delete |

Two tables that already had sweeps keep their own windows and are not in the
matrix: `workflow_runs` (per-org setting, 30 days by default) and
`outbox_events` (7 days, and its window doubles as the SSE replay floor).

A partition-drop window is enforced at partition granularity. `auth_attempt_log`
is partitioned by month, so a partition is dropped once its last day is older
than the window: with the 30-day standard window the oldest attempt row is
somewhere between 30 and 61 days old. Retention is an upper bound here, never a
guarantee of the exact day. Every other row in the matrix is a delete with a
day-precision predicate.

### What a window actually permits

Age is only half of every predicate. A row still owed work is never deleted,
whatever its age:

| Table | Eligible when |
|---|---|
| `jobs` | status is `done`, `failed` or `dead` |
| `email_outbound` | status is `sent` or `dead` |
| `consumer_deliveries` | settled or dead — never a row awaiting another attempt |
| `webhook_deliveries` | any age past the window. This is an attempt log, and the retrying surface is the `jobs` row |
| `notifications`, `email_messages`, `audit_events`, `dead_events` | age only |

Retention also bounds the dead-work list: a dead job leaves `ops.dead_work` when
its row is pruned, which is 30 days on the standard profile. The cancellation
ledger outlives it, because `dead_work_cancellations` deliberately has no foreign
key to the ledgers it points at, so the decision to cancel a dead item survives
the item.

Deleting an `email_messages` row cascades its `attachments` ROWS. The bytes stay
until the integrity pass reclaims them (§3). That is the one place where a
retention window creates work for another subsystem, and it is why both shipped
in the same release.

### The three profiles

`MARULA_RETENTION_PROFILE` takes `compact`, `standard` or `extended`. An unknown
value keeps `standard` and logs a warning. One table can be moved on its own:
`MARULA_RETENTION_<TABLE>_DAYS` (`MARULA_RETENTION_AUDIT_EVENTS_DAYS=3650`).

A floor wins over every other input, including an explicit override. A value
below the floor is refused with a warning naming both numbers, not clamped
silently — "we asked for 30 days of audit retention and kept a year" belongs on
the first line of the boot log, not in a discovery a year later.

The resolved matrix is logged once at worker boot.

## 2. Why a partition for one table and batched deletes for the rest

`auth_attempt_log` is the only table in the matrix that earns a range partition.
The properties that make one worth its keep are all present: append-only, ordered
by one timestamp, no foreign key pointing at it, no view or materialised view
reading it, a short window, and a primary key that can carry the partition column
as a suffix without changing what it guarantees.

Expiry for that table is now `DROP TABLE` on a month. That is metadata work, and
it removed a defect rather than adding a mechanism: the global delete that used to
run inside `app_auth_gate_fail` executed once per failed password, on the
unauthenticated path, against a growing table. The function keeps its signature
and its counters and lost the delete. The window is thirty days now, instead of
the one day a comment chose.

Everything else has a property that makes partitioning the wrong tool:

* `jobs`, `email_outbound` and `webhook_deliveries` are UPDATE-heavy state
  machines. Their eligibility lives in a status column, not in the partition key.
* `notifications` and `email_messages` are read by user or ticket and pruned by an
  age predicate that is not the leading column of their indexes.
* `consumer_deliveries` has a composite primary key on identity columns.
* `dead_events` carries a unique index on `(consumer_group, entry_id)` that would
  have to include the partition key, and that shape is a contract another lane
  set deliberately.

### The `audit_events` verdict, and the trigger to revisit

Partitioning `audit_events` would eventually pay. It is deferred, and the reasons
are specific rather than cosmetic:

* The conversion is not `CREATE TABLE … PARTITION BY`. It is an exclusive rewrite
  of the table, and it also forces a recreate of `org_usage`, the read model
  migration 0131 builds over this table — a view whose definition a partition
  rename would otherwise break.
* A two-year window deletes the oldest 1/24th of the table. On a fresh
  `(occurred_at)` index that is a bounded range scan in the low seconds per pass,
  not the forever-scan the table had before this release.

**Trigger to revisit:** partition `audit_events` when any of these hold. The
table passes 100 million rows. A single retention pass reports that it hit its
batch ceiling three passes in a row. Or the audit export surface lands: the
export reads by time range across tenants, and a partition is the natural pruning
unit for it. Until then, the batched delete is measured, not assumed: the sweep logs
the rows it removed per table.

### The trail is append-only in the database

Until migration 0168 the word "append-only" described a habit: the application
role held UPDATE and DELETE on `audit_events`, and row-level security only
decided which org's rows a statement could reach. A single `UPDATE`, from any
path, would have edited evidence without a trace.

Three layers now hold it:

* The application role keeps SELECT and INSERT and loses UPDATE and DELETE.
* A trigger refuses UPDATE, DELETE and TRUNCATE for every caller, the table owner
  included. A later `GRANT` cannot restore mutability, because the trigger runs
  after the privilege check.
* Three deletes stay sanctioned, each explicit. Retention calls
  `purge_audit_events()`, a SECURITY DEFINER function that enforces this
  table's 365-day floor and a batch ceiling. A tenant erasure deletes the `orgs`
  row and the foreign key cascades. `seed-demo --force` sets the dev-reset
  bypass from migration 0119.

The sweep calls that function instead of issuing its own `DELETE`, so the pass
still logs a real count. The floor lives in two places on purpose: the Rust
matrix decides the window, and the function refuses a caller that asks for less
than a year.

## 3. Object store: orphans, missing bytes, and the guard that matters

Rows and bytes are two systems with no shared transaction, and both directions of
drift are silent.

**Orphans** are bytes no row references. An upload wrote the object and failed to
commit its row. A row was deleted while the store delete failed. A restore brought
back a dump older than the object snapshot. Nothing used to reclaim them.

**Missing bytes** — a row whose object is gone. The row kept advertising a
download that failed as an internal error, which reads as a bug rather than as
missing storage.

`MARULA_OBJECT_GC` runs a pass that closes both:

| Setting | Default | Meaning |
|---|---|---|
| `MARULA_OBJECT_GC` | `off` | `off`, `dry_run` (report only) or `on` |
| `MARULA_OBJECT_GC_GRACE_HOURS` | `24` | How old an unreferenced object must be before it is deletable |
| `MARULA_OBJECT_GC_SECS` | `3600` | Pass interval |
| `OBJECT_STORE`, `OBJECT_STORE_LOCAL_DIR`, `S3_*` | shared with core-api | Which store the pass walks |

The pass is **off by default**, and the value that turns deletion on has to be
spelled exactly: an unrecognised value stays `off` and logs why.

Three properties make it safe to run:

1. **An empty store aborts the pass.** If the store lists zero objects while rows
   reference objects, the pass refuses to run and says the store is probably not
   mounted. This is the most likely production mistake: a worker container with no
   objects volume. Without the guard it would mark every attachment as missing,
   which looks exactly like data loss.
2. **The grace window is real time, from the store.** An object younger than the
   grace window is never deleted: an upload writes bytes and then commits its row,
   and a pass that deleted on "unreferenced right now" would race every upload in
   flight. A backend that cannot report an object's age cannot have its objects
   deleted.
3. **Deletion is recorded before it happens.** Each pass writes one
   `object_integrity_findings` row per reclaimed key and one per missing key,
   grouped by `pass_id`. A pass is then reviewable as a unit, and two passes
   cannot overwrite each other's evidence.

Repair is deliberately narrow. A missing object marks
`attachments.missing_since` / `documents_attachments.missing_since`, which a
download surface can report as unavailable instead of failing obscurely. The keys
are content addresses: re-uploading the same file stores the same key, and the
next pass clears the mark and closes the finding. Nothing pretends to reconstruct
bytes from nothing.

The bucket holds five key spaces, all under `org/<org uuid>/`: `attachments/<sha256>`
(ticket, comment, email and document files), `quarantine/<sha256>` (bytes an
admission gate refused), `exports/<id>/bundle.json`, `secrets/<ciphertext sha256>`
(sealed vault documents) and `plugins/<module digest>`. The pass compares the
store's listing against every row that can name one of them. That set is a single
declaration, `crates/core-db/src/object_inventory.rs`, and each reader derives its
own projection from it:

| Reader | Question it asks | Projection |
|---|---|---|
| Attachment refcount | May this row's delete reclaim the object? | owners in the attachment space |
| Orphan sweep | Is this object in the bucket garbage? | every row that names a key, in every space |
| Tenant erasure | Which keys must the caller remove? | the same set, narrowed to one tenant |

The declaration exists because the three readers ask different questions but must
agree about the world. A table can be an owner of attachment bytes, a mere claim
on them (a scan verdict, an export bundle), a claim in another space (a vault
record, a plugin module) or evidence about a key that is already gone, and each
answer is written down once with its reason.

`object_integrity_findings` holds a `storage_key` and counts for nobody: it
records what a past pass saw, so counting it would keep a deleted object's key
alive and protect a later upload at that address from ever being reclaimed.
`email_messages.raw_ref` is not a key here at all — it is a provider-side pointer
to a raw MIME payload.

Two checks keep the declaration honest. `tests/object_inventory_parity.rs` reads
the database's own catalogue for every key column (`storage_key`, `object_key`,
`module_digest`, `previous_module_digest`) and fails when a table holds one the
declaration never decided; the same suite seeds a row in every key space and
asserts exactly which keys the erasure collects. `tests/attachment_refcount.rs`
keeps the older, narrower gate for the `storage_key` columns alone.

One limit is worth stating: the catalogue scan knows those four column names. A
migration that invents a new name for a key column is invisible to it until the
name joins the list, so add the name in the same change that adds the column, and
run the pass in `dry_run` on a real bucket before turning it on.

### The volume this used to be missing

The stock compose file ran core-api with a read-only root filesystem and no
volume for `OBJECT_STORE_LOCAL_DIR`, so the documented local store could not
persist an upload, and a restart emptied it. It is a named volume now
(`objectdata`). Core-api and the worker mount it writable, and the backup runner
mounts it read-only for snapshots.

## 4. The erasure contract

An erasure is one command, and this is what it means surface by surface.

```sh
marula data purge-org --org-id <uuid> --dry-run
marula data purge-org --org-id <uuid> --confirm <label> \
  --requested-by <operator> --reason "<case reference>"
```

There is no HTTP route that deletes a tenant. Erasure is an operator decision with
a receipt, not a button.

| Surface | What happens | Notes |
|---|---|---|
| Every table with an `org_id` | Deleted by the `orgs` cascade | Includes the audit trail, the search index and the RAG vectors: those rows contain the tenant's personal data, so erasing them is the requirement, not a casualty |
| Object store | Every key under `org/<org uuid>/` is deleted | Includes orphans under the prefix, which a row-driven delete would leave behind. This covers all five key spaces, so a vault document's ciphertext and a plugin's module go with the tenant |
| Collected keys (the count) | `erasure::object_keys` names the keys the rows claim | Derived from the same declaration as the sweep, so it counts the vault and plugin spaces too. A CLIENT-scoped vault record carries a NULL `org_id`, so the collection follows its client into the org |
| Search index | Deleted by the cascade | `search_index.org_id` is a foreign key |
| RAG embeddings and deletion receipts | Deleted by the cascade | A per-source DSAR receipt inside the tenant is the tenant's, not the platform's |
| Event stream (Valkey) | **Not addressed per tenant** | Entries carry an org id but expire by stream length, not by erasure. A tenant's events can survive in the stream until it trims. The lever is `MARULA_EVENT_STREAM_MAXLEN` |
| Backups already written | **Not rewritten** | An erasure inside a live system cannot edit an archive written last week. See below |
| Metrics and logs | Not addressed here | Retention belongs to the observability profile. The alert pack owns the rules |

### Order of operations, and why

1. Read the org, count every org-scoped table, collect object keys.
2. `DELETE FROM orgs` — the cascade does the rest.
3. Delete the bytes under the org prefix.
4. Write the receipt, whatever step 3 did.

Rows first, bytes second: a crash between them leaves orphaned bytes, which the
integrity pass finds and reclaims. The reverse order would leave rows advertising
bytes that cannot be recovered.

The byte step deletes what the store lists under the org prefix, not the keys the
rows named in step 1. That is deliberate: the prefix is the tenant's whole
namespace, so an object no row claims any more still goes, and so does an
abandoned upload or a staged module whose row was refused. Step 1 is the
cross-check the operator reads before confirming: the dry run prints how many
objects the prefix holds against how many keys the rows claim, and the receipt
records what the delete actually removed. A table missing from the collection
therefore cannot leave bytes behind or free another tenant's; it makes the report
wrong, which is why the collection is derived and tested rather than typed out.

### Backups: the part no one likes

An erasure does not rewrite backups. It cannot: they are encrypted archives on
another system, and rewriting them would break the chain that makes them
restorable. The honest statement is a window and a remedy:

* A logical dump ages out of retention by the backup runner's own policy
  (`RESTIC_KEEP_DAILY` 7, `RESTIC_KEEP_WEEKLY` 4, `WALG_RETAIN_FULL` 4 by
  default). Until it does, an erased tenant can reappear in a restore.
* If a restore reintroduces an erased tenant, erase it again — the receipt is what
  proves the second erasure, and it records the original too.
* An operator who needs a shorter window can shorten the backup retention, at the
  cost of the restore points that window buys.

### The receipt

`org_erasure_receipts` records the org id and label, who asked, when, the per-table
row counts, how many objects were deleted or failed, and an outcome:
`completed`, or `objects_incomplete` with a count. It has no foreign key to `orgs`
because the row it names is gone by definition. Its tenant column is called
`purged_org_id` for the same reason. There is no application surface that reads
it: an operator reads it with `psql`, and it exists so that "we deleted that
tenant" is a fact with a timestamp rather than a recollection.

The verb refuses to run on the RLS-constrained application role. The cascade on
that role would delete nothing and report success.

## 5. Operating it

The retention matrix and the partition maintainer run in the worker's
housekeeping role:

| Setting | Default | Meaning |
|---|---|---|
| `MARULA_RETENTION_PROFILE` | `standard` | Which column of the matrix |
| `MARULA_RETENTION_<TABLE>_DAYS` | profile value | One table, clamped by its floor |
| `MARULA_RETENTION_SWEEP_SECS` | `3600` | Pass interval |
| `MARULA_RETENTION_BATCH` | `1000` | Rows per delete statement |

`/healthz` reports `retention_matrix` (partitions plus batched deletes) and
`object_gc`. The object pass reports `disabled` when it is off rather than dead. A
retention pass that hits a table's batch ceiling pauses thirty seconds instead of
an hour, so a backlog drains instead of trickling.

Useful queries:

```sql
-- what is still missing, by org
SELECT org_id, count(*), min(found_at)
  FROM object_integrity_findings
 WHERE kind = 'bytes_missing' AND resolved_at IS NULL
 GROUP BY org_id ORDER BY 2 DESC;

-- what the last pass reclaimed
SELECT pass_id, count(*), sum(size_bytes)
  FROM object_integrity_findings
 WHERE kind = 'orphan_deleted'
 GROUP BY pass_id ORDER BY max(found_at) DESC LIMIT 5;
```

## 6. What is not here

* **No per-tenant windows.** A window is a platform policy. A tenant-level setting
  would put one customer's retention decision into another's storage bill, and
  into the erasure contract.
* **No archive tier.** A row past its window is deleted, not moved somewhere else.
  An archive is a different promise and a different bill.
* **No legal hold.** An org that must be preserved beyond its window needs its
  profile raised (`extended`, or an override) before the pass reaches it.
* **No `audit_events` partition yet.** §2 has the trigger.
* **No erasure of the event stream or of written backups.** §4 says why.
