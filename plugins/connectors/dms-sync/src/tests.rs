//! Contract + conformance tests for the DMS sync connector:
//!
//! * fixture round-trips — the sample IT Glue / Hudu exports parse,
//!   sync, and render back out with their semantics intact;
//! * engine semantics — first-sync creates, second-sync updates (no
//!   twins), foreign deterministic matches become reviewable merge
//!   candidates, curated fields beat observed churn, links surface
//!   unresolved ends, and the vault boundary holds;
//! * wire conformance — the Meridian REST sink rides the public API
//!   shapes (x-api-key, instant-first idempotency keys, credentials
//!   ONLY on the audited vault path) against a stub server, and the
//!   generic REST source / push target speak their declared contracts;
//! * none mode — a disabled sink produces an honest empty report and
//!   zero writes.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use uuid::Uuid;

use crate::identity::Identity;
use crate::model::{Credential, DmsSnapshot, LinkEndpoint, LinkKind, Relation, SyncRecord};
use crate::push;
use crate::sink::NoneDmsSink;
use crate::sink::{
    AssetCreate, AssetRecord, AssetUpdate, ClientRecord, CredentialCreate, CredentialReceipt,
    DmsSink, DocumentCreate, DocumentRecord, DocumentUpdate, LayoutRecord, LinkCreate, LinkRecord,
    NewLayout, SinkError,
};
use crate::source::hudu::{HuduExport, HuduExportFiles};
use crate::source::it_glue::{ItGlueExport, ItGlueExportFiles};
use crate::source::DmsSource;
use crate::state::MemorySyncState;
use crate::state::{LedgerEntry, SyncState};
use crate::sync::{ApplyMode, SyncConfig, SyncEngine, SOURCE_REF_KEY};

// ---- fixtures -------------------------------------------------------------------

fn it_glue_fixture() -> Value {
    serde_json::from_str(include_str!("../tests/fixtures/it_glue_export.json"))
        .expect("it glue fixture")
}

fn hudu_fixture() -> Value {
    serde_json::from_str(include_str!("../tests/fixtures/hudu_export.json")).expect("hudu fixture")
}

fn it_glue_files(value: &Value) -> ItGlueExportFiles {
    ItGlueExportFiles {
        organizations: value.get("organizations.json").cloned(),
        flexible_asset_types: value.get("flexible_asset_types.json").cloned(),
        flexible_assets: value.get("flexible_assets.json").cloned(),
        configurations: value.get("configurations.json").cloned(),
    }
}

fn hudu_files(value: &Value) -> HuduExportFiles {
    HuduExportFiles {
        companies: value.get("companies").cloned(),
        asset_layouts: value.get("asset_layouts").cloned(),
        assets: value.get("assets").cloned(),
        websites: value.get("websites").cloned(),
        passwords: value.get("passwords").cloned(),
    }
}

fn it_glue_snapshot() -> DmsSnapshot {
    ItGlueExport::new(it_glue_files(&it_glue_fixture()))
        .pull()
        .expect("parse it glue export")
}

fn hudu_snapshot() -> DmsSnapshot {
    HuduExport::new(hudu_files(&hudu_fixture()))
        .pull()
        .expect("parse hudu export")
}

// ---- shared ids -----------------------------------------------------------------

fn acme() -> Uuid {
    Uuid::from_u128(0x0000_0000_0000_0AC0)
}

fn globex() -> Uuid {
    Uuid::from_u128(0x610B)
}

// ---- mock sink (conformance substrate) -------------------------------------------

#[derive(Default)]
struct MockState {
    layouts: Vec<LayoutRecord>,
    clients: Vec<ClientRecord>,
    assets: Vec<AssetRecord>,
    documents: Vec<DocumentRecord>,
    links: Vec<LinkRecord>,
    /// Vault-path captures: (title, secret) — the ONLY place a secret
    /// may appear in tests, to assert it crossed exactly once.
    vault: Vec<(String, String)>,
    next_id: u128,
}

impl MockState {
    fn next_id(&mut self) -> Uuid {
        self.next_id += 1;
        Uuid::from_u128(self.next_id)
    }
}

/// In-memory sink that records every call (the conformance substrate
/// for engine tests).
struct MockSink {
    state: Mutex<MockState>,
}

impl MockSink {
    fn new() -> Self {
        Self {
            state: Mutex::new(MockState::default()),
        }
    }

    fn with_clients() -> Self {
        let sink = Self::new();
        {
            let mut state = sink.state.lock().unwrap();
            state.clients = vec![
                ClientRecord {
                    id: acme(),
                    name: "Acme Corp".into(),
                },
                ClientRecord {
                    id: globex(),
                    name: "Globex Industries".into(),
                },
            ];
        }
        sink
    }

    fn with_system_layout(self, layout: LayoutRecord) -> Self {
        self.state.lock().unwrap().layouts.push(layout);
        self
    }

    fn with_asset(self, asset: AssetRecord) -> Self {
        self.state.lock().unwrap().assets.push(asset);
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MockState> {
        self.state.lock().unwrap()
    }
}

#[async_trait::async_trait]
impl DmsSink for MockSink {
    fn name(&self) -> &str {
        "mock"
    }

    async fn list_layouts(&self) -> Result<Vec<LayoutRecord>, SinkError> {
        Ok(self.lock().layouts.clone())
    }

    async fn create_layout(&self, layout: &NewLayout) -> Result<LayoutRecord, SinkError> {
        let mut state = self.lock();
        if state.layouts.iter().any(|l| l.slug == layout.slug) {
            return Err(SinkError::Conflict(format!("slug {} taken", layout.slug)));
        }
        let record = LayoutRecord {
            id: state.next_id(),
            slug: layout.slug.clone(),
            name: layout.name.clone(),
            is_system: false,
            fields: layout.fields.clone(),
        };
        state.layouts.push(record.clone());
        Ok(record)
    }

    async fn list_clients(&self) -> Result<Vec<ClientRecord>, SinkError> {
        Ok(self.lock().clients.clone())
    }

    async fn list_assets(&self) -> Result<Vec<AssetRecord>, SinkError> {
        Ok(self.lock().assets.clone())
    }

