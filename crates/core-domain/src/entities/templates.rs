//! Template entity (Wave 13 lane 5, TASKLIST KB3/KB4, ADR 0004).
//!
//! One `Template` entity, typed across the product's template kinds —
//! ticket / note / email / KB article / quote-invoice / project scaffold /
//! workflow playbook. No per-type template tables (ADR 0004). Shipped
//! defaults are immutable seed rows: orgs fork, never edit in place
//! (defaults-first doctrine, mirroring lane 6's preset-roles pattern).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The closed set of template kinds (ADR 0004: all ride the one entity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TemplateKind {
    Ticket,
    Note,
    Email,
    Kb,
    QuoteInvoice,
    Project,
    Workflow,
}

impl TemplateKind {
    pub const ALL: [TemplateKind; 7] = [
        TemplateKind::Ticket,
        TemplateKind::Note,
        TemplateKind::Email,
        TemplateKind::Kb,
        TemplateKind::QuoteInvoice,
        TemplateKind::Project,
        TemplateKind::Workflow,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            TemplateKind::Ticket => "ticket",
            TemplateKind::Note => "note",
            TemplateKind::Email => "email",
            TemplateKind::Kb => "kb",
            TemplateKind::QuoteInvoice => "quote_invoice",
            TemplateKind::Project => "project",
            TemplateKind::Workflow => "workflow",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.as_str() == s)
    }
}

/// One declared template variable (editor UX metadata). Keys are validated
/// against the kind's closed vocabulary at write time — a template can only
/// declare variables that resolve from its render context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TemplateVariable {
    pub key: String,
    pub label: String,
}

/// One template row: a shipped default (builtin + is_system), a fork of
/// one (forked_from), or an org-created template.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Template {
    pub id: Uuid,
    pub org_id: Uuid,
    pub kind: TemplateKind,
    pub slug: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub body: String,
    pub variables: Vec<TemplateVariable>,
    pub builtin: Option<String>,
    pub is_system: bool,
    pub forked_from: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
