//! The §4.6 surface registry host (Wave 34 lane 4): derives the
//! declarative surface entries a manifest's `surface_profile` declares,
//! gates each SECTION on its capability grant, and syncs the derived
//! entries into `plugin_surface_entries`.
//!
//! Posture:
//!   * deny-by-default — a section registers ONLY when its capability
//!     (`ui.widgets` / `automation.actions` / `reports.render` /
//!     `fields.validate` / `automation.triggers` / `automation.reads`) is
//!     in the install's granted set; ungranted declarations stay visible
//!     on consent but register nothing.
//!   * entries are DATA. Widgets are closed render kinds; automation
//!     actions map onto consented built-in effects; triggers declare the
//!     event type a rule may fire on and reads carry their own guardrails;
//!     validators are JSON-Schema subsets the core executes. Nothing here
//!     is or becomes executable content (agent-safety §4.8).
//!
//! Wave 43 lane 4 (ADR 0017 §3): the executor kinds are available to
//! `http_connector` manifests too, on the same grant mechanism — an
//! http_connector already holds credentials and network egress, so an
//! action grants it nothing new. The UI kinds stay wasm-only, and that
//! split is enforced in `PluginManifestV2::validate`.

use core_db::plugin_surface;
use core_domain::entities::plugins::{
    PluginCapability, PluginInstall, PluginManifestV2, PluginSurfaceProfile, SettingsSchema,
    SurfaceActionEffect,
};
use serde_json::{json, Value};
use sqlx::PgConnection;
use uuid::Uuid;

/// One derived entry ready for persistence: (entry_key, label, definition).
pub type DerivedEntry = (String, String, Value);

/// The derived per-section output for one install.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DerivedSurface {
    pub widgets: Vec<DerivedEntry>,
    pub actions: Vec<DerivedEntry>,
    pub render_kinds: Vec<DerivedEntry>,
    pub field_validators: Vec<DerivedEntry>,
    /// Wave 43 lane 4 (B): trigger sources. A plugin could contribute an
    /// action but never a trigger, so the symmetry with
    /// `automation_action` was open — this closes it.
    pub triggers: Vec<DerivedEntry>,
    /// Wave 43 lane 4 (D): read nodes, the only kind that lets a workflow
    /// FETCH from an external system.
    pub reads: Vec<DerivedEntry>,
}

impl DerivedSurface {
    pub fn is_empty(&self) -> bool {
        self.widgets.is_empty()
            && self.actions.is_empty()
            && self.render_kinds.is_empty()
            && self.field_validators.is_empty()
            && self.triggers.is_empty()
            && self.reads.is_empty()
    }
}

/// Derives the section-gated entries for an install. `slug` is the
/// install-safe name the approve path minted (the registry-key
/// namespace). Grants must already be a subset of the manifest's
/// declared capabilities — enforced by the caller (`authorize` first);
/// this fn re-checks the subset defensively.
pub fn derive_surface(install: &PluginInstall, manifest: &PluginManifestV2) -> DerivedSurface {
    let granted: Vec<PluginCapability> = install
        .granted_capabilities
        .iter()
        .filter_map(|k| PluginCapability::parse(k))
        .collect();
    let granted_of = |cap: PluginCapability| granted.contains(&cap);

    let Some(profile) = &manifest.surface_profile else {
        return DerivedSurface::default();
    };
    derive_profile(&install.slug, profile, granted_of)
}