    async fn create_asset(&self, plan: &AssetCreate) -> Result<AssetRecord, SinkError> {
        let mut state = self.lock();
        let record = AssetRecord {
            id: state.next_id(),
            client_id: plan.client_id,
            tag: plan.tag.clone(),
            name: plan.name.clone(),
            status: "active".into(),
            layout_id: plan.layout_id,
            fields: plan.fields.clone(),
        };
        state.assets.push(record.clone());
        Ok(record)
    }

    async fn update_asset(&self, id: Uuid, plan: &AssetUpdate) -> Result<AssetRecord, SinkError> {
        let mut state = self.lock();
        let Some(asset) = state.assets.iter_mut().find(|a| a.id == id) else {
            return Err(SinkError::NotFound(format!("asset {id}")));
        };
        if let Some(name) = &plan.name {
            asset.name = name.clone();
        }
        if let Some(status) = &plan.status {
            asset.status = status.clone();
        }
        if let Some(layout_id) = plan.layout_id {
            asset.layout_id = Some(layout_id);
        }
        if let Some(fields) = &plan.fields {
            asset.fields = fields.clone();
        }
        Ok(asset.clone())
    }

    async fn list_documents(&self) -> Result<Vec<DocumentRecord>, SinkError> {
        Ok(self.lock().documents.clone())
    }

    async fn get_document(&self, id: Uuid) -> Result<DocumentRecord, SinkError> {
        self.lock()
            .documents
            .iter()
            .find(|d| d.id == id)
            .cloned()
            .ok_or_else(|| SinkError::NotFound(format!("document {id}")))
    }

    async fn create_document(&self, plan: &DocumentCreate) -> Result<DocumentRecord, SinkError> {
        let mut state = self.lock();
        let record = DocumentRecord {
            id: state.next_id(),
            client_id: plan.client_id,
            title: plan.title.clone(),
            layout_id: plan.layout_id,
            fields: plan.fields.clone(),
            body: plan.body.clone(),
        };
        state.documents.push(record.clone());
        Ok(record)
    }

    async fn update_document(
        &self,
        id: Uuid,
        plan: &DocumentUpdate,
    ) -> Result<DocumentRecord, SinkError> {
        let mut state = self.lock();
        let Some(document) = state.documents.iter_mut().find(|d| d.id == id) else {
            return Err(SinkError::NotFound(format!("document {id}")));
        };
        if let Some(title) = &plan.title {
            document.title = title.clone();
        }
        if let Some(layout_id) = plan.layout_id {
            document.layout_id = Some(layout_id);
        }
        if let Some(fields) = &plan.fields {
            document.fields = fields.clone();
        }
        if let Some(body) = &plan.body {
            document.body = body.clone();
        }
        Ok(document.clone())
    }

    async fn list_links(
        &self,
        from_kind: LinkKind,
        from_id: Uuid,
    ) -> Result<Vec<LinkRecord>, SinkError> {
        Ok(self
            .lock()
            .links
            .iter()
            .filter(|l| l.from_kind == from_kind && l.from_id == from_id)
            .cloned()
            .collect())
    }

    async fn create_link(&self, plan: &LinkCreate) -> Result<LinkRecord, SinkError> {
        let mut state = self.lock();
        if state.links.iter().any(|l| {
            l.from_id == plan.from_id && l.to_kind == plan.to_kind && l.to_id == plan.to_id
        }) {
            return Err(SinkError::Conflict("link exists".into()));
        }
        let link = LinkRecord {
            id: state.next_id(),
            from_kind: plan.from_kind,
            from_id: plan.from_id,
            to_kind: plan.to_kind,
            to_id: plan.to_id,
        };
        state.links.push(link.clone());
        Ok(link)
    }

    async fn create_password(
        &self,
        plan: &CredentialCreate,
    ) -> Result<CredentialReceipt, SinkError> {
        let mut state = self.lock();
        state.vault.push((plan.title.clone(), plan.secret.clone()));
        Ok(CredentialReceipt {
            password_id: Uuid::now_v7(),
        })
    }
}

async fn run_apply(
    snapshot: &DmsSnapshot,
    sink: &MockSink,
    state: &MemorySyncState,
) -> crate::sync::SyncReport {
    run_with_config(SyncConfig::default(), snapshot, sink, state).await
}

async fn run_with_config(
    config: SyncConfig,
    snapshot: &DmsSnapshot,
    sink: &MockSink,
    state: &MemorySyncState,
) -> crate::sync::SyncReport {
    let engine = SyncEngine::new(config);
    engine.run(snapshot, sink, state).await.expect("run")
}

// ---- source parser tests -----------------------------------------------------------

#[test]
fn it_glue_export_parses_layouts_records_and_client_scope() {
    let snapshot = it_glue_snapshot();

    assert_eq!(snapshot.source, "it-glue");
    // Two flexible asset types parsed as layouts.
    assert_eq!(snapshot.layouts.len(), 2);
    let runbook = &snapshot.layouts[0];
    assert_eq!(runbook.name, "Runbook");
    assert_eq!(runbook.fields.len(), 3);
    assert_eq!(runbook.fields[0].kind.as_str(), "text");
    assert!(runbook.fields[0].required);
    assert_eq!(runbook.fields[1].kind.as_str(), "email");
    assert_eq!(runbook.fields[2].kind.as_str(), "date");

    // Documents: 2 runbooks; the password-typed asset is NOT extracted
    // by default (vault boundary: extraction is opt-in at the source).
    let documents: Vec<&SyncRecord> = snapshot
        .records
        .iter()
        .filter(|r| r.kind == LinkKind::Document)
        .collect();
    assert_eq!(documents.len(), 2);
    assert_eq!(documents[0].title, "Exchange runbook");
    assert_eq!(
        documents[0].fields.get("owner_email").map(String::as_str),
        Some("ops@acme.test")
    );

    // Configurations parse as identity-bearing assets.
    let assets: Vec<&SyncRecord> = snapshot
        .records
        .iter()
        .filter(|r| r.kind == LinkKind::Asset)
        .collect();
    assert_eq!(assets.len(), 2);
    let exchange = assets[0];
    assert_eq!(exchange.external_id, "301");
    assert_eq!(
        exchange.fields.get("serial").map(String::as_str),
        Some("SN-EXCH-001")
    );
    let identity = exchange.identity("it-glue");
    assert_eq!(identity.primary.kind.as_str(), "guid");
    assert!(identity
        .alternates
        .iter()
        .any(|k| k.kind.as_str() == "serial" && k.value == "SN-EXCH-001"));

    // Client scope rides the snapshot.
    assert_eq!(
        snapshot.client_refs.get("101").map(String::as_str),
        Some("Acme Corp")
    );
    assert_eq!(documents[0].client_ref.as_deref(), Some("101"));

    // Credentials: empty by default.
    assert!(snapshot.credentials.is_empty());
}

#[test]
fn it_glue_credentials_are_opt_in_and_debug_redacted() {
    let snapshot = ItGlueExport::new(it_glue_files(&it_glue_fixture()))
        .include_credentials()
        .pull()
        .expect("parse");

    assert_eq!(snapshot.credentials.len(), 1);
    let credential = &snapshot.credentials[0];
    assert_eq!(credential.title, "Firewall console");
    assert_eq!(credential.secret, "s3cr3t-vault-material");
    // The secret NEVER survives a Debug format (log-line safety).
    let rendered = format!("{credential:?}");
    assert!(!rendered.contains("s3cr3t-vault-material"));
    assert!(rendered.contains("[REDACTED]"));
}

#[test]
fn hudu_export_parses_with_identity_bearing_assets() {
    let snapshot = hudu_snapshot();

    assert_eq!(snapshot.source, "hudu");
    assert_eq!(snapshot.layouts.len(), 2);
    assert_eq!(
        snapshot.client_refs.get("7").map(String::as_str),
        Some("Globex Industries")
    );

    // The workstation (hostname+serial) is identity-bearing → asset.
    let assets: Vec<&SyncRecord> = snapshot
        .records
        .iter()
        .filter(|r| r.kind == LinkKind::Asset)
        .collect();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].external_id, "41");

    // The policy document and the website are documents.
    let documents: Vec<&SyncRecord> = snapshot
        .records
        .iter()
        .filter(|r| r.kind == LinkKind::Document)
        .collect();
    assert_eq!(documents.len(), 2);

    // Passwords: not extracted without the opt-in.
    assert!(snapshot.credentials.is_empty());

    let opted = HuduExport::new(HuduExportFiles {
        passwords: hudu_files(&hudu_fixture()).passwords,
        ..Default::default()
    })
    .include_credentials()
    .pull()
    .expect("parse credentials");
    assert_eq!(opted.credentials.len(), 1);
    assert_eq!(opted.credentials[0].secret, "hudu-vault-secret-42");
}

