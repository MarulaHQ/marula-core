---
description: "The search vector leg has one exact answer and two indexed shortcuts. Which one fits depends on the shape of the install, and the shapes pull in opposite…"
---

# How to choose the retrieval mode

> **Audience:** admin — this page changes how the whole install answers a
> search, and one of the three modes needs an extension on your database.

The search vector leg has one exact answer and two indexed shortcuts. Which one
fits depends on the shape of the install, and the shapes pull in opposite
directions: a box running hundreds of small tenants is fastest with no index at
all, and a per-client deployment whose tenant holds a hundred thousand
embeddings is not.

This page takes you from "which mode" to a running deployment, and covers the
refusal you get when the mode cannot run.

## 1. Decide which mode

| Mode | What it runs | Needs | When it is the answer |
|---|---|---|---|
| `exact` (default) | Sequential scan ordered by cosine distance. The true nearest neighbours. | Nothing | Many tenants on one machine, or any tenant below about 20,000 embeddings. |
| `iterative` | pgvector HNSW with `hnsw.iterative_scan=strict_order`, so the graph keeps walking until the tenant filter has enough rows. | pgvector 0.8 or newer, and the HNSW index | A tenant past ~20,000 embeddings on a managed Postgres that offers pgvector 0.8 but no VectorChord. |
| `index` | VectorChord's `vchordrq` with prefiltering: the tenant and space filters run inside the index. | the `vchord` extension, `vchord` in `shared_preload_libraries`, and the index | A per-client deployment whose tenant crossed ~20,000 embeddings, on Postgres you control. |

`exact` is the default for two reasons: it is the only mode whose result set is
the true nearest neighbours, and it is the only one that is always satisfiable.
An index does not make search more correct — it makes a large tenant's slice of
the budget smaller.

Ask the install where it stands before you change anything. With a reachable
database, `marula deploy describe` reads your tenants and tells you which ones
crossed:

```sh
marula deploy describe --json | jq '.retrieval.distribution'
```

```json
{
  "threshold_embeddings": 20000,
  "tenants": [
    { "org_id": "6d65…0002", "embeddings": 100000, "index_recommended": true },
    { "org_id": "6d65…0001", "embeddings": 2000, "index_recommended": false }
  ],
  "tenants_over_threshold": 1,
  "total_embeddings": 102000,
  "recommendation": "1 of 2 tenants are at or past 20000 embeddings (largest 100000). That is the shape `index` is for: a per-org deployment on Postgres you control. `iterative` is the managed alternative. Change the mode by hand."
}
```

The report is deliberately not a mean. A mean over two tenants hides the one
that crossed, and that tenant is the whole decision.

The same count is a gauge while the install runs:

```promql
# Embeddings stored per organization.
marula_embedding_count
```

## 2. Check that the database can run the mode

Every mode but `exact` needs something the database may not have. Ask before
you restart:

```sh
MARULA_RETRIEVAL_MODE=iterative marula deploy describe --json | jq '.retrieval.resolved'
```

```json
{
  "mode": "iterative",
  "refusal": null,
  "probe": "live",
  "pgvector_version": "0.8.6",
  "vchord_installed": false,
  "vchord_preloaded": false,
  "hnsw_index": true,
  "vchord_index": false
}
```

`refusal` is the exact sentence the boot will print if the requirement is not
met. `null` means it will start. `probe` says whether a database was actually
asked: `live` when one answered, `unreachable` when one was configured but did
not answer, and `not_run` when there was no database to ask — the descriptor
never claims a mode can run without checking.

No index exists yet on a fresh `iterative` deployment, so the first probe
refuses. Build it, then check again:

```sh
MARULA_RETRIEVAL_MODE=iterative marula migrate
MARULA_RETRIEVAL_MODE=iterative marula deploy describe --json | jq '.retrieval.resolved.refusal'
```

`marula migrate` runs as the schema owner and is the only path that builds the
index. The API builds it too when it runs migrations itself
(`RUN_MIGRATIONS=1`), from the same lever. A deployment on `exact` builds
nothing: an HNSW index at 100,000 vectors is 1.2 GB and 135 seconds of build, and
a small-tenant box should not pay for either.

## 3. Run the mode

Put the lever beside your other deployment settings and recreate the services:

```sh
MARULA_RETRIEVAL_MODE=iterative
```

```sh
docker compose up -d core-api worker
```

For `index`, use the Postgres image that carries VectorChord:

```sh
docker compose -f docker-compose.yml -f postgres/compose.extensions.yml up -d
```

That overlay installs VectorChord 0.5.3 for PostgreSQL 18 and adds `vchord` to
`shared_preload_libraries`. Both are required: the extension alone is not
enough, because VectorChord's index cannot be maintained unless the library is
preloaded. The overlay is opt-in because its base is not the hardened image.

## 4. Read a refusal

