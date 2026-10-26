---
description: "Marula asks organizations to bet their operating system on it. That bet is only rational if upgrades are boring. Two commitments define the whole document:"
---

# The upgrade compatibility policy

> The binding text is
> [`docs/upgrade-compatibility-policy.md`](../../upgrade-compatibility-policy.md).
> This page explains the reasoning; the [upgrade how-to](../how-to/upgrade.md)
> tells you what to *do*.

## The bet

Marula asks organizations to bet their operating system on it. That bet is
only rational if upgrades are boring. Two commitments define the whole
document:

1. **Legacy pathways live for at least one generation** — one major version.
2. **Every release is reachable from two versions back** — proven by CI on
   production-like fixtures, not by optimism.

## Why four boundaries, exactly

Compatibility is a property of boundaries, not of the whole system. The
policy promises stability across exactly four surfaces (REST + SSE
API, domain events & webhooks, the plugin API, and declared customizations)
plus the documented read model. Everything else (physical tables, internal
crate APIs, worker payloads) is explicitly *not* stable, ever.

Naming the unstable surface honestly is what keeps the stable promises
affordable. A vendor who pretends everything is stable must either freeze
all evolution or break promises; a vendor who says "these four boundaries
hold, the rest is off-limits" can keep both.

## Why expand/contract

The migration chain is forward-only and append-only: add the new shape,
backfill, switch reads/writes, and only in the *next* major drop the old
shape. This is the database-side twin of the one-generation deprecation
rule — nothing a deployed database depends on disappears underneath it in
the same release.

## How it is enforced

Shipped: migration safety lint (`--breaking` filename gate), RLS coverage,
fmt/clippy, the full test matrix. Planned gates: the deprecation clock
(`deprecated_in` / `removal_target` checked at build time) and the [N−2
upgrade matrix](../reference/upgrade-matrix.md) — restore each of the two
prior releases' fixtures, migrate to HEAD, run conformance suites.
