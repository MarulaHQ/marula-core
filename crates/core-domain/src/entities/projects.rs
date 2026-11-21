//! Projects: project → phase → task hierarchy, task dependencies, saved
//! task-list views, and reusable project templates (the private build tracker Phase 3
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

use crate::ids::{ClientId, ContractId, OrgId, PhaseId, ProjectId, TaskId, UserId};
use chrono::{DateTime, NaiveDate, Utc};

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
    /// The contract this project bills through (Wave 39 lane 4, report
    /// P2): its rate card, blocks, and caps price the project's T&M work.
    /// `None` = non-billable or not linked. The link validates in the
    /// app layer (same client, active-or-draft contract).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub billing_contract_id: Option<ContractId>,
    /// The reason the client is given while the project is on hold, in the
    /// PM's own words. It is the only reason text that crosses the portal
    /// boundary; the staff hold note stays staff-side. `None` = the PM has not
    /// written one (the portal then says only that work is paused), and it is
    /// cleared when the project leaves hold.
    #[serde(default)]
    pub client_hold_reason: Option<String>,
    /// The status the pause interrupted. Written when a project moves onto
    /// `on_hold` from another status and cleared when it leaves hold, so
    /// "Resume work" restores where it was instead of assuming `active`.
    /// `None` for a project created on hold or written before the column.
    #[serde(default)]
    pub pre_hold_status: Option<ProjectStatus>,
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
    /// The phase's PLANNED window (Wave 34 claims close-out, claim 6).
    ///
    /// Both ends are nullable and that is the honest state rather than a
    /// compromise: most real projects have phases nobody has dated yet, and
    /// a NOT NULL pair with a "today" default would draw a Gantt that lies
    /// about the plan. A phase is drawn on the bar chart only when BOTH
    /// ends are set; a half-dated phase stays in the ordered list and the
    /// UI says which it is.
    pub start_date: Option<NaiveDate>,
    /// Inclusive end date. CHECK-enforced `>= start_date` in the database,
    /// because a bar drawn backwards reads as a rendering bug and is
    /// actually a data problem the operator needs told about.
    pub end_date: Option<NaiveDate>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ProjectPhase {
    /// Can this phase be drawn on the dated Gantt?
    ///
    /// Both ends or neither: a phase with only a start has no length, and
    /// inventing one (a zero-width bar, or "start + 1 day") would put a
    /// duration on the timeline that nobody planned.
    pub fn is_dated(&self) -> bool {
        self.start_date.is_some() && self.end_date.is_some()
    }

    /// Planned working days in the window, inclusive of both ends.
    ///
    /// `None` when the phase is undated. Weekends are counted as days: this
    /// is a PLANNING span for laying a bar on a chart, not a commitment
    /// about working days, and a planner who needs working days excludes
    /// them in the calendar they schedule against.
    pub fn planned_days(&self) -> Option<i64> {
        match (self.start_date, self.end_date) {
            (Some(s), Some(e)) => Some((e - s).num_days() + 1),
            _ => None,
        }
    }
}

/// Where a unit of project work happens. The kind changes what a coordinator
/// needs to know before placing it: an onsite visit carries travel, a bench job
/// needs a bench, a vendor step waits on somebody outside the company.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum WorkType {
    Onsite,
    Bench,
    Remote,
    Vendor,
    Client,
    Coordination,
}

impl WorkType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Onsite => "onsite",
            Self::Bench => "bench",
            Self::Remote => "remote",
            Self::Vendor => "vendor",
            Self::Client => "client",
            Self::Coordination => "coordination",
        }
    }
}

impl std::fmt::Display for WorkType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for WorkType {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "onsite" => Ok(Self::Onsite),
            "bench" => Ok(Self::Bench),
            "remote" => Ok(Self::Remote),
            "vendor" => Ok(Self::Vendor),
            "client" => Ok(Self::Client),
            "coordination" => Ok(Self::Coordination),
            other => Err(crate::DomainError::Validation(format!(
                "invalid work type: {other}"
            ))),
        }
    }
}

/// A unit of project work. A task flagged `is_milestone` is a billing
/// milestone: completing it emits `project.milestone_completed`.
///
/// Time is stored, not derived (ADR 0034 decision 1). `start_at` and
/// `duration_minutes` are both-or-neither: a task an organisation has not dated
/// carries neither, and the plan draws it as undated rather than implying a
/// date. `baseline_*` holds what the plan said when it was agreed, written by
/// the baseline capture and never by an ordinary edit.
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
    pub start_at: Option<DateTime<Utc>>,
    pub duration_minutes: Option<i32>,
    pub effort_estimate_minutes: Option<i32>,
    pub effort_remaining_minutes: Option<i32>,
    pub work_type: Option<WorkType>,
    pub priority: Option<crate::status::Priority>,
    pub role_key: Option<String>,
    // Both are new since the 0.1.0 payloads and are not `Option`, so they must
    // default: a payload pinned before they existed still has to parse
    // (contract policy §2, additive-only). The event conformance suite proves
    // it with the committed goldens.
    #[serde(default)]
    pub required_skills: Vec<String>,
    #[serde(default)]
    pub is_critical: bool,
    pub baseline_start_at: Option<DateTime<Utc>>,
    pub baseline_duration_minutes: Option<i32>,
    pub external_owner: Option<String>,
    pub calendar_pin: Option<uuid::Uuid>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A project's own working pattern, where it differs from the organisation's.