A mode that cannot run stops the boot instead of falling back. The message names
every missing requirement, not only the first:

```
retrieval mode `index` cannot run on this database: the `vchord` extension is
not installed and this Postgres image does not offer it; `vchord` is not in
`shared_preload_libraries`, so its index cannot be maintained; the index
`search_embeddings_vchord` does not exist on `search_embeddings`. The vector leg
will not fall back to the exact scan: pick a mode this database can run, satisfy
the requirement, or set MARULA_RETRIEVAL_MODE=exact for the true nearest
neighbours
```

The absence of a fallback is the point. A silent downgrade to the scan looks to
an operator like search working badly rather than like a deployment that was
never configured for its data, and the two want opposite responses.

For `iterative`, the two common refusals are an old pgvector and a missing
index. For `index`, they are the extension, the preload and the index, in that
order. A managed instance usually cannot satisfy the preload; that is what
`iterative` is for.

## 5. Know what you are giving up

`index` returns every row at recall 1.000 in the measurements, which is why it
is the shipped index rather than the graph alternative. `iterative` is
approximate: at `ef_search=40` recall was 0.900 at 6,660 embeddings and only 2.6
of 10 rows came back at a 132-embedding tenant, which is why the mode pins
`ef_search=200` with strict ordering. At that setting it returned 10 of 10 rows
at both tenant sizes here (recall 0.96 and 1.000).

Neither indexed mode is faster at a small tenant. That is not a defect: the scan
is 0.5 ms there and the index has to walk a graph to add nothing. Pick an
indexed mode for the tenant that crossed the threshold, not for the install as a
whole.

The vector leg shares a 50 ms budget with the embedding model. At four cores the
default model spends about 47 ms of it, so the database's share is roughly 3 ms
at that profile — which is why the threshold is where it is and why a tenant at
1,500 embeddings can already be over budget even though the scan alone would fit
for 20,000. `marula deploy describe` prints that arithmetic next to every mode
instead of leaving it in a benchmark report.

One number is still open, and it is the one a large single-tenant install cares
about. The index measurements on vectors the product's own model produced stop
at 6,660 rows in one tenant. The only 100,000-embedding run in this lane used
random vectors, where the quantised VectorChord index measured 419 ms p95 and
lost to the exact scan — a number about the generator, not about your data. If
you plan to run `index` over a tenant that large, measure it on your corpus
first: `scripts/vector-ann-bench.sh --corpus real --vectors <dir>` is the
harness, and its README says what a corpus directory has to contain.

## 6. Roll back

Set the lever back and restart:

```sh
MARULA_RETRIEVAL_MODE=exact
docker compose up -d core-api worker
```

`exact` works with an index present. It turns index scans off for the duration
of the vector statement, so the planner cannot choose the index you built, and
turns them back on before the page is hydrated. You do not have to drop the
index to go back, and leaving it costs memory you can reclaim when you like
with `DROP INDEX search_embeddings_hnsw;` or
`DROP INDEX search_embeddings_vchord;`.

## 7. What not to do

**Do not set the mode per organization.** It is not a supported setting. One
table, one statement, one mode; the distribution report tells you which tenants
want a different deployment, and a different deployment is the answer.

**Do not expect `exact` to use your index.** It will not, by design. If you
built an index for a tenant and then switched the install back to `exact`, you
are paying for the index without reading it. That is allowed; it is not free.

**Do not read the mode as covering the connector RAG vectors.** It governs
`search_embeddings` only. `rag_embeddings` stays an exact scan in this release,
and the descriptor says so in `applies_to`.

**Do not change the mode and assume the index follows.** The index is built by
`marula migrate` (or by the API when it runs migrations), reading the same
lever. Flip the lever and run the migrate step; otherwise the boot refuses with
the missing index named, which is the honest outcome.

**Do not reach for `hnsw.iterative_scan=relaxed_order`.** It can return rows out
of distance order, and the fused ranking has not been measured under it. Only
`strict_order` ships.

## What you learned

- The mode is an install-wide decision about the shape of your data, not a
  quality setting.
- `exact` is the default, needs nothing, and does not use an index even when one
  exists.
- `iterative` and `index` refuse at boot with every missing requirement named,
  and never fall back to the scan.
- `marula deploy describe` reports per-tenant counts and the exact refusal
  sentence, so you can decide before you restart.
- `MARULA_RETRIEVAL_WATCH_SECS` (default 300) controls how often the worker
  records each tenant's count as `marula.embedding.count`.

## Related

- [Change the embedding model](operator-embedding-model.md) — the lever that
  decides *which* vectors exist; this page decides how they are searched.
- [Embedding model catalogue](../reference/embedding-model-catalogue.md) — the
  model table, floors and the space tag the index filters on.
- [Deploy descriptor](../../deploy-descriptor.md) — the `retrieval` section this
  page reads, and the other levers beside it.