// ---- push parity: round trips ------------------------------------------------------

/// A neutral Meridian snapshot mirroring the parsed snapshot's
/// semantics (deterministic ids; fields preserved verbatim).
fn meridian_snapshot_from(snapshot: &DmsSnapshot) -> push::MeridianSnapshot {
    let mut out = push::MeridianSnapshot {
        source: snapshot.source.clone(),
        ..Default::default()
    };
    for (external_id, name) in &snapshot.client_refs {
        let id = external_id.parse::<u128>().unwrap_or(7);
        out.clients.push((Uuid::from_u128(id), name.clone()));
    }
    for (position, layout) in snapshot.layouts.iter().enumerate() {
        out.layouts.push(push::SnapshotLayout {
            id: Uuid::from_u128(1000 + position as u128),
            slug: format!("layout{position}"),
            name: layout.name.clone(),
            is_system: false,
            fields: layout.fields.clone(),
        });
    }
    for (position, record) in snapshot.records.iter().enumerate() {
        let id = Uuid::from_u128(2000 + position as u128);
        let client_id = record
            .client_ref
            .as_deref()
            .and_then(|r| r.parse::<u128>().ok())
            .map(Uuid::from_u128);
        let fields: serde_json::Map<String, Value> = record
            .fields
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        match record.kind {
            LinkKind::Asset => out.assets.push(push::SnapshotAsset {
                id,
                client_id,
                tag: record.title.clone(),
                name: record.title.clone(),
                kind: "device".into(),
                status: "active".into(),
                layout_id: None,
                fields,
            }),
            _ => out.documents.push(push::SnapshotDocument {
                id,
                client_id,
                title: record.title.clone(),
                layout_id: None,
                fields,
                body: record.body.clone(),
            }),
        }
    }
    out
}

#[test]
fn round_trip_it_glue_export_preserves_semantics() {
    let snapshot = it_glue_snapshot();
    let rendered = push::render_it_glue_export(&meridian_snapshot_from(&snapshot));

    let files = ItGlueExportFiles {
        organizations: rendered.file("organizations").cloned(),
        flexible_asset_types: rendered.file("flexible_asset_types").cloned(),
        flexible_assets: rendered.file("flexible_assets").cloned(),
        configurations: rendered.file("configurations").cloned(),
    };
    let round_tripped = ItGlueExport::new(files).pull().expect("re-parse");

    // Every original record survives the round trip with its fields
    // (matched by title: the rendered ids are Meridian uuids, not the
    // external id space).
    for record in &snapshot.records {
        let matching = round_tripped
            .records
            .iter()
            .find(|r| r.title == record.title)
            .unwrap_or_else(|| panic!("record {:?} lost in round trip", record.title));
        for (key, value) in &record.fields {
            assert_eq!(
                matching.fields.get(key).map(String::as_str),
                Some(value.as_str()),
                "field {key} lost for record {:?}",
                record.title
            );
        }
    }
    for name in snapshot.client_refs.values() {
        assert!(
            round_tripped.client_refs.values().any(|n| n == name),
            "client {name} lost in round trip"
        );
    }
}

#[test]
fn round_trip_hudu_export_preserves_semantics() {
    let snapshot = hudu_snapshot();
    let rendered = push::render_hudu_export(&meridian_snapshot_from(&snapshot));

    let files = HuduExportFiles {
        companies: rendered.get("companies").cloned(),
        asset_layouts: rendered.get("asset_layouts").cloned(),
        assets: rendered.get("assets").cloned(),
        websites: rendered.get("websites").cloned(),
        passwords: None,
    };
    let round_tripped = HuduExport::new(files).pull().expect("re-parse");

    for record in &snapshot.records {
        let matching = round_tripped
            .records
            .iter()
            .find(|r| r.title == record.title)
            .unwrap_or_else(|| panic!("record {:?} lost", record.title));
        assert_eq!(
            matching.kind, record.kind,
            "asset/document split drifted for {:?}",
            record.title
        );
        for (key, value) in &record.fields {
            assert_eq!(
                matching.fields.get(key).map(String::as_str),
                Some(value.as_str())
            );
        }
    }
}

