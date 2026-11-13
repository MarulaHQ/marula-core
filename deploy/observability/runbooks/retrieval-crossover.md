# Retrieval crossover: a tenant outgrew the exact scan

**Tickets:** `MarulaRetrievalCrossover`

## What the alert means

One tenant holds at least 20,000 embeddings. That is the count at which the
exact sequential scan alone costs more than the 50 ms search leg, measured at
four cores with the default embedding model, so search still returns the true
nearest neighbours but spends the leg budget getting them. The number is the
product's own predicate (`INDEX_THRESHOLD_EMBEDDINGS` in
`crates/core-domain/src/retrieval.rs`), the same one that marks a row in
`marula deploy describe`.

Nothing is broken. The mode that fits the install is a decision, and the alert
exists so the decision is made before the tenant's searches get slow rather than
after.

## First three checks

1. Read the distribution, not the alert's single number. The mean over tenants
   hides the one that crossed, which is the whole decision:

   ```sh
   marula deploy describe --json | jq '.retrieval.distribution'
   ```

   The report carries `threshold_embeddings`, every tenant's count and
   `index_recommended`, and a recommendation naming the mode. Copy it into the
   change record; it is also what the next person will read when the count moves
   again.

2. Find out which modes this database can actually run. The descriptor probes
   pgvector, the `vchord` extension and the HNSW index when it is given a
   database, and reports `probe: not_run` rather than claiming a mode it did not
   check:

   ```sh
   marula deploy describe --json --database-url "$DATABASE_URL" | jq '.retrieval.modes'
   ```

   `iterative` needs pgvector 0.8 or newer plus the HNSW index, `index` needs
   `vchord` in `shared_preload_libraries` plus its index. A mode whose `requires`
   the probe could not satisfy is the wrong lever, whatever the counts say.

3. Look at what the tenant's searches cost today before changing anything, so
   the change has a before. The API latency panels carry p95 per route:

   ```promql
   histogram_quantile(0.95,
     sum by (le, route) (rate(marula_http_request_duration_milliseconds_bucket{route=~"/api/v1/search.*"}[10m])))
   ```

## Escape hatch

Set the mode on the install and restart the processes that read it:

```sh
# managed Postgres with pgvector 0.8 and no VectorChord
MARULA_RETRIEVAL_MODE=iterative

# Postgres you control, with the vchord extension preloaded
MARULA_RETRIEVAL_MODE=index
```

`exact` stays correct at every size, so doing nothing is a valid answer for a
tenant that has just crossed and is not yet slow. What is not valid is leaving
the decision unmade until the leg is over budget at every tenant: the recall and
memory numbers behind each mode are in
`docs/site/how-to/operator-retrieval-mode.md`, and the gauge that produced this
ticket keeps reading until the tenant stops growing.

## Afterwards

Record the tenant, its count, the mode chosen and the p95 before and after. If
the count keeps climbing and the index is memory-bound, the mode choice is
settled and the capacity question is the one left.
