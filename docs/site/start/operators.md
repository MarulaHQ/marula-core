---
description: "You are not the workspace admin. Admins configure boards, email routing and people."
---

# Operator tasks

> **Audience:** site operator — you run the instance. You have shell access, you
> own the backup and the upgrade window, and you get the call when it is down.

You are not the workspace admin. Admins configure boards, email routing and
people. You keep the process up, the data recoverable and the front door
correct. Where the two overlap, this page says so.

## "Stand it up"

- [Run the local demo stack](../how-to/local-demo-stack.md) — a seeded copy on one
  machine, for trying the product before you bet a client on it.
- [The local stack](../reference/self-host-local-stack.md) — services and ports,
  profiles, the migration-owner vs RLS app-role split, container hardening, and
  the secrets you must set.
- [Run one process instead of two](../../all-in-one.md) — for a demo or a one-box
  install.
- [The deploy descriptor](../../deploy-descriptor.md) — every runtime parameter in
  one machine-readable place, which is what an automated deployment reads.

## "Put it on the internet"

[Reverse proxy and TLS](../reference/reverse-proxy-tls.md) is the page to read
before the first DNS record. It covers the required upstream routes, the streamed
live leg and why it must not be buffered, upload size limits, Direct-mode ACME
against Cloudflare Edge mode, origin IP-range lockdown, and the trusted-proxy
settings that make a resolved client IP mean anything.

## "Back it up, and prove the backup works"

1. [Backup and restore](../reference/backup-restore.md) — `pg_dump`, object
   storage, `restic` or WAL-G for point-in-time recovery.
2. Rehearse the restore into a scratch database. The same page has the verify
   steps, and a restore you have not run is a hope rather than a backup.
3. Alert on archive silence. The runbook names the specific failure: an archive
   pipe that breaks keeps logging as if it worked.

## "Upgrade"

- [Upgrade to the latest release](../how-to/upgrade.md) — preflight, backup and
  migrations, digest-pinned images and verification, plus what to do when stuck.
- [The upgrade compatibility policy](../explanation/upgrade-policy.md) — the N−2
  window and the expand-then-contract rule that makes it safe.
- [Upgrade matrix](../reference/upgrade-matrix.md) — the paths CI actually
  verifies, so you know whether your jump is a tested one.
- [Postgres 17 to 18](../reference/pg18-upgrade.md) — the one-time volume cut,
  only if your deployment predates it.

## "Watch it"

- [Database observability](../reference/db-observability.md) — the query mix and
  the slow tail, the `EXPLAIN` signal, and scheduled maintenance inside
  PostgreSQL.
- [Observability](../reference/self-host-observability.md) — the optional
  OpenTelemetry profile (Tempo, Loki, Grafana) plus status monitoring, and the
  endpoint variable that turns export on.
- [Watch your self-hosted services](../how-to/admin-self-host-services.md) —
  container health probes, the ClamAV quarantine view, and links to the dashboards.
- [Perf program](../../perf.md) — the budgets and the harness, if you want to
  measure your install against a recorded baseline instead of guessing.
- [Capacity model](../../capacity-model.md) — how many agents one instance serves,
  from measurements.
- [Worker cadence](../../worker-cadence.md) — which loops run how often, and what a
  small install can trim.

## "Mail is being refused, or scanned wrong"

- [Configure email](../how-to/admin-email.md) — section 7 covers the optional
  malware scanner, section 8 is the self-host environment block.
- [Watch your self-hosted services](../how-to/admin-self-host-services.md) — the
  quarantine view, for when a message was held rather than delivered.

## "Run models on our own hardware"

- [Local AI](../reference/self-host-local-ai.md) — why weights stay outside the
  container image, and how the `ollama` profile serves transcription and
  embeddings.
- [Manage AI access](../how-to/admin-ai-access.md) — the switch that decides
  whether any of it is reachable, which is an admin decision, not yours.

## "It is down"

1. [Troubleshooting](../troubleshooting.md) — symptom first, with the checks in
   the order that separates a proxy problem from an application problem.
2. [Backup and restore](../reference/backup-restore.md) — the restore path, if the
   database is the part that is gone.
3. [Self-hosted services](../how-to/admin-self-host-services.md) — the health
   probes that say which container is unhappy.

## Not documented yet

This is the thinnest area on the site, and the gaps are worth naming plainly.

- Incident runbooks: no page walks an outbox backlog, an undelivered email queue,
  a dead-letter pile, or Postgres or Valkey being unavailable. The observability
  page says what is exported; it does not say what to do when a metric is bad.
- Alert rules: nothing in the docs ships a recommended threshold or an alert
  definition. A dashboard panel is not an alert.
- Retention and RPO/RTO: no page states how long data is kept, what the backup
  window guarantees, or what a restore costs in time.
- The restore drill is manual. Nothing schedules it or proves it ran.
