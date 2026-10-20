# Upgrade & Compatibility Policy

> **Status:** Adopted 2026-08-26 · applies to all development from adoption onward
> **Enforced by:** CI (`.github/workflows/ci.yml`) plus the gates defined in §8
> **Audience:** contributors, self-hosting operators, extension authors, and commercial products built on this core

## 1. Why this policy exists

Marula asks organizations to bet their operating system on it. That bet is only rational if upgrades are boring. Modular, customizable systems acquire a reputation for being painful to upgrade exactly when compatibility is treated as a hope instead of a property. This policy makes compatibility **bounded and mechanically enforced**: generous where it is cheap to be generous, and explicitly honest about where the guarantees end.

Two commitments define the whole document:

1. **Legacy pathways live for at least one generation.** A generation means one major version. Anything deprecated is announced, kept working through the following major, and only then removed.
2. **Every release is reachable from two versions back.** Before a version ships, upgrading to it directly from either of the two preceding releases must be proven — by CI, on production-like fixtures, not by optimism.

## 2. The compatibility surface (what is actually promised)

Compatibility is a property of boundaries, not of the whole system. Exactly four boundaries are stable, plus one documented read model:

| Surface | Contract | Stability rule |
|---|---|---|
| **REST + SSE API** | Path-versioned (`/api/v1`…). The generated OpenAPI spec (`utoipa`) is the contract, and `@marula-core/sdk` is generated from a committed snapshot of it | A supported API version never breaks inside its support window. New versions are added alongside, never swapped in place |
| **Domain events & webhooks** | Versioned envelope (`id`, `type`, `org_id`, `entity`, `occurred_at`, `payload_schema_ref`, `payload`) emitted once per mutation from the transactional outbox | Payload evolution is additive-only. Consumers are required to tolerate unknown fields (documented contract). Retired event types keep emitting alongside their replacements for one generation |
| **Plugin API (PDK)** | Extism/WASM host functions. Plugins declare their required `api_version` in the manifest | Host functions are add-only — new capability always means a new function, never a changed signature. The host supports PDK generations for one major cycle |
| **Declared customizations** | Custom fields, board statuses (always bound to semantic categories), automations, webhook subscriptions, settings registry values | These are declarative rows, migrated as data under the same rules as §4. See §5 |

Plus the **documented read model**: a stable, versioned set of SQL views over operational data. This is the supported interface for direct SQL access and external reporting tools. It exists because "your database" is a promise — but the promise attaches to the views, not to the physical tables.

**Explicitly not stable, ever:** internal crate APIs, physical table layouts, column names, Redis stream and queue internals, worker job payloads, MCP tool internals beyond the derived protocol surface. Code that reaches past the four boundaries above is unsupported and *will* break without notice. Documenting this clearly is part of keeping §1's promises affordable.

## 3. Versioning & support windows

- **Strict semver.** Breaking changes ship only in major versions. Pre-GA (`0.x`) allows breaking changes at minor boundaries, but each one must still follow the migration rules in §4 (flagged, documented).
- **Deprecation lifecycle.** A deprecation is (a) recorded in release notes, (b) surfaced at runtime (response warning header or structured log), and (c) given a `removal_target` version. Removal happens no earlier than that target, which is always a subsequent major.
- **The N−2 rule.** Each tagged release ships with a CI-proven upgrade path from each of the two previous tagged releases. See §8.
- **Support windows are published**, not folklore: current major (full support), previous major (security fixes + deprecated-path operation), older (upgrade only).

## 4. Database migrations: expand/contract

The migration chain (`crates/core-db/migrations`, SQLx) is forward-only and append-only. The rules:

1. **Additive-first.** New tables, nullable columns, new constraints backed by backfill — ship in ordinary (non-breaking) migrations. Nothing is renamed, retyped, or dropped at introduction time.
2. **Never edit a merged migration.** Corrections are new migrations. History is immutable so that any deployed database can replay it identically.
3. **Destructive steps are flagged and gated.** Any `DROP`/retype/truncate-class statement must carry the `--breaking` filename suffix (mechanically enforced by `scripts/check-migrations.sh`) and be documented with its data impact. Destructive migrations are only permitted during a major-release cycle, and only for constructs whose deprecation window (§3) has elapsed.
4. **Expand, backfill, contract.** Within one major cycle: add the new shape → copy data in ordinary transactions → switch reads/writes → and only in the *next* major, drop the old shape. Compat views may bridge the gap where external readers exist (read model, §2).
5. **Customization-aware.** A migration that touches any construct referenced by customization rows (custom field definitions, automation triggers/actions, webhook filters, settings keys) must carry the transform for those rows in the same transaction. CI cross-checks customization tables for references to anything a migration modifies (§8).
6. **Release artifact.** Every release ships its accumulated migration bundle and the proven upgrade path (§1, commitment 2). Because the chain is linear, jumping N−2 → latest is running the intervening migrations. The engineering effort goes into making the *code-level* transition correct, which is what the CI matrix verifies.

## 5. Customizations are data, not code

Everything an organization configures lives as declarative rows bound to stable semantic primitives — that is why the defaults-first doctrine (semantic status categories, settings registry) matters beyond UX: it keeps customization expressible without forkable logic, which is what makes it migratable.

- **Customization manifest** *(planned)*: a canonical, versioned export describing everything an org has customized. It doubles as backup documentation and as the input to the upgrade preflight.
- **Upgrade preflight** *(planned)*: `marula upgrade --check` runs offline against a manifest + database copy and reports exactly which customizations touch constructs deprecated in the target version — auto-migratable ones flagged as such, judgment-call ones described in plain language. Run before upgrading, not after.
- **Semantic-category guarantee**: renaming boards, statuses, or workflows never alters SLA, billing, automation, or reporting behavior, because those bind to categories, not labels. This is the reason custom workflow changes are *not* breaking changes under this policy.

## 6. Plugins & extensions

- Plugins pin their PDK generation via `api_version` in the manifest. The host supports generations for one major cycle. A stale plugin gets an actionable diagnostic at load, not undefined behavior — the WASM boundary contains the failure.
- Marketplace listings record the range of core versions each plugin was tested against. Staleness is made visible rather than impossible. Third-party plugin behavior itself is outside this project's jurisdiction.
- Connectors are ordinary web applications over the versioned API and signed webhooks. They inherit the API support window automatically.

## 7. Operators: what to expect at upgrade time

1. Read the release notes section **"Breaking & deprecated"** (present in every release, even when empty).
2. Optionally run the upgrade preflight (§5) against a backup.
3. Apply migrations (automatic on container start or via `marula-core-cli`).
4. Supported jump distance: your version may be up to two releases behind. Further jumps apply the same machinery sequentially.

## 8. Enforcement

Shipped and enforced today:

- Migration safety lint: destructive statements rejected unless flagged `--breaking`. Filename-shape check (`scripts/check-migrations.sh` in CI).
- RLS coverage gate. Fmt/clippy `-D warnings`. Full test matrix. Preview environment per PR.

Planned gates (tracked in the private build tracker):

- **Deprecation clock**: `deprecated_in` / `removal_target` metadata checked at build time — a PR fails if it removes anything before its target or carries anything past it.
- **N−2 upgrade matrix**: CI job that restores seeded fixture snapshots (production-like data including customizations and installed plugins) from each of the two prior releases, migrates to HEAD, and runs API/event/plugin conformance suites.
- **Golden conformance suites** per supported API version, generated from pinned OpenAPI snapshots.

## 9. Honest limits

- No guarantee survives direct SQL against physical tables or imports of internal crates. The read model (§2) is the supported path. Everything else is off-contract.
- Very long upgrade chains accumulate cost. If sequential migration becomes impractical at some future major, a rebase path (export via the logical model, reload into a fresh install) will be provided — deliberately deferred until needed.
- Third-party plugins and connectors are governed by their publishers. This policy bounds our surfaces and makes staleness visible, it cannot make third parties maintain their code.

## 10. Adoption status

This policy takes effect immediately for all new work. Nothing in it invalidates existing migrations (the `--breaking` convention is already in use). A retrofit backlog (customization manifest export, upgrade preflight command, read-model views, deprecation metadata, N−2 CI matrix) is tracked in the private build tracker and targeted no later than public beta, when the first external upgrade population exists. Changes to this policy go through `docs/rfc/`.
