# ADR 0003 — Configuration identity: feeds upsert, human-owned beats observed

**Status:** Accepted · 2026-08-30
**Scope:** device/configuration records (`ConfigItem`), RMM/NMS feeder
integrations, and field-level data ownership

## Context

MSPs feed device and configuration data into the PSA from several systems at
once: an RMM (NinjaOne), a documentation platform (IT Glue/Hudu), an NMS
(Auvik). No two of those systems agree on what a "device" is. Without an
identity model, the same physical server arrives as three records, feeds either
duplicate data or clobber human edits, and the asset list quietly becomes
unreliable. The trap is symmetric. Blind auto-merge annoys users with wrong
guesses. No merge at all produces duplicates.

The `Money`/conversion model (ADR 0002) established the house pattern for facts
that many systems touch: store the original fact, resolve identity, and never
let an automation silently erase a human decision. Configurations need the same
treatment.

## Decision

- **Feeds upsert through source links, never blind-insert.** Every feeder
  adapter writes through `ConfigSourceLink` (source system, external id,
  last-seen, payload reference) bound to one canonical `ConfigItem`. A CI
  carries zero or more source links. A source's second sync updates its own
  link, and it never creates a twin.
- **Deterministic matching first, AI judgment second.** Adapters declare stable
  identity keys per entity type (source GUID, serial/service tag, MAC set,
  hostname+domain). Low-cost deterministic rules resolve the common case (same
  source link, exact serial). Ambiguous matches become **merge candidates with
  visible evidence** in a review queue. Auto-merge only runs above an
  org-configurable confidence threshold, and every merge is audited and
  undoable (the undo contract in `instant-first.md`).
- **Human-owned beats observed.** Every CI field has two tiers. **Observed**
  fields are written by their feeder and churn silently with their source;
  each one carries a timestamp. **`Curated`** fields are human-set or
  human-approved: they are sticky, and no
  feed auto-overwrites them. Strong observed evidence against a human-owned value
  surfaces a one-click "accept the new value", never a silent change and never
  a silent ignore. Field-level precedence per source is configurable per CI
  type (defaults: serial ⇒ RMM wins; IP/last-seen ⇒ NMS wins; owner/location
  ⇒ human wins).
- **CI lifecycle rule.** A thing is its own CI when it has an independent
  lifecycle: its own identity (serial/service tag/MAC), and it fails, manages,
  or bills independently. Otherwise it is a structured attribute of a CI.
  General-purpose NICs are `CiInterface` records (name, MAC, IPs, VLANs) on
  the CI they serve. An iDRAC/iLO is its own CI (management IP, credentials,
  firmware lifecycle) linked to its server by a **composition** relationship.
  Typed relationships: part-of, runs-on (VM→host), connects-to (topology).
- **Categories bind to semantic classes, not labels.** CI types are
  org-configurable but bound to semantic classes (workstation, server, VM,
  network appliance, printer, IoT). Type controls field schema and identity
  keys, and renaming a type never changes behavior.

## Consequences

- Schema lands with the asset work. Feeder adapters (NinjaOne, IT Glue, Auvik)
  conform to source links plus identity keys when assets ship.
- Deterministic matching heuristics stay in the open core, so the community can
  trust and tune them. AI-assisted resolution is a commercial-edition layer
  over the same queue, and proposed links honor the agent-approval default.
- The merge queue must not become a chore. High-confidence auto-merge plus
  batch actions are part of the design, and every suggested merge carries a
  reason a human can verify or reject in one click.