/// Section-gated derivation against an already-parsed profile
/// (test seam: no manifest round-trip needed).
pub fn derive_profile(
    slug: &str,
    profile: &PluginSurfaceProfile,
    granted_of: impl Fn(PluginCapability) -> bool,
) -> DerivedSurface {
    let mut out = DerivedSurface::default();
    if granted_of(PluginCapability::UiWidgets) {
        for w in &profile.widgets {
            out.widgets.push((
                PluginSurfaceProfile::entry_key(slug, &w.id),
                w.title.clone(),
                json!({
                    "kind": w.kind.as_str(),
                    "slot": w.slot.as_str(),
                    "props_schema": w.props_schema.clone(),
                    "default_props": w.default_props.clone(),
                }),
            ));
        }
    }
    if granted_of(PluginCapability::AutomationActions) {
        for a in &profile.actions {
            out.actions.push((
                PluginSurfaceProfile::entry_key(slug, &a.id),
                a.label.clone(),
                json!({
                    "description": a.description,
                    "effect": a.effect.as_str(),
                    "params_schema": a.params_schema.clone(),
                    // A2 (Wave 43 lane 13): the node contract rides inside
                    // the definition as well as in its own columns, so a
                    // consumer reading the row directly sees the same five
                    // facts the registry parses. `sync_surface_section`
                    // lifts them into the columns.
                    "node_contract": {
                        "output_schema": a.output_schema.clone(),
                        "idempotent": a.idempotent,
                        "emits": a.emits.clone(),
                        "audience": a.audience.map(|x| x.as_str()),
                        "agent_description": a.agent_description.clone(),
                    },
                }),
            ));
        }
    }
    if granted_of(PluginCapability::ReportsRender) {
        for r in &profile.render_kinds {
            out.render_kinds.push((
                PluginSurfaceProfile::entry_key(slug, &r.id),
                r.label.clone(),
                json!({
                    "description": r.description,
                }),
            ));
        }
    }
    if granted_of(PluginCapability::FieldsValidate) {
        for v in &profile.field_validators {
            out.field_validators.push((
                PluginSurfaceProfile::entry_key(slug, &v.id),
                v.label.clone(),
                json!({
                    "schema": v.schema.clone(),
                }),
            ));
        }
    }
    if granted_of(PluginCapability::AutomationTriggers) {
        for t in &profile.triggers {
            out.triggers.push((
                PluginSurfaceProfile::entry_key(slug, &t.id),
                t.label.clone(),
                json!({
                    "description": t.description,
                    "emitted_event_type": t.emitted_event_type,
                    "payload_schema": t.payload_schema.clone(),
                }),
            ));
        }
    }
    if granted_of(PluginCapability::AutomationReads) {
        for r in &profile.reads {
            out.reads.push((
                PluginSurfaceProfile::entry_key(slug, &r.id),
                r.label.clone(),
                json!({
                    "description": r.description,
                    "params_schema": r.params_schema.clone(),
                    "output_schema": r.output_schema.clone(),
                    // The declared guardrails ride along so the consuming
                    // surface and the run ledger can enforce and report the
                    // SAME caps the manifest declared (ADR 0017 §5).
                    "guardrails": {
                        "max_calls_per_run": r.guardrails.max_calls_per_run,
                        "min_interval_secs": r.guardrails.min_interval_secs,
                        "max_output_bytes": r.guardrails.max_output_bytes,
                        "may_return_secrets": r.guardrails.may_return_secrets,
                    },
                    // A2 (Wave 43 lane 13): the four fields a read did not
                    // have. `output_schema` already existed above and is
                    // not duplicated here.
                    "node_contract": {
                        "idempotent": r.idempotent,
                        "emits": r.emits.clone(),
                        "audience": r.audience.map(|x| x.as_str()),
                        "agent_description": r.agent_description.clone(),
                    },
                }),
            ));
        }
    }
    out
}

/// Persists the derived sections: each section is replaced wholesale
/// inside the caller's transaction (an upgrade rewrites exactly what the
/// new manifest declares). An install whose profile is empty (or fully
/// ungranted) syncs to ZERO rows for those sections — the deny-by-default
/// residue is deletion, not retention.
pub async fn sync_install_surface(
    tx: &mut PgConnection,
    install: &PluginInstall,
    manifest: &PluginManifestV2,
) -> Result<(), core_db::DbError> {
    let derived = derive_surface(install, manifest);
    sync_derived(tx, install.org_id, install.id, &derived).await
}

