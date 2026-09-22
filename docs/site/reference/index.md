# Reference

Reference material is descriptive and exhaustive: it tells you what is true,
structured for lookup, with no narrative and no opinions.

- [API reference](api.md) — every route in the Meridian Core API, generated
  from the committed OpenAPI snapshot
- [The OpenAPI pin](openapi.md) — what the pin is, and the contract it
  enforces
- [Read model view catalog](read-model.md) — the stable `ops.v1_*` SQL views
  (per-view docs under `docs/reference/read-model/`)
- [Upgrade matrix](upgrade-matrix.md) — CI-proven upgrade paths per release
  (the N−2 rule)

## Self-host & operations (runbook spine, authored per `deploy/runbooks/README.md`)

- [The local stack](self-host-local-stack.md) — services, ports, profiles,
  the migration-owner vs RLS-app role split, and secrets to override
- [Observability](self-host-observability.md) — traces/logs/metrics,
  off by default, the profile that turns them on
- [Backup & restore runbook](backup-restore.md) — what to back up, how to
  restore, how to rehearse
- [Reverse proxy & TLS runbook](reverse-proxy-tls.md) — what a production
  proxy must serve and the seams it must survive
- Desktop packaging & release workflow:
  [`docs/desktop-release.md`](../../desktop-release.md) (canonical,
  linked here and never forked)
- Upgrade runbooks: the canonical scaffold lives at
  `deploy/runbooks/` ([`deploy/runbooks/README.md`](../../../deploy/runbooks/README.md)).
  Authored content lands in this directory.

For the narrative *why* behind any of these, see
[explanation](../explanation/index.md). For tasks, see
[how-to](../how-to/index.md).
