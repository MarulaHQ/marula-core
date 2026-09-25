# unifi

Purpose: first-party UniFi connector written against ONLY the public APIs
(Apache-2.0) — the UniFi Network Integrations API v1 on the controller side,
the Meridian feed-upsert / interface / alert-ingest seams on the Meridian
side. Reference implementation of the connector pattern (Wave 22 lane 3
`p22-ecosystem-connectors-gap`, leg A).

Status: LANDED — live REST transport over the controller's native
offset/limit pagination (`{offset, limit, count, totalCount, data[]}` page
envelope) and `X-API-KEY` header auth (key minted in Network → Settings →
Control Plane → Integrations), feeding the `connectors/ci-feeders` engine
(`FeederSync` semantics, deterministic identity keys, never a blind insert).

## What it does

- **Inventory pull** (`GET /proxy/network/integration/v1/sites` →
  `/sites/{siteId}/devices`): every adopted UniFi device (gateways,
  switches, access points) maps onto the feed contract as source system
  `unifi`, riding the same source-link upsert + merge-candidate queue as
  every other feeder.
- **Interfaces** (`POST /api/v1/config-items/{id}/interfaces`): when a
  device CREATES its canonical ConfigItem, the main NIC (device MAC + IP,
  `is_primary`), the uplink, and every physical port (`Port {idx}`) are
  recorded as `CiInterface` rows — ADR 0003's rule that NICs are
  attributes, not their own CIs. VLAN lists ride through when the payload
  exposes them; the Integrations v1 port overview does not publish VLAN
  membership, so ports land with empty VLAN lists rather than guessed ones.
- **Alert forwarding**: UniFi alerts (webhook-forwarded) become Meridian
  inbound events through `POST /api/v1/alerts/ingest/{source_ref}` —
  dedup-keyed, severity-mapped, storm-correlated on the Meridian side.

## Identity (ADR 0003)

The UniFi device id is the CI2 link key. The Integrations API v1 device
payload carries NO serial — the device `macAddress` (normalized to lower
colon form, sorted) is the CI3 identity signal, with the assigned device
name as the hostname signal. Any `serialNumber`/`serial` key a controller
version exposes is honored when present (observed, never assumed). A UniFi
record and an Auvik/NinjaOne record for the same physical switch converge
through the normalized MAC set exactly per ADR 0003's MAC identity class.
Observed tier carries firmware/state/adoptedAt — fields that churn with the
controller never outrank curated values.

## None mode (honest posture)

- **No Meridian connection** (`NoneFeederSink` / `NoneInterfaceSink`, or no
  site→client mapping): the controller pull still happens, every device is
  counted as "would have synced", and ZERO Meridian I/O is performed.
  No mapping at all is a dry inventory.
- **No controller configured/unreachable**: the connector reports a typed
  `UnifiSyncError::Vendor` pull failure — the UniFi controller is LOCAL
  hardware, so without it there is nothing to pull. The Meridian side
  stays fully functional: the feed-upsert seam, the interface seam, and
  the alert-source ingest all accept writes from the connector (and from
  any UniFi forwarding hop) independently of controller reachability, and
  the run report shows every push the connector would have made.
- Alert forwarding needs no controller: any hop that delivers UniFi alert
  events (the controller's webhook or an operator relay) maps onto the
  ingest contract and posts signed.

## Trust boundary for alerts

UniFi's controller webhooks are UNSIGNED. The connector's
`alerts::verify_delivery` is the receiver half: when the operator configures
a hop secret, deliveries are verified the way Meridian's own receivers must
— constant-time HMAC-SHA256 over `"{t}.{rawBody}"` with the
`t=<ts>,v1=<hex>` header (same scheme as `X-Meridian-Signature`), ±10-minute
anti-replay window, verified over RAW BYTES before any JSON parsing. Without
a hop secret the unauthenticated posture is reported loudly (the local LAN
endpoint remains reachable; the Meridian-bound leg is ALWAYS signed with the
alert source's own secret — Meridian 401s unsigned ingests, so no forged
event reaches Meridian through this connector).

Severity: UniFi's alert stream does not classify severity. An explicit
`severity`/`level` key wins (`critical|error|down` → `critical`,
`warning|warn` → `warning`, else `info`); absent → `warning` (UniFi alerts
signal operational faults, and the incident workflow's P2 default matches).

## Operator mapping

Sites are the org unit: the operator maps each UniFi site id onto one
Meridian client (`OrgMappingBuilder`). Devices in unmapped sites are
REPORTED in the run report — never misattributed to a guessed client.

## Tests

Conformance runs against contract-shaped stub servers (axum) reproducing
the controller's auth header, totalCount pagination envelope, device +
interface payloads, and Meridian's feed-upsert / interface / ingest
surfaces (signature format asserted byte-exact). No live controller, no
live tenant.
