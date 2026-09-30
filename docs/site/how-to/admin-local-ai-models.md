# How to run transcription and embeddings on your own hardware

> **Audience:** admin — this page changes AI provider configuration and
> starts infrastructure on your host.

Marula will call a model runtime you run. The image you already deploy
carries no model weights, so this is a separate opt-in step: start a local
runtime, pull two models, and point the AI settings at it. Twenty minutes
of work, and after that voice notes transcribe and knowledge search stays
vector-backed without a byte leaving your network.

Before you start, read
[Local AI: transcription and embeddings on your own hardware](../reference/self-host-local-ai.md)
for why the models are not in the Marula image. The short answer is that a
model is data, not a dependency, and you should be the one holding it.

## 1. Start the model runtime

```sh
docker compose -f deploy/docker-compose.yml --profile ai up -d ollama
```

Check it is up:

```sh
docker compose -f deploy/docker-compose.yml exec ollama ollama list
```

The slot publishes no host port on purpose — see step 5.

## 2. Pull the two models

```sh
# Transcription. Marula's own default name for this provider, so you can
# leave the model field empty in settings.
docker compose -f deploy/docker-compose.yml exec ollama ollama pull whisper

# Embeddings. nomic-embed-text is 768 dimensions natively, which is exactly
# the width Marula's search vectors require. A narrower model would leave
# vector search dark and silently fall back to full-text.
docker compose -f deploy/docker-compose.yml exec ollama ollama pull nomic-embed-text
```

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
| Provider | `ollama` |
| Base URL | `http://ollama:11434` |
| API key | leave empty — a local runtime does not check one |
| Transcription model | `whisper` (or empty, which means the same thing) |
| Embedding model | `nomic-embed-text` |

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
- **Knowledge search** fills its vector column once the embedding provider
  is configured. Existing articles are embedded on the next indexing pass,
  and new ones as they are saved.

## 5. What not to do

**Do not publish port 11434.** The runtime has no authentication of its
own, and loading a model means parsing a native format. If you put it on a
network, you have published an endpoint that runs downloaded code for
anyone who reaches it. The shipped slot deliberately publishes nothing, and
core-api reaches it over the compose network. If you need to reach it from
a laptop, tunnel it the same way you would a database.

**Do not point Marula at a public Ollama instance you do not run.**
Whatever it can reach, it can send.

**Do not skip the dimension check on embeddings.** If you swap in a
different embedding model, confirm it returns 768 dimensions first.
Marula refuses to store a wrong-width vector. The column stays dark and
search falls back to full-text, so a dimension mismatch shows up as
"search got worse" rather than as an error. That is an annoying way to
find out.

## What you learned

- Local models are a separate profile, a separate volume, and two
  `ollama pull` commands.
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
- [Local AI: transcription and embeddings on your own hardware](../reference/self-host-local-ai.md)
  — the decision behind operator-run models
