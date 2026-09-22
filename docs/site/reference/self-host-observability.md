# Observability: distributed traces, logs, metrics

> **Audience:** site operator / MSP admin — you host Meridian yourself and
> own its deployment.

Meridian's telemetry is **off by default**. With `OTEL_EXPORTER_OTLP_ENDPOINT`
unset, core-api, realtime, and the worker run exactly as they did before the
observability work shipped: no OTel SDK, no exporters, no network writes.
This is a posture, not an oversight — a self-hosted PSA core must not phone
home or spend CPU on telemetry you never asked for.

## Turn it on

```sh
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318 \
  docker compose -f deploy/docker-compose.yml --profile observability up
```

The three Meridian services export OTLP/HTTP (protobuf). The collector fans
traces out to Tempo, logs to Loki's native OTLP endpoint, and exposes
Prometheus metrics on its scrape target (`:18899`). Grafana (`http://localhost:3001`) arrives preprovisioned with the
Tempo, Loki and MeridianMetrics datasources and a starter dashboard; the demo grants
anonymous admin access, so touch that before exposing it anywhere.

Config files live under `deploy/observability/*.yaml`; the profile's member
services and memory ceilings live in `deploy/docker-compose.yml`.

Error tracking (GlitchTip) is stub-documented in the compose file but
deliberately not wired: no Meridian code references it until the DSN
distribution decision for self-host vs managed is made. Do not expect an
error pipeline in this profile yet.
