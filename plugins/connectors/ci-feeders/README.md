# ci-feeders

Support crate for configuration-item and alert feeders. It is not a
connector: it carries no vendor identity of its own, and the first-party
NinjaOne module is one implementation of its public surface. Apache-2.0.

## Purpose

Move records from an RMM or NMS into Marula through the public API only:

- **Devices** become configuration items through
  `POST /api/v1/config-items/feed-upsert`, authenticated with a scoped
  machine key. Every write is an upsert. The core resolves the record by
  `(source_system, external_id)` first and by deterministic identity keys
  second, so a second sync updates the link it created and never makes a
  twin.
- **Interfaces** ride the same device payload as `CiInterface` writes, in
  the connector that owns them. This crate maps the device, not the NICs.
- **Alerts** become Marula alerts through the signed
  `POST /api/v1/alerts/ingest/{source_ref}`. This crate maps the payload;
  the alert source's HMAC transport signs and delivers it. `unifi::alerts`
  is the reference sender for the same wire document.

## What it is not

No live vendor transport with token handling or paging, no org-to-client
mapping UI, no database access, no import of a `crates/` module, and no
entitlement check. A transport fetches vendor JSON and calls a mapper. A
runtime supplies the client id, the base URL and the key.

## The feed contract

`FeederRecord` is the only shape the engine and sink agree on. It carries
the source system id, the vendor's external id, the vendor GUID when one
exists, display defaults, deterministic identity keys and the observed
field tier.

Identity rules (ADR 0003) are applied by `identity`, not by each vendor:
serial and service tag normalize to uppercase alphanumerics, MACs to lower
colon form with a sorted set, hostname and domain to lowercase with no
trailing dot. Two feeds that spell the same device differently converge.

The idempotency key is
`ci-feeder-{source}-{external_id}-{fingerprint}`. An unchanged record on a
retry replays the stored result. Changed content gets a fresh key and lands
as that source's own update.

`FeederSync` runs a batch through a `FeederSink`. It never aborts on one
refused record; the report carries the failure and the rest of the batch
continues. `NoneFeederSink` is the shipped default: zero I/O, every record
counted as "would have synced".

## What a vendor module provides

One pure function that maps the vendor's documented record onto
`FeederRecord`, plus fixture-backed tests. Build identity with
`IdentityKeysBuilder`; use `adapters_core::vendor_fields` for field
extraction so a marketplace module does not fork that path. Return
`NormalizeError::Unrecognised` when the payload has no usable identity.
Never synthesize an id.

Two modules ship:

- `vendors::ninjaone` maps the v2 device object (`id`, `uid`,
  `displayName`, `systemName`, `serialNumber`, `macAddresses`, ...).
- `vendors::ninjaone_alerts` maps the documented alert object
  (`uid`, `deviceId`, `condition`, `severity`, `message`, `status`, ...).

## Alert dedupe and correlation

Two keys, two jobs.

`dedup_key` is the replay gate. It is the vendor's own alert id when the
payload carries one, otherwise `device:condition:start time`. Receipt time
is never part of it, because a retry would then look like a new alert. The
ingest's unique index on `(org, source, dedup_key)` answers a repeat with
the stored alert and writes nothing.

`correlation_key` is the storm identity: one condition on one device. The
alerts of a storm have different ids and share this key, so the ingest folds
them into the ONE open incident and increments its `alert_count`. One event
with a count, not one ticket per repeat. A payload with no condition sends
no key, and the ingest defaults it to the dedup key, which means "its own
storm".

`StormCoalescer` is a local guard, not the counter. It suppresses an exact
repeat inside the window so a duplicated delivery costs no round trip, and
its report carries the repeat count. The persistent count is the incident's.

Severity maps onto Marula's closed vocabulary: `CRITICAL` is critical,
`MAJOR` and `WARNING` are warning, `MINOR` and `NONE` are info. An unknown
grade is warning, never critical by accident. A terminal status
(`RESOLVED`, `CLEARED`, `CLOSED`, `ENDED`) returns `NinjaOneEvent::Resolved`
and opens no ticket, because the ingest has no verb for closing one.

## Fixtures and tests

`tests/fixtures/ninjaone_device.json` is the device shape. It proves the
identity keys, the observed tier, and the deterministic MAC normalization
across three spellings. `tests/fixtures/ninjaone_alert.json` is the alert
shape. It proves one payload to one ingest document, the severity map, the
shared storm key across distinct alerts, the repeat counter, the resolved
path, and the refusal of a payload with no identity.

The suite also runs the sink and engine against a stub API server: auth
header, idempotency key, every typed error per status, and a per-record
failure that does not stop the batch. No live tenant.

```bash
cargo test -p ci-feeders
```

## None mode

`NoneFeederSink` performs no network I/O and reports every record as "would
have synced". An org with no Marula connection runs standalone, and the
report says so. A mapper still runs, because the report is how an operator
sees what the connector would have done.
