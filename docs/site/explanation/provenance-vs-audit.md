# Provenance vs audit: two append-only ledgers, different granularity

> **Audience:** both — staff read provenance during disputes; admins own
> both surfaces and the query posture.

Meridian keeps **two** permanent records, and both are append-only. They
are different tools that answer different questions, and confusing them
is the most common documentation question the surfaces generate. This
page keeps them distinct. (Their mechanics, provenance as a field-level write record and audit
as an event-level ledger, complement the
[customizations-are-data](customizations-are-data.md) doctrine: both
ledgers exist *because* nothing else gets rewritten.)

## Provenance: field-level, per applied write

`/provenance`'s own banner scopes it, quoted in full to keep every
verified word:

> "field-level provenance query — who changed what, field by field,
> across every applied write — including agent sessions and imports…
> never rewrites history"

Concretely: provenance answers **"how did this field get to its current
value, and through which applied write?"** The query rows carry
Before→After value chips plus a Seq (sequence) and an **actor kind**. The
actor kinds are explicit: `human`, `agent`, `import` — who did the write
as a first-class column, no guessing about "somehow the row changed".

Use it when the question is about **a record**: how a client's address
moved from A to B, who wrote an asset's field, whether an agent or a
human performed a change.

## Audit: event-level, per security lifecycle event

`/audit` carries a different sentence:

> "Append-only ledger of security, administrative and domain lifecycle events for the org."

Event-level, with filters that show the shape. Action-prefix chips
(`auth.*`, `secret.*`, `plugin.*`, `portal.*`, `billing.*`), an actor
UUID-or-prefix filter, a from/to date range, and Apply/Clear. Rows carry
human/system actor chips plus an Inspect detail expander and pagination.

Audit answers **"what happened in the life of this org, as events"** —
sign-ins and their outcomes, plugin mutations, billing events, invites,
secrets access. It is the administrative and security record, not a
field-by-field history of config items.

## The granularity divide in one table

| | Provenance | Audit |
|---|---|---|
| Unit | one applied write | one event |
| Scope | every applied write, field by field | lifecycle- and security-shaped events |
| Actor kinds | human / agent / import | human / system |
| Question it answers | "who changed this field, from what, to what, when" | "what security/administrative/domain events happened" |
| Natural consumer | a record's dispute or lineage | a compliance or forensics sweep |

## The shared promise: append-only, never rewritten

Both records share one guarantee the products lean on: **nothing in
either ledger is rewritten**. Corrections do not edit history; later
writes append. That is why downstream surfaces (the FX rate book, the
issue ladder, time-entry locks) can all make their own "never rewrites
history" claims and not contradict anything: no one in the stack edits
the record of the past anywhere.

Agents and imports being provenance actor kinds (not audit-only noise)
is the same promise applied to non-human writers: when an agent session
runs a tool, or a batch import writes rows, each applied write lands with
its actor kind — treated like any other write, no shadow trail.
