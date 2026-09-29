# How to watch your self-hosted services

> **Audience:** admin — this page reads infrastructure state and links out
> to operator tools. Nothing here changes workspace configuration.

The Settings → Self-host services pane answers two operator questions in
one place: is the stack running, and where do I look when it is not. It is
read-only. Every link and probe in it comes from the core-api process's
environment, so what you see is what this deployment actually is.

## Prerequisites

- An owner or admin account. The API rejects everyone else.
- The stack running: the compose file in `deploy/` or the Helm chart.
- For the link cards, the matching URL variable set on core-api (each
  section below names it). Until then a card reads **not configured**,
  which is an honest state, not an error.

## 1. Read the health sweep

Open **Settings → Self-host services** and press **Refresh** to re-probe.
Each row reports one check and one of four states: **up**, **down**,
**not configured**, or **unknown**.

| Row | What it checks |
| --- | --- |
| core-api | The process serving the pane. |
| PostgreSQL | A `SELECT 1` on the API's own pool. |
| Valkey (Redis) | A bounded `PING` over the wire. The row is up only when `+PONG` comes back. |
| realtime hub | `GET /healthz` at `REALTIME_INTERNAL_URL`. |
| worker | `GET /healthz` at `WORKER_INTERNAL_URL`. The worker's per-loop flags (relay, email, queue, delivery, and the rest) ride the same payload. |

**Not configured** means the URL variable for that service is unset on
core-api. Compose sets both internal URLs by default. On the chart, set
`REALTIME_INTERNAL_URL` and `WORKER_INTERNAL_URL` on the core-api workload
to the service DNS names.

A **down** row is the one that needs you. Start with
`docker compose -f deploy/docker-compose.yml ps` and the logs. The detail
text next to the row carries the probe's own error.

## 2. Watch ClamAV quarantine detections

The scanner row shows the mode this API booted with. `none` means inbound
mail is recorded with scan status `skipped` — the bytes are kept unscanned,
and nothing is ever quarantined. That is the standalone-first default.

To turn scanning on:

```bash
# In the compose stack: start the ClamAV slot once, then point the API at it.
docker compose -f deploy/docker-compose.yml --profile stage2 up -d clamav

# core-api environment:
EMAIL_SCAN=clamd        # switches the ScanProvider from none to clamd
CLAMD_ADDR=clamav:3310  # the compose service and clamd's TCP port
```

Restart core-api after setting them. The first ClamAV boot loads its
signature database and can take a few minutes.

Once scanning is on, the pane's list shows every quarantined inbound
message, newest first: subject, sender, the time, and the ingest source
(`webhook/clamd` for mail that arrived through the signed webhook). The
files that rode the message are listed with their sizes. The scanner
returns one verdict per message. It covers the body and every attachment
byte, so those files are what was scanned with the message, not per-file
verdicts.

Two paths land in the list, and both matter: a signature hit, and the
fail-closed path where the scanner was unreachable. A quarantined message
never becomes a ticket or a comment. The pane shows the lifetime
quarantine count and the time of the newest inbound scan. **Never** means
no inbound message has been scanned since the ledger began.

Owners and admins get a notification for each quarantine — one durable
in-app row, plus the email leg when the recipient's preferences allow it.
The kind is named **Malware quarantine** under Settings → Notifications,
so you can turn either channel off per person.

The raw clamd socket stays a command-line probe, not a pane control. From
the host, against the compose port mapping:

```bash
printf 'zPING\0' | nc -w 3 localhost 13310   # expects: PONG
```

## 3. GlitchTip error tracking

GlitchTip's own UI is the right surface, so the pane links to it instead
of forking it. Set the browser-reachable URL on core-api:

```bash
MARULA_GLITCHTIP_URL=http://localhost:8010   # compose's GlitchTip port
```

The card also lists which services have a DSN configured. The DSNs are
per-service (`GLITCHTIP_DSN_CORE_API`, `GLITCHTIP_DSN_REALTIME`,
`GLITCHTIP_DSN_WORKER`), carried as environment variables and never
printed by any surface — the pane reports presence only. Each service
reports its own state: core-api reads its process environment, and
realtime and the worker answer on `/healthz`.

Presence is all the pane can prove today. The SDK wiring that actually
sends events from the three Rust binaries is its own work item. Until it
lands, a **configured** row means the credential is in place, not that
errors are flowing.

## 4. Operator UIs for dashboards, traces, logs and metrics

Marula gives links, not forks. Start the observability profile, then
point the pane at the browser-reachable URLs:

```bash
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318 \
  docker compose -f deploy/docker-compose.yml --profile observability up

# core-api environment (browser URLs, not service names):
MARULA_GRAFANA_URL=http://localhost:3001       # dashboards
MARULA_OTEL_METRICS_URL=http://localhost:18899 # collector scrape target
MARULA_TEMPO_URL=http://localhost:3200         # traces
MARULA_LOKI_URL=http://localhost:3100          # logs
```

The reference page [Observability: distributed traces, logs, metrics](../reference/self-host-observability.md)
covers the profile, the datasources, and the anonymous-admin caveat on
the demo Grafana.

## 5. Garage object storage

Garage's admin API is CLI-first, so the card gives you the facts and the
commands instead of a control panel: the two named volumes (the compose
project prefix applies at runtime), the container mount points, and the
bootstrap sequence, run once from the repo root:

```bash
docker compose --profile stage2 up -d garage
docker compose --profile stage2 exec garage /garage status
docker compose --profile stage2 exec garage /garage layout assign -z dc1 -c 1G <NODE_ID>
docker compose --profile stage2 exec garage /garage layout apply --version 1
docker compose --profile stage2 exec garage /garage bucket create marula
docker compose --profile stage2 exec garage /garage key create marula
docker compose --profile stage2 exec garage /garage bucket allow --read --write marula --key marula
```

The card's **S3 mode** tag comes from core-api's own object-store config,
and the endpoint and bucket rows show what it is pointed at. Access keys
never appear. The admin API reference is linked from the card. Storage
usage reporting lives with the attachment surfaces, not here.

## When a row is down

1. `docker compose -f deploy/docker-compose.yml ps` — is the container
   even up?
2. `docker compose -f deploy/docker-compose.yml logs <service> --tail 50`.
3. Probe the service's own endpoint from the host: core-api at
   `/api/v1/health`, realtime and the worker at `/healthz`.
4. For Valkey, the pane's own detail text names the address it dialed.

## What you learned

- One pane shows the stack sweep, the scan state, the quarantines, and
  the links to every operator UI.
- Scanning is opt-in (`EMAIL_SCAN=clamd` plus `CLAMD_ADDR`). Without it,
  mail is stored with scan status `skipped`.
- GlitchTip, Grafana, Tempo, Loki, and the collector are link-out cards
  driven by `MARULA_*_URL` variables. Garage is CLI-first and stays
  that way.
