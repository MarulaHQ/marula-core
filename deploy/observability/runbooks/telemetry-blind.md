# Telemetry is blind

**Pages:** `MarulaCollectorDown`

## What the alert means

The OTel collector is not answering Prometheus scrapes. Every other rule in the
pack reads series that arrive through it, so from this moment the rest of the
alerts are reading stale values or nothing at all. A green board after this fires
is an empty board.

This alert is the pack defending itself. It is also the one alert that still
works when the product is comfortable, which is why it is a page.

## First three checks

1. Is the collector container up, and what did it say last:

   ```sh
   docker compose ps otel-collector
   docker compose logs --tail=60 otel-collector
   ```

   A config error stops it at start and names the line. A collector that is
   running but not answering is usually out of memory, since its own limiter is
   set to a share of the container limit.

2. Does the exposition endpoint answer from inside the network, which separates
   "collector down" from "scrape target unreachable":

   ```sh
   curl -sS http://localhost:18899/metrics | grep -c '^marula_'
   ```

   A count of zero with the collector healthy means the processes stopped
   exporting, not that the collector stopped collecting. Check the API and worker
   next, starting with `worker-down.md`.

3. Is Prometheus itself the thing that is wrong? Its own target page answers that
   without guessing:

   ```promql
   up{job="otel-collector"}
   ```

   One from a target that answers curl is a stale scrape configuration. Zero from
   both is the collector.

## Escape hatch

Restart the collector. It holds no state worth keeping: traces go to Tempo, logs
to Loki and metrics are re-derived from the next push, so a restart costs the
series that were in flight and nothing else. The API and worker reconnect on
their own and re-export their current values on the next interval.

If the collector will not stay up, drop it and keep the product serving.
`OTEL_EXPORTER_OTLP_ENDPOINT` unset makes both processes run without export,
which loses observability and keeps delivery working. That trade is worth writing
down when you make it, because the next person will read the quiet board as a
quiet system.

## Afterwards

Check whether the product was also failing during the blind window. The API's and
worker's own logs still cover that period, and the incident note should say
plainly which part of the window has no metric evidence.