// ---- engine conformance ---------------------------------------------------------

#[tokio::test]
async fn engine_first_sync_creates_assets_documents_and_connector_layouts() {
    let snapshot = it_glue_snapshot();
    let sink = MockSink::with_clients();
    let state = MemorySyncState::new();

    let report = run_apply(&snapshot, &sink, &state).await;

    // 2 documents + 2 assets created; the password-typed flexible asset
    // is neither (vault boundary holds at the source).
    assert_eq!(report.documents_created, 2, "report: {report:?}");
    assert_eq!(report.assets_created, 2);
    // Connector org layouts: Runbook + Passwords types, plus the two
    // synthesized configuration types (Server, Firewall).
    assert_eq!(report.layouts_created.len(), 4, "report: {report:?}");
    assert!(report
        .layouts_created
        .iter()
        .any(|slug| slug == "it-glue-runbook"));
    assert!(report
        .layouts_created
        .iter()
        .any(|slug| slug == "it-glue-server"));
    assert!(report.layouts_mapped_system.is_empty());
    assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
    assert!(report.merge_candidates.is_empty());

    // Provenance: created assets carry the source_ref identity handle.
    let exchange = sink
        .lock()
        .assets
        .iter()
        .find(|a| a.name == "EXCH-01")
        .expect("exchange asset")
        .clone();
    let identity = snapshot
        .records
        .iter()
        .find(|r| r.external_id == "301")
        .expect("record")
        .identity("it-glue");
    assert_eq!(
        exchange.fields.get("source_ref").and_then(Value::as_str),
        Some(identity.handle().as_str())
    );
    // The ledger owns every created record.
    assert!(state
        .lookup(&format!("asset:{}", identity.handle()))
        .is_some());
    assert!(
        sink.lock().vault.is_empty(),
        "no credential crossed the seam"
    );
}

#[tokio::test]
async fn engine_second_sync_updates_its_own_records_never_twins() {
    let mut snapshot = it_glue_snapshot();
    let sink = MockSink::with_clients();
    let state = MemorySyncState::new();

    let first = run_apply(&snapshot, &sink, &state).await;
    assert_eq!(first.assets_created, 2);
    assert_eq!(first.documents_created, 2);

    // Second run, unchanged source: nothing happens.
    let second = run_apply(&snapshot, &sink, &state).await;
    assert_eq!(second.assets_created, 0, "no twins on unchanged re-sync");
    assert_eq!(second.assets_updated, 0);
    assert_eq!(second.documents_created, 0);
    assert_eq!(second.documents_updated, 0);

    // Changed source content: the same record updates, still no twin.
    let exchange = snapshot
        .records
        .iter_mut()
        .find(|r| r.external_id == "301")
        .expect("exchange");
    exchange
        .fields
        .insert("primary_ip".to_string(), "10.0.0.9".to_string());
    let third = run_apply(&snapshot, &sink, &state).await;
    assert_eq!(third.assets_created, 0, "twin created!");
    assert_eq!(third.assets_updated, 1, "own record must update in place");
    let locked = sink.lock();
    assert_eq!(
        locked.assets.iter().filter(|a| a.name == "EXCH-01").count(),
        1
    );
    assert_eq!(
        locked
            .assets
            .iter()
            .find(|a| a.name == "EXCH-01")
            .and_then(|a| a.fields.get("primary_ip"))
            .and_then(Value::as_str),
        Some("10.0.0.9")
    );
}

#[tokio::test]
async fn engine_foreign_serial_match_is_a_reviewable_merge_candidate() {
    let snapshot = it_glue_snapshot();
    // A foreign (human-created) Meridian asset already owns this serial.
    let foreign = AssetRecord {
        id: Uuid::from_u128(9_001),
        client_id: acme(),
        tag: "SRV-LEGACY".into(),
        name: "Old server".into(),
        status: "active".into(),
        layout_id: None,
        fields: json!({ "serial": "SN-EXCH-001" }),
    };
    let sink = MockSink::with_clients().with_asset(foreign);
    let state = MemorySyncState::new();

    let report = run_apply(&snapshot, &sink, &state).await;
    assert_eq!(
        report.assets_created, 1,
        "only the non-matching asset creates"
    );
    assert_eq!(report.merge_candidates.len(), 1, "report: {report:?}");
    let candidate = &report.merge_candidates[0];
    assert_eq!(candidate.external_id, "301");
    assert!(candidate.matched_on.contains("serial"));
    // The matching key itself is the evidence; field diffs may be empty
    // when the foreign record shares only the matched key.
    assert_eq!(
        candidate.meridian_label,
        "asset \"Old server\" (SRV-LEGACY)"
    );
    // Never a silent overwrite: the foreign record is untouched.
    let locked = sink.lock();
    let foreign = locked
        .assets
        .iter()
        .find(|a| a.id == Uuid::from_u128(9_001))
        .expect("foreign");
    assert!(foreign.fields.get("hostname").is_none());
}

#[tokio::test]
async fn engine_auto_link_adopts_and_only_fills_empty_fields() {
    let snapshot = it_glue_snapshot();
    let foreign = AssetRecord {
        id: Uuid::from_u128(9_001),
        client_id: acme(),
        tag: "SRV-LEGACY".into(),
        name: "Old server".into(),
        status: "active".into(),
        layout_id: None,
        // Populated conflicting value + empty slots to fill.
        fields: json!({ "serial": "SN-EXCH-001", "os": "Human-typed OS" }),
    };
    let sink = MockSink::with_clients().with_asset(foreign);
    let state = MemorySyncState::new();
    let config = SyncConfig {
        auto_link_conflicts: true,
        ..Default::default()
    };

    let report = run_with_config(config, &snapshot, &sink, &state).await;
    assert_eq!(
        report.merge_candidates.len(),
        0,
        "adoption on: no candidate"
    );
    let locked = sink.lock();
    let adopted = locked
        .assets
        .iter()
        .find(|a| a.id == Uuid::from_u128(9_001))
        .expect("adopted");
    // Empty hostname filled; the populated os value is NOT overwritten.
    assert_eq!(
        adopted.fields.get("hostname").and_then(Value::as_str),
        Some("exch01.acme.test")
    );
    assert_eq!(
        adopted.fields.get("os").and_then(Value::as_str),
        Some("Human-typed OS"),
        "curated populated value must survive adoption"
    );
    // Provenance stamped: the connector now owns the record.
    let identity = snapshot
        .records
        .iter()
        .find(|r| r.external_id == "301")
        .expect("record")
        .identity("it-glue");
    assert_eq!(
        adopted.fields.get("source_ref").and_then(Value::as_str),
        Some(identity.handle().as_str())
    );
}

