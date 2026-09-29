# Event golden fixtures (policy §2 / TASKLIST U2)

Purpose: the pinned wire shape of every payload Marula publishes on the
domain-event stream, one file per (event type, entity, variant) under
`v1/` — the additive-only conformance baseline. The envelope's
`payload_schema_ref` (`core-domain/v1`) names this generation.

Format: `v1/<event_type with dots as underscores>[@__variant].json` — each
document carries `event_type`, `entity`, `variant`, `payload_schema_ref`,
`pinned_in` (workspace version that last regenerated it), and the `payload`
itself with deterministic sample values. Variant suffixes (`__deleted`,
`__via_automation`, …) cover event types that publish multiple stable
payload shapes.

Contract enforced by `crates/core-events/tests/event_conformance.rs`:
payloads evolve additively only; consumers tolerate unknown fields; retired
event types co-emit with their replacement for one generation (registered in
`crates/core-events/src/contract.rs::RETIRED_EVENTS` and root
`deprecations.json`).

Regenerate only when a payload shape deliberately changes — review the diff:
`cargo test -p core-events --test event_conformance generate_golden_fixtures -- --ignored`.
A regeneration that removes or renames keys is BREAKING: it needs a
release-notes "Breaking & deprecated" entry (policy §3a) and, when retiring
a type, a deprecation-registry entry (policy §3c).
