//! Agent skills (Wave 16 lane 7, ADR 0009): the distribution EXTENSION of
//! the migration-0040 versioned skills seam. Skills are data — versioned
//! prompt/rule manifests — never executable content. They reuse ONLY the
//! skills half of the lifecycle (import/edit/enable/version/rollback; no
//! plugin executable lifecycle).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The declarative skill manifest (docs/10-agent-platform.md §4:
/// `AgentSkill` — version, capabilities, prompts/rules). Content is
/// UNTRUSTED RAG-class data: consumption-time layering quotes and scopes
/// it into rules layers; nothing here is ever executed as instructions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SkillManifest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// Agent capability scopes the skill REQUESTS (data for the consent
    /// surface; enforcement stays in the agent platform's rule layers).
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Prompt snippets — quoted verbatim at consumption time.
    #[serde(default)]
    pub prompts: Vec<String>,
    /// Rules snippets — quoted verbatim at consumption time, deny wins.
    #[serde(default)]
    pub rules: Vec<String>,
}

impl SkillManifest {
    pub fn validate(&self) -> Result<(), crate::error::DomainError> {
        if self.name.trim().is_empty() {
            return Err(crate::error::DomainError::Validation(
                "skill manifest name must not be empty".into(),
            ));
        }
        Ok(())
    }
}

/// One skill VERSION row (the migration-0040 shape + the distribution
/// columns from migration 0079). History is append-only; rollback
/// appends a NEW version carrying an old manifest. Enablement rides the
/// org's active version row — deny-by-default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentSkillVersion {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: Uuid,
    pub slug: String,
    pub version: i32,
    pub manifest: SkillManifest,
    pub description: String,
    pub source_kind: crate::entities::plugins::PluginSourceKind,
    pub source_url: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub fetched_ref: Option<String>,
    pub artifact_digest: String,
    /// Deny-by-default enable gate on the active row.
    pub enabled: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub enabled_by: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "date-time"))]
    pub enabled_at: Option<chrono::DateTime<chrono::Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Alias for the org-facing "one skill" view: the skill's ACTIVE version
/// row (the skills half has no install row of its own).
pub type AgentSkill = AgentSkillVersion;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_manifest_should_require_a_name() {
        let mut m = SkillManifest {
            name: "Triage Helper".into(),
            version: "1.0.0".into(),
            description: String::new(),
            capabilities: vec!["tickets.read".into()],
            prompts: vec!["Summarize: {{ticket}}".into()],
            rules: vec!["Never disclose secrets.".into()],
        };
        assert!(m.validate().is_ok());
        m.name = " ".into();
        assert!(m.validate().is_err());
    }
}