#[tokio::test]
async fn engine_curated_field_beats_observed_source_value() {
    let mut snapshot = it_glue_snapshot();
    let sink = MockSink::with_clients();
    let state = MemorySyncState::new();

    let first = run_apply(&snapshot, &sink, &state).await;
    assert_eq!(first.assets_created, 2);

    // A human edits the sink record (curated), then the source moves.
    {
        let mut locked = sink.lock();
        let exchange = locked
            .assets
            .iter_mut()
            .find(|a| a.name == "EXCH-01")
            .expect("asset");
        exchange
            .fields
            .as_object_mut()
            .expect("object")
            .insert("os".into(), json!("Human-typed OS"));
    }
    let exchange = snapshot
        .records
        .iter_mut()
        .find(|r| r.external_id == "301")
        .expect("record");
    exchange
        .fields
        .insert("os".to_string(), "Windows Server 2025".to_string());

    let report = run_apply(&snapshot, &sink, &state).await;
    // The curated value survives; the source value is the evidence.
    assert_eq!(report.curated_conflicts.len(), 1, "report: {report:?}");
    let conflict = &report.curated_conflicts[0];
    assert_eq!(conflict.field, "os");
    assert_eq!(conflict.curated_value, "Human-typed OS");
    assert_eq!(conflict.source_value, "Windows Server 2025");
    assert_eq!(
        sink.lock()
            .assets
            .iter()
            .find(|a| a.name == "EXCH-01")
            .and_then(|a| a.fields.get("os"))
            .and_then(Value::as_str),
        Some("Human-typed OS")
    );
}

#[tokio::test]
async fn engine_credentials_never_reach_sink_unless_mapped() {
    let mut snapshot = it_glue_snapshot();
    snapshot.credentials.push(Credential {
        external_id: "cred-1".into(),
        client_ref: Some("101".into()),
        title: "Router admin".into(),
        username: "admin".into(),
        url: String::new(),
        secret: "super-secret-material".into(),
        notes: None,
        expires_at: None,
    });

    // Default: vault boundary holds — counted and skipped, no sink call.
    let sink = MockSink::with_clients();
    let state = MemorySyncState::new();
    let report = run_apply(&snapshot, &sink, &state).await;
    assert_eq!(report.credentials_skipped, 1);
    assert_eq!(report.credentials_mapped, 0);
    assert!(sink.lock().vault.is_empty(), "credential reached the sink!");

    // Explicit mapping: the audited vault path is exercised exactly once.
    let sink_mapped = MockSink::with_clients();
    let state_mapped = MemorySyncState::new();
    let config = SyncConfig {
        map_credentials: true,
        ..Default::default()
    };
    let report = run_with_config(config, &snapshot, &sink_mapped, &state_mapped).await;
    assert_eq!(report.credentials_mapped, 1);
    let vault = sink_mapped.lock().vault.clone();
    assert_eq!(vault.len(), 1);
    assert_eq!(vault[0].1, "super-secret-material");
}

#[tokio::test]
async fn engine_maps_system_layout_only_when_schema_fits() {
    let snapshot = hudu_snapshot();
    // Pre-create the SYSTEM workstation layout (the seeded is_system
    // row the connector must match but never touch).
    let system = LayoutRecord {
        id: Uuid::from_u128(7_700),
        slug: "workstation".into(),
        name: "Workstation".into(),
        is_system: true,
        fields: vec![
            crate::model::SyncField::new("hostname", "Hostname", crate::model::FieldKind::Text, 0)
                .required(),
            crate::model::SyncField::new(
                "serial",
                "Serial number",
                crate::model::FieldKind::Text,
                1,
            ),
            crate::model::SyncField::new(
                "os",
                "Operating system",
                crate::model::FieldKind::Text,
                2,
            ),
            crate::model::SyncField::new(
                "assigned_user",
                "Assigned user",
                crate::model::FieldKind::Text,
                3,
            ),
        ],
    };
    let sink = MockSink::new().with_system_layout(system);
    {
        let mut locked = sink.lock();
        locked.clients = vec![ClientRecord {
            id: globex(),
            name: "Globex Industries".into(),
        }];
    }
    let engine_state = MemorySyncState::new();

    let report = run_apply(&snapshot, &sink, &engine_state).await;

    // The Workstation external type fits the system schema (subset +
    // required carried): mapped onto the SYSTEM row, never duplicated.
    assert!(
        report
            .layouts_mapped_system
            .iter()
            .any(|n| n == "Workstation"),
        "report: {report:?}"
    );
    assert!(!report
        .layouts_created
        .iter()
        .any(|slug| slug == "workstation"));
    // Policy + Website types have no system counterpart → org layouts.
    assert!(report
        .layouts_created
        .iter()
        .any(|slug| slug == "hudu-policy"));
    assert!(report
        .layouts_created
        .iter()
        .any(|slug| slug == "hudu-website"));
    // is_system rows are never created: the seeded row is the only one.
    assert_eq!(
        sink.lock().layouts.iter().filter(|l| l.is_system).count(),
        1
    );
    // The workstation record synced under the system layout.
    assert_eq!(report.assets_created, 1);
    assert!(sink.lock().assets.iter().any(|a| a.name == "WS-FIN-01"));
}

