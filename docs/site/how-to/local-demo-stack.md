---
description: "Take a fresh clone to a booted, seeded demo, with its two containers and the sample organization."
---

# How to run the local demo stack

> **Audience:** admin — you run Marula on your own machine to show it to
> someone, and you want one command between a clone and a working demo.

`scripts/demo-up.sh` takes a fresh clone to a booted, seeded demo. Postgres
and Valkey run in Docker. core-api, the worker, and the web
console run natively from the repo. Nothing is deployed and nothing leaves
your machine.

## What you need first

- Docker, running. Postgres and Valkey are containers, and the app itself is not.
- The Rust toolchain (cargo) and pnpm.
- About 10 GB of free disk. The script checks and refuses to build below that.

## Run it

```sh
scripts/demo-up.sh
```

The script preflights the tools, starts Postgres on 15432 and Valkey on
16379 (`deploy/docker-compose.yml`, the ports the [local stack
reference](../reference/self-host-local-stack.md) pins), applies the
schema, seeds the demo org, builds and boots the three services, then
prints the logins. The first run compiles the Rust workspace and the web
app, so expect minutes. After that, re-runs take seconds.

Run it twice and the second run is a no-op that reports what is already
up. The demo org is seeded once, so a second `up` never wipes changes you
made during a demo.

## What you get

| Service | URL | Notes |
|---|---|---|
| web console | http://localhost:3000 | dev server, or `--web prod` for a build |
| core-api | http://localhost:8080 | the single write surface, spec at /openapi.json; also serves the SSE live stream |
| worker | http://127.0.0.1:8082/healthz | background loops |

Four logins, password `demo-password-123`:

- `owner@demo.local` — full admin
- `dispatcher@demo.local` — board and scheduling
- `tech@demo.local` — field work
- `bookkeeper@demo.local` — billing

The seeded org is deterministic. The tickets, the invoices, and the assets
are the same every run, so a demo you rehearsed is the demo you give.

## Before a prospect is in the room

Two pages to open first:

- **http://localhost:3000/settings/ai** — the BYOK checklist. The org
  switch, the provider, the key, and the per-surface grants. [How to
  control AI access](admin-ai-access.md) walks the settings, and [local
  AI on your own hardware](../reference/self-host-local-ai.md) covers the
  reasoning behind the local-model option.
- **http://localhost:3000/dev** — the dev-ops page: seed, reset the org,
  and time-travel the SLA clocks.

No key handy? Add `--ai` to the boot and the script starts the local
Ollama slot alongside the stack:

```sh
scripts/demo-up.sh --ai
```

Ollama comes up on 127.0.0.1:11434, loopback only. The shipped compose
file publishes nothing, on purpose. Pull the transcription model that
matches its contract, then point the AI settings at `http://127.0.0.1:11434`:

```sh
docker compose -f deploy/docker-compose.yml --profile ai exec ollama ollama pull whisper
```

Embeddings need no pull: the profile's one-shot `ollama-model` job fetches
EmbeddingGemma 2, the bundled default. The transcription pull is a few
gigabytes, so the script prints the command instead of running it. The first
inference after a pull is slow while the model loads, and every call after
that is fast.

## Reset and stop

```sh
scripts/demo-up.sh reset          # reseed the demo org, keep the stack
scripts/demo-up.sh reset --fresh  # destroy the database volume, rebuild, reseed
scripts/demo-up.sh stop           # stop the three app processes
scripts/demo-up.sh stop --infra   # ... and the Postgres/Valkey containers
```

`reset` reseeds from scratch and clears the volatile state: auth
lockouts, onboarding progress, and tour dismissals. The next login is the
first login. `--fresh` also destroys the database volume, and it counts
down five seconds first so a stray Enter does not cost you the data.

Logs for every service land in `.demo-up/logs/` inside the repo. Every
failure the script can catch ahead of time stops the run with a message
that says what to fix: Docker down, a port held by something else, low
disk, a stray non-hex `JWT_SECRET`. A failure mid-boot leaves the
half-up stack running with its logs, and `stop` clears the processes this
script started. It never kills a listener it does not recognize.

## The desktop alternative: no Docker, double-clickable

A second way to run a demo needs no Docker at all. The desktop test app
([ADR 0005](../../adr/0005-local-desktop-test-app.md)) bundles an
embedded Postgres, core-api, and the web console into one application
you launch by double-clicking. Same demo org, same four logins.

Choose it when:

- **Docker is not on the machine.** The app carries its own Postgres, so
  the only prerequisites are the repo, node, and `brew install pgvector`.
- **You are handing the demo to someone else.** A double-clickable
  artifact beats "clone the repo and run this script" when the person on
  the other end is a prospect, not an engineer.

Build it with `scripts/build-desktop.sh`; the vendored
`rebuild-local-test-app` skill documents the rebuild loop. The first
build on a machine takes roughly three minutes, because it downloads the
Postgres binaries and compiles the app. Every rebuild after that is
dominated by the web build. The app's data lives in `~/Library/Application
Support/Meridian/` and survives rebuilds, migrations included. Delete
that directory to start from a blank machine.

Where it gives ground, so you can choose honestly: the desktop app runs
a local-mode config with external integrations off, and its port space
is not the one `demo-up.sh` uses. Anything involving real email,
Stripe, or shared testing is compose territory, so run
`scripts/demo-up.sh` or the full compose stack for that.

## If you outgrow the machine

The same demo runs as the full container stack with `docker compose -f
deploy/docker-compose.yml up --build`, and on a server behind TLS. When
you get there, [how to upgrade](upgrade.md) covers the release path and
[watching your self-hosted services](admin-self-host-services.md) covers
the health pane.
