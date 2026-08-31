//! Field catalog for the versioned read-model views (`ops.v1_*`, migration
//! 0026) — the FIXED identifier allowlist ADR 0001 mandates.
//!
//! Purpose: every column/filter/aggregate reference downstream flows
//! originates HERE as a Rust constant. The lookup functions are the single
//! funnel report builders go through; they fail closed (`None`/`Err`) on
//! anything that is not an enumerated identifier, which is what makes
//! identifier-interpolation injection structurally impossible: there is no
//! code path that turns attacker-controlled text into SQL text.
//!
//! Non-purpose: no SQL emission (query.rs builds statements FROM these
//! identifiers), no execution, no HTTP concerns.

/// Versioned read-model views. Variant ↔ physical view name is frozen
/// alongside the schema-version suffix; adding a view is additive.
///
/// The serde encoding is the view SLUG (`snake_case`) — the same string the
/// stored-spec `kind` column carries (R3 registry backing) — so a definition
/// round-trips through JSON without a parallel name table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ViewId {
    Tickets,
    TimeEntries,
    SlaOutcomes,
    GapDays,
    FirstResponse,
}

impl ViewId {
    /// Fully-qualified view name as it appears in generated SQL. Literal
    /// constants only — never composed from runtime strings.
    pub const fn qualified_name(self) -> &'static str {
        match self {
            ViewId::Tickets => "ops.v1_tickets",
            ViewId::TimeEntries => "ops.v1_time_entries",
            ViewId::SlaOutcomes => "ops.v1_sla_outcomes",
            ViewId::GapDays => "ops.v1_gap_days",
            ViewId::FirstResponse => "ops.v1_first_response",
        }
    }

    /// All views exposed in v1 (R6 picker surface / external BI docs).
    pub const ALL: [ViewId; 5] = [
        ViewId::Tickets,
        ViewId::TimeEntries,
        ViewId::SlaOutcomes,
        ViewId::GapDays,
        ViewId::FirstResponse,
    ];

    /// Strict slug parser (the inverse of the serde encoding). Used when
    /// reassembling stored specs; anything unknown fails closed.
    pub fn parse(slug: &str) -> Option<ViewId> {
        Self::ALL.into_iter().find(|v| v.slug() == slug)
    }

    /// Stable URL/API/JSON slug. Constants only.
    pub const fn slug(self) -> &'static str {
        match self {
            ViewId::Tickets => "tickets",
            ViewId::TimeEntries => "time_entries",
            ViewId::SlaOutcomes => "sla_outcomes",
            ViewId::GapDays => "gap_days",
            ViewId::FirstResponse => "first_response",
        }
    }

    /// The personal-dimension column RBAC self-scoping narrows on (R1
    /// convention: non-managers are structurally forced onto their own
    /// rows). `None` would mean the view has no personal dimension.
    pub const fn personal_dim(self) -> Option<&'static str> {
        match self {
            ViewId::Tickets => Some("assignee_id"),
            ViewId::TimeEntries => Some("user_id"),
            ViewId::SlaOutcomes => Some("assignee_id"),
            ViewId::GapDays => Some("user_id"),
            ViewId::FirstResponse => Some("assignee_id"),
        }
    }

    /// The primary time anchor used to apply a run-time window when a spec
    /// is executed with one (R4/R6 execution lanes). Typed so the bind is
    /// Date vs Timestamp without re-probing the catalog.
    pub const fn time_anchor(self) -> Option<(&'static str, ValueType)> {
        match self {
            ViewId::Tickets => Some(("created_at", ValueType::Timestamp)),
            ViewId::TimeEntries => Some(("started_at", ValueType::Timestamp)),
            ViewId::SlaOutcomes => Some(("ticket_created_at", ValueType::Timestamp)),
            ViewId::GapDays => Some(("day", ValueType::Date)),
            ViewId::FirstResponse => Some(("created_at", ValueType::Timestamp)),
        }
    }

    /// Column inventory of this view, mirroring the migration-0026 header
    /// comments (the binding documentation contract).
    pub const fn columns(self) -> &'static [ColumnDef] {
        const TICKETS: &[ColumnDef] = &[
            ColumnDef::new("id", ValueType::Uuid),
            ColumnDef::new("org_id", ValueType::Uuid),
            ColumnDef::new("number", ValueType::Text),
            ColumnDef::new("board_id", ValueType::Uuid),
            ColumnDef::new("board_name", ValueType::Text),
            ColumnDef::new("status_id", ValueType::Uuid),
            ColumnDef::new("status_name", ValueType::Text),
            ColumnDef::new("category", ValueType::Text),
            ColumnDef::new("priority", ValueType::Text),
            ColumnDef::new("title", ValueType::Text),
            ColumnDef::new("client_id", ValueType::Uuid),
            ColumnDef::new("client_name", ValueType::Text),
            ColumnDef::new("site_id", ValueType::Uuid),
            ColumnDef::new("assignee_id", ValueType::Uuid),
            ColumnDef::new("assignee_name", ValueType::Text),
            ColumnDef::new("requester_id", ValueType::Uuid),
            ColumnDef::new("due_at", ValueType::Timestamp),
            ColumnDef::new("resolved_at", ValueType::Timestamp),
            ColumnDef::new("created_at", ValueType::Timestamp),
            ColumnDef::new("updated_at", ValueType::Timestamp),
            ColumnDef::new("first_response_due", ValueType::Timestamp),
            ColumnDef::new("resolution_due", ValueType::Timestamp),
            ColumnDef::new("breached", ValueType::Bool),
        ];
        const TIME_ENTRIES: &[ColumnDef] = &[
            ColumnDef::new("id", ValueType::Uuid),
            ColumnDef::new("org_id", ValueType::Uuid),
            ColumnDef::new("ticket_id", ValueType::Uuid),
            ColumnDef::new("ticket_number", ValueType::Text),
            ColumnDef::new("client_id", ValueType::Uuid),
            ColumnDef::new("client_name", ValueType::Text),
            ColumnDef::new("user_id", ValueType::Uuid),
            ColumnDef::new("user_display_name", ValueType::Text),
            ColumnDef::new("started_at", ValueType::Timestamp),
            ColumnDef::new("ended_at", ValueType::Timestamp),
            ColumnDef::new("minutes", ValueType::Int),
            ColumnDef::new("billable", ValueType::Bool),
            ColumnDef::new("work_type", ValueType::Text),
            ColumnDef::new("day_utc", ValueType::Date),
            ColumnDef::new("timer_session_id", ValueType::Uuid),
            ColumnDef::new("created_at", ValueType::Timestamp),
        ];
        const SLA_OUTCOMES: &[ColumnDef] = &[
            ColumnDef::new("ticket_id", ValueType::Uuid),
            ColumnDef::new("org_id", ValueType::Uuid),
            ColumnDef::new("ticket_number", ValueType::Text),
            ColumnDef::new("priority", ValueType::Text),
            ColumnDef::new("assignee_id", ValueType::Uuid),
            ColumnDef::new("ticket_created_at", ValueType::Timestamp),
            ColumnDef::new("policy_id", ValueType::Uuid),
            ColumnDef::new("policy_name", ValueType::Text),
            ColumnDef::new("calendar_name", ValueType::Text),
            ColumnDef::new("calendar_timezone", ValueType::Text),
            ColumnDef::new("first_response_due", ValueType::Timestamp),
            ColumnDef::new("first_responded_at", ValueType::Timestamp),
            ColumnDef::new("response_met", ValueType::Bool),
            ColumnDef::new("resolution_due", ValueType::Timestamp),
            ColumnDef::new("resolved_at", ValueType::Timestamp),
            ColumnDef::new("resolution_met", ValueType::Bool),
            ColumnDef::new("breached", ValueType::Bool),
            ColumnDef::new("paused_business_secs", ValueType::BigInt),
        ];
        const GAP_DAYS: &[ColumnDef] = &[
            ColumnDef::new("id", ValueType::Uuid),
            ColumnDef::new("org_id", ValueType::Uuid),
            ColumnDef::new("user_id", ValueType::Uuid),
            ColumnDef::new("user_display_name", ValueType::Text),
            ColumnDef::new("user_timezone", ValueType::Text),
            ColumnDef::new("day", ValueType::Date),
            ColumnDef::new("activity", ValueType::Bool),
            ColumnDef::new("logged_minutes", ValueType::Int),
            ColumnDef::new("expected_minutes", ValueType::Int),
            ColumnDef::new("has_gap", ValueType::Bool),
            ColumnDef::new("nudge_sent_at", ValueType::Timestamp),
            ColumnDef::new("updated_at", ValueType::Timestamp),
        ];
        const FIRST_RESPONSE: &[ColumnDef] = &[
            ColumnDef::new("ticket_id", ValueType::Uuid),
            ColumnDef::new("org_id", ValueType::Uuid),
            ColumnDef::new("ticket_number", ValueType::Text),
            ColumnDef::new("client_id", ValueType::Uuid),
            ColumnDef::new("assignee_id", ValueType::Uuid),
            ColumnDef::new("created_at", ValueType::Timestamp),
            ColumnDef::new("first_staff_comment_at", ValueType::Timestamp),
            ColumnDef::new("first_response_secs", ValueType::BigInt),
            ColumnDef::new("first_response_due", ValueType::Timestamp),
        ];
        match self {
            ViewId::Tickets => TICKETS,
            ViewId::TimeEntries => TIME_ENTRIES,
            ViewId::SlaOutcomes => SLA_OUTCOMES,
            ViewId::GapDays => GAP_DAYS,
            ViewId::FirstResponse => FIRST_RESPONSE,
        }
    }

    /// Columns this view allows v1 REPORT filters on (the writable part of
    /// the catalog boundary; narrower than the full projection on purpose —
    /// filtered-but-unselected columns surprise report consumers).
    pub const fn filterable(self) -> &'static [&'static str] {
        const TICKETS: &[&str] = &["board_id", "client_id", "site_id", "priority", "category"];
        const TIME_ENTRIES: &[&str] = &[
            "ticket_id",
            "user_id",
            "client_id",
            "billable",
            "work_type",
            "day_utc",
        ];
        const SLA_OUTCOMES: &[&str] = &[
            "policy_id",
            "assignee_id",
            "priority",
            "breached",
            "response_met",
            "resolution_met",
        ];
        const GAP_DAYS: &[&str] = &["user_id", "has_gap"];
        const FIRST_RESPONSE: &[&str] = &["client_id", "assignee_id"];
        match self {
            ViewId::Tickets => TICKETS,
            ViewId::TimeEntries => TIME_ENTRIES,
            ViewId::SlaOutcomes => SLA_OUTCOMES,
            ViewId::GapDays => GAP_DAYS,
            ViewId::FirstResponse => FIRST_RESPONSE,
        }
    }
}

