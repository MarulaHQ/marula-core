# deploy

> **Purpose:** docker-compose.yml (Postgres+pgvector, Redis/Valkey, MinIO (helm; Garage in compose), Mailpit, services, web), Helm chart, upgrade runbooks, self-host backup/DR guide, and the TLS edge in `edge/`.
> **Non-purpose:** No SaaS/Kubernetes control-plane infrastructure — managed multi-tenant hosting is a commercial concern outside this repo.

## What is here

| Directory | What it is |
|---|---|
| `docker-compose.yml` | The local stack: Postgres, Valkey, **PgBouncer**, migrations, core-api, worker, web, plus the `stage2`, `observability`, `ai` and `backup` profiles. It ships no secret value and requires eight (`JWT_SECRET`, `MARULA_SECRET_KEY`, the two seam tokens, `POSTGRES_PASSWORD`, `MARULA_APP_PASSWORD`, `GARAGE_RPC_SECRET` and `GLITCHTIP_SECRET_KEY`). No TLS and no published-proxy boundary, because it targets development. |
| `dev.env` | The dev marker as a file: the repository's published dev values plus `MARULA_DEV_TOOLS=1`, passed with `--env-file`. Never for a host you do not own. The stack warns at boot while it is in use. |
| `pooler/` | The PgBouncer config every application pool dials: transaction mode, `DISCARD ALL` after every transaction, no prepared-statement cache. `docs/reference/connection-pooling.md` says why each setting is load-bearing. |
| `Dockerfile.rust` / `Dockerfile.web` | The workspace images. `NEXT_PUBLIC_API_URL` is baked into the web bundle at build time. |
| `edge/` | **The public front door.** A Caddy proxy that terminates TLS, the three TLS modes (Direct ACME, Cloudflare Origin CA, Cloudflare DNS-01), and the origin firewall helper. |
| `preview/` | The per-PR preview environment: sizing overlay, publishing options, and the CI definition for it. |
| `postgres/` | The opt-in Postgres image carrying `pg_cron` and `hypopg`, and the compose overlay that swaps it in. The database instrumentation itself (statement statistics, slow-plan logging) is in `docker-compose.yml`, because the stock image already has it. |
| `helm/marula/` | The Helm chart, with the same routing and both TLS modes expressed through `ingress`. |
| `backup/` | WAL-G point-in-time recovery and restic snapshots, plus the restore rehearsal. |
| `observability/` | OTel collector, Tempo, Loki, Grafana provisioning, and the alert pack (Prometheus rules, SLOs and incident runbooks) for the `observability` profile. Start from [`observability/README.md`](observability/README.md). |
| `runbooks/` | Operator runbook scaffolds. |
| `garage.toml` | The Garage config the `stage2` profile binds read-only. |

## Starting the stack

The base file ships no secret value, and compose refuses to render it without
`JWT_SECRET`, `MARULA_SECRET_KEY`, `MARULA_INTERNAL_TOKEN`,
`EMAIL_INTERNAL_TOKEN`, `POSTGRES_PASSWORD`, `MARULA_APP_PASSWORD`,
`GARAGE_RPC_SECRET` and `GLITCHTIP_SECRET_KEY`. A laptop stack takes the
repository's published dev values, which live in `dev.env` and are marked by
`MARULA_DEV_TOOLS=1`:

```sh
docker compose -f docker-compose.yml --env-file dev.env up --build
```

The `dev-posture` service prints a warning into the boot log every time those
published values are in play, because anyone who has read this repository can
mint an owner token against such a stack. For anything else, generate the
values yourself (`openssl rand -hex 32` each) and export them before `up`.
Two exceptions to "generate each one": `POSTGRES_PASSWORD` must be the value
the pgdata volume was initialised with (a fresh volume takes a generated
value; an existing one had `marula`), and `MARULA_APP_PASSWORD` must be
`marula_app` until a migration parameterises the role — migration 0114 mints
that literal, and a generated value locks the API out. If you change either
Postgres password, rewrite `pooler/userlist.txt` to match: PgBouncer's auth
file cannot read the environment.

## Starting a public deployment

The local stack publishes raw ports and speaks plaintext http, which is
the right posture for a laptop and the wrong one for a host on the
internet. Layer the edge overlay instead of editing the base file:

```sh
export MARULA_DOMAIN=marula.example.com
export ACME_EMAIL=ops@example.com          # direct and cf-dns01 modes
export JWT_SECRET=$(openssl rand -hex 32)
export MARULA_SECRET_KEY=$(openssl rand -hex 32)
export MARULA_INTERNAL_TOKEN=$(openssl rand -hex 32)
export EMAIL_INTERNAL_TOKEN=$(openssl rand -hex 32)
export POSTGRES_PASSWORD=$(openssl rand -hex 32)   # fresh volume; else keep the existing value
export MARULA_APP_PASSWORD=marula_app      # must match the marula_app role
export GARAGE_RPC_SECRET=$(openssl rand -hex 32)
export GLITCHTIP_SECRET_KEY=$(openssl rand -hex 32)

docker compose -f docker-compose.yml \
               -f edge/compose.edge.yml \
               --profile edge up --build -d
```

The overlay is last on purpose: it replaces the base file's published port
lists (compose appends an overlay's ports to them otherwise), so `8080`,
`3000`, `8082` and `15432` come off the network and the stage2 and
observability slots are re-pinned to loopback. `edge/check-published-ports.sh` asserts that on the
merged config and, with `--live`, on the running containers.

`edge/README.md` has the whole decision — the three TLS modes, the
Cloudflare dashboard steps behind the edge one, and the origin lockdown.
The operator runbook is
[`docs/site/reference/reverse-proxy-tls.md`](../docs/site/reference/reverse-proxy-tls.md).

## Connections

The application pools dial PgBouncer (`pgbouncer:6432`), not Postgres, and the
`migrate` job plus each process's one `LISTEN` connection dial Postgres
directly. That split is what keeps the connection count flat as instances
are added: 54 connections per instance, 15 server connections per
(database, user) pair. `pooler/README.md` is how to run and inspect it;
[`docs/reference/connection-pooling.md`](../docs/reference/connection-pooling.md)
is the reasoning and the measurements. To go back to a direct connection, set
`MARULA_APP_DATABASE_URL` / `MARULA_WORKER_APP_DATABASE_URL` at Postgres and
raise `MARULA_DB_STATEMENT_CACHE`, then accept the 100-connection ceiling that
comes with it.

## Profiles

`stage2` (object storage + mail sink), `observability` (OTLP, Tempo, Loki,
Prometheus with the alert pack, Grafana, status page), `ai` (local model
runtime), `backup` (WAL-G + restic), and `edge` (the TLS proxy) are all opt-in.
`docker compose up` without a `--profile` flag boots the same stack it always
did.

## Stopping the stack

`core-api`, `worker` and `web` run with `restart: unless-stopped` and a 30s
`stop_grace_period`. A crash brings the container back; a `docker compose
stop` or `down` sends SIGTERM, and the process drains for up to
`MARULA_SHUTDOWN_GRACE_SECS` (20s by default) before exiting. The binaries
log what finished and what they cut.

The compose stack has no container healthcheck for those three services: the
runtime image carries no shell and no `curl`, so a `HEALTHCHECK` could not
exec anything. The Helm chart is where HTTP probes live (`/livez`, `/readyz`,
`/healthz`).
