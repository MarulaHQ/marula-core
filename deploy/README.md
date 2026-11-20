# deploy

> **Purpose:** docker-compose.yml (Postgres+pgvector, Valkey, MinIO (helm; Garage in compose), Mailpit, services, web), Helm chart, upgrade runbooks, self-host backup/DR guide, and the TLS edge in `edge/`.
> **Non-purpose:** No SaaS/Kubernetes control-plane infrastructure — managed multi-tenant hosting is a commercial concern outside this repo.

## What is here

| Directory | What it is |
|---|---|
| `docker-compose.yml` | The local stack: Postgres, Valkey, migrations, core-api, worker, web, plus the `pooler`, `stage2`, `observability`, `ai` and `backup` profiles. It ships no secret value and requires eight (`JWT_SECRET`, `MARULA_SECRET_KEY`, the two seam tokens, `POSTGRES_PASSWORD`, `MARULA_APP_PASSWORD`, `GARAGE_RPC_SECRET` and `GLITCHTIP_SECRET_KEY`). No TLS and no published-proxy boundary, because it targets development. |
| `dev.env` | The dev marker as a file: the repository's published dev values plus `MARULA_DEV_TOOLS=1`, passed with `--env-file`. Never for a host you do not own. The stack warns at boot while it is in use. |
| `pooler/` | The PgBouncer config for installs that turn the `pooler` profile on: transaction mode, `DISCARD ALL` after every transaction, no prepared-statement cache. `docs/reference/connection-pooling.md` says why each setting is load-bearing. |
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
value, and an existing one had `marula`). `MARULA_APP_PASSWORD` is generated
like the rest. The `app-role-password` one-shot applies it to the `marula_app`
role as the owner after migrations and before core-api starts, so it does not
have to match anything the volume already holds. "Rotating the app role
password" below is the rotation path.

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

`edge/README.md` has the whole decision: the three TLS modes, the
Cloudflare dashboard steps behind the edge one, and the origin lockdown.
The operator runbook is
[`docs/site/reference/reverse-proxy-tls.md`](../docs/site/reference/reverse-proxy-tls.md).

## Connections

A single-node stack dials Postgres directly. core-api and the worker hold 54
connections per instance, which fits inside the 100 a stock Postgres allows, so
the default install does not start a pooler or ask anyone to read about one.
That is what `MARULA_APP_DATABASE_URL` and `MARULA_WORKER_APP_DATABASE_URL`
point at unless you say otherwise.

The `pooler` profile adds PgBouncer in transaction mode for installs that need
it: several API instances, or many concurrent clients against a database that
caps connections. Turn it on, then set `MARULA_APP_DATABASE_URL` and
`MARULA_WORKER_APP_DATABASE_URL` in your env file to the same credentials with
`pgbouncer:6432` as the host:

```sh
docker compose -f docker-compose.yml --env-file my.env --profile pooler up -d
```

Transaction mode keeps 15 server connections per (database, user) pair instead
of one per client, which is what keeps the count flat as instances are added.
It also means the application cannot keep prepared statements, so
`MARULA_DB_STATEMENT_CACHE=0` goes with the profile. The migration job and each
process's one `LISTEN` connection stay direct, because neither can ride a pooled
session. `pooler/README.md` is how to run and inspect it, and
[`docs/reference/connection-pooling.md`](../docs/reference/connection-pooling.md)
has the reasoning and the measurements.

## Rotating the app role password

`MARULA_APP_PASSWORD` is the one role password the stack applies itself, so a
rotation is a value change and a restart. No migration and no hand-edited file.
Compose applies it in the `app-role-password` one-shot, Helm in the
`pg-bootstrap` Job. Both re-run on every deploy, which is why nothing has to be
remembered afterwards.

**Compose.** Put the new value in the env file you pass with `--env-file`
(not on the command line). If you set `MARULA_APP_DATABASE_URL` by hand, change
the password inside that URL as well: the variable is the whole URL, so a new
`MARULA_APP_PASSWORD` never reaches it. The worker's URL dials the owner role by
default, so this rotation never touches it; if you pointed the worker at
`marula_app` instead, change the password inside
`MARULA_WORKER_APP_DATABASE_URL` too and add `worker` to the commands below.
Then bring the affected services back:

```sh
docker compose -f deploy/docker-compose.yml --env-file my.env \
  up -d app-role-password core-api
```

With the `pooler` profile on, the same run has to re-render the auth file,
recreate core-api against the new URL, and restart PgBouncer so it reads that
file:

```sh
docker compose -f deploy/docker-compose.yml --env-file my.env \
  --profile pooler up -d app-role-password core-api pooler-auth
docker compose -f deploy/docker-compose.yml --env-file my.env \
  --profile pooler restart pgbouncer
```

A plain `up -d` leaves PgBouncer alone when its own service definition has not
changed, and a running pooler keeps the auth file it read at start. The explicit
`restart` is what makes it read the new one.

Verify from a container on the compose network, not from inside the Postgres
container, where `pg_hba` trusts loopback and *any* password is accepted:

```sh
PGPASSWORD="$NEW" docker run --rm --network marula_default -e PGPASSWORD \
  --entrypoint psql \
  dhi.io/pgvector:0.8-pg18@sha256:069119d1ad25e1155e165786230ef3b4a570f1857b315166924b895c4d2c6946 \
  -h postgres -U marula_app -d marula -tAc 'select current_user'
```

**Helm.** The release reads `POSTGRES_APP_PASSWORD` from a Secret. Change that
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
restarted (`pooler/README.md`). The chart deploys none.

