# ADR 0020 — RAG and media retrieval: a published vector-namespace extension surface

**Status:** Accepted · 2026-10-07 · accepted 2026-10-15 (Wave 45 lane 1, `p45-rag-vector-namespace`, migration 0132)
**Scope:** the shape of the tenant vector store for retrieval-augmented generation
and multimodal (image/video/audio) search: a second namespace beside unified
search, its dimension contract, the ingest/query extension surface, and where
media preprocessing does and does not live. Does not decide the commercial
layer's embedder (that is a Marula HQ ADR) or change unified search's ranking.

## Context

Unified search already carries a vector leg (migration `0062_search_vector.sql`,
BUILD-PROMPT §4.5): `search_embeddings`, one vector per
`(org_id, entity_kind, entity_id)` over core entities, the searchable text being
`title\nbody` (`search_embed_text`, `crates/core-api/src/ai.rs`). The width is
fixed at `SEARCH_EMBEDDING_DIM = 768` (`crates/core-domain/src/completion.rs`),
chosen as the intersection of the bge-base, nomic-embed-text and truncated
OpenAI text-embedding-3 defaults. A provider that cannot produce 768 leaves the
row unembedded and the vector leg degrades to FTS. Storage is a side table, the
FTS table is byte-stable, and there is no ANN index in v1 (exact KNN under RLS,
deliberately).

That leg answers one question: rank the core's own entities for unified search.
It does not answer the layer above it, and the layer above it is real:

- **RAG.** The intelligence layer ingests the native corpus as *chunks* and
  needs passage-level vectors with mandatory source attribution, so an agent
  retrieves evidence rather than a whole record. Core exposes no way to store
  or query such vectors.
- **Media.** Attachments (error screenshots, device photos, scanned paperwork,
  call recordings, screen recordings) are a large part of MSP knowledge and are
  **not searchable at all**: an `attachment` entity exists
  (`crates/core-domain/src/entities/attachment.rs`) but no embedding covers it,
  the entity-kind vocabulary is text-only
  (`ticket, comment, client, site, contact, kb_article, document`), and the query
  path takes a string.
- **The fixed width and text-only kinds are load-bearing constraints, not
  oversights.** A RAG layer running a 1024-dimension text model, or any media
  model, has nowhere sanctioned to land its vectors in the tenant database.

Three doctrines collide if this is not answered in the core:

1. Tenant vectors are tenant state and live in the tenant's own database
   (`docs/planning/03-marula-implementation-plan.md`, REL-I2, and ADR 0019). A
   shadow store outside the tenant DB forfeits structural isolation, deletion
   provability and GDPR erasure receipts.
2. The commercial layer reaches the core only through public extension points
   (HQ prime directive, the same rule that produced ADR 0014's out-of-tree
   routing contract). A private hook or a fork is forbidden.
3. Media weights are per-architecture and re-released on their own cadence. The
   core ships no model weights and pins multi-arch images
   (`docs/site/reference/self-host-local-ai.md`). The core must not grow a
   media-decoding dependency to serve a surface whose preprocessing belongs to
   the caller.

## Decision

**Core owns a second, modality-aware vector namespace in the tenant database,
and publishes ingest and query as extension points.**

1. **A distinct namespace, not `search_embeddings`.** Working name
   `rag_embeddings`, keyed `(org_id, namespace, source_kind, source_id,
   chunk_index, modality)` with provenance (`model`, `content_hash`) and an
   `embedded_at` timestamp, RLS by `org_id`, exact KNN in v1. Chunks are rows.
   A whole-entity vector row cannot express `chunk_index`, and folding chunks
   into `search_embeddings` would break its exactly-once upsert contract and
   its byte-stability goal.
2. **One width: 768.** `SEARCH_EMBEDDING_DIM` is the contract for both
   namespaces. Deliberate, because the candidate multimodal model
   (EmbeddingGemma 2) is native-768 and shares one space across text, image and
   audio. A second width would mean two query semantics and a re-embed
   migration on every model change. A vendor that cannot produce 768 is refused
   as it is today — never stored at the wrong width.
