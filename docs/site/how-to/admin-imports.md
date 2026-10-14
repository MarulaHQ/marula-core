# How to run a dry-run import

> **Audience:** admin — imports are an administrative data-migration
> surface; the run takes commit-level intent.

Moving clients, contacts, tickets, or hardware assets from a legacy PSA or spreadsheet shouldn't be a blind leap of faith. In Marula, every import runs a **dry-run preview** first so you can inspect exactly what will be created, skipped, or merged before a single row is written to your database. Column mappings save per source system so your next import from the same tool maps itself automatically.

## Prerequisites

- Admin access to **Imports** (`/imports`).
- A CSV or JSON export from your previous tool. CSV files must include a **header row** so you can map columns in step 2.
- Your desired conflict rule (the default is `skip-existing` so existing records are not overwritten).

## The three-step import habit

> "dry-run first (nothing is written), then commit as a batch. Field
> mappings save per source system so the next import maps itself."

1. **Dry-run first:** Preview every row and validation warning before committing.
2. **Save your column mapping:** Name the source system (for example, `ConnectWise` or `Syncro`) once and reuse the saved template on future batches.
3. **Trace imported rows anytime:** Every imported record carries an `import` actor badge in [Provenance](../explanation/provenance-vs-audit.md) so you can always filter by what came from a batch.

## 1. Choose the record type and source system

On **Imports** (`/imports`), complete step 1 of the guided flow:

- Pick the **entity** you are importing (clients, contacts, devices, tickets, Knowledge Base articles, etc.).
- Choose the **conflict policy** (`skip-existing` by default).
- Enter the **source system** name and pick an existing **mapping template** if you have imported from this tool before.

## 2. Upload your file and map columns

Upload or paste your CSV/JSON file (including the header row). Match each column from your file to the corresponding Marula field. When you save, Marula stores this mapping under the source system name you chose in step 1.

## 3. Run the dry-run preview, then commit

1. Click **Dry-run**. Marula validates every row and shows a summary of records to create, records to skip, and any formatting errors—without writing anything to the database.
2. Once the preview looks clean, click **Commit** to write the batch in one transaction. The run history panel keeps a permanent record of every batch.

## 4. Where imported records appear

- **Provenance (`/provenance`):** Every field populated by an import is tagged with the `import` actor chip (alongside `human` and `agent`), making it easy to audit when and how a record arrived.
- **Inventory (`/devices`):** Imported hardware passes through the identity matcher and merge queue so overlapping serial numbers or MAC addresses don't create duplicate devices ([see Your first inventory sync](../tutorials/first-inventory-sync.md)).

## What you learned

- Nothing is written to the database until you review the **dry-run** preview and click **Commit**.
- Column mappings are saved per source system so repeat imports take seconds.
- Every imported record keeps its batch provenance and `import` actor tag.
