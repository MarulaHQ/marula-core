# Working on the code

This page takes a fresh clone to a running stack you can edit. It is the path
for someone changing Marula. If you want a server that serves a team, read
[install on a machine](install.md) instead.

## What you need

- Docker, with Compose 2.24 or newer. A registry install uses `!reset` in an
  override file, and older Compose ignores it.
- The Rust toolchain. [`rust-toolchain.toml`](../rust-toolchain.toml) pins
  stable, with `rustfmt` and `clippy`.
- Node 22 or newer and pnpm 12.16.1. [`package.json`](../package.json) pins the
  pnpm version through `packageManager`, so `corepack enable` is enough to
  match it.
- Room for the Docker build cache. The first build compiles the whole Rust
  workspace and the web bundle into images, and that is the largest thing you
  will run locally.

## Two commands

```sh
git clone https://github.com/MarulaHQ/marula-core && cd marula-core
docker compose -f deploy/docker-compose.yml --env-file deploy/dev.env up -d --build
```

The first run builds the Rust workspace and the Next.js bundle into the images,
so give it minutes. Later runs reuse the layers and come up in seconds.

[`deploy/dev.env`](../deploy/dev.env) holds published development secrets. The
stack prints a warning at every boot while they are in use, and it belongs on a
machine you own. An install that serves anyone else generates its own values;
[docs/install.md](install.md) explains the eight it needs.

## What you get

| URL | What it is |
|---|---|
| http://localhost:3000 | The workspace: tickets, dispatch, projects, billing |
| http://localhost:3000/portal | The client portal, as a client sees it |
| http://localhost:8080 | `core-api`, the only writer to the database |
| http://localhost:8080/docs | The API reference, generated from the OpenAPI document |
| http://localhost:8082/healthz | The worker's health endpoint |

The seeded demo organisation is not in a fresh stack. Use **Create org** on
`/login` to make one, or run [`scripts/demo-up.sh`](../scripts/demo-up.sh) for a
booted stack with demo tickets, invoices and logins already in place.

## Work on the web app with hot reload

The `web` container serves a production build on port 3000. To run the dev
server on the host instead, stop the container first, or the two fight over the
port:

```sh
docker compose -f deploy/docker-compose.yml --env-file deploy/dev.env stop web
pnpm install
pnpm dev                                 # Next.js on :3000, reloading on save
```

The API and the worker stay in Docker and the dev server talks to them on
`localhost:8080`.

## The pooler is opt-in

A single-node stack dials Postgres directly. The transaction pooler exists for
installs that run several API instances or many concurrent clients against a
Postgres that caps connections, so it sits behind the `pooler` profile and the
default stack never starts it.

```sh
docker compose -f deploy/docker-compose.yml --env-file deploy/dev.env \
  --profile pooler up -d
```

The profile starts PgBouncer and its auth renderer, but nothing dials it yet:
neither URL is set in `deploy/dev.env`, so the services keep connecting to
Postgres. Add these three lines to the env file you pass with `--env-file`,
using that file's passwords. The published dev values are `marula_app` and
`marula`; substitute your own anywhere else.

```sh
MARULA_APP_DATABASE_URL=postgres://marula_app:marula_app@pgbouncer:6432/marula
MARULA_WORKER_APP_DATABASE_URL=postgres://marula:marula@pgbouncer:6432/marula
MARULA_DB_STATEMENT_CACHE=0
```

If you pass the tracked `deploy/dev.env`, copy it first and pass the copy, so
the repository file stays clean: `cp deploy/dev.env deploy/dev.local.env`, add
the lines to `deploy/dev.local.env`, and point `--env-file` at it.

Transaction mode hands a connection to the next client after each transaction,
so the application cannot keep prepared statements. That is what the
`MARULA_DB_STATEMENT_CACHE=0` line above does. The migration job and the one
`LISTEN` connection stay direct either way, because neither can ride a pooled
session.
[`deploy/pooler/README.md`](../deploy/pooler/README.md) has the settings and
[`docs/reference/connection-pooling.md`](reference/connection-pooling.md) has the
measurements behind them.

## Run the end-to-end stack

`scripts/e2e-stack.sh` boots the stack the Playwright suite runs against. Each
run gets its own Postgres container, its own Valkey, random high ports and a
unique run id, so two runs on one machine never share a database or a port. It
migrates, seeds the demo organisation, starts the API and the worker, writes
`e2e/.e2e-stack.env` for Playwright, and returns once both services answer.

```sh
scripts/e2e-stack.sh                      # up, for this run only
E2E_FOLLOW=1 scripts/e2e-stack.sh         # stay attached and stream both logs
scripts/dev-status.sh                     # every registered run on this machine
scripts/e2e-stack.sh down <RUN_ID>        # tear down one run, and only that run
```

`E2E_DB_PROFILE=pooler` adds PgBouncer to the run and points the services at it.
`e2e/README.md` covers the per-run contract and how a flow logs in.

## Gates

Run the checks that cover what you changed, cheapest first.

```sh
cargo fmt --check
cargo clippy -p <crate> --all-targets -- -D warnings
pnpm typecheck
scripts/p13-test.sh -p core-api --test <flow>     # one target, not the workspace
```

`scripts/p13-test.sh` runs nextest when it is installed and falls back to
`cargo test`. A package whose integration targets fill the disk can be run in
chunks, so peak usage is one chunk instead of every binary:
`scripts/p13-test.sh --batch 12 -p core-api`.

Workflows and shell scripts have their own linters, and CI runs the same
commands:

```sh
bash -n scripts/release/*.sh
shellcheck -S warning scripts/release/*.sh
docker run --rm -v "$PWD:/repo" -w /repo rhysd/actionlint:1.7.12
```

The full gate set, the review rules and the pull request contract are in
[`CONTRIBUTING.md`](../CONTRIBUTING.md) and [`AGENTS.md`](../AGENTS.md).