/// Static column descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnDef {
    pub name: &'static str,
    pub value_type: ValueType,
}

impl ColumnDef {
    pub const fn new(name: &'static str, value_type: ValueType) -> Self {
        Self { name, value_type }
    }
}

/// Value classes shared between catalog types, bind parameters and output
/// decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Uuid,
    Text,
    Timestamp,
    Date,
    Int,
    BigInt,
    Bool,
}

/// Filter operators supported by the v1 compiled reports. Operand typing is
/// enforced through [`ValueType`] matches in the builder, so e.g. a bool
/// operand can never reach a timestamp column. The serde encoding is the
/// `snake_case` slug (`lt_eq`, `is_null`, …) stored specs carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    IsNull,
    IsNotNull,
}

impl FilterOp {
    /// SQL fragment for this operator against a `$N` placeholder. Constants
    /// only.
    pub const fn fragment(self) -> &'static str {
        match self {
            FilterOp::Eq => "=",
            FilterOp::NotEq => "<>",
            FilterOp::Lt => "<",
            FilterOp::LtEq => "<=",
            FilterOp::Gt => ">",
            FilterOp::GtEq => ">=",
            FilterOp::IsNull | FilterOp::IsNotNull => "IS",
        }
    }

    pub const fn takes_value(self) -> bool {
        !matches!(self, FilterOp::IsNull | FilterOp::IsNotNull)
    }
}

