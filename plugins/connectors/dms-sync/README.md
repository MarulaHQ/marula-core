# dms-sync

First-party documentation-system (DMS) sync adapter for Project Meridian —
**IT Glue / Hudu exports + generic REST**, riding the PUBLIC Meridian API
only (Apache-2.0; separate license from the AGPL core, same posture as the
QBO/Xero export-sync crates).

## Purpose

Move an org's documentation BETWEEN an external DMS (IT Glue, Hudu, or a
generic REST system) and Meridian's docs-platform without losing data, via
the API/connector seam:

* **Pull** — `source`: parse IT Glue JSON:API export files (organizations,
  flexible_asset_types, flexible_assets, configurations), Hudu export JSON
  (companies, asset_layouts, assets, websites, passwords), or pull a
  generic REST DMS with a declared endpoint map. Everything normalizes to
  one vendor-neutral `DmsSnapshot`.
* **Sync engine** — ADR 0003 semantics: feeds UPSERT through deterministic
  identity keys (source GUID, serial, hostname+domain). A source's second
  sync updates its own record, never creates a twin. Ambiguous or foreign
  deterministic matches become reviewable merge candidates with evidence —
  never silent overwrites. Curated (human-edited) fields beat observed
  source churn; the source's newer value is reported as one-click
  "accept the new value" evidence. Adoption of foreign matches exists
  behind `auto_link_conflicts` (default OFF).
* **Sink** — the Meridian side over public v1 REST (`x-api-key` machine
  key, instant-first envelope): external layouts map onto
  `document_layouts` (connector org layouts carry the full external
  schema + a `source_ref` provenance field; seeded `is_system` rows are
  READ and matched, never mutated), asset instances land in
  `device_assets` (`layout_id` + schema-validated `fields`), relationships
  land in `document_links` (unresolved ends are surfaced in the run
  report — broken targets are never invented, never cascade-deleted).
  Writes ride instant-first idempotency keys derived from identity keys,
  so server-side audit, provenance, IF7 undo, and the registered
  `asset.*` / `document.*` contract events apply unchanged.
* **Push (parity)** — read Meridian through the public API and render back
  out in the SAME export formats (`render_it_glue_export`,
  `render_hudu_export`), or push neutral records to a generic REST
  endpoint (`RestPushTarget`).
* **None mode** — `NoneDmsSink` is the shipped default sink: zero network,
  every call returns `Disabled`, and the engine's report honestly says
  "would have synced". A connector is an enhancement, never a
  prerequisite.

## Vault boundary (binding)

External credentials/passwords are NOT pulled into Meridian unless the org
explicitly maps them. Two gates hold by default:

1. **Source gate** — the IT Glue / Hudu parsers extract passwords ONLY
   when constructed with `include_credentials()`; without it, password
   records are skipped entirely (never leaked as document content).
2. **Engine gate** — `SyncConfig::map_credentials` (default `false`)
   counts and skips credentials; when turned on, each secret crosses the
   seam EXACTLY ONCE to the audited `POST /api/passwords` vault path
   (sealed AES-256-GCM at rest server-side).

The crate never logs, caches, or serializes secrets: `Credential`'s
`Debug` output redacts, the type is not `Serialize`, and the run report
carries counts only. An agent sees only what the `Module::Passwords`
matrix already allows their role — there is no blanket credential surface.

## Operating a sync (driver sketch)

```rust
use dms_sync::source::it_glue::{ItGlueExport, ItGlueExportFiles};
use dms_sync::sink::meridian::{MeridianConfig, MeridianRestSink};
use dms_sync::state::MemorySyncState;
use dms_sync::sync::{SyncConfig, SyncEngine};
use dms_sync::DmsSource;

// 1. Parse the export (files from the vendor's export zip).
let files = ItGlueExportFiles { /* … */ };
let snapshot = ItGlueExport::new(files).pull()?;

// 2. Sink: scoped machine API key from the operator's secret store —
//    supplied just-in-time, never persisted by this crate.
let sink = MeridianRestSink::new(MeridianConfig {
    base_url: "https://meridian.example".into(),
    api_key: std::env::var("MERIDIAN_API_KEY")?,
});

// 3. Sync. map_credentials stays false unless the org mapped vault
//    material deliberately.
let report = SyncEngine::new(SyncConfig::default())
    .run(&snapshot, &sink, &MemorySyncState::new())
    .await?;

// 4. The report is the review surface: created/updated, merge
//    candidates (never auto-merged), curated conflicts, unmapped
//    fields, unresolved links, credentials skipped/mapped.
println!("{}", serde_json::to_string_pretty(&report)?);
```

Per-connector enable/disable is an operator decision at the driver: no
source constructed + `NoneDmsSink` = fully standalone (none mode).
Credentials for the connector itself (Meridian API key, DMS API key) come
from the operator's own secret store just-in-time — the crate persists
nothing (BYOK-adjacent posture).

## Non-purposes

No OAuth/token refresh (operators rotate scoped keys in their own secret
store), no Meridian schema changes (external shapes map onto
`document_layouts` / `device_assets` / `document_links` as-is — expected
migration: NONE), no new event families (writes ride the registered
`asset.*` / `document.*` contract events by construction), no server-side
merge-queue UI (the ADR 0003 merge-queue surface is a future
ConfigItem-track concern), and no vendor write-API clients (parity push
ships the export renderers + generic REST first).

## Tests

`cargo test -p dms-sync` — fixture round-trips (sample IT Glue / Hudu
exports under `tests/fixtures/`), engine conformance (no twins, merge
candidates, curated-beats-observed, vault boundary, system-layout
mapping, dry run, none mode), and stub-server wire conformance for the
REST sink/source/push targets.