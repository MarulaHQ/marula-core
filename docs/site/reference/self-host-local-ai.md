# Local AI: transcription and embeddings on your own hardware

> **Audience:** site operator / MSP admin — you host Marula yourself, you
> decide what runs inside your network, and you are the one who has to
> answer for it.

Marula's AI features talk to a model runtime you choose. The core ships
no model weights, in no image, under no profile. If you want transcription
or embeddings to happen on your own hardware, this page is how.

## The short version

```sh
docker compose -f deploy/docker-compose.yml --profile ai up -d ollama
docker compose -f deploy/docker-compose.yml exec ollama ollama pull whisper
docker compose -f deploy/docker-compose.yml exec ollama ollama pull nomic-embed-text
```

Then point Marula at `http://ollama:11434` in the AI settings, enable the
surfaces you want, and supply a credential if the provider asks for one.
Ollama ignores an empty credential, so a local runtime needs none.

## Why there are no weights in the Marula image

This is a decision, not an omission, and it is worth defending because
"bundled local Whisper" sounds like a feature you are missing.

**A model is not a dependency, it is data.** The Docker layers Marula
ships are rebuilt for every security patch to core-api, realtime, and the
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
| Embeddings | exactly 768 dimensions | `nomic-embed-text` | 768 is its native width, so it satisfies the contract with no truncation. |

A smaller Whisper variant works too — Marula sends whatever model name
you configure and falls back to `whisper` when the field is empty or
`none`. Pick the smallest one that transcribes your audio acceptably;
transcription quality differences between variants are large and the size
differences are several hundred megabytes.

## What this costs you

Nothing runs on your CPU until you pull a model, and the first request
after a pull is slow while the model loads. The slot is capped at 4 GB of
memory and 2 CPUs, which is enough for `whisper` and `nomic-embed-text`
on CPU. If you have a GPU, this slot will not use it — the image is
pinned without a GPU runtime, and adding one is your call to make in your
own override rather than something the shipped compose guesses at.

## What this does not do

- **It is not authenticated.** Ollama has no auth of its own. That is
  why the slot publishes no host port at all: core-api reaches it by
  service name inside the compose network, and nothing else can. If you
  expose it, you are exposing an unauthenticated endpoint that loads and
  executes a native model format. Do not.
- **It does not leave your network.** With `OLLAMA_NO_CLOUD=1` set, the
  runtime does not phone out to fetch models. Every model comes from the
  `docker compose exec … ollama pull` you ran, into the `ollamadata`
  volume, which you can inspect and delete.
- **It does not disable the BYOK posture.** A network provider is still a
  first-class choice, and still the right one for a lot of deployments.
  Nothing here pushes you toward running models locally.

## Related

- [How to control AI access](../how-to/admin-ai-access.md) covers the org
  switch and the per-user grants.
- [Observability](self-host-observability.md) documents the other opt-in
  profile in the same compose file.
