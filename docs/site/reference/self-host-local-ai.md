# Local AI: transcription and embeddings on your own hardware

> **Audience:** site operator / MSP admin — you host Marula yourself, you
> decide what runs inside your network, and you are the one who has to
> answer for it.

Marula's AI features talk to a model runtime you choose. The core ships
no model weights, in no image, under no profile. If you want transcription
or embeddings to happen on your own hardware, this page is how.

## The short version

```sh
docker compose -f deploy/docker-compose.yml --profile ai up -d
docker compose -f deploy/docker-compose.yml exec ollama ollama pull whisper
```

The profile pulls the embedding model itself. Transcription is the one
model you fetch by hand.

Then point Marula at `http://ollama:11434` in the AI settings, enable the
surfaces you want, and supply a credential if the provider asks for one.
Ollama ignores an empty credential, so a local runtime needs none.

## Why there are no weights in the Marula image

This is a decision, not an omission, and it is worth defending because
"bundled local Whisper" sounds like a feature you are missing.

**A model is not a dependency, it is data.** The Docker layers Marula
ships are rebuilt for every security patch to core-api and the
worker. Put a few gigabytes of weights in those layers and every
self-hoster re-downloads all of it every time you fix a CVE in something
unrelated — and pays the registry bandwidth and the disk for models their
deployment may never load. Most of Marula's AI surface is behind a
per-org switch and a per-user grant, so for a great many deployments the
weights would be dead weight in the image and live weight nowhere else.

**Weights are per-architecture, binaries are not.** The `faster-whisper`
path is a compiled native extension with per-architecture builds. This
repository's stack is multi-arch by construction — the compose file pins
one digest per image and those digests have to resolve for both `amd64`
and `arm64`, which is why every slot in it carries an index digest rather
than an architecture-specific one. Weights bundled into a multi-arch
image multiply the same problem across architectures that do not even run
the same binaries.

**Whose weights, and whose updates?** Whisper and bge-class weights carry
their own licences and their own release cadence, and they get re-released
on their own schedule. Bundling them pins a Marula image to a specific
model revision, and the only way to update it is a Marula release — for
something that changes weekly and has nothing to do with ticket routing.

**The seam already exists, and it is the better design.** Transcription
speaks the OpenAI-compatible `audio/transcriptions` endpoint, and
`ollama` is already a first-class provider on that seam. Embeddings ride
the `CompletionProvider` trait with a fixed 768-dimension contract
(`SEARCH_EMBEDDING_DIM` in `core-domain`). A provider that cannot produce
768 dimensions leaves vector search dark and falls back to full-text. It
never stores a wrong-width vector.

So "bundled local Whisper" is delivered as a one-command model runtime you
operate, not as a tarball in someone else's image. What you gain: the
weights are on a disk you can see and delete, you update them without
waiting for a Marula release, and a self-hoster who never enables
transcription never downloads a byte of them.

## The two models that match the two contracts

| Feature | Contract | Model | Why this one |
| --- | --- | --- | --- |
| Transcription | OpenAI-compatible `audio/transcriptions` | `whisper` | Marula's own default model name for the `ollama` provider. The call is a plain multipart POST. |
| Embeddings | exactly 768 dimensions | `embeddinggemma-2:270m-q8_0-text` | The bundled default. 768 is its native width, so it satisfies the contract with no truncation, and it is Apache 2.0. |

A smaller Whisper variant works too — Marula sends whatever model name
you configure and falls back to `whisper` when the field is empty or
`none`. Pick the smallest one that transcribes your audio acceptably;
transcription quality differences between variants are large and the size
differences are several hundred megabytes.

## What this costs you

The embedding model arrives with the profile: a one-shot `ollama-model`
service pulls it, and the `ai` profile is complete when that job exits.
Nothing runs on your CPU until a request needs it — the model loads on the
first search and is dropped again after five idle minutes, so an idle
install holds no weights in memory.

The slot is capped at 4 GB of memory and 4 CPUs. On the 4-core reference
profile the text-only model embeds about 16 ticket-sized chunks a second and
holds roughly half a gigabyte resident, which backfills a 50,000-chunk
install in under an hour. The first request after an idle period pays the
model load, about two seconds. If you have a GPU, this slot will not use it —
the image is pinned without a GPU runtime, and adding one is your call to
make in your own override rather than something the shipped compose guesses
at.

If your host is smaller than that, keep the vector leg on a cloud embedder
(see the BYOK setting) or point the model field at a bge-base-class model
instead. A two-core host embeds at roughly ten chunks a second, which turns a
first index into an overnight job.

### The bf16 build is faster on Apple Silicon, and slower on some x86 hosts

The default is the `q8_0` tag because it is the one that behaves the same on
every architecture. On an M-series Mac it is not the fast one: measured on a
10-core M1 Pro, the same model as `embeddinggemma-2:270m-bf16-text` embedded
49 chunks a second against the `q8_0` build's 16, and answered a search query
in 20 ms against 47 ms, for the same ranking. The bf16 build holds about
700 MB resident instead of 500 MB and takes 574 MB of disk instead of 304 MB.

So if you run Marula on arm64 and the vector leg is in the hot path, set
`MARULA_EMBEDDING_MODEL=embeddinggemma-2:270m-bf16-text` and keep the tag you
measured. On an x86 host without bf16 vector instructions, the quantized tag
is the safer bet. These numbers are arm64 only; nothing in this page claims
them for x86.

## What this does not do

- **It is not authenticated.** Ollama has no auth of its own. That is
  why the slot publishes no host port at all: core-api reaches it by
  service name inside the compose network, and nothing else can. If you
  expose it, you are exposing an unauthenticated endpoint that loads and
  executes a native model format. Do not.
- **It does not leave your network.** With `OLLAMA_NO_CLOUD=1` set, the
  runtime does not phone out to fetch models behind an inference request.
  Every model arrives through the profile's `ollama-model` job or a
  `docker compose exec … ollama pull` you ran, into the `ollamadata` volume,
  which you can inspect and delete.
- **It does not disable the BYOK posture.** A network provider is still a
  first-class choice, and still the right one for a lot of deployments.
  Nothing here pushes you toward running models locally.

## Related

- [How to control AI access](../how-to/admin-ai-access.md) covers the org
  switch and the per-user grants.
- [Observability](self-host-observability.md) documents the other opt-in
  profile in the same compose file.