///
/// `weekdays` is a bitmask with Monday at bit 0 and Sunday at bit 6, so 31 is
/// Monday to Friday and 63 adds Saturday. No row means "use the organisation's
/// calendar"; the project's row wins for the project's own dates (ADR 0034
/// decision 3). `minutes_per_day` excludes lunch.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectWorkingCalendar {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub project_id: ProjectId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub weekdays: i16,
    pub minutes_per_day: i32,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ProjectWorkingCalendar {
    /// Whether the given weekday is worked. Monday is `0`, Sunday is `6`.
    pub fn works_on(&self, weekday: u8) -> bool {
        weekday < 7 && self.weekdays & (1_i16 << weekday) != 0
    }

    /// Minutes of work in one full week of this pattern.
    pub fn weekly_minutes(&self) -> i32 {
        self.weekdays.count_ones() as i32 * self.minutes_per_day
    }
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

/// A phase's billing budget and/or fixed-fee record (Wave 39 lane 4,
/// report P3/P4). One optional row per phase: `budget_minor` is the
/// contracted denominator the burn math consumes (T&M phases), and
/// `fee_minor` is the fixed fee a completed NON-milestone phase releases
/// (milestone phases carry their own amount on the phase row).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectPhaseBudget {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub phase_id: PhaseId,
    /// Contracted budget for the phase (minor units); `None` = unbudgeted.
    pub budget_minor: Option<i64>,
    /// ISO 4217 code for `budget_minor`.
    pub currency: Option<String>,
    /// Fixed fee released on phase completion (minor units); `None` = no fee.
    pub fee_minor: Option<i64>,
    /// ISO 4217 code for `fee_minor`.
    pub fee_currency: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The burn read model for one phase: consumed minutes priced against the
/// project's linked contract versus the contracted budget, with a typed
/// alert state. Amounts are in the budget's currency.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PhaseBurn {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub phase_id: PhaseId,
    pub phase_name: String,
    pub phase_status: PhaseStatus,
    /// Consumed minutes (approved billable entries scoped to the phase).
    pub consumed_minutes: i64,
    /// Consumed value in minor units (unit_amount_minor per-line pricing
    /// folded up; 0 when the phase carries a pure fee).
    pub consumed_minor: i64,
    /// The contracted budget (minor units); `None` = unbudgeted phase.
    pub budget_minor: Option<i64>,
    /// Currency of both amounts.
    pub currency: Option<String>,
    /// Typed alert: `none` under budget, `warn` past 80%, `over` past 100%.
    pub alert: PhaseBurnAlert,
}

/// The phase-burn alert state. `warn` fires at ≥80% consumed; `over` at
/// ≥100%. Never silent: the read model carries the label, the UI renders it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PhaseBurnAlert {
    #[default]
    None,
    Warn,
    Over,
}

impl PhaseBurnAlert {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Warn => "warn",
            Self::Over => "over",
        }
    }
}

impl std::fmt::Display for PhaseBurnAlert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One released milestone: the durable record that a
/// completed milestone or fixed-fee phase released its amount to the
/// invoicing pipeline. `pending` rows bill on the first run that reaches
/// the window; the run flips them `billed` with the consuming invoice,
/// so a replay can never double-bill.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectBillingRelease {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub project_id: ProjectId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub phase_id: Option<PhaseId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub task_id: Option<TaskId>,
    /// The milestone name at release time (frozen from the event).
    pub label: String,
    pub amount_minor: i64,
    pub currency: String,
    pub released_on: chrono::NaiveDate,
    pub status: ReleaseStatus,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub invoice_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub invoice_line_id: Option<uuid::Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Release lifecycle: `pending` (released, not yet invoiced), `billed`
/// (carried on an invoice line), `cancelled` (terminal — an admin
/// correction; the same milestone may re-release afterwards).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReleaseStatus {
    #[default]
    Pending,
    Billed,
    Cancelled,
}

impl ReleaseStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Billed => "billed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::fmt::Display for ReleaseStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ReleaseStatus {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "billed" => Ok(Self::Billed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(crate::DomainError::Validation(format!(
                "invalid release status: {other}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calendar(weekdays: i16, minutes_per_day: i32) -> ProjectWorkingCalendar {
        ProjectWorkingCalendar {
            project_id: uuid::Uuid::nil(),
            org_id: uuid::Uuid::nil(),
            weekdays,
            minutes_per_day,
            note: None,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            updated_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }

    #[test]
    fn monday_to_friday_is_thirty_one() {
        let cal = calendar(31, 480);
        assert!(cal.works_on(0));
        assert!(cal.works_on(4));
        assert!(!cal.works_on(5));
        assert_eq!(cal.weekly_minutes(), 2400);
    }

    #[test]
    fn saturday_is_bit_five() {
        let cal = calendar(63, 600);
        assert!(cal.works_on(5));
        assert_eq!(cal.weekly_minutes(), 3600);
    }

    #[test]
    fn a_weekday_out_of_range_is_never_worked() {
        assert!(!calendar(127, 480).works_on(7));
    }

    #[test]
    fn work_type_round_trips_through_its_wire_name() {
        for work in [
            WorkType::Onsite,
            WorkType::Bench,
            WorkType::Remote,
            WorkType::Vendor,
            WorkType::Client,
            WorkType::Coordination,
        ] {
            assert_eq!(work.as_str().parse::<WorkType>(), Ok(work));
        }
        assert!("flying".parse::<WorkType>().is_err());
    }
}