/// The persistence half of [`sync_install_surface`], split for tests.
pub async fn sync_derived(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    derived: &DerivedSurface,
) -> Result<(), core_db::DbError> {
    plugin_surface::sync_surface_section(tx, org_id, install_id, "widget", &derived.widgets)
        .await?;
    plugin_surface::sync_surface_section(
        tx,
        org_id,
        install_id,
        "automation_action",
        &derived.actions,
    )
    .await?;
    plugin_surface::sync_surface_section(
        tx,
        org_id,
        install_id,
        "report_render",
        &derived.render_kinds,
    )
    .await?;
    plugin_surface::sync_surface_section(
        tx,
        org_id,
        install_id,
        "field_validator",
        &derived.field_validators,
    )
    .await?;
    // Wave 43 lane 4 (B)/(D): the two new EXECUTOR kinds sync through the
    // same per-section wholesale replace, so an ungranted section's
    // deny-by-default residue is deletion, exactly like the other four.
    plugin_surface::sync_surface_section(
        tx,
        org_id,
        install_id,
        "automation_trigger",
        &derived.triggers,
    )
    .await?;
    plugin_surface::sync_surface_section(tx, org_id, install_id, "read", &derived.reads).await?;
    Ok(())
}

/// The six surface sections a manifest can declare, as the INVOCATION path
/// names them (W53-5).
///
/// [`derive_profile`] writes these six into `plugin_surface_entries` and
/// [`SurfaceKind::as_str`] is the `surface` column value it uses, so the
/// derivation and the invocation read one vocabulary. The capability is the
/// same one the section gate above already applied — an invocation cannot ask
/// for a capability its section did not register under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    Widget,
    Action,
    ReportRender,
    FieldValidator,
    Trigger,
    Read,
}

impl SurfaceKind {
    /// Every kind, in the order the derivation writes them.
    pub const ALL: [SurfaceKind; 6] = [
        SurfaceKind::Widget,
        SurfaceKind::Action,
        SurfaceKind::ReportRender,
        SurfaceKind::FieldValidator,
        SurfaceKind::Trigger,
        SurfaceKind::Read,
    ];

    /// The `plugin_surface_entries.surface` value (and the `surface` a caller
    /// passes to [`core_db::plugin_surface::get_entry_by_key`]).
    pub const fn as_str(self) -> &'static str {
        match self {
            SurfaceKind::Widget => "widget",
            SurfaceKind::Action => "automation_action",
            SurfaceKind::ReportRender => "report_render",
            SurfaceKind::FieldValidator => "field_validator",
            SurfaceKind::Trigger => "automation_trigger",
            SurfaceKind::Read => "read",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "widget" => Some(SurfaceKind::Widget),
            "automation_action" => Some(SurfaceKind::Action),
            "report_render" => Some(SurfaceKind::ReportRender),
            "field_validator" => Some(SurfaceKind::FieldValidator),
            "automation_trigger" => Some(SurfaceKind::Trigger),
            "read" => Some(SurfaceKind::Read),
            _ => None,
        }
    }

    /// The ONE capability this section registers under. A closed mapping: the
    /// caller cannot ask for a different one, and the invocation path checks
    /// the install holds exactly this.
    pub const fn capability(self) -> PluginCapability {
        match self {
            SurfaceKind::Widget => PluginCapability::UiWidgets,
            SurfaceKind::Action => PluginCapability::AutomationActions,
            SurfaceKind::ReportRender => PluginCapability::ReportsRender,
            SurfaceKind::FieldValidator => PluginCapability::FieldsValidate,
            SurfaceKind::Trigger => PluginCapability::AutomationTriggers,
            SurfaceKind::Read => PluginCapability::AutomationReads,
        }
    }

    /// Only an automation action applies a domain effect. The other five
    /// return data (a widget's rows, a read's payload) or declare a shape (a
    /// render kind, a validator, a trigger) and write nothing, so the parent
    /// applies no effect for them.
    pub const fn applies_effect(self) -> bool {
        matches!(self, SurfaceKind::Action)
    }
}

/// What an entry's declared `params_schema` turned out to be.
///
/// Three states rather than `Option<Result<…>>` because each one is a
/// different decision at the call site: nothing to check, check it, or refuse
/// the entry.
#[derive(Debug, Clone)]
pub enum DeclaredSchema {
    /// The entry declares no params schema.
    Absent,
    /// A schema the manifest subset accepted.
    Valid(SettingsSchema),
    /// A schema the subset refuses. The entry is unusable, so a caller refuses
    /// the invocation instead of running it unvalidated.
    Invalid,
}