3. **Modality is a first-class column** (`text | image | video | audio`) on the
   row and in the query. Because the space is shared, a text query may retrieve
   media and a media query may retrieve text within a namespace. Cross-namespace
   comparison is out of scope. Queries and documents share a namespace.
4. **Published surface, general by construction.** Ingest (batch upsert of
   chunks, accepting **precomputed vectors** for media the core cannot
   preprocess, and text for the org's configured embedder) and query (hybrid
   BM25 + vector, RRF fusion, mandatory attribution) ship on the OpenAPI
   surface and the SDK, available to installed plugins and connectors and to a
   commercially operated intelligence service as **just another client**. The
   surface is not special-cased for any first party.
5. **Core does not preprocess media.** Image decode, video frame sampling
   (the model's 1 fps default), audio resample/decode and any transcription are
   the caller's job. The core stores and serves vectors plus the metadata needed
   to attribute and delete them. This keeps the core free of per-architecture
   native model dependencies, per the local-AI doctrine.
6. **Deletion propagates.** Deleting a source removes its chunk rows on the
   next cycle through a cascade on `source_id`, and a deletion receipt records what
   was removed. DSAR/erasure rides the same path, so vector deletion is
   provable, not asserted.
7. **Isolation is tested, not assumed.** Row-level security per org carries a
   hostile test that a namespace stays isolated, alongside the existing
   guarantees that tenants cannot read each other's data.

The commercial layer's embedder and its ingestion pipeline are decided
separately (Marula HQ ADR 0025). This ADR fixes only the surface they consume.

## Rejected

- **Reuse `search_embeddings` for RAG and media.** Its entity kinds are whole
  core entities. RAG works at chunk level, and media is not a core entity. Mixing
  would also put the byte-stable FTS contract at risk.
- **A configurable vector width (or a second `vector(1024)` table).** Reopens
  per-model schema churn, two query code paths, and a re-embed migration on
  every embedder change. The 768 common denominator already covers the
  candidate set, including the multimodal option. A wider space earns its own
  ADR only on measured underperformance, not in advance.
- **An HQ-owned shadow vector store outside the tenant DB.** Cheapest to build,
  and it gives up deletion provability, structural isolation and the single
  placement — the three things this layer exists to protect.
- **Core running its own media encoders.** Puts multi-arch native weights inside
  Core images, which the local-AI doctrine explicitly rejects.

## Consequences

- Core gains one stable, documented retrieval surface that every
  embedding-backed feature (RAG, media search, migration mapping assist) rides.
  Self-hosters get the same capability through the same public API the
  commercial layer uses.
- The commercial layer stops needing a fork or a private hook, and can be
  rebuilt or replaced without touching tenant data.
- Cost is a new migration, ingest/query routes, SDK regen, and the isolation
  tests — no new service, no new datastore, no ANN operator.
- The FTS table and unified-search ranking are untouched. The new namespace is
  additive.
- **What would falsify this:** a real workload where 768 materially
  underperforms a wider space for media retrieval, which would justify a
  per-namespace width in a superseding ADR — or a decision that RAG vectors
  should not live in the tenant database at all, which reopens placement, not
  the embedder.

## References

`crates/core-db/migrations/0062_search_vector.sql` ·
`crates/core-domain/src/completion.rs` (`SEARCH_EMBEDDING_DIM`,
`CompletionProvider::embed`) · `crates/core-api/src/ai.rs` ·
ADR 0019 (tenancy boundary) · ADR 0014 (out-of-tree provider contract
precedent) · ADR 0009 (plugin distribution) ·
`docs/site/reference/self-host-local-ai.md` · BUILD-PROMPT §4.5 ·
`docs/planning/03-marula-implementation-plan.md` (REL-I2) ·
Marula HQ ADR 0025 (the consuming embedder decision)
