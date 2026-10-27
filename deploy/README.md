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
One exception to "generate each one": `POSTGRES_PASSWORD` must be the value
the pgdata volume was initialised with (a fresh volume takes a generated
value; an existing one had `marula`). `MARULA_APP_PASSWORD` is generated like
the rest — the `app-role-password` one-shot applies it to the `marula_app`
role as the owner after migrations and before core-api starts, so it does not
have to match anything the volume already holds. PgBouncer's auth file is
rendered from both variables on every `up` (`pooler/render-userlist.sh`); no
credential is checked in for it. "Rotating the app role password" below is the
rotation path.

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
export MARULA_APP_PASSWORD=$(openssl rand -hex 32)   # applied to the role on every up
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

## Rotating the app role password

`MARULA_APP_PASSWORD` is the one role password the stack applies itself, so a
rotation is a value change and a restart — no migration, no hand-edited file.
Compose applies it in the `app-role-password` one-shot, Helm in the
`pg-bootstrap` Job; both re-run on every deploy, which is why nothing has to be
remembered afterwards.

**Compose.** Put the new value in the env file you pass with `--env-file`
(not on the command line), then bring the affected services back:

```sh
docker compose -f deploy/docker-compose.yml --env-file my.env \
  up -d app-role-password pooler-auth pgbouncer core-api
```

The one-shot ALTERs the role, `pooler-auth` rewrites PgBouncer's auth file,
`pgbouncer` restarts onto it, and core-api comes back with the new URL. Verify
from a container on the compose network — not from inside the Postgres
container, where `pg_hba` trusts loopback and *any* password is accepted:

```sh
PGPASSWORD="$NEW" docker run --rm --network marula_default -e PGPASSWORD \
  --entrypoint psql \
  dhi.io/pgvector:0.8-pg18@sha256:069119d1ad25e1155e165786230ef3b4a570f1857b315166924b895c4d2c6946 \
  -h postgres -U marula_app -d marula -tAc 'select current_user'
```

**Helm.** The release reads `POSTGRES_APP_PASSWORD` from a Secret — one you
created when `existingSecret` is set, otherwise the chart's own. Change that
key, upgrade (the `pg-bootstrap` Job is a `pre-upgrade` hook now, and applies
whatever the Secret holds at that moment), then roll the API so its pods read
the new value:

```sh
# existingSecret (the staging profile does this): update the key you own
kubectl -n <ns> create secret generic <secret> \
  --from-literal=POSTGRES_APP_PASSWORD="$NEW" ... --dry-run=client -o yaml \
  | kubectl apply -f -
helm upgrade <release> deploy/helm/marula -n <ns> -f <values>

# chart-generated Secret: the release owns it, so the value goes through
# values and the same upgrade carries it. `--set` also leaves the value in
# your shell history and in the release record; use --set-file from a file
# you delete afterwards if that matters.
helm upgrade <release> deploy/helm/marula -n <ns> -f <values> \
  --set postgresql.appPassword="$NEW"

kubectl -n <ns> rollout restart deployment/<release>-core-api
```

A pooler outside the release needs its auth file re-rendered and the pooler
restarted (`pooler/README.md`); the chart deploys none.

**What breaks, and for how long.** The old password stops working the moment
the ALTER commits. Requests already riding an open pooled connection keep
working; requests that need a *new* database connection fail with an
authentication error until the last client — core-api, and the pooler if you
run one — holds the new value. On compose that is the one `up` above; on Helm
it is the gap between the hook Job finishing and the rollout finishing. Do it
when you can watch it, and roll back by repeating the same steps with the
previous value.

`deploy/check-app-role-secret.sh` asserts the parts of this that a manifest can
show: the hook annotation, the Secret reference, the compose ordering, and
that no userlist is checked in.

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
