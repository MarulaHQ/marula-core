---
description: "Take a fresh clone to a running stack, run the web app with hot reload, and find the checks a pull request has to pass."
---

# Install Marula from source

> **Audience:** developer — you are changing Marula, and the stack on your
> machine is there to run your code.

This page stops at a working local stack. A server that serves a team takes the
release bundle instead, which [install on a server](install-on-a-server.md)
covers.

## What you need first

- Docker, with Compose 2.24 or newer.
- Rust stable, with `rustfmt` and `clippy`. The repository's
  `rust-toolchain.toml` pins the channel.
- Node 22 or newer and pnpm 12.16.1. `corepack enable` matches the pinned pnpm
  version.

## Two commands

```sh
git clone https://github.com/MarulaHQ/marula-core && cd marula-core
docker compose -f deploy/docker-compose.yml --env-file deploy/dev.env up -d --build
```

The first build compiles the Rust workspace and the Next.js bundle into the
images, so it takes minutes. Later runs reuse the layers and come up in
seconds.

| URL | What it is |
|---|---|
| http://localhost:3000 | The workspace |
| http://localhost:3000/portal | The client portal |
| http://localhost:8080 | `core-api` |
| http://localhost:8080/docs | The API reference |

`deploy/dev.env` holds published development secrets. The stack warns at every
boot while they are in use, and it belongs on a machine you own.

## Hot reload for the web app

Stop the `web` container first, or it keeps port 3000 and the dev server has
nowhere to bind:

```sh
docker compose -f deploy/docker-compose.yml --env-file deploy/dev.env stop web
pnpm install
pnpm dev
```

The API and the worker stay in Docker, and the Next.js dev server talks to
`localhost:8080`.

## The pooler is opt-in

A single-node stack dials Postgres directly. PgBouncer sits behind the `pooler`
profile for installs that run several API instances or many concurrent clients.
[Deploy](https://github.com/MarulaHQ/marula-core/blob/main/deploy/README.md)
has the connection arithmetic and the command that turns it on.

## Tests and checks

`scripts/e2e-stack.sh` boots its own isolated stack for the Playwright suite:
its own Postgres, Valkey, random ports and run id, so two runs never collide.

```sh
scripts/e2e-stack.sh
scripts/e2e-stack.sh down <RUN_ID>
```

The Rust tests, the workflow lint and the shell lint are the same commands CI
runs:

```sh
scripts/p13-test.sh -p core-api --test <flow>
docker run --rm -v "$PWD:/repo" -w /repo rhysd/actionlint:1.7.12
shellcheck -S warning scripts/release/*.sh
```

The full contributor path, including chunked test runs for a full package, is
[`docs/install-from-source.md`](https://github.com/MarulaHQ/marula-core/blob/main/docs/install-from-source.md)
in the repository. [`CONTRIBUTING.md`](https://github.com/MarulaHQ/marula-core/blob/main/CONTRIBUTING.md)
covers the review rules and the sign-off every commit carries.
