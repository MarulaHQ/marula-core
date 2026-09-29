# Self-host reference: the local stack

> **Audience:** site operator / MSP admin — you host Meridian yourself and
> own its deployment.

Allocation: `deploy/docker-compose.yml` is the canonical definition of the
local stack (Postgres, Redis, schema migration, core-api, realtime
WebSocket hub, worker, web console); the header comments there are the
ground truth for every flag listed below. What follows is the descriptive
map — upgrade procedures live in [How to upgrade](../how-to/upgrade.md)
and the [backup/restore runbook](backup-restore.md).

## Services and ports

| Service | Port | Notes |
|---|---|---|
| Postgres (pg18 + pgvector) | 15432 | user/db `meridian` |
| Redis (Valkey) | 16379 | `noeviction`, 192 MB cap |
| core-api | 8080 | the single write surface |
| realtime (WS hub) | 8081 | |
| worker | 8082 | health port |
| web console | 3000 | `NEXT_PUBLIC_API_URL` baked at build |
| Garage (stage2 profile) | 3900-3903 | S3-compatible object storage (S3 API 3900, admin 3903) |
| Mailpit (stage2 profile) | 1025 / 8025 | SMTP sink + web UI |
| otel-collector (observability profile) | 4317 / 4318 / 18899 | OTLP gRPC/HTTP + Prometheus exposition |
| Tempo (observability profile) | 3200 | traces |
| Loki (observability profile) | 3100 | logs |
| Grafana (observability profile) | 3001 | preprovisioned datasources |

## Bring-up

```sh
git clone https://github.com/Marlin-Hofmeyer/meridian && cd meridian
docker compose -f deploy/docker-compose.yml up --build
```

Migrations run once as the table-owner role before core-api starts;
`core-api` itself never holds that role (see "role split" below). The web
console bakes `NEXT_PUBLIC_API_URL` into the client bundle at build time —
behind your own domain, set it to the browser-reachable API URL, not a
container service name.

First bring-up applies the full forward-only migration chain. The desktop
test app additionally seeds the demo org the tutorials use
([ADR 0005](../../adr/0005-local-desktop-test-app.md)).

## Profiles

- **Default:** everything above without the stage-2 slots.
- **`--profile stage2`** adds object storage (Garage) and the Mailpit email
  sink. Default object storage is the local-fs none mode, so plain `up`
  needs no Garage; flip `OBJECT_STORE=s3` when you enable the profile.
- **`--profile observability`** adds the OTel collector, Tempo, Loki,
  Grafana. Telemetry export is disabled by default: it activates only when
  `OTEL_EXPORTER_OTLP_ENDPOINT` is set (e.g. `http://otel-collector:4318`)
  together with the profile. Grafana ships preprovisioned Tempo/Loki/
  MeridianMetrics datasources plus a starter dashboard.

## Role split (why two database roles)

Migrations and the worker connect as the **table owner** (`meridian`) — the
worker's outbox relay must read every organization's rows and therefore
bypasses row-level security. The API connects as `meridian_app`
(RLS-constrained) so tenant isolation is enforced first by the database
itself, with the application as the second line. `meridian_app` and its
grants are provisioned by migration 0001. Do not collapse the two roles.

## Secrets

The compose file carries dev-only defaults for `JWT_SECRET` and the
at-rest seal key. For anything beyond a local test harness, override
`JWT_SECRET`, `MARULA_SECRET_KEY` (worker and core-api must match, or
sealed webhook secrets become undecryptable) and set `EMAIL_INBOUND_HMAC_SECRET`
from the start — the inbound webhook mail path stays disabled until it is.

Optional: asymmetric access-token signing. Set
`MARULA_JWT_EDDSA_PRIVATE_KEY` and `MARULA_JWT_EDDSA_PUBLIC_KEY` (PEM
contents) on core-api, and `MARULA_JWT_EDDSA_PUBLIC_KEY` on realtime, to
sign with Ed25519 instead of the shared secret. `MARULA_JWT_KID` names
the key (default `m1`). The pair must be complete and parse, or core-api
refuses to boot. HS256 keeps verifying during a rollout, so you can
switch without logging anyone out. The [threat
model](../../reference/auth-threat-model.md) §A4 covers when to retire the
shared secret.

## Container security posture

Every container in this stack runs least-privilege. The practical facts an
operator needs:

**Who runs.** All three Meridian images drop to a fixed numeric user at
build time. Core-api, realtime, worker, and the CLI run as the hardened
base's `nonroot` user (uid 65532), the web console as uid 1000, the backup
runner as uid 10003. Nothing runs as root except Postgres' one-time
first-boot init: the official postgres entrypoint needs root to chown its
data directory, then re-execs as `postgres` (uid 70 in the hardened
image) before accepting connections.

**Which images.** Third-party slots use Docker Hardened Images
(`dhi.io/...`, Apache-2.0, free for everyone) wherever the hardened
catalog covers them: Postgres+pgvector, Valkey, Grafana, Tempo, Loki,
ClamAV, and the bootstrap curl image. Garage, Mailpit, GlitchTip, and the
OpenTelemetry collector have no hardened equivalent yet (checked
2026-09-29), so they run upstream images under the same pinning and
least-privilege rules. Note that hardened registry pulls want a `docker
login` with a free Docker account.

**What is writable.** Most services mount a read-only root filesystem and
write only to their own volume or a tmpfs. Valkey writes its data volume,
the web console and the Rust services get tmpfs scratch only (core-api
additionally writes the object-store directory when `OBJECT_STORE=local`).
Three services keep a writable rootfs because a read-only one breaks their
own boot logic, and each says so in `deploy/docker-compose.yml`: Postgres
(init), ClamAV (signature database setup), Grafana (sqlite and log state).
`cap_drop: [ALL]` and `no-new-privileges` apply to every service,
exemptions included.

**Volumes and permissions.** If you pre-create Docker volumes or bind-mount
host directories for Garage, the object store, or backups, chown them to
the service uid listed above (Garage runs as 10004, backup dumps as 10003,
valkey and the observability slots as 65532).
A volume owned by root that a non-root uid cannot write is the most common
first-boot failure after enabling a profile.

**Version pinning.** Every image reference is pinned: an exact tag plus a
registry digest, in the three Meridian Dockerfiles and every third-party
slot in the compose file and Helm chart. When you bump a version, move the
tag and the digest together. The digest is what actually runs.

**Kubernetes.** The Helm chart sets the same posture: pod-level
`runAsNonRoot` with per-component `runAsUser` and `fsGroup`, and per
container `allowPrivilegeEscalation: false`, `capabilities.drop: [ALL]`,
seccomp `RuntimeDefault`, and `readOnlyRootFilesystem` where the service
tolerates it. The chart refuses to render unless every image reference
carries a digest (`digestPinningEnforced`, on by default). That gate covers
every bundled third-party image, curl bootstrap job included.
