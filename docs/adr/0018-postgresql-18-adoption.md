# ADR 0018: PostgreSQL 18 adoption — per-feature notes for new tables

**Status:** Draft (p36-infra-transition) · 2026-09-24
**Renumbered 2026-10-01.** Written as `0012-postgresql-18-adoption.md`. The p36
lane had recorded the number as provisional, in its own words "number TBD by lane;
draft", and it then collided with `0012-internal-org-asset-scope.md`, which is
Accepted and already cited by that number. So the Draft took the next free slot,
**0018**, and the Accepted ADR kept 0012. No decision content changed, only the
identifier.
**Scope:** which PG18 features new tables and the audit write-trail path
should adopt, and which stay off-limits. Doctrine: **no retrofits** —
existing tables keep their shape; these notes bind only new migrations.

## Context

The self-host stack cut to PG18 in 2026-09 (compose, helm, and the desktop
embed; see `deploy/runbooks/pg17-to-pg18.md` for the volume cut). PG18
ships four features that touch how we write schema. This ADR fixes the
adoption stance so future migrations don't each re-litigate it.

## Decisions

**1. `uuidv7()` for new table primary keys.** PG18 builds it in
(`uuidv7()`), so new tables stop minting v4 PKs. v7 is time-ordered. That
removes the B-tree write amplification and index fragmentation random v4
causes at our insert rates, and it makes `created_at`-adjacent ordering
free on the PK index itself. Applications that need per-row generation in
older Postgres (the desktop embed before its cut) can use the SQL shim —
but new schema should just declare `DEFAULT uuidv7()` and rely on 18.
Existing UUID columns stay as-is. A value-format retrofit would rewrite
every FK index for zero behavior gain.

**2. `RETURNING OLD/NEW` on the audit write-trail path.** PG18 lets a
`RETURNING` clause pick pre- and post-image columns of the row it touched.
The audit trail currently writes a second `UPDATE ... RETURNING` to
capture the before-state. That is an extra round trip per mutation and a
race window under concurrent edits on the same row. New audit writes should
emit the old and new values from the single mutation statement. This is a
write-path pattern, not a schema change — it lands with the next touch of
the audit trail code, not as a migration.

**3. `WITHOUT OVERLAPS` / `PERIOD` where a window table appears.** New
tables whose rows carry an effective time range must express the exclusion
constraint PG18's temporal syntax gives us. The old pattern was a
hand-rolled constraint over a gist index on a range column. With the
native form the database rejects overlapping assignments at write time
rather than trusting application code. Note this is for **new** window
tables only. Retrofitting an existing table rebuilds its indexes and is
explicitly out of scope.

**4. What we did not adopt.** `io_method` async I/O stays a server config
concern (it lands in compose/host tuning, not schema). Skip scans and
virtual generated columns are runtime/optimizer wins we get for free and
need no schema decision.

## Consequences

- New migrations should declare `uuidv7()` PK defaults and temporal
  constraints directly. Reviewers should reject hand-rolled exclusion
  constraints on new window tables.
- The migration suite runs green on PG18 (76/76, `core-db` db_suite,
  2026-09-23) and the perf smoke stayed inside budget on 18, so no
  feature above carries a performance caveat.
- PG17 remains supportable only through the SQL shim era. Anything that
  must run on 17 cannot use `uuidv7()` natively and should keep v4 plus
  an application-side shim until the fleet is on 18.