#[tokio::test]
async fn engine_unresolved_link_is_surfaced_never_invented() {
    let mut snapshot = it_glue_snapshot();
    // A relation whose target does not exist in the source either.
    snapshot.relations.push(Relation {
        from: LinkEndpoint {
            external_id: "201".into(),
            kind: LinkKind::Document,
        },
        to: LinkEndpoint {
            external_id: "ghost-404".into(),
            kind: LinkKind::Asset,
        },
    });

    let sink = MockSink::with_clients();
    let state = MemorySyncState::new();
    let report = run_apply(&snapshot, &sink, &state).await;

    assert_eq!(report.unresolved_links.len(), 1);
    assert_eq!(report.unresolved_links[0].missing, "to");
    assert!(
        sink.lock().links.is_empty(),
        "a broken link must not be invented"
    );
}

#[tokio::test]
async fn engine_resolves_and_creates_links_between_synced_records() {
    let mut snapshot = it_glue_snapshot();
    // Document 201 (a runbook) relates to configuration 301 (its server).
    snapshot.relations.push(Relation {
        from: LinkEndpoint {
            external_id: "201".into(),
            kind: LinkKind::Document,
        },
        to: LinkEndpoint {
            external_id: "301".into(),
            kind: LinkKind::Asset,
        },
    });

    let sink = MockSink::with_clients();
    let state = MemorySyncState::new();
    let report = run_apply(&snapshot, &sink, &state).await;

    assert!(report.unresolved_links.is_empty(), "report: {report:?}");
    assert_eq!(report.links_created, 1);
    // A second run sees the existing link (idempotent, no duplicate).
    let second = run_apply(&snapshot, &sink, &state).await;
    assert_eq!(second.links_existing, 1);
    assert_eq!(second.links_created, 0);
    assert_eq!(sink.lock().links.len(), 1);
}

#[tokio::test]
async fn engine_none_mode_reports_without_writes() {
    let snapshot = it_glue_snapshot();
    let state = MemorySyncState::new();
    let engine = SyncEngine::new(SyncConfig::default());
    let report = engine
        .run(&snapshot, &NoneDmsSink, &state)
        .await
        .expect("none mode is a report, not an error");
    assert!(!report.dry_run);
    assert_eq!(report.records_skipped, snapshot.records.len());
    assert!(
        report.errors.iter().any(|e| e.error.contains("none mode")),
        "report: {report:?}"
    );

    // And the sink itself is honest: Disabled means zero network.
    let err = NoneDmsSink
        .create_asset(&AssetCreate {
            client_id: acme(),
            tag: "t".into(),
            name: "n".into(),
            layout_id: None,
            fields: json!({}),
            idempotency_key: "k".into(),
            change_note: String::new(),
        })
        .await
        .expect_err("disabled");
    assert!(matches!(err, SinkError::Disabled));
}

#[tokio::test]
async fn engine_dry_run_writes_nothing() {
    let snapshot = it_glue_snapshot();
    let sink = MockSink::with_clients();
    let state = MemorySyncState::new();
    let config = SyncConfig {
        apply: ApplyMode::DryRun,
        ..Default::default()
    };
    let report = run_with_config(config, &snapshot, &sink, &state).await;
    assert!(report.dry_run);
    assert_eq!(report.assets_created, 2, "report: {report:?}");
    assert!(sink.lock().assets.is_empty(), "dry run wrote to the sink!");
    assert!(sink.lock().documents.is_empty());
    // The ledger is untouched too.
    assert!(state.lookup("asset:guid:it-glue:301").is_none());
}

#[tokio::test]
async fn engine_unmapped_client_scope_is_reported_not_guessed() {
    let mut snapshot = it_glue_snapshot();
    for record in &mut snapshot.records {
        // Point everything at an organization the Meridian org has no
        // client for.
        record.client_ref = Some("999".into());
    }
    let sink = MockSink::with_clients();
    let state = MemorySyncState::new();
    let report = run_apply(&snapshot, &sink, &state).await;
    assert_eq!(report.unmapped_clients.len(), 4, "report: {report:?}");
    assert_eq!(report.assets_created, 0);
    assert_eq!(report.documents_created, 0);
}

#[test]
fn identity_keys_are_deterministic_and_ordered() {
    let mut fields = BTreeMap::new();
    fields.insert("hostname".to_string(), "EXCH01".to_string());
    fields.insert("domain".to_string(), "Acme.TEST".to_string());
    fields.insert("serial".to_string(), "SN-1".to_string());
    let identity = crate::identity::derive("it-glue", "301", &fields);
    // Primary = source GUID handle; alternates = serial then fqdn.
    assert_eq!(identity.primary.value, "it-glue:301");
    assert_eq!(identity.alternates[0].kind.as_str(), "serial");
    assert_eq!(identity.alternates[1].kind.as_str(), "hostname_domain");
    assert_eq!(identity.alternates[1].value, "exch01.acme.test");
    assert_eq!(identity.handle(), "guid:it-glue:301");

    // Content fingerprints: equal content → equal fingerprint; any
    // field change moves it.
    let record = SyncRecord {
        external_id: "301".into(),
        kind: LinkKind::Asset,
        client_ref: None,
        title: "EXCH-01".into(),
        type_name: None,
        fields: fields.clone(),
        body: String::new(),
        tag: None,
        credentials: Vec::new(),
    };
    let a = Identity::fingerprint(&record);
    let b = Identity::fingerprint(&record);
    assert_eq!(a, b);
    let mut changed = record.clone();
    changed
        .fields
        .insert("serial".to_string(), "SN-2".to_string());
    assert_ne!(a, Identity::fingerprint(&changed));
}

// ---- wire conformance: stub servers ----------------------------------------------

/// Shared stub plumbing: an axum server on an ephemeral port with a
/// call log.
#[derive(Clone)]
struct CallLog {
    calls: Arc<Mutex<Vec<String>>>,
}

impl CallLog {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn record(&self, line: String) {
        self.calls.lock().unwrap().push(line);
    }

    fn lines(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

async fn start_stub(app: axum::Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), handle)
}

/// The Meridian public-API stub: the shapes the REST sink consumes.
struct MeridianStub {
    base_url: String,
    log: CallLog,
    _server: tokio::task::JoinHandle<()>,
}

