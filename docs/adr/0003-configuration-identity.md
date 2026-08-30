# ADR 0003 — Configuration identity: feeds upsert, curated beats observed

**Status:** Accepted · 2026-08-30
**Scope:** device/configuration records (`ConfigItem`), RMM/NMS feeder
integrations, and field-level data ownership

## Context

MSPs feed device and configuration data into the PSA from several systems at
once — an RMM (NinjaOne), a documentation platform (IT Glue/Hudu), an NMS
(Auvik) — and none of those systems can agree on what a "device" is. Without
an identity model, the same physical server arrives as three records, feeds
either duplicate data or clobber human edits, and the asset list quietly
becomes unreliable. The trap is symmetric: blind auto-merge annoys users with
wrong guesses, and no merge at all produces duplicates.

The `Money`/conversion model (ADR 0002) established the house pattern for
facts that many systems touch: store the original fact, resolve identity, and
never let an automation silently erase a human decision. Configurations need
the same treatment.

## Decision

- **Feeds upsert through source links, never blind-insert.** Every feeder
  adapter writes through `ConfigSourceLink` — (source system, external id,
  last-seen, payload reference) — bound to one canonical `ConfigItem`. A CI
  carries 0..N source links; a source's second sync updates its own link, it
  never creates a twin.
- **Deterministic matching first, AI judgment second.** Adapters declare
  stable identity keys per entity type (source GUID, serial/service tag, MAC
  set, hostname+domain). Low-cost deterministic rules resolve the common case
  (same source link, exact serial); ambiguous matches become **merge
  candidates with visible evidence** in a review queue — the REL-M2
  confidence-dashboard pattern applied to daily operations. Auto-merge only
  above an org-configurable confidence threshold; every merge is audited and
  undoable (ADR-compatible with the undo contract in `instant-first.md`).
- **Curated beats observed.** Every CI field has two tiers: **observed**
  (feed-written, timestamped, churns silently with its source) and **curated**
  (human-set or human-approved, sticky — never auto-overwritten by any feed).
  Strong observed evidence contradicting a curated value surfaces a one-click
  "accept the new value", never a silent change and never a silent ignore.
  Field-level precedence per source is configurable per CI type (defaults:
  serial ⇒ RMM wins; IP/last-seen ⇒ NMS wins; owner/location ⇒ human wins).
- **CI lifecycle rule.** A thing is its own CI when it has an independent
  lifecycle — its own identity (serial/service tag/MAC), fails, manages, or
  bills independently. Otherwise it is a structured attribute of a CI:
  general-purpose NICs are `CiInterface` records (name, MAC, IPs, VLANs) on
  the CI; iDRAC/iLO is its own CI (management IP, credentials, firmware
  lifecycle) linked to its server by a **composition** relationship. Typed
  relationships: part-of, runs-on (VM→host), connects-to (topology).
- **Categories bind to semantic classes, not labels.** CI types are
  org-configurable but bound to semantic classes (workstation, server, VM,
  network-appliance, printer, IoT); type controls field schema and identity
  keys, and renaming a type never changes behavior.

## Consequences

- Schema lands with **MER-P2** assets; feeder adapters (NinjaOne, IT Glue,
  Auvik) conform to source links + identity keys when assets ship.
- Deterministic matching heuristics stay in the open core (the community can
  tune and trust them); AI-assisted resolution is a Relay layer over the same
  queue, and proposed links honor the agent-approval default.
- The merge queue must not become a chore: high-confidence auto-merge plus
  batch actions are part of the design, and every suggested merge carries a
  reason a human can verify or reject in one click.