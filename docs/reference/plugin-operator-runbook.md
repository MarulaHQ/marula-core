# Plugin operator runbook

**Decision of record:** [ADR 0023](../adr/0023-plugin-runtime-isolation.md)
**Related:** [plugin-security-model.md](plugin-security-model.md), [plugin-runtime.md](plugin-runtime.md)

What an operator does with plugins on a running instance: install and grant, run and quarantine,
then roll back or revoke, with the audit trail open beside it. Every step names the surface it happens on and the fact it leaves
behind.

## Where plugins live in the UI

Settings → **Plugins** (`apps/web/app/settings/plugins`). Four tabs matter:

| Tab | What it shows |
| --- | --- |
| Marketplace | the catalog, staged entries waiting for approval |
| Registry | published entries, versions, revocation |
| Installed | every install with its status, grants, version and pin |
| Agent skills | skills a plugin contributes to the agent platform |

Installing is a staged flow, not a button that runs immediately: stage, review the consent screen,
approve. The consent screen renders the manifest's declarations verbatim (capabilities, surfaces,
egress scopes, settings schema) because that is the moment a human decides.

## The lifecycle verbs

Six verbs, each with an API route and a UI action. All six are audited.

| Verb | API | What it does |
| --- | --- | --- |
| install | `POST /api/v1/plugins/installed` | creates a **disabled** install, and refuses a second one for the same catalog entry |
| enable | `PATCH /api/v1/plugins/installed/{id}` | registers the surfaces the grants allow |
| disable | `PATCH /api/v1/plugins/installed/{id}` | removes the surfaces and keeps the install |
| update | `POST /api/v1/plugins/installed/{id}/update` | moves to a version, parking the previous pin |
| rollback | `POST /api/v1/plugins/installed/{id}/rollback` | returns to the parked pin, one step |
| revoke | `POST /api/v1/plugins/installed/{id}/revoke` | terminal, and requires a reason |

Plus `DELETE /api/v1/plugins/installed/{id}` for an uninstall, which takes the install's KV with it.

Two behaviours to know before an incident rather than during one:

- **Install is not enable.** A new install is disabled and registers nothing until enabled, which
  is why a freshly installed plugin appears in no ticket pane.
- **Revoke is terminal.** No verb reverses it, and revoking the catalog entry
  (`POST /api/v1/plugins/catalog/{slug}/revoke`) revokes every install of it in the same
  transaction.

Grants are part of the install record. A grant must be a subset of the manifest's declared
capabilities, and the write is rejected otherwise. A capability that is dropped takes its surfaces with
it on the next surface sync.

## Runtime knobs

Three environment variables, read at boot by the pool (`crates/plugin-host/src/runtime.rs:74`).

| Variable | Values | Default |
| --- | --- | --- |
| `MARULA_PLUGIN_ISOLATION` | `pool`, `in_process`, `disabled` | `pool` |
| `MARULA_PLUGIN_WORKERS` | 1 to 8 | 1 on `all-in-one`, 2 on `split` |
| `MARULA_PLUGIN_WORKER_BIN` | a path | the sibling of the running binary |
| `MARULA_ALL_IN_ONE` | `1` for the single-process profile | unset |

An unknown isolation mode fails the boot rather than silently choosing `pool`: a typo in a security
knob must stop a deployment, not pick for it. `disabled` is the outermost gate — it refuses every
invocation with `plugin_runtime_disabled` before a worker exists.

`in_process` exists for a box that cannot spare a worker's resident set. It changes nothing about
what a plugin may do: same gates, same host functions, same limits.

## Quarantine

An install that takes its worker down three times in a row is quarantined. The rule and the row:

- the counter is *consecutive* failures, and any successful invocation resets it to zero,
- a guest trap is a refusal, not a failure, so it never counts,
- the row lives in `plugin_install_health`, one per install, with `consecutive_failures`,
  `quarantined_at`, `quarantine_reason` and the operator's clear,
- while quarantined, every invocation is refused with `plugin_install_quarantined` before a worker
  is reached, and the counter stops moving so the number an operator reads is the number that
  caused it.

The quarantine decision reads the database at most once per install per thirty seconds, so a clear
reaches every replica inside that window rather than instantly.

**The gap, stated plainly:** there is no admin route to clear a quarantine in this build.
`PluginPool::clear_quarantine` and `PluginPool::quarantine_state` exist and are covered by the
conformance battery, but nothing in `core-api` calls them. Until a route ships, an operator clears
a quarantine on the database, in the install's org scope:

```sql
-- Run inside the install's org scope, as the app role.
UPDATE plugin_install_health
   SET consecutive_failures = 0,
       quarantined_at = NULL,
       quarantine_reason = NULL,
       cleared_at = now()
 WHERE install_id = '<install id>';
```