impl MeridianStub {
    async fn start() -> Self {
        let log = CallLog::new();

        let layouts_log = log.clone();
        let layouts = axum::routing::get(move || {
            let log = layouts_log.clone();
            async move {
                log.record("GET /api/documents/layouts".into());
                axum::Json(json!([{
                    "id": Uuid::from_u128(1),
                    "slug": "server",
                    "name": "Server",
                    "is_system": true,
                    "fields": [
                        { "key": "hostname", "label": "Hostname", "type": "text", "required": true, "position": 0 }
                    ],
                }]))
            }
        });

        let clients_log = log.clone();
        let clients = axum::routing::get(move || {
            let log = clients_log.clone();
            async move {
                log.record("GET /api/clients".into());
                axum::Json(json!([{ "id": acme(), "name": "Acme Corp" }]))
            }
        });

        let assets_log = log.clone();
        let assets_create = axum::routing::post(
            move |headers: axum::http::HeaderMap, body: axum::Json<Value>| {
                let log = assets_log.clone();
                async move {
                    let key = headers
                        .get("x-api-key")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default();
                    log.record(format!(
                        "POST /api/assets x-api-key:{key} key:{}",
                        body.0
                            .get("idempotency_key")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                    ));
                    axum::Json(json!({
                        "id": Uuid::now_v7(),
                        "client_id": body.0.get("client_id"),
                        "tag": body.0.get("tag"),
                        "name": body.0.get("name"),
                        "status": "active",
                        "layout_id": body.0.get("layout_id"),
                        "fields": body.0.get("fields"),
                    }))
                }
            },
        );

        let passwords_log = log.clone();
        let passwords_create = axum::routing::post(
            move |headers: axum::http::HeaderMap, body: axum::Json<Value>| {
                let log = passwords_log.clone();
                async move {
                    let key = headers
                        .get("x-api-key")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default();
                    log.record(format!(
                        "POST /api/passwords x-api-key:{key} secret:{}",
                        body.0.get("secret").and_then(Value::as_str).unwrap_or("")
                    ));
                    axum::Json(json!({
                        "id": Uuid::now_v7(),
                        "title": body.0.get("title"),
                    }))
                }
            },
        );

        let app = axum::Router::new()
            .route("/api/documents/layouts", layouts)
            .route("/api/clients", clients)
            .route("/api/assets", assets_create)
            .route("/api/passwords", passwords_create);
        let (base_url, server) = start_stub(app).await;
        Self {
            base_url,
            log,
            _server: server,
        }
    }

    fn base_url(&self) -> String {
        self.base_url.clone()
    }

    fn http(&self) -> reqwest::Client {
        reqwest::Client::new()
    }

    fn lines(&self) -> Vec<String> {
        self.log.lines()
    }
}

#[tokio::test]
async fn meridian_sink_rides_the_public_api_shapes() {
    let stub = MeridianStub::start().await;
    let sink = crate::sink::meridian::MeridianRestSink::with_http(
        crate::sink::meridian::MeridianConfig {
            base_url: stub.base_url(),
            api_key: "mrd_test_key".into(),
        },
        stub.http(),
    );

    // Layouts list maps the wire shape into the neutral records.
    let layouts = sink.list_layouts().await.expect("layouts");
    assert_eq!(layouts.len(), 1);
    assert!(layouts[0].is_system);
    assert_eq!(layouts[0].fields[0].kind.as_str(), "text");

    // Asset create carries the idempotency key + x-api-key.
    let created = sink
        .create_asset(&AssetCreate {
            client_id: acme(),
            tag: "EXCH-01".into(),
            name: "EXCH-01".into(),
            layout_id: Some(Uuid::from_u128(1)),
            fields: json!({ "source_ref": "guid:it-glue:301", "serial": "SN-1" }),
            idempotency_key: "dms-it-glue-asset-guid:301".into(),
            change_note: "synced from it-glue".into(),
        })
        .await
        .expect("create");
    assert_eq!(created.tag, "EXCH-01");

    // Credentials ride POST /api/passwords ONLY, once, with the secret.
    let receipt = sink
        .create_password(&CredentialCreate {
            client_id: Some(acme()),
            title: "Router admin".into(),
            username: "admin".into(),
            url: String::new(),
            secret: "the-secret".into(),
            notes: None,
            expires_at: None,
            idempotency_key: "dms-cred-1".into(),
        })
        .await
        .expect("vault path");
    assert_ne!(receipt.password_id, Uuid::nil());

    let lines = stub.lines();
    assert!(lines.iter().any(|l| l.contains("x-api-key:mrd_test_key")));
    assert!(
        lines
            .iter()
            .any(|l| l.contains("POST /api/assets") && l.contains("dms-it-glue-asset-guid:301")),
        "idempotency key must ride the create body: {lines:?}"
    );
    let password_line = lines
        .iter()
        .find(|l| l.contains("POST /api/passwords"))
        .expect("password call logged");
    assert!(password_line.contains("the-secret"));
    // The secret crossed exactly once, on the vault path alone.
    assert_eq!(lines.iter().filter(|l| l.contains("the-secret")).count(), 1);
}

/// A generic REST DMS stub: one wrapped collection.
struct RestStub {
    base_url: String,
    _server: tokio::task::JoinHandle<()>,
}

impl RestStub {
    async fn start_with(body: Value) -> Self {
        let app = axum::Router::new().route(
            "/api/v2/assets",
            axum::routing::get(move || {
                let body = body.clone();
                async move { axum::Json(body) }
            }),
        );
        let (base_url, server) = start_stub(app).await;
        Self {
            base_url,
            _server: server,
        }
    }

    fn base_url(&self) -> String {
        self.base_url.clone()
    }
}

