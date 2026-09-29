//! The sync engine: ADR 0003 semantics over a [`DmsSnapshot`] and a
//! [`DmsSink`].
//!
//! Semantics (binding per the lane brief):
//!
//! * **Feeds upsert, never blind-insert.** Every record resolves
//!   through its deterministic identity keys — ledger first (the
//!   connector's own mapping), then deterministic search over the
//!   sink's records (`source_ref` = ours; serial / hostname+domain =
//!   fallback). A source's second sync updates its own record; it never
//!   creates a twin.
//! * **Ambiguity becomes a reviewable merge candidate.** When a
//!   deterministic match points at a record this connector does not
//!   own (someone else's server with the same serial), the engine does
//!   NOT pick a winner: the run report carries the candidate with
//!   evidence and the record is skipped. Adoption exists behind
//!   `auto_link_conflicts` (default OFF — the agent-approval default).
//! * **Curated beats observed.** The engine tracks what IT wrote (the
//!   ledger's `last_written_fields`). A field whose sink value drifted
//!   from the last-written value is a human edit (curated): source
//!   churn must not erase it. Curated fields stay; the source's newer
//!   value surfaces in the report as one-click "accept the new value"
//!   evidence.
//! * **Layouts map onto `document_layouts`.** Connector org layouts are
//!   created for external types (full lossless schema + a `source_ref`
//!   provenance field). Seeded `is_system` rows are only ever READ and
//!   MATCHED: a system layout is used when the external type's field
//!   keys fit inside its schema and every required system key is
//!   carried by the external schema — otherwise the external type gets
//!   its own org layout. The engine never edits a layout it did not
//!   create.
//! * **Links land in `document_links`.** Both ends must resolve; an
//!   unresolved end is reported (broken target surfaced), never
//!   invented, never silently dropped, never cascade-deleted.
//! * **Vault boundary.** Credentials reach the sink ONLY when
//!   `map_credentials` is on; otherwise they are counted and skipped
//!   without a single byte crossing the seam.
//!
//! Writes ride the sink's instant-first envelope (deterministic
//! idempotency keys), so server-side audit, provenance, IF7 undo, and
//! the registered `asset.*` / `document.*` contract events apply
//! unchanged.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use uuid::Uuid;

use crate::identity::{Identity, IdentityKeyKind};
use crate::model::{FieldKind, LinkKind, SyncField, SyncLayout, SyncRecord};
use crate::sink::{
    AssetCreate, AssetRecord, AssetUpdate, DmsSink, DocumentCreate, DocumentRecord, DocumentUpdate,
    LayoutRecord, LinkCreate, SinkError,
};
use crate::state::{LedgerEntry, SyncState};

/// The provenance field the connector owns on the layouts it creates:
/// carries the record's deterministic identity handle — how a
/// cold-ledger sync rediscovers its own records.
pub const SOURCE_REF_KEY: &str = "source_ref";

/// How a run applies: full apply, or a plan-only dry run (report
/// without any sink mutation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApplyMode {
    /// Apply everything uncontested; report the rest.
    #[default]
    Apply,
    /// No sink mutation; the report describes what WOULD happen.
    DryRun,
}

/// The sync configuration.
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Apply or dry-run.
    pub apply: ApplyMode,
    /// Vault boundary: forward source credentials to the audited vault
    /// path. DEFAULT OFF — credentials are counted and skipped until
    /// an operator turns this on deliberately.
    pub map_credentials: bool,
    /// Adopt deterministic matches this connector does not own (fill
    /// EMPTY fields + record provenance; never overwrite populated
    /// values). Default OFF: such matches are merge candidates.
    pub auto_link_conflicts: bool,
    /// Map external types onto seeded system `document_layouts` when
    /// their field schema fits cleanly (field keys are a subset and
    /// every required system key is carried). When off, every external
    /// type gets its own connector org layout.
    pub use_system_layouts: bool,
    /// External client/organization name → Marula client name
    /// overrides (applied before case-insensitive name matching).
    pub client_aliases: BTreeMap<String, String>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            apply: ApplyMode::default(),
            map_credentials: false,
            auto_link_conflicts: false,
            use_system_layouts: true,
            client_aliases: Default::default(),
        }
    }
}

