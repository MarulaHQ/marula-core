# Embedding model catalogue

Marula turns tickets, comments, clients and knowledge articles into vectors so search can match on meaning as well as words. One embedding model serves the whole install. This page is the table of models you can choose, what each one needs from the machine, and the bookkeeping that makes a change safe.

The model is an instance setting, not an organization setting. Organizations differ in what they may *see*, not in which vector space their text lives in: two models in one table would produce rankings made of two incomparable number systems. An operator picks the model once with environment variables, and every organization writes into that one space.

## Shipped entries

Every entry stores 768 dimensions, because that is the width of the vector column. A model that is natively wider is asked for 768 by the provider (OpenAI's `dimensions` parameter). A model that is narrower cannot be used.

| Model id | Tier | Provider | Modality | Floor | Notes |
|---|---|---|---|---|---|
| `bge-base-en-v1.5:109m-q8_0` | `minimal` | `ollama` | text | 2 cores, 512 MB | The smallest local text model here. Estimated floor, not measured. |
| `embeddinggemma-2:270m-q8_0-text` | `standard` | `ollama` | text | 4 cores, 768 MB | The shipped default. 16.0 chunks/s, 508 MB resident, 47 ms per query. |
| `embeddinggemma-2:270m-bf16-text` | `standard` | `ollama` | text | 4 cores, 1024 MB | 49.0 chunks/s, same top-10 on every measured query. Measured on arm64 only. |
| `embeddinggemma-2:740m-q8_0` | `advanced` | `ollama` | text, image, audio | 4 cores, 2048 MB | The multimodal build. Same ranking on text. The memory buys image and audio. |
| `text-embedding-3-small` | `minimal` | `openai` | text | none | Cloud. Needs a credential. |
| `text-embedding-3-large` | `standard` | `openai` | text | none | Cloud. Needs a credential. |

A floor is local capacity: logical cores, and resident megabytes with one batch in flight. Cloud entries have no floor because they run elsewhere.

## How a selection resolves

Three levers, read in this order:

1. `MARULA_EMBEDDING_MODEL` names an id. The catalogue supplies the entry, including its floor and its space tag.
2. `MARULA_EMBEDDING_TIER` (`minimal`, `standard`, `advanced`) plus `MARULA_EMBEDDING_PROVIDER` (`ollama` by default, `openai` for a cloud endpoint) picks the first catalogue entry with that tier and provider.
3. With neither set, the standard entry for `ollama` runs.

`MARULA_EMBEDDING_MODEL` wins over the tier. A model id the catalogue does not know is still accepted, because an operator may have pinned something newer than this build. It borrows the tier's floor and records the version `custom`.

If the provider has no entry in the requested tier, the boot fails and names the tiers that provider does offer. A missing model is a startup error, not a search that quietly degrades.

## Capacity floors and the refusal

`core-api` checks the selected entry's floor against the machine at boot. Cores are always checked. Memory is checked only when the host publishes a reading (a cgroup limit or `/proc/meminfo`). An unknown memory reading never refuses, because taking a working install down over a missing number is worse than running.

A shortfall stops the boot with the numbers in the message:

```
embedding model `embeddinggemma-2:270m-q8_0-text` needs at least 4 cores and 768 MB of RAM
(tier `standard`); this host offers 2 cores and 512 MB — give the host more capacity, select a
smaller tier with MARULA_EMBEDDING_TIER (minimal, standard), or run the model on a cloud
provider with MARULA_EMBEDDING_PROVIDER=openai
```

The worker reads the same levers and refuses a selection error, because a worker that disagreed with the API about the model would queue re-index runs into a space nothing writes. It only *warns* about a floor shortfall: in a split deployment the model runs on the API's host, and the worker's memory says nothing about whether it fits there.

`MARULA_HOST_MEMORY_MB` overrides the detected memory, which is useful when a container limit is not visible or when you want a boot to prove a floor before you move it. `marula deploy describe --json` prints the catalogue, what the current environment resolves to, and the refusal it would print, without starting anything.

## The space tag

Every stored vector carries `model@version/dimensions`: `embeddinggemma-2:270m-q8_0-text@1/768`, for example. Retrieval filters on that tag, so a query vector from one model is never ranked against stored vectors from another. The failure that prevents is quiet rather than loud: two models' distances are not comparable, and the result is a confident-looking order built from nothing.

Rows written before tags existed carry `<model>@unversioned/<width>`, read from the stored vector. That population is honest about what it is: the model is known, the build is not.

## Custom entries

`MARULA_EMBEDDING_CUSTOM_MODELS` adds or replaces entries. Records are separated by `;` and each has ten `|`-separated fields:

```
id|display|dimensions|modality|license|provider|version|tier|cores|ram_mb
```

```
MARULA_EMBEDDING_CUSTOM_MODELS='my-embedder:1|My Embedder|768|text|MIT|ollama|2|standard|4|900'
```

Rules:

- `dimensions` must be 768, because the column is fixed.
- `tier` is `minimal`, `standard` or `advanced`.
- An empty `version` becomes `custom`.
- A record whose `id` matches a shipped entry replaces it. That is how you pin a floor you measured yourself.
- A malformed record is dropped with a boot warning that quotes it. A typo in an optional lever must not stop an install that would run fine on the shipped table.

A custom entry with `provider` set to anything other than `ollama` speaks the OpenAI wire shape and uses `OPENAI_BASE_URL`.

## Changing the model

Changing `MARULA_EMBEDDING_MODEL` (or the tier) records a switch and starts a re-index. The steps, in order:

1. **Boot records the new space.** `core-api` writes it to `embedding_space_state`, with the previous space kept readable only when the two populations are comparable: same model id, different recorded build. A different model id leaves the previous space unreadable, so an un-reindexed row is a miss rather than a bad ranking.
2. **The worker queues one run per organization** that holds rows without a fresh vector in the target space. The insert spans organizations, so the worker does it. `core-api` owns every vector write, so it does the embedding.
3. **Sweeps embed batches and report progress.** `MARULA_EMBEDDING_REINDEX_BATCH` rows per pass (128 by default), paced by `MARULA_EMBEDDING_REINDEX_ROWS_PER_MIN` (600 by default): a pass is followed by `batch × 60000 / rows_per_min` milliseconds of pause, 12.8 s at the defaults. An idle sweep looks again every `MARULA_EMBEDDING_REINDEX_SECS` (60). `MARULA_EMBEDDING_REINDEX_WINDOW=22-06` confines the work to UTC hours, a start after the end wraps midnight, and unset means any hour.
4. **Completion removes the previous space's rows** for that organization, in the same transaction that marks the run complete. When the last run finishes, the readable window closes.

While a run is unfinished, search reads the target space first and the previous space second, as two separate ranking legs. It never compares distances across the two.

### Resume

There is no cursor. The job's selection is "rows with no fresh vector in the target space", so a crash repeats at most the batch in flight and cannot skip a row. Restarting the worker, the API, or both resumes from that predicate. The run row records counts for reporting, not position.

A provider outage or a missing credential keeps the run `running` with the reason stored in `last_error`, and the next sweep retries it. Marking it failed would mean an install never finishes after an operator sets the key. A run whose target is no longer the instance's target is failed on purpose, because the model changed again and retrying would write rows into a space nothing reads.

### What the job covers

The re-index owns `search_index`: tickets, comments, clients, sites, contacts, knowledge articles, documents. It does not re-embed `rag_embeddings`. Those vectors arrive precomputed from the connector that produced them, and core cannot recompute a media vector. They carry the space tag and retrieval filters on it. Their repair is the re-ingest path that already exists for a model change (`upsert_chunks` with `(model, content_hash)` idempotency).

### Rollback

Set the environment back to the previous model and restart. That is a switch like any other: the runs embed the old space again. If the forward switch had completed, its rows were deleted, so the rollback re-indexes the whole population. If it had not completed, the partial new-space rows are deleted when the rollback run for that organization completes.

Automatic rollback does not exist. A run that keeps failing records why and waits for a person.

## Watching a re-index

Three series, all documented in the observability coverage table:

| Metric | Attributes | Meaning |
|---|---|---|
| `marula.embedding.reindex.remaining` | `org` | rows still lacking a vector in the target space. Zero means that organization is done |
| `marula.embedding.reindex.runs_pending` | none | unfinished runs on the whole install |
| `marula.embedding.reindex.embedded` | `org` | rows embedded since this process started |

None of them has an alert rule. A value that is supposed to fall to zero would page every operator who changes the model. A job that has stalled shows up as these series sitting flat, which is a dashboard question.

`marula.embedding.reindex.runs_pending` also tells you whether the job is running at all: it needs `MARULA_INTERNAL_TOKEN` so the worker can reach `core-api`'s backfill surface. Without the token the job is skipped with a log line and the runs stay queued.

## Credentials

The default is local `ollama`, which needs no key. Every organization reaches it.

A cloud provider is an instance-wide choice, so its credential is resolved in this order:

1. `MARULA_EMBEDDING_API_KEY`, set on the instance. It serves every organization.
2. The organization's own stored key, consulted only when the instance has no key, and only for the provider the instance selected. An organization that stored an `openai` key is not asked for it when the instance runs `ollama`, and a key stored for a provider the instance does not use is ignored.

An organization with no credential serving a cloud provider gets a dark vector leg with a typed reason (`missing_credential`), and search falls back to full text for that organization. It never gets a different model: model selection is not per organization, and an organization's `ai.completion_provider` setting now only means `none` (switch this organization's AI off) or supplies a credential. The organization's `ai.completion_model` setting selects nothing.

## Related

- [Change the embedding model and watch the re-index](../how-to/operator-embedding-model.md) — the procedure, including the refusal and rollback.
- [Local AI: transcription and embeddings on your own hardware](self-host-local-ai.md) — why weights stay outside the core image.
- [Reliability levers](../../reference/reliability-levers.md) — every descriptor parameter, including the ten on this page.
