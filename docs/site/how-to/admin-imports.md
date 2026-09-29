# How to run a dry-run import

> **Audience:** admin — imports are an administrative data-migration
> surface; the run takes commit-level intent.

## Prerequisites

- Admin access to **Imports** (/imports).
- A CSV/JSON export from the source system with a **header row** required
  by step 2 of the guided flow — no header, no import.
- Decide the conflict policy you want (skip-existing is the seeded
  default).

## The doctrine, in the surface's own words

> "dry-run first (nothing is written), then commit as a batch. Field
> mappings save per source system so the next import maps itself."

Three habits to carry on every import, not just the first:

1. Dry-run, look, commit.
2. Reuse the saved mapping — never re-map by hand what is already mapped.
3. Review the result in the built-in lineage: every import lands in
   provenance with its own actor kind.

## 1. Source

Step 1 of the guided flow: choose the **entity**, the **conflict policy**
(skip-existing), the **source system** label, and the **mapping
template**. Source-system matters because mappings save per source —
name it exactly as you intend to call it later.

## 2. Upload

Step 2: choose the file, drop, or paste the payload; the header row is
required. Field mappings wire source columns to Marula fields and **save
per source system** so the next import maps itself.

## 3. Dry-run, then commit

Run the dry-run pass first — nothing is written. Diff the proposal,
expect nothing surprising, then commit as a batch. The run history panel
keeps each batch with its probes.

## 4. Where the rows went and what they claim

- **Provenance** answers "who brought this in": imports are their own
  actor kind, chip-able besides `human` and `agent` rows on
  [/provenance](../explanation/provenance-vs-audit.md) — search by actor
  = import.
- **Inventory** surfaces the mergeable results and what the identity
  matcher merged; the tutorial for that leg is [first inventory
  sync](../tutorials/first-inventory-sync.md).

## What you learned

- Dry-run-then-commit is enforced, not recommended: the surface
  guarantees nothing is written until you commit.
- Saved mappings per source system make import 2+ cheap.
- Import provenance is per-row and stays queryable, so a bad batch is
  findable by its actor chips rather than by memory.