/// Parses the `params_schema` an action or read entry carries. Widgets carry
/// `props_schema`, validators carry `schema` and triggers carry
/// `payload_schema`; none of those is a `params_schema`, so they answer
/// [`DeclaredSchema::Absent`] here and keep their own consumer.
pub fn params_schema(definition: &Value) -> DeclaredSchema {
    let Some(raw) = definition.get("params_schema").filter(|v| !v.is_null()) else {
        return DeclaredSchema::Absent;
    };
    match SettingsSchema::new(raw.clone()) {
        Ok(schema) => DeclaredSchema::Valid(schema),
        Err(_) => DeclaredSchema::Invalid,
    }
}

/// The event types one entry declares in its `node_contract.emits`.
///
/// The entry's OWN declaration, not the whole profile's: an action that emits
/// nothing gets an empty list, so `emit_event` is refused for it even when a
/// sibling entry in the same manifest declares a type. Least authority per
/// entry, and the manifest is still the upper bound.
pub fn declared_events(definition: &Value) -> Vec<String> {
    definition
        .get("node_contract")
        .and_then(|contract| contract.get("emits"))
        .and_then(Value::as_array)
        .map(|emits| {
            emits
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The consented effect an action entry declares, in the closed
/// [`SurfaceActionEffect`] vocabulary.
///
/// `None` covers both "declares no effect" and "declares one the vocabulary
/// does not know". Both refuse: an effect nobody consented to is not an effect
/// the engine may apply.
pub fn consented_effect(definition: &Value) -> Option<SurfaceActionEffect> {
    definition
        .get("effect")
        .and_then(Value::as_str)
        .and_then(SurfaceActionEffect::parse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_domain::entities::plugins::{
        ReadGuardrails, SurfaceAction, SurfaceActionEffect, SurfaceFieldValidator, SurfaceRead,
        SurfaceRenderKind, SurfaceSlot, SurfaceTrigger, SurfaceWidget, SurfaceWidgetKind,
    };

    fn install(slug: &str, grants: &[&str]) -> PluginInstall {
        PluginInstall {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            catalog_id: Uuid::now_v7(),
            slug: slug.into(),
            version: "1.0.0".into(),
            module_digest: String::new(),
            status: core_domain::entities::plugins::PluginInstallStatus::Enabled,
            granted_capabilities: grants.iter().map(|s| s.to_string()).collect(),
            installed_by: None,
            installed_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            revoked_at: None,
            revoked_by: None,
            revoked_reason: None,
            previous_version: None,
            previous_module_digest: None,
            previous_capabilities: Vec::new(),
        }
    }

    fn profile() -> PluginSurfaceProfile {
        PluginSurfaceProfile {
            widgets: vec![SurfaceWidget {
                id: "csat_today".into(),
                title: "CSAT today".into(),
                slot: SurfaceSlot::Dashboard,
                kind: SurfaceWidgetKind::Stat,
                props_schema: None,
                default_props: None,
            }],
            actions: vec![SurfaceAction {
                id: "triage_note".into(),
                label: "Triage note".into(),
                description: "Adds the triage note.".into(),
                effect: SurfaceActionEffect::AddInternalComment,
                params_schema: None,
                output_schema: None,
                idempotent: false,
                emits: Vec::new(),
                audience: None,
                agent_description: None,
            }],
            render_kinds: vec![SurfaceRenderKind {
                id: "gauges".into(),
                label: "Gauges".into(),
                description: "Gauge rows.".into(),
            }],
            field_validators: vec![SurfaceFieldValidator {
                id: "asset_tag".into(),
                label: "Asset tag".into(),
                schema: json!({"type": "object", "properties": {}}),
            }],
            triggers: vec![SurfaceTrigger {
                id: "device_offline".into(),
                label: "Device offline".into(),
                description: "An RMM device stops reporting.".into(),
                emitted_event_type: "rmm.device_offline".into(),
                payload_schema: None,
            }],
            reads: vec![SurfaceRead {
                id: "site_devices".into(),
                label: "Site devices".into(),
                description: "Fetches a site's device list.".into(),
                params_schema: None,
                output_schema: None,
                idempotent: true,
                emits: Vec::new(),
                audience: None,
                agent_description: None,
                guardrails: ReadGuardrails {
                    max_calls_per_run: 5,
                    min_interval_secs: core_domain::entities::plugins::MIN_READ_INTERVAL_SECS,
                    max_output_bytes: 8192,
                    may_return_secrets: false,
                },
            }],
        }
    }

    fn all_granted(cap: PluginCapability) -> bool {
        matches!(
            cap,
            PluginCapability::UiWidgets
                | PluginCapability::AutomationActions
                | PluginCapability::ReportsRender
                | PluginCapability::FieldsValidate
                | PluginCapability::AutomationTriggers
                | PluginCapability::AutomationReads
        )
    }

    #[test]
    fn derive_should_key_entries_by_install_slug_and_carry_definitions() {
        let derived = derive_profile("csat-widget", &profile(), all_granted);
        assert_eq!(derived.widgets[0].0, "plugin.csat-widget.csat_today");
        assert_eq!(derived.widgets[0].1, "CSAT today");
        assert_eq!(derived.widgets[0].2["kind"], "stat");
        assert_eq!(derived.widgets[0].2["slot"], "dashboard");
        assert_eq!(derived.actions[0].2["effect"], "add_internal_comment");
        assert_eq!(derived.render_kinds[0].0, "plugin.csat-widget.gauges");
        assert_eq!(
            derived.field_validators[0].0,
            "plugin.csat-widget.asset_tag"
        );
        assert_eq!(derived.triggers[0].0, "plugin.csat-widget.device_offline");
        assert_eq!(
            derived.triggers[0].2["emitted_event_type"],
            "rmm.device_offline"
        );
        assert_eq!(derived.reads[0].0, "plugin.csat-widget.site_devices");
        assert_eq!(derived.reads[0].2["guardrails"]["max_calls_per_run"], 5);
    }

    #[test]
    fn ungranted_sections_register_nothing() {
        let only_widgets = |cap: PluginCapability| cap == PluginCapability::UiWidgets;
        let derived = derive_profile("p", &profile(), only_widgets);
        assert_eq!(derived.widgets.len(), 1);
        assert!(derived.actions.is_empty());
        assert!(derived.render_kinds.is_empty());
        assert!(derived.field_validators.is_empty());
        assert!(derived.triggers.is_empty());
        assert!(derived.reads.is_empty());
        assert!(!derived.is_empty());

        let none = |_cap: PluginCapability| false;
        let empty = derive_profile("p", &profile(), none);
        assert!(empty.is_empty());
    }

    #[test]
    fn a_connector_contributes_executor_kinds_under_grants() {
        let m = PluginManifestV2 {
            name: "RMM Connector".into(),
            version: "1.0.0".into(),
            interface_version: core_domain::entities::plugins::PLUGIN_RUNTIME_INTERFACE_VERSION,
            description: "Reads an RMM.".into(),
            capabilities: vec![
                "automation.actions".into(),
                "automation.triggers".into(),
                "automation.reads".into(),
            ],
            settings_schema: None,
            artifact_kind: Some(core_domain::entities::plugins::PluginRuntimeKind::HttpConnector),
            connector_profile: None,
            surface_profile: Some(PluginSurfaceProfile {
                actions: vec![SurfaceAction {
                    id: "triage_note".into(),
                    label: "Triage note".into(),
                    description: "Adds the triage note.".into(),
                    effect: SurfaceActionEffect::AddInternalComment,
                    params_schema: None,
                    output_schema: None,
                    idempotent: false,
                    emits: Vec::new(),
                    audience: None,
                    agent_description: None,
                }],
                triggers: profile().triggers,
                reads: profile().reads,
                ..PluginSurfaceProfile::default()
            }),
        };
        m.validate().expect("connector manifest validates");

        // WithOUT the grants: nothing registers, exactly like the UI kinds.
        let none = derive_surface(&install("rmm", &[]), &m);
        assert!(none.is_empty());

        // With the grants: the executor sections register.
        let granted = derive_surface(
            &install("rmm", &["automation.triggers", "automation.reads"]),
            &m,
        );
        assert_eq!(granted.triggers.len(), 1);
        assert_eq!(granted.reads.len(), 1);
        assert!(granted.actions.is_empty(), "actions stay ungranted");
        assert!(
            granted.widgets.is_empty(),
            "a connector declares no widgets"
        );
    }

    #[test]
    fn derive_surface_should_read_manifest_profile_and_install_grants() {
        let m = PluginManifestV2 {
            name: "CSAT Widget".into(),
            version: "1.0.0".into(),
            interface_version: core_domain::entities::plugins::PLUGIN_RUNTIME_INTERFACE_VERSION,
            description: "CSAT".into(),
            capabilities: vec![
                "ui.widgets".into(),
                "automation.actions".into(),
                "reports.render".into(),
                "fields.validate".into(),
            ],
            settings_schema: None,
            artifact_kind: None,
            connector_profile: None,
            surface_profile: Some(profile()),
        };
        m.validate().expect("manifest validates");
        let full = derive_surface(&install("csat", &["ui.widgets"]), &m);
        assert_eq!(full.widgets.len(), 1);
        assert!(full.actions.is_empty());
    }

    #[test]
    fn every_surface_kind_reads_and_writes_the_sections_own_string() {
        // The six kinds are the six sections `sync_derived` persists; a drift
        // here would make an invocation look up a surface the sync never wrote.
        let persisted = [
            "widget",
            "automation_action",
            "report_render",
            "field_validator",
            "automation_trigger",
            "read",
        ];
        for (kind, surface) in SurfaceKind::ALL.into_iter().zip(persisted) {
            assert_eq!(kind.as_str(), surface);
            assert_eq!(SurfaceKind::parse(surface), Some(kind));
        }
        assert_eq!(SurfaceKind::parse("widgets"), None);
        assert_eq!(SurfaceKind::parse(""), None);
    }

    #[test]
    fn each_surface_kind_resolves_onto_the_capability_its_section_gates_on() {
        for kind in SurfaceKind::ALL {
            // The derivation registers a section only under this capability,
            // so the invocation path must require the same one.
            let derived = derive_profile("p", &profile(), |cap| cap == kind.capability());
            let section = match kind {
                SurfaceKind::Widget => derived.widgets,
                SurfaceKind::Action => derived.actions,
                SurfaceKind::ReportRender => derived.render_kinds,
                SurfaceKind::FieldValidator => derived.field_validators,
                SurfaceKind::Trigger => derived.triggers,
                SurfaceKind::Read => derived.reads,
            };
            assert_eq!(section.len(), 1, "{:?} registered nothing", kind);
        }
        assert!(SurfaceKind::Action.applies_effect());
        for kind in SurfaceKind::ALL {
            if kind != SurfaceKind::Action {
                assert!(!kind.applies_effect(), "{kind:?} must apply no effect");
            }
        }
    }

    #[test]
    fn a_params_schema_is_valid_absent_or_refused() {
        assert!(matches!(
            params_schema(&json!({"effect": "add_internal_comment"})),
            DeclaredSchema::Absent
        ));
        assert!(matches!(
            params_schema(&json!({"params_schema": null})),
            DeclaredSchema::Absent
        ));
        assert!(matches!(
            params_schema(&json!({
                "params_schema": {
                    "type": "object",
                    "properties": {"body": {"type": "string"}}
                }
            })),
            DeclaredSchema::Valid(_)
        ));
        // A construct the manifest subset does not support is a refusal, not a
        // silently weaker check.
        assert!(matches!(
            params_schema(&json!({
                "params_schema": {
                    "type": "object",
                    "properties": {"body": {"type": "object"}}
                }
            })),
            DeclaredSchema::Invalid
        ));
    }

    #[test]
    fn only_the_entrys_own_emits_are_declared() {
        assert_eq!(
            declared_events(&json!({
                "node_contract": {"emits": ["ticket.updated", "rmm.device_offline"]}
            })),
            vec![
                "ticket.updated".to_string(),
                "rmm.device_offline".to_string()
            ]
        );
        assert!(declared_events(&json!({"effect": "add_internal_comment"})).is_empty());
        assert!(declared_events(&json!({"node_contract": {"emits": "not-a-list"}})).is_empty());
    }

    #[test]
    fn an_effect_outside_the_closed_vocabulary_is_not_consented() {
        assert_eq!(
            consented_effect(&json!({"effect": "add_internal_comment"})),
            Some(SurfaceActionEffect::AddInternalComment)
        );
        assert_eq!(consented_effect(&json!({"effect": "drop_database"})), None);
        assert_eq!(consented_effect(&json!({"effect": 7})), None);
        assert_eq!(consented_effect(&json!({})), None);
    }
}
