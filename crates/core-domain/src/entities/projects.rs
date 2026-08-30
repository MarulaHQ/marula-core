//! Projects: project → phase → task hierarchy, task dependencies, saved
//! task-list views, and reusable project templates (TASKLIST Phase 3
//! projects line).
//!
//! Statuses are closed enums stored as snake_case text in migrations 0030
//! with matching CHECK constraints; behavior binds to these enums, never to
//! display labels (semantic-primitive doctrine, policy §5).
//!
//! Milestone billing seam: a phase or task flagged `is_milestone` may carry
//! an optional billing amount (minor units + ISO 4217 currency). Completing
//! it emits `project.milestone_completed` through the transactional outbox —
//! the event envelope is the ONLY seam to core-billing (Wave 9 invoicing
//! consumes it); no billing internals are imported here.

use crate::ids::{ClientId, OrgId, PhaseId, ProjectId, TaskId, UserId};
use chrono::{DateTime, Utc};

/// Lifecycle of a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Planning,
    Active,
    OnHold,
    Completed,
    Cancelled,
}

impl ProjectStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Active => "active",
            Self::OnHold => "on_hold",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::fmt::Display for ProjectStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ProjectStatus {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "planning" => Ok(Self::Planning),
            "active" => Ok(Self::Active),
            "on_hold" => Ok(Self::OnHold),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(crate::DomainError::Validation(format!(
                "invalid project status: {other}"
            ))),
        }
    }
}

/// How the project turns delivered work into money. The value rides the
/// milestone event payload so core-billing can route without re-reading the
/// project row (event payloads are additive-only, policy §2).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum BillingMethod {
    Milestone,
    TimeAndMaterials,
    FixedFee,
    #[default]
    NonBillable,
}

impl BillingMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Milestone => "milestone",
            Self::TimeAndMaterials => "time_and_materials",
            Self::FixedFee => "fixed_fee",
            Self::NonBillable => "non_billable",
        }
    }
}

impl std::fmt::Display for BillingMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for BillingMethod {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "milestone" => Ok(Self::Milestone),
            "time_and_materials" => Ok(Self::TimeAndMaterials),
            "fixed_fee" => Ok(Self::FixedFee),
            "non_billable" => Ok(Self::NonBillable),
            other => Err(crate::DomainError::Validation(format!(
                "invalid billing method: {other}"
            ))),
        }
    }
}

/// Lifecycle of a phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    Pending,
    Active,
    Completed,
    Cancelled,
}

impl PhaseStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::fmt::Display for PhaseStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for PhaseStatus {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "active" => Ok(Self::Active),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(crate::DomainError::Validation(format!(
                "invalid phase status: {other}"
            ))),
        }
    }
}

/// Lifecycle of a task. `completed` is reached ONLY through the dedicated
/// complete endpoint, which enforces dependency readiness and emits the
/// completion (+ milestone billing seam) events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Blocked,
    Completed,
    Cancelled,
}

impl TaskStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "in_progress" => Ok(Self::InProgress),
            "blocked" => Ok(Self::Blocked),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(crate::DomainError::Validation(format!(
                "invalid task status: {other}"
            ))),
        }
    }
}

/// A project engagement under a client (or internal when `client_id` is
/// `None`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Project {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: ProjectId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<ClientId>,
    pub name: String,
    pub description: Option<String>,
    pub status: ProjectStatus,
    pub billing_method: BillingMethod,
    /// Project budget in minor units; `None` = unbudgeted.
    pub budget_minor: Option<i64>,
    /// ISO 4217 currency for `budget_minor` (required when a budget is set).
    pub currency: Option<String>,
    pub start_date: Option<chrono::NaiveDate>,
    pub due_date: Option<chrono::NaiveDate>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A phase groups tasks into a delivery stage. A phase flagged
/// `is_milestone` is itself a billing milestone.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectPhase {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: PhaseId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub project_id: ProjectId,
    pub name: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub status: PhaseStatus,
    pub is_milestone: bool,
    /// Billing amount released when this milestone phase completes
    /// (minor units; `None` = informational milestone).
    pub milestone_amount_minor: Option<i64>,
    /// ISO 4217 code for `milestone_amount_minor`.
    pub milestone_currency: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A unit of project work. A task flagged `is_milestone` is a billing
/// milestone: completing it emits `project.milestone_completed`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectTask {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: TaskId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub project_id: ProjectId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub phase_id: Option<PhaseId>,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assignee_id: Option<UserId>,
    pub sort_order: i32,
    pub is_milestone: bool,
    pub milestone_amount_minor: Option<i64>,
    pub milestone_currency: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A saved task-list view: filter/group/sort spec over one project's tasks
/// (`project_id: None` = org-wide). The `spec` is a versioned, additive-only
/// JSON document validated at the route layer; unknown fields are tolerated
/// (policy §2 posture for declared customizations-as-data).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectView {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub project_id: Option<ProjectId>,
    pub name: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub spec: serde_json::Value,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A reusable project blueprint. `spec` follows `core-projects::template`
/// (`TemplateSpec`, additive-only evolution); instantiation is a plain
/// transactional insert plan — templates are data, not code (policy §5).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectTemplate {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    pub description: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub spec: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
