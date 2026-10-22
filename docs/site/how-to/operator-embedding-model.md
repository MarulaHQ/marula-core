# How to change the embedding model

> **Audience:** admin — this page changes what the whole install runs and
> starts a re-index of every organization's search vectors.

The embedding model decides what search can match on meaning. It serves the
whole install, so a change is a deployment decision, not an organization
setting. This page takes you from "which model" to a finished re-index, and
covers the two ways the change can stop: a capacity refusal at boot, and a
re-index that parks because a credential is missing.

Read [the catalogue reference](../reference/embedding-model-catalogue.md) for
the model table, the space tag and the custom-entry grammar. This page is the
procedure.

## 1. Decide what to change

You have two levers and you rarely need both:

- `MARULA_EMBEDDING_TIER` moves you between `minimal`, `standard` and
  `advanced`. The tier picks the model and its capacity floor together.
- `MARULA_EMBEDDING_MODEL` names one model id from the catalogue. It wins over
  the tier and keeps that entry's floor.

The shipped default is `standard`, which is
`embeddinggemma-2:270m-q8_0-text`: 768 dimensions, 4 cores, 768 MB.

Before you restart, check that the host can host the choice. If you know what
the host offers:

```sh
MARULA_EMBEDDING_TIER=advanced MARULA_HOST_MEMORY_MB=1024 \
  marula deploy describe --json | jq '.embedding.resolved'
```

`refusal` in that output is the exact sentence the boot will print if the
floor is not met. `null` means it will start.

## 2. Set the lever and restart

Put the variable where the other deployment settings live — an `.env` file
beside the compose file, or your orchestrator's environment:

```sh
MARULA_EMBEDDING_TIER=advanced
```

Then recreate the services so both processes read it:

```sh
docker compose up -d core-api worker
```

The local model has to exist in the runtime before the first call. The shipped
`ai` profile pulls whatever `MARULA_EMBEDDING_MODEL` names:

```sh
docker compose --profile ai run --rm ollama-model
```

The worker reads the same levers as the API. It refuses a lever it cannot
resolve, because a worker that disagreed about the model would queue work into
a space nothing writes. It only warns about a floor shortfall, which is correct
in a split deployment where the model runs on the API's host.

## 3. Read the boot if it refuses

A model that does not fit stops the boot instead of going dark later. The
message carries the numbers:

```
embedding model `embeddinggemma-2:740m-q8_0` needs at least 4 cores and 2048 MB of RAM
(tier `advanced`); this host offers 2 cores and 1024 MB — give the host more capacity, select a
smaller tier with MARULA_EMBEDDING_TIER (minimal, standard), or run the model on a cloud
provider with MARULA_EMBEDDING_PROVIDER=openai
```

Three ways out, in the order most operators take them: give the host the
capacity, drop to a smaller tier, or move the model to a cloud provider.

A misspelled tier or a tier the provider does not offer stops the boot with its
own message, listing the tiers that provider has. A record in
`MARULA_EMBEDDING_CUSTOM_MODELS` that fails to parse does not stop anything: it
is logged with the text you wrote, and the shipped table still runs.

## 4. Watch the re-index

The boot records the new space and leaves the previous one readable when the
two populations are comparable. The worker then queues one run per organization
that holds rows without a fresh vector, and sweeps them:

- 128 rows per pass, paused 12.8 s after each pass to stay under 600 rows a
  minute. Both numbers are levers: `MARULA_EMBEDDING_REINDEX_BATCH` and
  `MARULA_EMBEDDING_REINDEX_ROWS_PER_MIN`. A pass costs
  `batch × 60000 / rows_per_min` milliseconds of pause.
- An idle sweep looks again every 60 seconds
  (`MARULA_EMBEDDING_REINDEX_SECS`).
- `MARULA_EMBEDDING_REINDEX_WINDOW=22-06` keeps the work inside UTC hours. The
  end is exclusive and a start after the end wraps midnight. Leave it unset to
  work immediately.

Watch two series:

```promql
# Rows still missing a fresh vector, per organization.
marula_embedding_reindex_remaining

# Unfinished runs on the install. Zero means nothing is queued.
marula_embedding_reindex_runs_pending
```

`runs_pending` sitting still with work outstanding means the job is not
running. The common cause is a missing `MARULA_INTERNAL_TOKEN`: the worker
reaches `core-api`'s backfill surface with it, and without it the job is
skipped with a log line and the runs stay queued.

The job needs no babysitting. Its selection is "rows without a fresh vector in
the target space", so a restart resumes where the data says, not where a cursor
says. A provider outage parks the run with the reason in `last_error` and the
next sweep retries it.

When an organization's run completes, its previous-space rows are deleted in
the same transaction. When the last run finishes, search stops reading the
previous space at all.

## 5. Roll back if you need to

Set the lever back and restart. That is a switch like any other, and it
re-indexes the population into the old space:

```sh
MARULA_EMBEDDING_TIER=standard
docker compose up -d core-api worker
```

If the forward switch had finished, its space's rows are already gone, so the
rollback embeds everything again. If it had not finished, the partial rows in
the abandoned space are deleted as each organization's rollback run completes.

Nothing rolls back automatically. A run that keeps failing records why and
waits for a person.

## 6. Running a cloud model

Set the provider and give the install a credential:

```sh
MARULA_EMBEDDING_PROVIDER=openai
MARULA_EMBEDDING_API_KEY=sk-...
```

The instance key serves every organization. An organization's own stored key is
consulted only when the instance has none, and only for the provider the
instance selected. An organization that ends up with no credential gets a dark
vector leg with the reason `missing_credential` and falls back to full-text
search. It never silently switches to another model.

Point `OPENAI_BASE_URL` at a compatible endpoint if you are not calling OpenAI
directly.

## 7. What not to do

**Do not try to set the model per organization.** It is not a supported
setting, and it is the hazard the space tag exists to remove: two models in one
table produce rankings made of two incomparable number systems. An
organization's `ai.completion_provider` means `none` (switch this organization's
AI off) or supplies a credential. Its `ai.completion_model` selects nothing.

**Do not run two models at once for a faster re-index.** The job embeds the
target model only, with the same model the live search path uses. When a switch
is in progress the previous space is read-only and never re-embedded.

**Do not add a vector index to speed this up on its own.** The retrieval mode
decides whether an index exists and which one; see
[how to choose the retrieval mode](operator-retrieval-mode.md). `exact` is the
default and builds nothing.

**Do not expect the job to re-embed uploaded media.** `rag_embeddings` holds
vectors a connector computed. They carry the tag and are filtered on it. Their
repair is re-ingesting the file, which the connector already does when its model
changes.

## What you learned

- The model is one instance-wide choice. The tier picks both the model and its
  floor.
- A model that does not fit stops the boot with the exact numbers, and the
  message names the levers that fix it.
- A change re-indexes in the background at a paced rate, resumable from the
  data rather than a cursor.
- Two series tell you whether it is moving. A missing internal token is the
  usual reason it is not.
- Rollback is the same lever set back, and it re-embeds everything.

## Related

- [Embedding model catalogue](../reference/embedding-model-catalogue.md) — the
  table, floors, space tag, custom entries, credentials.
- [Run transcription and embeddings on your own hardware](admin-local-ai-models.md)
  — the local runtime, the compose profile and the one pull.
- [Manage AI access](admin-ai-access.md) — the master switch and per-surface
  grants an organization needs for AI features.
