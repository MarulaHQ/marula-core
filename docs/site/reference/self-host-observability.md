# Observability: distributed traces, logs, metrics

> **Audience:** site operator / MSP admin — you host Marula yourself and
> own its deployment.

Marula's telemetry is **off by default**. With `OTEL_EXPORTER_OTLP_ENDPOINT`
unset, core-api and the worker run exactly as they did before the
observability work shipped: no OTel SDK, no exporters, no network writes.
This is a posture, not an oversight — a self-hosted PSA core must not phone
home or spend CPU on telemetry you never asked for.

## Turn it on

```sh
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318 \
  docker compose -f deploy/docker-compose.yml --profile observability up
```

The two Marula services export OTLP/HTTP (protobuf). The collector fans
traces out to Tempo, logs to Loki's native OTLP endpoint, and exposes
Prometheus metrics on its scrape target (`:18899`). Grafana (`http://localhost:3001`) arrives preprovisioned with the
Tempo, Loki and MarulaMetrics datasources and a starter dashboard; the demo grants
anonymous admin access, so touch that before exposing it anywhere.

Config files live under `deploy/observability/*.yaml`; the profile's member
services and memory ceilings live in `deploy/docker-compose.yml`.

Error tracking (GlitchTip) has its DSN distribution settled: per-service
`GLITCHTIP_DSN_CORE_API` and `GLITCHTIP_DSN_WORKER` environment variables.
The Settings →
Self-host services pane reports which of the two are configured,
presence only, never the value. The SDK wiring that sends events from
the two Rust binaries is still its own work item, so a configured DSN
means the credential is in place, not that errors flow yet. See
[How to watch your self-hosted services](../how-to/admin-self-host-services.md).

## Uptime Kuma and the status page

The same profile carries Uptime Kuma on `http://localhost:3002`, the one
slot in this profile that answers a question the others cannot. Tempo
tells you how the system behaved after a customer already noticed. Uptime
Kuma probes the endpoints a customer actually loads, so you learn about a
broken login page before they tell you. It is standalone, with its own
store under the `uptimedata` volume and no Marula service in its
dependency order, so it never becomes a link that can stop the core stack
from coming up.

First boot asks you to create the admin account. Do that once, then add
monitors. A reasonable starting set for a self-hosted deployment:

| Monitor | Type | Target |
| --- | --- | --- |
| API health | HTTP(s) | `https://<your-host>/api/v1/health`. |
| Web app | HTTP(s) | `https://<your-host>/`. |
| Sign-in page | HTTP(s) | `https://<your-host>/login`. |

Keep the public status page on a separate hostname from Marula itself. A
status page that goes down with the thing it reports on is worse than no
status page, and customers remember which one it was.

Marula does not create monitors, rotate credentials, or read results out
of Kuma. It ships the slot, the profile, and the port. What you monitor
and who gets paged is your call as the operator.

One honest gap: the service runs with a writable rootfs. Every other slot
in this file pins `read_only` with a named volume for its data. Kuma
writes engine scratch outside `/app/data` at first boot, and pinning the
rootfs would need a boot-verified inventory of what exactly. Capabilities
are still dropped and `no-new-privileges` is still on, so the exposure is
narrower than an unhardened container. It is not the same posture as its
neighbours though, and you should know that before you expose it.
