# How to run transcription and embeddings on your own hardware

> **Audience:** admin — this page changes AI provider configuration and
> starts infrastructure on your host.

Marula will call a model runtime you run. The image you already deploy
carries no model weights, so the models are a separate opt-in step: start
the local runtime, and it fetches the embedding model Marula uses by
default. Embeddings then work without a byte leaving your network, and
transcription is one more pull away.

Before you start, read
[Local AI: transcription and embeddings on your own hardware](../reference/self-host-local-ai.md)
for why the models are not in the Marula image. The short answer is that a
model is data, not a dependency, and you should be the one holding it.

## 1. Start the model runtime

```sh
docker compose -f deploy/docker-compose.yml --profile ai up -d
```

That starts Ollama and a one-shot `ollama-model` job that pulls the bundled
embedding model, EmbeddingGemma 2 (text-only build,
`embeddinggemma-2:270m-q8_0-text`). Check what landed:

```sh
docker compose -f deploy/docker-compose.yml exec ollama ollama list
```

The slot publishes no host port on purpose — see step 5.

## 2. Pull the transcription model

The embedding model is already there. Transcription is a separate pull:

```sh
# Marula's own default name for this provider, so you can leave the model
# field empty in settings.
docker compose -f deploy/docker-compose.yml exec ollama ollama pull whisper
```

Embeddings need no pull and no model field. EmbeddingGemma 2 is 768
dimensions natively, which is exactly the width Marula's search vectors
require, so it fills the column with no truncation. A model of another width
would leave vector search dark and silently fall back to full-text.

Confirm what landed on disk:

```sh
docker compose -f deploy/docker-compose.yml exec ollama ollama list
docker volume inspect <project>_ollamadata
```

If you decide later you do not want local models, delete the volume and the
weights go with it:

```sh
docker compose -f deploy/docker-compose.yml --profile ai down -v
```

## 3. Point Marula at it

In **Settings → AI**, add a provider:

| Field | Value |
| --- | --- |
| Provider | `ollama` (already the default; setting it explicitly does no harm) |
| Base URL | `http://ollama:11434` |
| API key | leave empty — a local runtime does not check one |
| Transcription model | `whisper` (or empty, which means the same thing) |

An install that never opens this page already has the local embedder: the
instance-wide selection is the local `standard` model, loaded on first use and
dropped after five idle minutes. This page's provider row covers transcription
and the completion models an organization calls. It does not choose the
embedding model: that is one instance setting, documented in
[How to change the embedding model](operator-embedding-model.md).

`http://ollama:11434` is the compose **service name**, and it has to be:
it is the address that resolves inside the compose network, where Marula
runs. `localhost` here means the Marula container itself, which is not
where Ollama is.

## 4. Turn the surfaces on

A provider being configured does not enable anything. Transcription and
embeddings each have their own switch:

- **Settings → AI → surfaces**: enable the surface, then grant it to the
  users who should have it.
- **Transcription** rides the plugin surface's admission rules — org
  switch, surface enabled, per-user grant, catalog allowlist, all of it.
  A user without the grant gets a visibly-absent state, not a silent
  empty transcript.
- **Knowledge search** fills its vector column from the instance's embedding
  model. New and edited rows are embedded as they are indexed; an existing
  population is re-indexed when the model changes, which is a deployment
  setting rather than a setting on this page.

## 5. What not to do

**Do not publish port 11434.** The runtime has no authentication of its
own, and loading a model means parsing a native format. If you put it on a
network, you have published an endpoint that runs downloaded code for
anyone who reaches it. The shipped slot deliberately publishes nothing, and
core-api reaches it over the compose network. If you need to reach it from
a laptop, tunnel it the same way you would a database.

**Do not point Marula at a public Ollama instance you do not run.**
Whatever it can reach, it can send.

**Do not swap the embedding model without the re-index.** If you change it,
do it through the instance levers in
[How to change the embedding model](operator-embedding-model.md), which records
the new space and re-embeds the population. A model of another width is refused
when a vector is stored: the column stays dark and search falls back to
full-text, so a dimension mismatch shows up as "search got worse" rather than
as an error. The multimodal build of the same model
(`embeddinggemma-2:740m-q8_0`) is 768 as well, and it is the one to choose if
you need image or audio chunks embedded, at a larger resident set.

## What you learned

- Local models are a separate profile and a separate volume. The embedding
  model arrives with the profile; transcription is one extra pull.
- The provider URL is a service name, never `localhost`.
- Configuring a provider enables nothing; the surfaces and grants are
  separate switches.
- Embeddings must be 768 dimensions or search quietly falls back to
  full-text.
- The slot publishes no port. That is the security property, not an
  oversight.

## Related

- [How to manage AI access](admin-ai-access.md), which covers the switches
  and what a denied call looks like
- [How to change the embedding model](operator-embedding-model.md) — the
  instance-wide model choice and its re-index
- [Local AI: transcription and embeddings on your own hardware](../reference/self-host-local-ai.md)
  — the decision behind operator-run models
