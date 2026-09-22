# immybot

Purpose: first-party HTTP connector written against ONLY the public API +
`@meridian/sdk` (Apache-2.0) — the Stage-2 acceptance pattern proven by the
NinjaOne and Hudu reference connectors.

Status: LANDED (Wave 12 lane 9 `p12-adapters2`) — live REST transport over
the vendor's native pagination/auth flows, feeding the lane-8
`connectors/ci-feeders` engine (`FeederSync` + `FeederSink`, deterministic
identity keys, never a blind insert).

**None mode:** construct the sync with `NoneFeederSink` (or no org/client
mapping, which is a dry inventory) — zero Meridian I/O, every record
counted honestly as "would have synced". Vendor pulls run regardless, so
the run report reflects a real inventory even with no Meridian connection.
Dry-run isolation is also available via `ApplyMode::DryRun`.

**Identity (ADR 0003, updated Wave 22 lane 3):** the device id is the CI2
link key; serial + hostname + the WMI/DMI MAC (`MAC` / `macAddress` /
`macAddresses`, normalized to lower colon form) are the CI3 identity keys.
The MAC signal closed the identity-key drift with the NinjaOne/Auvik
feeders — the same physical machine present in both sources now converges
on identical normalized identity keys (pinned by
`same_physical_machine_converges_across_ninjaone_and_immybot`), which the
feed-upsert seam resolves to one canonical item with one link per source.

Live-credential smoke is out of scope: CI runs against contract-shaped
stub servers (axum) that reproduce each vendor's auth header, pagination
flow, and error surfaces.
