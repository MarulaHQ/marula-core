# Reference

Reference pages give you exact endpoints, database view columns, environment variables, and operator runbooks for lookup. For step-by-step lessons, see [Tutorials](../tutorials/index.md). For task checklists, see [How-to guides](../how-to/index.md). For design reasoning, see [Explanation](../explanation/index.md). For the vocabulary the rest of the site assumes, start with the [glossary](glossary.md).

## Glossary and lookup

- [Glossary](glossary.md) — the recurring PSA and Marula terms, each linked to the page that explains it. Boards and stages are in there, as are contract and billing vocabulary, provenance and audit, grants, plugins, and the N−2 window.
- [Troubleshooting](../troubleshooting.md) — symptom first, for when something is broken rather than unconfigured. It points back into these pages.

## API, data views, and schema reference

- [API reference](api.md) — Every endpoint in `core-api`, generated directly from the committed OpenAPI specification.
- [The OpenAPI pin](openapi.md) — How `packages/sdk/openapi.json` and `fixtures/openapi/v1/openapi.json` stay locked in sync.
- [Read model view catalog](read-model.md) — The stable `ops.v1_*` SQL views for custom reports and external BI tools (with per-view column definitions in `docs/reference/read-model/`).
- [Billing schema reference](billing-schemas.md) — Tables and constraints for work-type categories, billing verification, contract pricing, and project milestone billing.
- [Routing & travel reference](routing.md) — Environment variables, organization settings keys, warning codes, and endpoints for technician travel estimates and mileage reimbursement.
- [Upgrade matrix](upgrade-matrix.md) — Automated CI upgrade paths verified across releases under the N-2 support policy.

## Self-hosting and operations runbooks

- [The local stack](self-host-local-stack.md) — Docker Compose services, ports, optional profiles, the PostgreSQL migration-owner vs. RLS app-role split, container hardening, and required secrets.
- [Local AI: transcription and embeddings on your own hardware](self-host-local-ai.md) — Why model weights stay outside the core container image and how the local `ollama` profile runs Whisper and 768-dimension embeddings.
- [Observability](self-host-observability.md) — Optional OpenTelemetry export (`Tempo`, `Loki`, `Grafana`) plus `Uptime Kuma` status monitoring.
- [Backup & restore runbook](backup-restore.md) — How to back up PostgreSQL and Garage object storage using `pg_dump`, `restic`, or `WAL-G` point-in-time recovery, and how to rehearse restores with `deploy/backup/restore.sh`.
- [Postgres 17 to 18 upgrade](pg18-upgrade.md) — One-time data migration guide for deployments created before the PostgreSQL 18 transition.
- [Database observability](db-observability.md) — The query mix (`ops.query_tail`), the plan of one statement (`ops.explain_plan`), the signal readiness view (`ops.db_signals`), scheduled maintenance, and which of these a managed Postgres will not let you enable.
- [Reverse proxy & TLS runbook](reverse-proxy-tls.md) — The public front door: required upstream routes, the streamed SSE live leg, upload size limits, Direct-mode ACME, Cloudflare Edge mode (Origin CA vs DNS-01), origin IP-range lockdown, and the trusted-proxy settings that make the resolved client IP mean something.
- Desktop packaging and release workflow: [`docs/desktop-release.md`](../../desktop-release.md).
- Runbook scaffold index: [`deploy/runbooks/README.md`](../../../deploy/runbooks/README.md).
