//! The §4.6 surface registry host (Wave 34 lane 4): derives the
//! declarative surface entries a manifest's `surface_profile` declares,
//! gates each SECTION on its capability grant, and syncs the derived
//! entries into `plugin_surface_entries`.
//!
//! Posture:
//!   * deny-by-default — a section registers ONLY when its capability
//!     (`ui.widgets` / `automation.actions` / `reports.render` /
//!     `fields.validate`) is in the install's granted set; ungranted
//!     declarations stay visible on consent but register nothing.
//!   * entries are DATA. Widgets are closed render kinds; automation
//!     actions map onto consented built-in effects; validators are
//!     JSON-Schema subsets the core executes. Nothing here is or becomes
//!     executable content (agent-safety §4.8).

use core_db::plugin_surface;
use core_domain::entities::plugins::{
    PluginCapability, PluginInstall, PluginManifestV2, PluginSurfaceProfile,
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
}

impl DerivedSurface {
    pub fn is_empty(&self) -> bool {
        self.widgets.is_empty()
            && self.actions.is_empty()
            && self.render_kinds.is_empty()
            && self.field_validators.is_empty()
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_domain::entities::plugins::{
        SurfaceAction, SurfaceActionEffect, SurfaceFieldValidator, SurfaceRenderKind, SurfaceSlot,
        SurfaceWidget, SurfaceWidgetKind,
    };

    fn install(slug: &str, grants: &[&str]) -> PluginInstall {
        PluginInstall {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            catalog_id: Uuid::now_v7(),
            slug: slug.into(),
            version: "1.0.0".into(),
            status: core_domain::entities::plugins::PluginInstallStatus::Enabled,
            granted_capabilities: grants.iter().map(|s| s.to_string()).collect(),
            installed_by: None,
            installed_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            revoked_at: None,
            revoked_by: None,
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
        }
    }

    fn all_granted(cap: PluginCapability) -> bool {
        matches!(
            cap,
            PluginCapability::UiWidgets
                | PluginCapability::AutomationActions
                | PluginCapability::ReportsRender
                | PluginCapability::FieldsValidate
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
    }

    #[test]
    fn ungranted_sections_register_nothing() {
        let only_widgets = |cap: PluginCapability| cap == PluginCapability::UiWidgets;
        let derived = derive_profile("p", &profile(), only_widgets);
        assert_eq!(derived.widgets.len(), 1);
        assert!(derived.actions.is_empty());
        assert!(derived.render_kinds.is_empty());
        assert!(derived.field_validators.is_empty());
        assert!(!derived.is_empty());

        let none = |_cap: PluginCapability| false;
        let empty = derive_profile("p", &profile(), none);
        assert!(empty.is_empty());
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
}
