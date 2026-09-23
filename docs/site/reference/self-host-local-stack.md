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
| Postgres (pg17 + pgvector) | 15432 | user/db `meridian` |
| Redis | 16379 | `noeviction`, 192 MB cap |
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
`JWT_SECRET`, `MERIDIAN_SECRET_KEY` (worker and core-api must match, or
sealed webhook secrets become undecryptable) and set `EMAIL_INBOUND_HMAC_SECRET`
from the start — the inbound webhook mail path stays disabled until it is.
