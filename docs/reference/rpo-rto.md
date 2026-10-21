# RPO and RTO, per deploy profile

> **Audience:** whoever promises a customer a recovery number, and whoever has to
> meet it at 03:00. This page states what each shipped deployment shape actually
> delivers, how the numbers were derived, and which of them are still a model
> rather than a measurement.

Two definitions, used strictly:

* **RPO** — how much committed work can be lost. It is a property of the backup
  layer: how often a restore point exists and how far behind the newest one is.
* **RTO** — how long from "we are down" to "the product serves the last restore
  point". It is a property of the restore path: how big the artifact is, how fast
  this host writes it back, and how much of the path is manual.

Neither is a property of a retry preset. `MARULA_ROBUSTNESS` chooses how hard the
product retries work (`balanced`, `durable`, `single_box`, `best_effort`), and the
three names below are deployment shapes that happen to line up with those presets.
The numbers come from `deploy/backup` and the drill, not from the retry matrix.

## 1. What is backed up

| Surface | Layer | Where it goes |
|---|---|---|
| Postgres | `pg_dump -Fc` nightly at `BACKUP_AT` (03:00 UTC), verified with `pg_restore --list` | `backupdumps` volume |
| Postgres, continuous | WAL-G `backup-push` nightly plus `wal-push` per segment, when `WALG_S3_PREFIX` is set | S3-compatible bucket |
| Object store | `restic backup` of `BACKUP_SOURCES` (the `objectdata` volume and `/pgdumps`) in the same pass | restic repository |
| Anything else | Nothing is left | Attachments, documents, exports and signatures are all objects or rows |

The object snapshot and the database dump are taken in the same pass but not in the
same transaction. A few seconds of drift is possible: a row can reference an
object the snapshot missed. That direction is the recoverable one — the integrity
pass marks the row as missing bytes and a re-upload of the same content address
repairs it. The opposite drift (bytes with no row) is an orphan, and the same pass
reclaims it.

## 2. RPO per profile

| Profile | Shipped backup layer | RPO | What moves it |
|---|---|---|---|
| `single_box` | Nightly dump + restic snapshot, no WAL archiving | **24 h** | Nothing, short of enabling WAL-G. This is the shape most small installs run, and the number to quote is a day |
| `balanced` | The same nightly pass, WAL-G available as an override | **24 h** shipped, and **≤ 5 min** with `deploy/backup/compose.walg.example.yml` | `archive_timeout` (60 s in the shipped override), plus upload latency and the retry window after a failed push |
| `durable` | WAL-G + restic, backup repository copied off-host, longer retention | **≤ 5 min**, and the number is verified weekly | The same 60 s timeout. The difference from `balanced` is that the repository is off-host, retention is longer, and the drill is part of the default cadence |

How the PITR number is derived, because "point in time" is often quoted as if it
were zero:

1. A WAL segment is archived when it fills **or** when `archive_timeout` expires.
   The stock override sets 60 s; Postgres's default is 0, which means "only when
   it fills" — on a quiet install that is hours, and the real RPO of an
   unconfigured PITR tier is "however long 16 MB of writes takes". That is why the
   override carries the setting and this page quotes one minute, not zero.
2. A failed `archive_command` retries, and the segments queue in `pg_wal`. The RPO
   then becomes the age of the OLDEST unarchived segment, which is exactly what
   `pg_stat_archiver.oldest_wal_segment` reports. A silently broken archive pipe
   is the classic way to believe in a five-minute RPO that is really four hours.
3. Five minutes, not one, is the number in the table: it allows for one failed
   push, its retry, and the segment that was in flight.

**The signal that guards the RPO** is the backup stamp the runner writes
(`$PGDUMPS_DIR/.last_success_epoch`) and the worker turns into
`marula.backup.age_seconds`. A pass with any failed layer does not advance the
stamp, so a broken backup reads as a growing age rather than a fresh success. The
stock compose now mounts the stamp path into the worker; before that the gauge was
code that never ran in the default profile.

## 3. RTO per profile

RTO is a sum, not a single skill:

```
RTO ≈ detect + decide + fetch + restore + verify + route
       ~0        ~5 min  (a)    (b)      5 min    (c)
```

| Term | Value used here | Where it comes from |
|---|---|---|
| detect | 0 | An operator usually knows first. The backup-age gauge is the backstop, not the trigger |
| decide | 5 min | Human: confirm the failure, pick the restore point, tell the customer |
| (a) fetch | `repository size ÷ 50 MB/s` | Assumes an in-region S3 read. A cross-region or cold archive changes it by an order of magnitude |
| (b) restore | `dump size ÷ 40 MB/s` + `object bytes ÷ 100 MB/s` | **An assumption.** `restore.sh` restores single-threaded into a scratch database. The drill measures the real number and records it |
| verify | 5 min | The drift check below: an open ticket renders, today's time entries exist, the audit trail is intact |
| (c) route | 2 min (`balanced`, `durable`), 0 (`single_box`, same host) | A DNS or load-balancer change, or nothing |

Worked numbers, using (b) at the assumed rates:

| Dump size | Postgres restore | + objects (1 GB) | RTO, `single_box` | RTO, `durable` |
|---|---|---|---|---|
| 1 GB | 25 s | 10 s | ~11 min | ~13 min |
| 5 GB | 2 min | 10 s | ~14 min | ~16 min |
| 20 GB | 8.5 min | 10 s | ~20 min | ~22 min |
| 100 GB | 42 min | 10 s | ~54 min | ~56 min |

Read the table as a planning tool, not a promise. Two of its three terms are
assumptions, and the third (detect, decide) is a human.

### The drill is how the model becomes a measurement

`deploy/backup/drill.sh` runs every `DRILL_EVERY_DAYS` (default 7) from the same
container as the nightly pass:

1. Pick the newest local dump (or the newest WAL-G base backup when no dump
   exists).
2. Restore it into the scratch database through `restore.sh`, which asserts the
   schema landed and RLS is enabled on every public table.
3. Assert the restored database carries data: migration history present, `orgs`,
   `users`, `tickets` and `audit_events` counts, all non-zero where the source had
   rows.
4. Record one row in `ops.backup_drill_runs`: outcome, which artifact, the row
   counts, and **the wall-clock duration**.

That duration is the term (b) above, measured on this hardware. An operator who
needs to quote an RTO should quote the drill's number for their largest recent
dump rather than the table.

```sql
-- the last ten drills, newest first
SELECT started_at, outcome, source, artifact, duration_ms
  FROM ops.backup_drill_runs ORDER BY started_at DESC LIMIT 10;

-- how stale is the proof?
SELECT now() - max(started_at) AS since_last_drill,
       count(*) FILTER (WHERE outcome = 'failed') AS failures_last_30d
  FROM ops.backup_drill_runs
 WHERE started_at > now() - interval '30 days';
```

### What the drill proves, and what it does not

| Proves | Does not prove |
|---|---|
| The dump is restorable end to end: `pg_restore` reads it, the schema lands, RLS is on every org-scoped table, and the restored rows are the product's data | That the restore finishes inside an RTO you promised a customer — unless you read the duration |
| The nightly path works with the backups **this host holds** | That the off-host copy is intact. Restic and WAL-G retrievability are separate checks |
| For WAL-G, that the base backup is **retrievable**: `backup-fetch` succeeds and the payload carries a `PG_VERSION` | That Postgres **starts** on it. Starting a server on a fetched base backup plus WAL replay needs a second instance, which the backup container does not have. A full PITR rehearsal is an incident-level exercise, documented in [backup and restore](../site/reference/backup-restore.md) |
| Object-store snapshots exist (restic `backup` succeeded in the pass) | That an object restore lands byte-identical. The verification for that is a restore into a scratch directory plus `restic check`, run by hand |

A drill failure does not block the backup stamp. The backup may be perfectly fine
and the drill may have failed for its own reason — no scratch space, a lock. Both
signals are kept separate so that one does not explain away the other, and both
are recorded.

## 4. The verification ladder

Each rung is cheaper than the next and catches a different failure:

| Rung | When | Catches | Cost |
|---|---|---|---|
| `pg_restore --list` on the new dump | every pass | a truncated or unreadable archive — `pg_dump` exiting 0 on a half-written file | milliseconds |
| Restore drill | weekly by default | a schema that will not restore, a dump that lost its data, an RTO that is a fiction | minutes, into a scratch database |
| Full PITR rehearsal | before trusting a `durable` claim, and after any Postgres upgrade | a WAL chain that will not replay | an hour, on a separate instance |
| `restic check` + a scratch object restore | monthly, by hand | a repository that has lost a pack | depends on repository size |

The first two are automated. The last two are not, and this page says so rather
than implying a machine does them.

## 5. Failure responses

| Symptom | Reading | Response |
|---|---|---|
| `marula.backup.age_seconds` growing past 2× the cadence | No pass has completed since | Read the runner log: a failed layer is named. Fix the layer, and the stamp follows |
| A drill in `ops.backup_drill_runs` with `outcome = 'failed'` | The backup this pass produced did not restore | Do not trust that artifact. Read `detail`, fix, and run `drill.sh` by hand before the next scheduled pass |
| `pg_stat_archiver.oldest_wal_segment` age growing | WAL is not shipping; the PITR RPO is now that age | Check the archive command and the bucket credentials. Segments stay in `pg_wal` until they ship, which is why this is recoverable |
| `object_integrity_findings` with open `bytes_missing` rows | Rows reference bytes the store does not have | Restore the objects from the restic snapshot. A matching content address clears the mark on the next pass |
| Restore succeeds but the product is empty | The dump is older than the tenant, or the wrong database | Check `_sqlx_migrations` and the org count in the restored copy before pointing the product at it |

## 6. Honest gaps

* The 40 MB/s restore figure and the 50 MB/s repository read are assumptions until
  a drill lands on hardware with a large enough dump. The drill exists to replace
  them.
* A PITR *rehearsal* is not automated. The drill proves the base backup is
  retrievable; replaying WAL to a recovery target needs a second instance.
* Off-host copies of the restic repository are a deployment obligation, not
  something this repository configures. `durable` assumes one exists.
* The alert rules over `ops.backup_drill_runs` and the backup-age gauge belong to
  the alert pack. This page gives the queries, and nothing pages anyone yet.