/// The single public entry point for turning a (view, column name) pair
/// into a validated identifier handle. Returns `None` for ANYTHING else —
/// this is the deliberate fail-closed seam.
#[must_use]
pub fn lookup_column(view: ViewId, name: &str) -> Option<&'static ColumnDef> {
    view.columns().iter().find(|c| c.name == name)
}

/// Funnels a filter-column name through BOTH the existence check and the
/// v1 filter allowlist. Fail closed on every miss.
///
/// # Errors
/// Unknown column names — the entire string space outside the static
/// catalogs, including any injection attempt — produce
/// [`ReportsError::UnknownColumn`].
pub fn resolve_filter_column(
    view: ViewId,
    name: &str,
) -> Result<&'static ColumnDef, crate::error::ReportsError> {
    if !view.filterable().contains(&name) {
        return Err(crate::error::ReportsError::UnknownColumn {
            view: view.qualified_name().to_string(),
            name: name.to_string(),
        });
    }
    lookup_column(view, name).ok_or_else(|| crate::error::ReportsError::UnknownColumn {
        view: view.qualified_name().to_string(),
        name: name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_column_is_unique_per_view() {
        for view in ViewId::ALL {
            let mut seen: Vec<&str> = Vec::new();
            for col in view.columns() {
                assert!(
                    !seen.contains(&col.name),
                    "{view:?}: duplicate column {}",
                    col.name
                );
                seen.push(col.name);
            }
        }
    }

    #[test]
    fn filterable_columns_are_a_subset_of_the_projection() {
        for view in ViewId::ALL {
            for name in view.filterable() {
                assert!(
                    lookup_column(view, name).is_some(),
                    "{view:?}: filterable '{name}' missing from projection"
                );
            }
        }
    }

    #[test]
    fn qualified_names_carry_the_version_prefix() {
        for view in ViewId::ALL {
            let name = view.qualified_name();
            assert!(name.starts_with("ops.v1_"), "{name} lost its version");
            assert!(name[name.len() - name.len()..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.'));
        }
    }

    #[test]
    fn view_slugs_round_trip_strictly() {
        for view in ViewId::ALL {
            assert_eq!(ViewId::parse(view.slug()), Some(view));
            // The serde encoding must be exactly the slug.
            let json = serde_json::to_string(&view).unwrap();
            assert_eq!(json, format!("\"{}\"", view.slug()));
            assert_eq!(serde_json::from_str::<ViewId>(&json).unwrap(), view);
        }
        for bad in ["", "ops.v1_tickets", "TICKETS", "ticket", "' OR 1=1 --"] {
            assert_eq!(ViewId::parse(bad), None, "slug {bad:?} must not parse");
            assert!(serde_json::from_str::<ViewId>(&format!("\"{bad}\"")).is_err());
        }
    }

    #[test]
    fn personal_dimensions_are_real_columns() {
        for view in ViewId::ALL {
            let dim = view.personal_dim().expect("every v1 view has one");
            assert!(
                lookup_column(view, dim).is_some(),
                "{view:?}: personal dim {dim} missing from projection"
            );
            assert_ne!(dim, "org_id", "tenancy column is never a report dimension");
        }
    }

    #[test]
    fn time_anchors_are_typed_columns() {
        for view in ViewId::ALL {
            let (name, ty) = view.time_anchor().expect("every v1 view has one");
            let def = lookup_column(view, name).expect("anchor exists");
            assert_eq!(def.value_type, ty, "{view:?}: anchor type drift");
        }
    }
}