/// The run report — the operator's reviewable output. Serialized; the
/// secret-carrying types never reach it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncReport {
    pub source: String,
    pub dry_run: bool,
    pub layouts_created: Vec<String>,
    pub layouts_mapped_system: Vec<String>,
    /// (external type name, sink layout slug)
    pub layouts_mapped_org: Vec<(String, String)>,
    pub assets_created: usize,
    pub assets_updated: usize,
    pub documents_created: usize,
    pub documents_updated: usize,
    pub records_skipped: usize,
    pub links_created: usize,
    pub links_existing: usize,
    /// Relationship ends that resolved to nothing — surfaced, never
    /// silently dropped, never cascade-deleted.
    pub unresolved_links: Vec<UnresolvedLink>,
    /// Deterministic matches against records this connector does not
    /// own — reviewable evidence, never auto-merged.
    pub merge_candidates: Vec<MergeCandidate>,
    /// Curated-field conflicts: human-edited values the source tried
    /// to overwrite. The source value rides the entry.
    pub curated_conflicts: Vec<CuratedConflict>,
    /// Records whose client scope resolved to no Marula client.
    pub unmapped_clients: Vec<UnmappedRecord>,
    /// Fields a system-layout mapping could not carry (data preserved
    /// in the report, visible to the operator).
    pub unmapped_fields: Vec<UnmappedField>,
    /// Required schema fields the source record did not supply (the
    /// record is skipped rather than invented).
    pub missing_required: Vec<UnmappedRecord>,
    pub credentials_skipped: usize,
    pub credentials_mapped: usize,
    /// Records skipped for ambiguous deterministic matches.
    pub ambiguous: Vec<UnmappedRecord>,
    /// Unrecoverable per-record errors (typed; the run continues).
    pub errors: Vec<RunError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnresolvedLink {
    pub from_external_id: String,
    pub from_kind: String,
    pub to_external_id: String,
    pub to_kind: String,
    /// Which end failed: "from" | "to" | "both".
    pub missing: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergeCandidate {
    pub external_id: String,
    pub external_title: String,
    pub record_kind: String,
    pub marula_id: Uuid,
    /// Why the engine matched (key kind + value).
    pub matched_on: String,
    pub marula_label: String,
    /// Field evidence: (field, external value, marula value).
    pub evidence: Vec<(String, String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CuratedConflict {
    pub external_id: String,
    pub marula_id: Uuid,
    pub field: String,
    /// The human-edited value that stays.
    pub curated_value: String,
    /// The source's newer value (accept-the-new-value evidence).
    pub source_value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnmappedRecord {
    pub external_id: String,
    pub title: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnmappedField {
    pub external_id: String,
    pub field: String,
    pub layout_slug: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunError {
    pub external_id: String,
    pub error: String,
}

/// Layout decision for one external type.
enum LayoutResolution {
    /// A seeded system layout fits the external schema cleanly.
    System(Uuid),
    /// An existing org layout with the matching slug.
    ExistingOrg(Uuid),
    /// Create the connector org layout (lossless schema + source_ref).
    Create,
}

/// Deterministic indexes over the sink's records, rebuilt each run.
#[derive(Debug, Default)]
struct RecordIndexes {
    /// fields.source_ref == handle → id (this connector's own records).
    ours_by_ref: HashMap<String, Uuid>,
    /// serial value → ids (any record, ours or not).
    serials: HashMap<String, Vec<Uuid>>,
    /// hostname value (lowercased) → ids.
    hostnames: HashMap<String, Vec<Uuid>>,
}

impl RecordIndexes {
    fn from_assets(assets: &[AssetRecord]) -> Self {
        let mut indexes = Self::default();
        for asset in assets {
            indexes.absorb(asset.fields.as_object(), asset.id);
        }
        indexes
    }

    fn from_documents(documents: &[DocumentRecord]) -> Self {
        let mut indexes = Self::default();
        for document in documents {
            indexes.absorb(document.fields.as_object(), document.id);
        }
        indexes
    }

    fn absorb(&mut self, fields: Option<&serde_json::Map<String, serde_json::Value>>, id: Uuid) {
        let Some(fields) = fields else {
            return;
        };
        if let Some(reference) = fields
            .get(SOURCE_REF_KEY)
            .and_then(serde_json::Value::as_str)
        {
            self.ours_by_ref.insert(reference.to_string(), id);
        }
        if let Some(serial) = fields
            .get("serial")
            .and_then(serde_json::Value::as_str)
            .filter(|v| !v.is_empty())
        {
            self.serials.entry(serial.to_string()).or_default().push(id);
        }
        if let Some(hostname) = fields
            .get("hostname")
            .and_then(serde_json::Value::as_str)
            .filter(|v| !v.is_empty())
        {
            self.hostnames
                .entry(hostname.to_lowercase())
                .or_default()
                .push(id);
        }
    }
}

/// The shaped field values for one record under one target layout.
struct FieldShape {
    /// Schema-covered source fields (external key → value).
    fields: BTreeMap<String, String>,
    /// Source fields the layout schema cannot carry (reported).
    unmapped_keys: Vec<String>,
    /// Required schema keys the source record did not supply.
    missing_required: Vec<String>,
}

impl FieldShape {
    /// The record's field values as the docs-platform `fields` object,
    /// plus the connector's `source_ref` when a handle is given and the
    /// layout carries the provenance field.
    fn to_fields_value(
        &self,
        identity_handle: Option<&str>,
        layout: &LayoutRecord,
    ) -> serde_json::Value {
        let mut object = serde_json::Map::new();
        for (key, value) in &self.fields {
            object.insert(key.clone(), serde_json::Value::String(value.clone()));
        }
        if let (Some(handle), true) = (
            identity_handle,
            layout.fields.iter().any(|f| f.key == SOURCE_REF_KEY),
        ) {
            object.insert(
                SOURCE_REF_KEY.to_string(),
                serde_json::Value::String(handle.to_string()),
            );
        }
        serde_json::Value::Object(object)
    }
}

/// Shape a source record's fields to a target layout schema: keys in
/// the schema carry; keys outside it are REPORTED, never dropped from
/// the run's visibility and never invented into the schema.
fn shape_fields(record: &SyncRecord, layout: &LayoutRecord) -> FieldShape {
    let mut fields = BTreeMap::new();
    let mut unmapped_keys = Vec::new();
    for (key, value) in &record.fields {
        if layout.fields.iter().any(|f| &f.key == key) {
            fields.insert(key.clone(), value.clone());
        } else {
            unmapped_keys.push(key.clone());
        }
    }
    let mut missing_required = Vec::new();
    for field in &layout.fields {
        if field.required && !record.fields.contains_key(&field.key) {
            missing_required.push(field.key.clone());
        }
    }
    FieldShape {
        fields,
        unmapped_keys,
        missing_required,
    }
}

/// The engine. Stateless beyond its configuration.
pub struct SyncEngine {
    config: SyncConfig,
}

/// A resolved Marula endpoint for link building.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ResolvedEndpoint {
    kind: LinkKind,
    id: Uuid,
}

impl SyncEngine {
    pub fn new(config: SyncConfig) -> Self {
        Self { config }
    }

    /// Run one sync pass: layouts → client index → records (assets,
    /// documents) → links → credentials. Returns the reviewable
    /// report.
    pub async fn run(
        &self,
        snapshot: &crate::model::DmsSnapshot,
        sink: &dyn DmsSink,
        state: &dyn SyncState,
    ) -> Result<SyncReport, crate::model::SyncError> {
        let mut report = SyncReport {
            source: snapshot.source.clone(),
            dry_run: self.config.apply == ApplyMode::DryRun,
            ..Default::default()
        };
        let dry_run = report.dry_run;
        let mut resolved_endpoints: HashMap<(String, LinkKind), ResolvedEndpoint> =
            Default::default();

        // ---- Phase 0: layouts (records validate against them) -------------
        let mut layout_catalog = match sink.list_layouts().await {
            Ok(layouts) => layouts,
            Err(SinkError::Disabled) => {
                // None mode: zero network performed, "would have synced"
                // is the honest report.
                report.records_skipped = snapshot.records.len();
                report.credentials_skipped = snapshot.credentials.len();
                report.errors.push(RunError {
                    external_id: "*".into(),
                    error: "none mode: no Marula connection configured (sink disabled)".into(),
                });
                return Ok(report);
            }
            Err(e) => return Err(crate::model::SyncError::Sink(e.to_string())),
        };
        let mut layout_by_type: HashMap<String, Option<Uuid>> = Default::default();
        for source_layout in &snapshot.layouts {
            match self.resolve_layout(source_layout, &layout_catalog) {
                LayoutResolution::System(id) => {
                    layout_by_type.insert(source_layout.name.clone(), Some(id));
                    report
                        .layouts_mapped_system
                        .push(source_layout.name.clone());
                }
                LayoutResolution::ExistingOrg(id) => {
                    layout_by_type.insert(source_layout.name.clone(), Some(id));
                    let slug = layout_catalog
                        .iter()
                        .find(|l| l.id == id)
                        .map(|l| l.slug.clone())
                        .unwrap_or_default();
                    report
                        .layouts_mapped_org
                        .push((source_layout.name.clone(), slug));
                }
                LayoutResolution::Create => {
                    match self
                        .ensure_connector_layout(snapshot, sink, source_layout, dry_run)
                        .await
                    {
                        Ok(created) => {
                            report.layouts_created.push(created.slug.clone());
                            layout_catalog.push(created.clone());
                            layout_by_type.insert(source_layout.name.clone(), Some(created.id));
                        }
                        Err(e) => {
                            report.errors.push(RunError {
                                external_id: source_layout.external_id.clone(),
                                error: format!("layout: {e}"),
                            });
                            layout_by_type.insert(source_layout.name.clone(), None);
                        }
                    }
                }
            }
        }

        // Record types with no source layout get one synthesized from
        // their observed fields (union) via the same connector-layout
        // path — external records are never dropped for lacking a
        // declared type schema.
        for record in &snapshot.records {
            let type_name = record.type_name.clone().unwrap_or_default();
            if type_name.is_empty() || layout_by_type.contains_key(&type_name) {
                continue;
            }
            let synthesized = synthesize_layout(&snapshot.source, &snapshot.records, &type_name);
            match self
                .ensure_connector_layout(snapshot, sink, &synthesized, dry_run)
                .await
            {
                Ok(created) => {
                    report.layouts_created.push(created.slug.clone());
                    layout_catalog.push(created.clone());
                    layout_by_type.insert(type_name.clone(), Some(created.id));
                }
                Err(e) => {
                    report.errors.push(RunError {
                        external_id: record.external_id.clone(),
                        error: format!("layout for type {type_name:?}: {e}"),
                    });
                    layout_by_type.insert(type_name.clone(), None);
                }
            }
        }

        // ---- Client index -----------------------------------------------------
        let clients = match sink.list_clients().await {
            Ok(clients) => clients,
            Err(SinkError::Disabled) => return Ok(report),
            Err(e) => return Err(crate::model::SyncError::Sink(e.to_string())),
        };
        let client_by_name: BTreeMap<String, Uuid> = clients
            .iter()
            .map(|c| (c.name.trim().to_lowercase(), c.id))
            .collect();

        // ---- Record indexes (deterministic resolution substrate) ---------------
        let assets = match sink.list_assets().await {
            Ok(assets) => assets,
            Err(SinkError::Disabled) => return Ok(report),
            Err(e) => return Err(crate::model::SyncError::Sink(e.to_string())),
        };
        let documents = match sink.list_documents().await {
            Ok(documents) => documents,
            Err(SinkError::Disabled) => return Ok(report),
            Err(e) => return Err(crate::model::SyncError::Sink(e.to_string())),
        };
        let asset_index = RecordIndexes::from_assets(&assets);
        let document_index = RecordIndexes::from_documents(&documents);
        let assets_by_id: HashMap<Uuid, &AssetRecord> = assets.iter().map(|a| (a.id, a)).collect();
        let documents_by_id: HashMap<Uuid, &DocumentRecord> =
            documents.iter().map(|d| (d.id, d)).collect();

        // ---- Records -----------------------------------------------------------
        for record in &snapshot.records {
            let type_name = record.type_name.clone().unwrap_or_default();
            let layout_id = layout_by_type.get(&type_name).copied().flatten();
            let Some(layout_id) = layout_id else {
                report.records_skipped += 1;
                report.errors.push(RunError {
                    external_id: record.external_id.clone(),
                    error: format!("no resolvable layout for type {:?}", type_name),
                });
                continue;
            };
            let Some(layout) = layout_catalog.iter().find(|l| l.id == layout_id) else {
                report.records_skipped += 1;
                continue;
            };

            // Client scope resolution. Assets are client-scoped by
            // construction (device_assets is client-bound); org-global
            // documents are legitimate scope.
            let client_id = match &record.client_ref {
                None => match record.kind {
                    LinkKind::Document => None,
                    _ => {
                        report.unmapped_clients.push(UnmappedRecord {
                            external_id: record.external_id.clone(),
                            title: record.title.clone(),
                            reason: "asset carries no client scope".into(),
                        });
                        report.records_skipped += 1;
                        continue;
                    }
                },
                Some(_) => match resolve_client(record, snapshot, &self.config, &client_by_name) {
                    Some(id) => Some(id),
                    None => {
                        report.unmapped_clients.push(UnmappedRecord {
                            external_id: record.external_id.clone(),
                            title: record.title.clone(),
                            reason: format!("no Marula client matches {:?}", record.client_ref),
                        });
                        report.records_skipped += 1;
                        continue;
                    }
                },
            };

            // Schema shaping.
            let shaped = shape_fields(record, layout);
            if !shaped.missing_required.is_empty() {
                report.missing_required.push(UnmappedRecord {
                    external_id: record.external_id.clone(),
                    title: record.title.clone(),
                    reason: format!(
                        "required field(s) {} missing for layout {:?}",
                        shaped.missing_required.join(", "),
                        layout.slug
                    ),
                });
                report.records_skipped += 1;
                continue;
            }
            for field in &shaped.unmapped_keys {
                report.unmapped_fields.push(UnmappedField {
                    external_id: record.external_id.clone(),
                    field: field.clone(),
                    layout_slug: layout.slug.clone(),
                });
            }

            let identity = record.identity(&snapshot.source);
            match record.kind {
                LinkKind::Asset => {
                    let outcome = self
                        .sync_asset(
                            snapshot,
                            record,
                            &identity,
                            layout,
                            client_id,
                            &asset_index,
                            &assets_by_id,
                            sink,
                            state,
                            dry_run,
                            &mut report,
                        )
                        .await;
                    register_outcome(
                        outcome,
                        record,
                        &identity,
                        LinkKind::Asset,
                        "asset",
                        "asset",
                        &mut report,
                        &mut resolved_endpoints,
                    );
                }
                LinkKind::Document => {
                    let outcome = self
                        .sync_document(
                            snapshot,
                            record,
                            &identity,
                            layout,
                            client_id,
                            &document_index,
                            &documents,
                            &documents_by_id,
                            sink,
                            state,
                            dry_run,
                            &mut report,
                        )
                        .await;
                    register_outcome(
                        outcome,
                        record,
                        &identity,
                        LinkKind::Document,
                        "document",
                        "document",
                        &mut report,
                        &mut resolved_endpoints,
                    );
                }
                _ => {}
            }
        }

        // ---- Links ------------------------------------------------------------
        for relation in &snapshot.relations {
            let from = resolved_endpoints
                .get(&(relation.from.external_id.clone(), relation.from.kind))
                .copied();
            let to = resolved_endpoints
                .get(&(relation.to.external_id.clone(), relation.to.kind))
                .copied();
            let (from_missing, to_missing) = (from.is_none(), to.is_none());
            if from_missing || to_missing {
                report.unresolved_links.push(UnresolvedLink {
                    from_external_id: relation.from.external_id.clone(),
                    from_kind: relation.from.kind.as_str().to_string(),
                    to_external_id: relation.to.external_id.clone(),
                    to_kind: relation.to.kind.as_str().to_string(),
                    missing: match (from_missing, to_missing) {
                        (true, true) => "both".into(),
                        (true, false) => "from".into(),
                        (false, true) => "to".into(),
                        (false, false) => unreachable!(),
                    },
                });
                continue;
            }
            let from = from.expect("checked above");
            let to = to.expect("checked above");
            // Dry-run endpoints for created records carry nil ids — the
            // link would resolve in a real run.
            if from.id.is_nil() || to.id.is_nil() {
                report.links_created += 1;
                continue;
            }
            match sink.list_links(from.kind, from.id).await {
                Ok(links) => {
                    if links
                        .iter()
                        .any(|l| l.to_kind == to.kind && l.to_id == to.id)
                    {
                        report.links_existing += 1;
                        continue;
                    }
                }
                Err(SinkError::Disabled) => return Ok(report),
                Err(e) => {
                    report.errors.push(RunError {
                        external_id: relation.from.external_id.clone(),
                        error: format!("list links: {e}"),
                    });
                    continue;
                }
            }
            if dry_run {
                report.links_created += 1;
                continue;
            }
            let plan = LinkCreate {
                from_kind: from.kind,
                from_id: from.id,
                to_kind: to.kind,
                to_id: to.id,
                idempotency_key: format!(
                    "dms-{}-link-{}:{}-{}:{}",
                    snapshot.source,
                    relation.from.kind.as_str(),
                    relation.from.external_id,
                    relation.to.kind.as_str(),
                    relation.to.external_id
                ),
            };
            match sink.create_link(&plan).await {
                Ok(_) => report.links_created += 1,
                // An identical existing link is success (idempotent).
                Err(SinkError::Conflict(_)) => report.links_existing += 1,
                Err(SinkError::Disabled) => return Ok(report),
                Err(e) => report.errors.push(RunError {
                    external_id: relation.from.external_id.clone(),
                    error: format!("link: {e}"),
                }),
            }
        }

        // ---- Credentials: the vault boundary is HERE ---------------------------
        report.credentials_skipped = snapshot.credentials.len();
        if self.config.map_credentials && !snapshot.credentials.is_empty() {
            for credential in &snapshot.credentials {
                // Credential client refs resolve through the same
                // client-ref map + aliases as records.
                let client_id = credential.client_ref.as_deref().and_then(|reference| {
                    let display = snapshot
                        .client_refs
                        .get(reference)
                        .map(String::as_str)
                        .unwrap_or(reference);
                    resolve_client_name(display, &self.config.client_aliases, &client_by_name)
                });
                if credential.client_ref.is_some() && client_id.is_none() {
                    report.unmapped_clients.push(UnmappedRecord {
                        external_id: credential.external_id.clone(),
                        title: credential.title.clone(),
                        reason: "credential has no resolvable client scope".into(),
                    });
                    continue;
                }
                if dry_run {
                    report.credentials_mapped += 1;
                    continue;
                }
                let plan = crate::sink::CredentialCreate {
                    client_id,
                    title: credential.title.clone(),
                    username: credential.username.clone(),
                    url: credential.url.clone(),
                    secret: credential.secret.clone(),
                    notes: credential.notes.clone(),
                    expires_at: credential.expires_at,
                    idempotency_key: format!(
                        "dms-{}-credential-{}",
                        snapshot.source, credential.external_id
                    ),
                };
                match sink.create_password(&plan).await {
                    Ok(_) => report.credentials_mapped += 1,
                    Err(SinkError::Disabled) => return Ok(report),
                    Err(e) => report.errors.push(RunError {
                        external_id: credential.external_id.clone(),
                        error: format!("password: {e}"),
                    }),
                }
            }
        }

        Ok(report)
    }

    // ---- layout resolution -------------------------------------------------

    fn resolve_layout(&self, source: &SyncLayout, layouts: &[LayoutRecord]) -> LayoutResolution {
        // Exact slug match on the external type's own name.
        let slug = slugify(&source.name);
        if let Some(existing) = layouts.iter().find(|l| l.slug == slug) {
            if existing.is_system {
                if self.config.use_system_layouts && system_compatible(source, existing) {
                    return LayoutResolution::System(existing.id);
                }
                // The system layout exists but does not fit: fall
                // through to a connector org layout (is_system rows
                // are never touched).
                return LayoutResolution::Create;
            }
            // An org layout with this exact slug: use it; the connector
            // never edits it (field diffs surface as unmapped_fields).
            return LayoutResolution::ExistingOrg(existing.id);
        }
        // No match: the external type gets its own connector org
        // layout — created losslessly, never a system row.
        LayoutResolution::Create
    }

    /// Creates the connector org layout for an external type (or adopts
    /// an existing org layout with the same slug). Never creates a
    /// system row.
    async fn ensure_connector_layout(
        &self,
        snapshot: &crate::model::DmsSnapshot,
        sink: &dyn DmsSink,
        source_layout: &SyncLayout,
        dry_run: bool,
    ) -> Result<LayoutRecord, SinkError> {
        let slug = connector_slug(&snapshot.source, &source_layout.name);
        if dry_run {
            // Distinct pseudo-ids keep the dry-run catalog lookups
            // unambiguous (a nil id would collide across types).
            return Ok(LayoutRecord {
                id: dry_run_id(&slug),
                slug,
                name: format!("{} {}", source_label(&snapshot.source), source_layout.name),
                is_system: false,
                fields: source_layout.fields.clone(),
            });
        }
        let mut fields = source_layout.fields.clone();
        if !fields.iter().any(|f| f.key == SOURCE_REF_KEY) {
            fields.push(SyncField::new(
                SOURCE_REF_KEY,
                "Source reference",
                FieldKind::Text,
                i32::try_from(fields.len()).unwrap_or(i32::MAX),
            ));
        }
        let new_layout = crate::sink::NewLayout {
            slug: slug.clone(),
            name: format!("{} {}", source_label(&snapshot.source), source_layout.name),
            description: if source_layout.description.is_empty() {
                format!("Synced from {} ({})", source_layout.name, snapshot.source)
            } else {
                source_layout.description.clone()
            },
            icon: source_layout.icon.clone(),
            fields,
        };
        match sink.create_layout(&new_layout).await {
            Ok(created) => Ok(created),
            Err(SinkError::Conflict(_)) => {
                // Slug taken (another run created it): adopt the
                // existing ORG layout — never a system row.
                let layouts = sink.list_layouts().await?;
                layouts
                    .into_iter()
                    .find(|l| l.slug == slug && !l.is_system)
                    .ok_or_else(|| {
                        SinkError::InvalidRequest(format!(
                            "layout slug {slug:?} is taken by a system row; the external type cannot map"
                        ))
                    })
            }
            Err(e) => Err(e),
        }
    }

    // ---- asset phase ---------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    async fn sync_asset(
        &self,
        snapshot: &crate::model::DmsSnapshot,
        record: &SyncRecord,
        identity: &Identity,
        layout: &LayoutRecord,
        client_id: Option<Uuid>,
        index: &RecordIndexes,
        assets_by_id: &HashMap<Uuid, &AssetRecord>,
        sink: &dyn DmsSink,
        state: &dyn SyncState,
        dry_run: bool,
        report: &mut SyncReport,
    ) -> Outcome {
        let Some(client_id) = client_id else {
            return Outcome::Skipped("no client scope".into());
        };
        let handle = identity.handle();
        let ledger_key = format!("asset:{}", handle);
        let fingerprint = Identity::fingerprint(record);
        let shaped = shape_fields(record, layout);

        // 1) Ledger first: "second sync updates its own record".
        let mut ours: Option<Uuid> = None;
        if let Some(entry) = state.lookup(&ledger_key).filter(|e| e.is_asset()) {
            if assets_by_id.contains_key(&entry.marula_id) {
                ours = Some(entry.marula_id);
            } else {
                // The record vanished Marula-side; forget and fall
                // through to deterministic rediscovery.
                state.forget(&ledger_key);
            }
        }
        // 2) Cold-ledger rediscovery through the provenance field.
        if ours.is_none() {
            ours = index.ours_by_ref.get(&handle).copied();
        }

        // 3) Deterministic alternates (serial, then hostname+domain).
        if ours.is_none() {
            let mut candidate: Option<(Uuid, String)> = None;
            for key in &identity.alternates {
                let hits: &[Uuid] = match key.kind {
                    IdentityKeyKind::Serial => index
                        .serials
                        .get(&key.value)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                    IdentityKeyKind::HostnameDomain => index
                        .hostnames
                        .get(&key.value)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                    IdentityKeyKind::Guid => &[],
                };
                if hits.len() > 1 {
                    return Outcome::Ambiguous(format!(
                        "{} matches {} Marula assets on {}",
                        key.value,
                        hits.len(),
                        key.kind.as_str()
                    ));
                }
                if let Some(hit) = hits.first() {
                    candidate = Some((*hit, format!("{}={}", key.kind.as_str(), key.value)));
                    break;
                }
            }
            let Some((matched_id, matched_on)) = candidate else {
                // 4) Fresh create — deterministic idempotency key, so a
                //    retried create replays the stored result, never a
                //    twin.
                let plan = AssetCreate {
                    client_id,
                    tag: tag_of(record),
                    name: record.title.clone(),
                    layout_id: Some(layout.id),
                    fields: shaped.to_fields_value(Some(&handle), layout),
                    idempotency_key: format!("dms-{}-asset-{}", snapshot.source, handle),
                    change_note: format!("synced from {}", snapshot.source),
                };
                if dry_run {
                    return Outcome::CreatedDryRun;
                }
                return match sink.create_asset(&plan).await {
                    Ok(created) => {
                        state.remember(LedgerEntry {
                            source_key: ledger_key,
                            marula_kind: "asset",
                            marula_id: created.id,
                            last_written_fields: plan.fields.clone(),
                            last_written_fingerprint: fingerprint,
                            updated_at: chrono::Utc::now(),
                        });
                        Outcome::Created(created.id)
                    }
                    Err(e) => Outcome::Failed(format!("create: {e}")),
                };
            };

            // Deterministic match against a record we do NOT own.
            let Some(matched) = assets_by_id.get(&matched_id) else {
                return Outcome::Skipped("matched record vanished mid-sync".into());
            };
            if !self.config.auto_link_conflicts {
                return Outcome::MergeCandidate {
                    matched_on,
                    marula_id: matched_id,
                    label: asset_label(matched),
                    evidence: field_evidence(record, matched.fields.as_object()),
                };
            }
            // Adoption: fill EMPTY fields + record provenance; never
            // overwrite a populated value (those surface as merge
            // candidates via the evidence, and the operator decides).
            let mut next_fields = serde_json::Map::new();
            if let Some(existing) = matched.fields.as_object() {
                next_fields = existing.clone();
            }
            let mut filled_any = false;
            for (key, value) in &shaped.fields {
                let empty = next_fields
                    .get(key)
                    .map(|v| v.as_str().map(str::is_empty).unwrap_or(true))
                    .unwrap_or(true);
                if empty {
                    next_fields.insert(key.clone(), serde_json::Value::String(value.clone()));
                    filled_any = true;
                }
            }
            let empty_ref = next_fields
                .get(SOURCE_REF_KEY)
                .map(|v| v.as_str().map(str::is_empty).unwrap_or(true))
                .unwrap_or(true);
            if empty_ref {
                next_fields.insert(
                    SOURCE_REF_KEY.to_string(),
                    serde_json::Value::String(handle.clone()),
                );
                filled_any = true;
            }
            if dry_run {
                return Outcome::AdoptedDryRun;
            }
            if !filled_any {
                // Nothing to fill: record provenance so the ledger
                // owns this record from now on (no write needed).
                state.remember(LedgerEntry {
                    source_key: ledger_key,
                    marula_kind: "asset",
                    marula_id: matched_id,
                    last_written_fields: serde_json::to_value(&next_fields).unwrap_or_default(),
                    last_written_fingerprint: fingerprint,
                    updated_at: chrono::Utc::now(),
                });
                return Outcome::Adopted(matched_id);
            }
            let plan = AssetUpdate {
                name: None,
                status: None,
                layout_id: matched.layout_id,
                fields: Some(serde_json::Value::Object(next_fields.clone())),
                change_note: None,
                idempotency_key: format!(
                    "dms-{}-asset-{}-adopt-{}",
                    snapshot.source,
                    handle,
                    short(&fingerprint)
                ),
            };
            return match sink.update_asset(matched_id, &plan).await {
                Ok(_) => {
                    state.remember(LedgerEntry {
                        source_key: ledger_key,
                        marula_kind: "asset",
                        marula_id: matched_id,
                        last_written_fields: serde_json::Value::Object(next_fields),
                        last_written_fingerprint: fingerprint,
                        updated_at: chrono::Utc::now(),
                    });
                    Outcome::Adopted(matched_id)
                }
                Err(e) => Outcome::Failed(format!("adopt: {e}")),
            };
        }

        // Ours: curated-beats-observed diff. Skip before diffing when
        // the source content is unchanged since the last write (no
        // version churn).
        let Some(marula_id) = ours else {
            return Outcome::Skipped("resolution fell through".into());
        };
        let Some(current) = assets_by_id.get(&marula_id) else {
            return Outcome::Skipped("our record vanished mid-sync".into());
        };
        let entry = state.lookup(&ledger_key);
        if entry
            .as_ref()
            .map(|e| e.last_written_fingerprint == fingerprint)
            .unwrap_or(false)
        {
            return Outcome::Unchanged(marula_id);
        }
        let last_written = entry
            .map(|e| e.last_written_fields)
            .unwrap_or(current.fields.clone());
        let (next_fields, conflicts) = merge_fields(
            &shaped,
            current.fields.as_object(),
            last_written.as_object(),
            record,
            marula_id,
            report,
        );
        if conflicts == 0 && !fields_differ(current.fields.as_object(), &next_fields) {
            return Outcome::Unchanged(marula_id);
        }
        if dry_run {
            return Outcome::UpdatedDryRun;
        }
        let plan = AssetUpdate {
            name: None,
            status: None,
            layout_id: Some(layout.id),
            fields: Some(serde_json::Value::Object(next_fields.clone())),
            change_note: None,
            idempotency_key: format!(
                "dms-{}-asset-{}-u-{}",
                snapshot.source,
                handle,
                short(&fingerprint)
            ),
        };
        match sink.update_asset(marula_id, &plan).await {
            Ok(_) => {
                state.remember(LedgerEntry {
                    source_key: ledger_key,
                    marula_kind: "asset",
                    marula_id,
                    last_written_fields: serde_json::Value::Object(next_fields),
                    last_written_fingerprint: fingerprint,
                    updated_at: chrono::Utc::now(),
                });
                Outcome::Updated(marula_id)
            }
            Err(e) => Outcome::Failed(format!("update: {e}")),
        }
    }

    // ---- document phase -------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    async fn sync_document(
        &self,
        snapshot: &crate::model::DmsSnapshot,
        record: &SyncRecord,
        identity: &Identity,
        layout: &LayoutRecord,
        client_id: Option<Uuid>,
        index: &RecordIndexes,
        documents: &[DocumentRecord],
        documents_by_id: &HashMap<Uuid, &DocumentRecord>,
        sink: &dyn DmsSink,
        state: &dyn SyncState,
        dry_run: bool,
        report: &mut SyncReport,
    ) -> Outcome {
        let handle = identity.handle();
        let ledger_key = format!("document:{}", handle);
        let fingerprint = Identity::fingerprint(record);
        let shaped = shape_fields(record, layout);

        // 1) Ledger + provenance-field resolution (same as assets).
        let mut ours: Option<Uuid> = None;
        if let Some(entry) = state.lookup(&ledger_key).filter(|e| e.is_document()) {
            if documents_by_id.contains_key(&entry.marula_id) {
                ours = Some(entry.marula_id);
            } else {
                state.forget(&ledger_key);
            }
        }
        if ours.is_none() {
            ours = index.ours_by_ref.get(&handle).copied();
        }

        // 2) Deterministic fallback for documents: an exact title
        //    match in the same client scope is evidence, not a
        //    decision — it surfaces as a merge candidate.
        if ours.is_none() {
            let title_matches: Vec<Uuid> = documents
                .iter()
                .filter(|d| d.title == record.title && d.client_id == client_id)
                .map(|d| d.id)
                .collect();
            let Some(matched_id) = title_matches.first().copied() else {
                // 3) Fresh create.
                let plan = DocumentCreate {
                    client_id,
                    title: record.title.clone(),
                    layout_id: Some(layout.id),
                    fields: shaped.to_fields_value(Some(&handle), layout),
                    body: record.body.clone(),
                    idempotency_key: format!("dms-{}-doc-{}", snapshot.source, handle),
                    change_note: format!("synced from {}", snapshot.source),
                };
                if dry_run {
                    return Outcome::CreatedDryRun;
                }
                return match sink.create_document(&plan).await {
                    Ok(created) => {
                        state.remember(LedgerEntry {
                            source_key: ledger_key,
                            marula_kind: "document",
                            marula_id: created.id,
                            last_written_fields: plan.fields.clone(),
                            last_written_fingerprint: fingerprint,
                            updated_at: chrono::Utc::now(),
                        });
                        Outcome::Created(created.id)
                    }
                    Err(e) => Outcome::Failed(format!("create: {e}")),
                };
            };
            let Some(matched) = documents_by_id.get(&matched_id) else {
                return Outcome::Skipped("matched document vanished mid-sync".into());
            };
            if !self.config.auto_link_conflicts {
                return Outcome::MergeCandidate {
                    matched_on: format!("title={:?}", record.title),
                    marula_id: matched_id,
                    label: matched.title.clone(),
                    evidence: field_evidence(record, matched.fields.as_object()),
                };
            }
            // Adoption: stamp provenance + fill EMPTY fields; the body
            // of an adopted document is NOT overwritten (a human may
            // have authored it).
            let mut next_fields = serde_json::Map::new();
            if let Some(existing) = matched.fields.as_object() {
                next_fields = existing.clone();
            }
            let mut filled_any = false;
            for (key, value) in &shaped.fields {
                let empty = next_fields
                    .get(key)
                    .map(|v| v.as_str().map(str::is_empty).unwrap_or(true))
                    .unwrap_or(true);
                if empty {
                    next_fields.insert(key.clone(), serde_json::Value::String(value.clone()));
                    filled_any = true;
                }
            }
            let empty_ref = next_fields
                .get(SOURCE_REF_KEY)
                .map(|v| v.as_str().map(str::is_empty).unwrap_or(true))
                .unwrap_or(true);
            if empty_ref {
                next_fields.insert(
                    SOURCE_REF_KEY.to_string(),
                    serde_json::Value::String(handle.clone()),
                );
                filled_any = true;
            }
            if dry_run {
                return Outcome::AdoptedDryRun;
            }
            if !filled_any {
                state.remember(LedgerEntry {
                    source_key: ledger_key,
                    marula_kind: "document",
                    marula_id: matched_id,
                    last_written_fields: serde_json::to_value(&next_fields).unwrap_or_default(),
                    last_written_fingerprint: fingerprint,
                    updated_at: chrono::Utc::now(),
                });
                return Outcome::Adopted(matched_id);
            }
            let plan = DocumentUpdate {
                title: None,
                layout_id: matched.layout_id,
                fields: Some(serde_json::Value::Object(next_fields.clone())),
                body: None,
                change_note: Some(format!("synced from {}", snapshot.source)),
                idempotency_key: format!(
                    "dms-{}-doc-{}-adopt-{}",
                    snapshot.source,
                    handle,
                    short(&fingerprint)
                ),
            };
            return match sink.update_document(matched_id, &plan).await {
                Ok(_) => {
                    state.remember(LedgerEntry {
                        source_key: ledger_key,
                        marula_kind: "document",
                        marula_id: matched_id,
                        last_written_fields: serde_json::Value::Object(next_fields),
                        last_written_fingerprint: fingerprint,
                        updated_at: chrono::Utc::now(),
                    });
                    Outcome::Adopted(matched_id)
                }
                Err(e) => Outcome::Failed(format!("adopt: {e}")),
            };
        }

        // Ours: diff (same curated rule at the document level; the
        // fingerprint gate skips no-op runs — documents version
        // append-only, so churn must be earned).
        let Some(marula_id) = ours else {
            return Outcome::Skipped("resolution fell through".into());
        };
        let Some(current) = documents_by_id.get(&marula_id) else {
            return Outcome::Skipped("our document vanished mid-sync".into());
        };
        let entry = state.lookup(&ledger_key);
        if entry
            .as_ref()
            .map(|e| e.last_written_fingerprint == fingerprint)
            .unwrap_or(false)
        {
            return Outcome::Unchanged(marula_id);
        }
        let last_written = entry
            .map(|e| e.last_written_fields)
            .unwrap_or(current.fields.clone());
        let (next_fields, conflicts) = merge_fields(
            &shaped,
            current.fields.as_object(),
            last_written.as_object(),
            record,
            marula_id,
            report,
        );
        if conflicts == 0 && !fields_differ(current.fields.as_object(), &next_fields) {
            return Outcome::Unchanged(marula_id);
        }
        if dry_run {
            return Outcome::UpdatedDryRun;
        }
        let plan = DocumentUpdate {
            title: Some(record.title.clone()),
            layout_id: Some(layout.id),
            fields: Some(serde_json::Value::Object(next_fields.clone())),
            body: Some(record.body.clone()),
            change_note: Some(format!("synced from {}", snapshot.source)),
            idempotency_key: format!(
                "dms-{}-doc-{}-u-{}",
                snapshot.source,
                handle,
                short(&fingerprint)
            ),
        };
        match sink.update_document(marula_id, &plan).await {
            Ok(_) => {
                state.remember(LedgerEntry {
                    source_key: ledger_key,
                    marula_kind: "document",
                    marula_id,
                    last_written_fields: serde_json::Value::Object(next_fields),
                    last_written_fingerprint: fingerprint,
                    updated_at: chrono::Utc::now(),
                });
                Outcome::Updated(marula_id)
            }
            Err(e) => Outcome::Failed(format!("update: {e}")),
        }
    }
}

// ---- outcome accounting -------------------------------------------------------

enum Outcome {
    Created(Uuid),
    CreatedDryRun,
    Updated(Uuid),
    UpdatedDryRun,
    Unchanged(Uuid),
    /// Deterministic match adopted under auto_link.
    Adopted(Uuid),
    AdoptedDryRun,
    MergeCandidate {
        matched_on: String,
        marula_id: Uuid,
        label: String,
        evidence: Vec<(String, String, String)>,
    },
    Ambiguous(String),
    Skipped(String),
    Failed(String),
}

#[allow(clippy::too_many_arguments)]
fn register_outcome(
    outcome: Outcome,
    record: &SyncRecord,
    identity: &Identity,
    kind: LinkKind,
    kind_name: &str,
    count_name: &str,
    report: &mut SyncReport,
    endpoints: &mut HashMap<(String, LinkKind), ResolvedEndpoint>,
) {
    let endpoint = |id: Uuid| ResolvedEndpoint { kind, id };
    match outcome {
        Outcome::Created(id) => {
            match count_name {
                "asset" => report.assets_created += 1,
                _ => report.documents_created += 1,
            }
            endpoints.insert((record.external_id.clone(), kind), endpoint(id));
        }
        Outcome::CreatedDryRun => {
            match count_name {
                "asset" => report.assets_created += 1,
                _ => report.documents_created += 1,
            }
            endpoints.insert((record.external_id.clone(), kind), endpoint(Uuid::nil()));
        }
        Outcome::Updated(id) => {
            match count_name {
                "asset" => report.assets_updated += 1,
                _ => report.documents_updated += 1,
            }
            endpoints.insert((record.external_id.clone(), kind), endpoint(id));
        }
        Outcome::UpdatedDryRun => {
            match count_name {
                "asset" => report.assets_updated += 1,
                _ => report.documents_updated += 1,
            }
            endpoints.insert((record.external_id.clone(), kind), endpoint(Uuid::nil()));
        }
        Outcome::Unchanged(id) | Outcome::Adopted(id) => {
            endpoints.insert((record.external_id.clone(), kind), endpoint(id));
        }
        Outcome::AdoptedDryRun => {
            endpoints.insert((record.external_id.clone(), kind), endpoint(Uuid::nil()));
        }
        Outcome::MergeCandidate {
            matched_on,
            marula_id,
            label,
            evidence,
        } => {
            report.merge_candidates.push(MergeCandidate {
                external_id: record.external_id.clone(),
                external_title: record.title.clone(),
                record_kind: kind_name.to_string(),
                marula_id,
                matched_on,
                marula_label: label,
                evidence,
            });
            report.records_skipped += 1;
        }
        Outcome::Ambiguous(reason) => {
            report.ambiguous.push(UnmappedRecord {
                external_id: record.external_id.clone(),
                title: record.title.clone(),
                reason,
            });
            report.records_skipped += 1;
        }
        Outcome::Skipped(reason) => {
            report.records_skipped += 1;
            report.errors.push(RunError {
                external_id: record.external_id.clone(),
                error: reason,
            });
        }
        Outcome::Failed(error) => {
            report.errors.push(RunError {
                external_id: record.external_id.clone(),
                error,
            });
        }
    }
    let _ = identity;
}

// ---- helpers -------------------------------------------------------------------

/// Deterministic pseudo-id for dry-run layouts (sha256 of the slug —
/// distinct per type, never nil).
fn dry_run_id(slug: &str) -> Uuid {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"marula-dms-dry-run-layout");
    hasher.update(slug.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}

fn short(fingerprint: &str) -> String {
    fingerprint.chars().take(12).collect()
}

fn tag_of(record: &SyncRecord) -> String {
    record
        .tag
        .clone()
        .unwrap_or_else(|| record.title.clone())
        .chars()
        .take(64)
        .collect()
}

fn asset_label(asset: &AssetRecord) -> String {
    format!("asset {:?} ({})", asset.name, asset.tag)
}

fn slugify(name: &str) -> String {
    let s: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    s.trim_matches('-').to_string()
}

fn connector_slug(source: &str, type_name: &str) -> String {
    format!("{}-{}", slugify(source), slugify(type_name))
}

fn source_label(source: &str) -> String {
    let mut chars = source.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A system layout fits an external type when every external field key
/// exists in the system schema AND every REQUIRED system key is carried
/// by the external schema (records can supply required values), with
/// matching field kinds.
fn system_compatible(source: &SyncLayout, system: &LayoutRecord) -> bool {
    source.fields.iter().all(|f| {
        system
            .fields
            .iter()
            .find(|s| s.key == f.key)
            .map(|s| s.kind == f.kind)
            .unwrap_or(false)
    }) && system
        .fields
        .iter()
        .filter(|f| f.required)
        .all(|f| source.fields.iter().any(|s| s.key == f.key))
}

/// Synthesizes a layout for record types the source did not declare
/// (e.g. IT Glue configurations have no type schema): the union of the
/// type's observed fields, text-typed — lossless, connector-owned.
fn synthesize_layout(source: &str, records: &[SyncRecord], type_name: &str) -> SyncLayout {
    let mut fields: Vec<SyncField> = Vec::new();
    for record in records
        .iter()
        .filter(|r| r.type_name.as_deref() == Some(type_name))
    {
        for key in record.fields.keys() {
            if !fields.iter().any(|f| &f.key == key) {
                fields.push(SyncField::new(
                    key.clone(),
                    key.clone(),
                    FieldKind::Text,
                    i32::try_from(fields.len()).unwrap_or(i32::MAX),
                ));
            }
        }
    }
    SyncLayout {
        external_id: format!("synthetic:{type_name}"),
        name: type_name.to_string(),
        description: format!("Synced from {} ({})", type_name, source),
        icon: "file".into(),
        fields,
    }
}

/// Client resolution: the snapshot's client-ref names (id → display
/// name), then operator aliases, then case-insensitive name matching.
fn resolve_client(
    record: &SyncRecord,
    snapshot: &crate::model::DmsSnapshot,
    config: &SyncConfig,
    client_by_name: &BTreeMap<String, Uuid>,
) -> Option<Uuid> {
    let reference = record.client_ref.as_deref()?;
    let display = snapshot
        .client_refs
        .get(reference)
        .map(String::as_str)
        .unwrap_or(reference);
    resolve_client_name(display, &config.client_aliases, client_by_name)
}

fn resolve_client_name(
    name: &str,
    aliases: &BTreeMap<String, String>,
    client_by_name: &BTreeMap<String, Uuid>,
) -> Option<Uuid> {
    let aliased = aliases
        .get(name.trim())
        .map(String::as_str)
        .unwrap_or(name.trim());
    client_by_name.get(&aliased.to_lowercase()).copied()
}

/// Field evidence for a merge candidate: shared keys that DIFFER.
fn field_evidence(
    record: &SyncRecord,
    fields: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Vec<(String, String, String)> {
    let mut evidence = Vec::new();
    for (key, source_value) in &record.fields {
        let Some(value) = fields
            .and_then(|f| f.get(key))
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        if value != source_value {
            evidence.push((key.clone(), source_value.clone(), value.to_string()));
        }
    }
    evidence
}

/// Curated-beats-observed merge: build the NEXT fields object. A field
/// whose sink value equals the last-written value is ours to churn
/// (observed); a drifted sink value is a human edit (curated) — keep it
/// and report the conflict with the source's newer value as evidence.
fn merge_fields(
    shaped: &FieldShape,
    current: Option<&serde_json::Map<String, serde_json::Value>>,
    last_written: Option<&serde_json::Map<String, serde_json::Value>>,
    record: &SyncRecord,
    marula_id: Uuid,
    report: &mut SyncReport,
) -> (serde_json::Map<String, serde_json::Value>, usize) {
    let mut next = serde_json::Map::new();
    if let Some(current) = current {
        next = current.clone();
    }
    let mut conflicts = 0;
    for (key, source_value) in &shaped.fields {
        let current_value = current
            .and_then(|c| c.get(key))
            .and_then(serde_json::Value::as_str);
        let written_value = last_written
            .and_then(|l| l.get(key))
            .and_then(serde_json::Value::as_str);
        match (current_value, written_value) {
            // Empty target: fill.
            (None | Some(""), _) => {
                next.insert(key.clone(), serde_json::Value::String(source_value.clone()));
            }
            // Untouched since we wrote it: source churn applies.
            (Some(cur), Some(prev)) if cur == prev => {
                if cur != source_value {
                    next.insert(key.clone(), serde_json::Value::String(source_value.clone()));
                }
            }
            // Human edit (curated): the source value must not win
            // silently — it surfaces as the accept-the-new-value
            // evidence instead.
            (Some(cur), _) => {
                if cur != source_value {
                    conflicts += 1;
                    report.curated_conflicts.push(CuratedConflict {
                        external_id: record.external_id.clone(),
                        marula_id,
                        field: key.clone(),
                        curated_value: cur.to_string(),
                        source_value: source_value.clone(),
                    });
                }
            }
        }
    }
    (next, conflicts)
}

/// Value-level compare of the fields object: did anything change?
fn fields_differ(
    current: Option<&serde_json::Map<String, serde_json::Value>>,
    next: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    match current {
        None => !next.is_empty(),
        Some(current) => current != next,
    }
}