**What breaks, and for how long.** The old password stops working the moment
the ALTER commits. Requests already riding an open pooled connection keep
working. Requests that need a *new* database connection fail with an
authentication error until the last client holds the new value: core-api, and
the pooler if you run one. On compose that is the one `up` above. On Helm it is
the gap between the hook Job finishing and the rollout finishing. Do it when you
can watch it, and roll back by repeating the same steps with the previous value.

`deploy/check-app-role-secret.sh` asserts the parts of this that a manifest can
show: the hook annotation, the Secret reference, the compose ordering, and
that no userlist is checked in.

## Operator reference

### Images

Every reference carries an exact tag and a registry digest, and the digest is
what runs. The four Marula images are built per release and published to GHCR,
where they pull without a token once the package is public. That visibility is
a package setting the publish job cannot make for you, and
[`docs/release-images.md`](../docs/release-images.md) says so. The third-party
slots come from Docker Hardened Images wherever the hardened catalog covers
them. Those images are free and pull without an account. If a pull ever asks for
credentials, `docker login dhi.io` with a free Docker account is the fallback.

| Image | Source | Notes |
|---|---|---|
| `ghcr.io/marulahq/marula-core-api` | Release pipeline | Built from `Dockerfile.rust`. Digest comes from the release's `release.json`. |
| `ghcr.io/marulahq/marula-worker` | Release pipeline | Same Dockerfile, `BIN=worker`. |
| `ghcr.io/marulahq/marula-migrate` | Release pipeline | Same Dockerfile. Runs the `marula-core-cli` binary. |
| `ghcr.io/marulahq/marula-web` | Release pipeline | Built from `Dockerfile.web`. |
| `dhi.io/pgvector:0.8-pg18` | Docker Hardened Images | Postgres 18 with pgvector. Pulls anonymously. |
| `dhi.io/valkey:8.1.10` | Docker Hardened Images | Redis-compatible cache. |
| `dhi.io/pgbouncer:1.26` | Docker Hardened Images | Only with the `pooler` profile. |
| `dhi.io/grafana:11`, `dhi.io/tempo:2.7`, `dhi.io/loki:3.6`, `dhi.io/clamav:1.5.4` | Docker Hardened Images | `observability` and antivirus slots. |
| `dxflrs/garage:v1.1.0` | Upstream | Object store for `stage2`. No hardened equivalent yet. |
| `axllent/mailpit:v1.31.3` | Upstream | Mail sink for `stage2`. |
| `otel/opentelemetry-collector-contrib:0.118.0`, `prom/prometheus`, `glitchtip/glitchtip:6.2.6`, `louislam/uptime-kuma:2.5.5` | Upstream | `observability` profile. |
| `ollama/ollama:0.40.7` | Upstream | Local model runtime for the `ai` profile. |

A release's `release.json` names the four application images by digest, and the
installer pins those digests rather than a tag. `docs/release-images.md` covers
how the pipeline produces them and why a digest is the identity.

### Secrets

The stack refuses to render without these eight. The installer generates any
that are missing and keeps what an existing `marula.env` already holds, so
applying twice does not rotate a key.

| Secret | What it protects | What a rotation costs |
|---|---|---|
| `JWT_SECRET` | Signs access and refresh tokens. | Everyone signs in again. |
| `MARULA_SECRET_KEY` | Encrypts sealed integration secrets at rest. | Sealed webhook and connector secrets become unreadable. core-api and the worker must hold the same value. |
| `MARULA_INTERNAL_TOKEN` | The bearer half of the worker-to-API seam. | Restart both processes with the new value together. |
| `EMAIL_INTERNAL_TOKEN` | The email half of the same seam. | Same as above. |
| `POSTGRES_PASSWORD` | The `marula` owner role. | Postgres applies it when it creates the data volume. A later file change does not change the role. |
| `MARULA_APP_PASSWORD` | The `marula_app` role. | The `app-role-password` one-shot applies it on every start, so a rotation is a value change and a restart. |
| `GARAGE_RPC_SECRET` | Garage's inter-node RPC secret. | Restart the Garage nodes together. |
| `GLITCHTIP_SECRET_KEY` | GlitchTip's Django signing key. | Signs out the tracker's own sessions, not Marula's. |

### Profiles

`docker compose up` with no `--profile` flag boots the base stack. Everything
else is opt-in.

| Profile | What it adds | Why you would turn it on |
|---|---|---|
| `pooler` | PgBouncer and its auth renderer | Several API instances, or many clients against a capped Postgres |
| `stage2` | Garage object storage and a Mailpit mail sink | Attachments on the host, or mail captured locally |
| `observability` | OTel collector, Tempo, Loki, Prometheus with the alert pack, Grafana, GlitchTip, Uptime Kuma | Traces, logs, metrics and a status page on the box |
| `ai` | A local Ollama runtime | A local embedding or chat model |
| `backup` | WAL-G and restic | Scheduled backups and point-in-time recovery |

`edge` is not a profile of this file. It is the overlay in
`edge/compose.edge.yml`, which publishes 80 and 443 and removes the base file's
raw port mappings. The command that layers it is under "Starting a public
deployment" above, and [`edge/README.md`](edge/README.md) explains the TLS modes.

The pooler stays out of the default for the reason above: one instance's
connections fit, and a second moving part in front of Postgres is a thing to
operate. Add it when the count stops fitting, not before.

## Stopping the stack

`core-api`, `worker` and `web` run with `restart: unless-stopped` and a 30s
`stop_grace_period`. A crash brings the container back. A `docker compose stop`
or `down` sends SIGTERM, and the process drains for up to
`MARULA_SHUTDOWN_GRACE_SECS` (20s by default) before exiting. The binaries log
what finished and what they cut.

The compose stack has no container healthcheck for those three services: the
runtime image carries no shell and no `curl`, so a `HEALTHCHECK` could not
exec anything. The Helm chart is where HTTP probes live (`/livez`, `/readyz`,
`/healthz`).
