# connectors

> **Purpose:** First-party HTTP connectors (Apache-2.0). The standard set (ADR 0013) is NinjaOne, Hudu, M365 Graph, UniFi, QBO, Xero, 3CX, Slack/Teams, Rewst, plus the dms-sync generic REST target and the signed-webhook alert ingestor — reference implementations of the connector pattern, free forever, no account needed.
> **Non-purpose:** No hosted OAuth apps, credential vaulting, or sync-monitoring operations — hosting/operating connections commercially is out of scope for this repo. Connectors beyond the standard set publish through the marketplace and are built in the commercial repos (first-party: `relay/connectors/<name>`, one git repo per connector); nothing here ever carries an entitlement check. Every adapter documents its none-mode.

The shared support crates sit alongside the connectors: `adapters-core` (live-REST
transport: auth, pagination, org→client mapping) and `ci-feeders` / `dms-sync`
(the feeder and DMS engines whose public mapper surfaces community connectors
build on).