#[tokio::test]
async fn rest_source_pulls_declared_collections() {
    let stub = RestStub::start_with(json!({ "data": [
        {
            "id": "a-1",
            "type_name": "Server",
            "company": "Acme Corp",
            "hostname": "srv01.acme.test",
            "serial_number": "SN-R-1"
        }
    ]}))
    .await;

    let mut field_map = BTreeMap::new();
    field_map.insert("serial_number".to_string(), "serial".to_string());
    let collection = crate::source::rest::RestCollection {
        path: "api/v2/assets".into(),
        record_kind: crate::source::rest::RecordKind::Asset,
        id_key: "id".into(),
        type_key: Some("type_name".into()),
        client_key: Some("company".into()),
        title_key: "name".into(),
        body_key: None,
        field_map,
        field_kinds: Default::default(),
    };
    let source = crate::source::rest::RestSource::new(stub.base_url(), vec![collection]);
    let snapshot = source.pull_async().await.expect("pull");

    assert_eq!(snapshot.records.len(), 1);
    let record = &snapshot.records[0];
    assert_eq!(record.external_id, "a-1");
    assert_eq!(record.kind, LinkKind::Asset);
    assert_eq!(record.type_name.as_deref(), Some("Server"));
    assert_eq!(record.client_ref.as_deref(), Some("Acme Corp"));
    // field_map renamed the serial key; hostname carried under its own
    // name; the type/client keys did NOT become record fields.
    assert_eq!(
        record.fields.get("serial").map(String::as_str),
        Some("SN-R-1")
    );
    assert_eq!(
        record.fields.get("hostname").map(String::as_str),
        Some("srv01.acme.test")
    );
    assert!(!record.fields.contains_key("type_name"));
    assert!(!record.fields.contains_key("company"));
    // Identity derived for any DMS: serial alternate present.
    let identity = record.identity("generic-rest");
    assert!(identity.alternates.iter().any(|k| k.value == "SN-R-1"));
    // Layout synthesized from the observed keys (hostname, serial).
    let layout = snapshot
        .layouts
        .iter()
        .find(|l| l.name == "Server")
        .expect("layout");
    assert!(layout.fields.iter().any(|f| f.key == "serial"));
}

#[tokio::test]
async fn rest_push_target_pushes_neutral_records() {
    let pushed: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
    let layouts_capture = pushed.clone();
    let assets_capture = pushed.clone();
    let documents_capture = pushed.clone();
    let app = axum::Router::new()
        .route(
            "/dms/layouts",
            axum::routing::post(move |body: axum::Json<Value>| {
                let captured = layouts_capture.clone();
                async move {
                    captured
                        .lock()
                        .unwrap()
                        .push(("layouts".into(), body.0.clone()));
                    axum::http::StatusCode::OK
                }
            }),
        )
        .route(
            "/dms/assets",
            axum::routing::post(move |body: axum::Json<Value>| {
                let captured = assets_capture.clone();
                async move {
                    captured
                        .lock()
                        .unwrap()
                        .push(("assets".into(), body.0.clone()));
                    axum::http::StatusCode::OK
                }
            }),
        )
        .route(
            "/dms/documents",
            axum::routing::post(move |body: axum::Json<Value>| {
                let captured = documents_capture.clone();
                async move {
                    captured
                        .lock()
                        .unwrap()
                        .push(("documents".into(), body.0.clone()));
                    axum::http::StatusCode::OK
                }
            }),
        );
    let (base_url, _server) = start_stub(app).await;

    let target = push::RestPushTarget::new(
        base_url,
        push::RestPushPaths {
            layouts: "dms/layouts".into(),
            assets: "dms/assets".into(),
            documents: "dms/documents".into(),
        },
    );
    let snapshot = push::MeridianSnapshot {
        source: "meridian".into(),
        clients: vec![(acme(), "Acme Corp".into())],
        layouts: vec![push::SnapshotLayout {
            id: Uuid::from_u128(1),
            slug: "server".into(),
            name: "Server".into(),
            is_system: false,
            fields: vec![crate::model::SyncField::new(
                "hostname",
                "Hostname",
                crate::model::FieldKind::Text,
                0,
            )],
        }],
        assets: vec![push::SnapshotAsset {
            id: Uuid::from_u128(2),
            client_id: Some(acme()),
            tag: "EXCH-01".into(),
            name: "EXCH-01".into(),
            kind: "device".into(),
            status: "active".into(),
            layout_id: Some(Uuid::from_u128(1)),
            fields: json!({ "hostname": "exch01.acme.test" })
                .as_object()
                .cloned()
                .unwrap(),
        }],
        documents: vec![push::SnapshotDocument {
            id: Uuid::from_u128(3),
            client_id: None,
            title: "Runbook".into(),
            layout_id: None,
            fields: Default::default(),
            body: "content".into(),
        }],
        links: Vec::new(),
        credential_metadata: Vec::new(),
    };

    let accepted = target.push_snapshot(&snapshot).await.expect("push");
    assert_eq!(accepted, 3);
    let captured = pushed.lock().unwrap();
    assert_eq!(captured.len(), 3);
    assert_eq!(captured[0].0, "layouts");
    assert_eq!(
        captured[0].1.get("slug").and_then(Value::as_str),
        Some("server")
    );
    assert_eq!(captured[1].0, "assets");
    assert_eq!(captured[2].0, "documents");
}

// ---- ledger + state semantics -----------------------------------------------------

#[test]
fn memory_ledger_remember_forget_and_reverse_lookup() {
    let state = MemorySyncState::new();
    let meridian_id = Uuid::from_u128(42);
    state.remember(LedgerEntry {
        source_key: "asset:guid:it-glue:301".into(),
        meridian_kind: "asset",
        meridian_id,
        last_written_fields: json!({ "serial": "SN-1" }),
        last_written_fingerprint: "abc".into(),
        updated_at: chrono::Utc::now(),
    });

    let entry = state.lookup("asset:guid:it-glue:301").expect("entry");
    assert!(entry.is_asset());
    assert_eq!(
        state.source_key_of(meridian_id).as_deref(),
        Some("asset:guid:it-glue:301")
    );

    // Re-keying moves the reverse mapping.
    state.remember(LedgerEntry {
        source_key: "asset:guid:it-glue:301".into(),
        meridian_kind: "asset",
        meridian_id: Uuid::from_u128(43),
        last_written_fields: json!({}),
        last_written_fingerprint: "def".into(),
        updated_at: chrono::Utc::now(),
    });
    assert!(state.source_key_of(meridian_id).is_none());
    assert_eq!(
        state.source_key_of(Uuid::from_u128(43)).as_deref(),
        Some("asset:guid:it-glue:301")
    );

    state.forget("asset:guid:it-glue:301");
    assert!(state.lookup("asset:guid:it-glue:301").is_none());
    assert!(state.source_key_of(Uuid::from_u128(43)).is_none());
}

#[test]
fn source_ref_key_is_the_provenance_anchor() {
    // The provenance key must be part of every connector layout and
    // stable across releases (cold-ledger rediscovery depends on it).
    assert_eq!(SOURCE_REF_KEY, "source_ref");
}