The pool re-reads within its thirty-second window, so the install runs again without a restart.
Clearing is a decision, not a reset: read the audit rows for the failed invocations first, because
three failures in a row means the artifact is taking its worker down every time and will do it
again.

## Triage by symptom

| Symptom | Likely cause | Where to look |
| --- | --- | --- |
| a widget or action is missing from the UI | install disabled, grant absent, or the surface row was removed | the install's status and grants, then `plugin_surface_entries` for the org |
| an action skipped with `plugin_entry_unavailable` | the install is not enabled, was revoked, or the entry key is wrong | the audit row, then the install row |
| `plugin_not_implemented` | the guest called `emit_event` or `read_model_query`, both deferred | the host-function reference |
| `plugin_fuel_exhausted` | a loop, or 64 host calls spent | the guest, remembering the cap is per invocation and not per install |
| `plugin_wall_clock_exceeded` | a slow host call inside a tight loop | the guest's `http_request` count |
| `plugin_guest_trap` every time | a panic on one input, or a memory cap | the guest's logs up to the trap, which the trap drains too |
| `plugin_worker_failed` repeatedly | the artifact takes its worker down | `plugin_install_health`, expecting quarantine after three |
| `plugin_install_quarantined` | three consecutive worker failures | clear it, after reading why |
| `plugin_rate_limited` | over 300 invocations a minute for one install | the rule that calls it, with a burst of 60 |
| `plugin_egress_profile_missing` | the profile was not consented at install | the pinned manifest's `outbound_scopes` and the consent record |
| `plugin_egress_host_denied` | the URL is outside the allowlist, or resolved outside it | the allowlist, then the guest's URL |
| `plugin_credential_refused` | no binding, a withdrawn category grant, or an empty record | the vault binding for the install, since the cause is logged and not framed |

The audit row is the first stop for all of them: it carries the outcome, the reason, the capability,
the fuel spent, the duration and the worker that answered.

```sql
SELECT occurred_at, actor_kind, actor_id,
       detail->>'outcome'     AS outcome,
       detail->>'reason'      AS reason,
       detail->>'capability'  AS capability,
       detail->>'fuel_used'   AS fuel_used,
       detail->>'worker'      AS worker
  FROM audit_events
 WHERE org_id = '<org id>' AND action = 'plugin.invoke'
 ORDER BY occurred_at DESC
 LIMIT 50;
```

## Capacity

The worker count is the invocation capacity, because a worker answers one invocation at a time
(`crates/plugin-host/src/runtime.rs`). The measured cost of the boundary is small: at the capacity
anchor of 695 requests per second, a pooled invocation adds 0.4 ms at p95 over the in-process path,
inside the ADR's 4.66 ms budget, and the pool's absolute p95 was 12.4 ms against the ADR's own
28.9 ms at the deepest queue.

Two numbers decide whether a deployment needs more workers: the share of requests that invoke a
plugin, and the p95 the org already tolerates. A single worker covers the median request rate at
the anchor. A deployment where every request runs a plugin needs more, and `MARULA_PLUGIN_WORKERS`
is capped at 8 by design so a typo cannot ask for a hundred resident sets.

## Data the runtime holds

| Store | What it contains | How it leaves |
| --- | --- | --- |
| `plugin_installs`, `plugin_versions`, `plugin_catalog` | the install, its pins, the catalog | tenant export |
| `plugin_surface_entries` | the registered surfaces, present only while enabled and granted | re-derived from the manifest on enable |
| `plugin_kv` | per-install key/value rows, quota'd | keyset export and erase, with a surviving receipt |
| `plugin_install_health` | consecutive failures and the quarantine | cascade with the install |
| `audit_events` | one row per attempt | the audit export, incremental with a cursor |

An uninstall removes the install and cascades to its health row. The KV erase is explicit and
leaves a receipt so an operator can prove the data is gone. Retention windows are in
[data-lifecycle.md](data-lifecycle.md).

## Upgrade and rollback

An update moves the install to another version and parks the previous pin, including its digest and
capability set. Rollback returns to that parked pin in one step, and only one: two rollbacks in a
row do not walk further back. An update that would drop a capability the install already holds is
refused, because a silent widening or narrowing of what a running plugin may do is exactly the
change consent exists to prevent.

## Publishing to a registry

Publishing an entry, appending a version, and revoking are admin actions
(`special:manage_plugins` plus an owner or admin role). A registry document verified against a
trusted key lands `manifest_authenticated = true`. One ingested with no trusted key configured is
structural-only and lands `false` whatever it claims.
