//! Report definitions as data (TASKLIST R3; ADR 0001 §Decision).
//!
//! A [`ReportSpec`] is a STORED report definition's query shape: which
//! versioned view it reads, its filters, group-by dimensions, aggregate
//! measures, and the visualization hint the builder UI renders with. Every
//! identifier flows from the fixed field catalog ([`crate::catalog`]) —
//! the typed registry ADR 0001 mandates — so a spec can never name a view
//! or column that is not part of the versioned read-model contract:
//!
//! * `view` is the [`ViewId`] enum (its slug serde encoding is the stored
//!   `kind` column);
//! * filter/group-by/measure columns resolve through the catalog lookups,
//!   which fail closed on anything outside the static inventories (the
//!   same seam that makes identifier-interpolation injection structurally
//!   impossible for the prebuilt kinds);
//! * values bind exclusively as `$N` parameters.
//!
//! [`ReportSpec::compile`] turns a validated spec into parameterized SQL
//! over the `ops.v1_*` views ONLY — never physical tables (binding R0/U3
//! policy). Definitions are windowless templates: the run-time window is
//! an execution-lane concern (R4/R6) applied via [`ViewId::time_anchor`];
//! this compiler bounds every statement with a hard LIMIT instead.
//!
//! Non-purpose: no storage (core-db `report_definitions` owns rows), no
//! HTTP shapes (core-api `routes::report_defs`), no scheduling (R4), no
//! cursor pagination — grouped spec output is a single bounded page (the
//! R1 keyset layouts are kind-specific and do not generalize).

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::builder::{Bind, CompiledReport, CursorShape, OutType, OutputColumn};
use crate::catalog::{lookup_column, resolve_filter_column, FilterOp, ValueType, ViewId};
use crate::error::ReportsError;
use crate::params::Scope;

/// Filter-count bound per spec.
pub const MAX_FILTERS: usize = 8;
/// Group-by dimension bound per spec.
pub const MAX_GROUP_BY: usize = 3;
/// Measure bound per spec.
pub const MAX_MEASURES: usize = 4;

/// Visualization hint the report builder renders with (ADR 0001: Recharts
/// bar/line/pie/table on the frontend). Pure presentation metadata — it
/// never influences the compiled SQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum VizHint {
    Bar,
    Line,
    Pie,
    Table,
}

impl VizHint {
    pub const ALL: [VizHint; 4] = [VizHint::Bar, VizHint::Line, VizHint::Pie, VizHint::Table];

    pub const fn slug(self) -> &'static str {
        match self {
            VizHint::Bar => "bar",
            VizHint::Line => "line",
            VizHint::Pie => "pie",
            VizHint::Table => "table",
        }
    }

    /// Strict parser (inverse of the serde encoding) for the stored `viz`
    /// column. Anything unknown fails closed.
    pub fn parse(slug: &str) -> Option<VizHint> {
        Self::ALL.into_iter().find(|v| v.slug() == slug)
    }
}

/// Aggregation function of a measure. `Count` may omit the column
/// (`COUNT(*)`); every other aggregate requires one, and the catalog
/// decides which column types each accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Aggregate {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl Aggregate {
    pub const fn slug(self) -> &'static str {
        match self {
            Aggregate::Count => "count",
            Aggregate::Sum => "sum",
            Aggregate::Avg => "avg",
            Aggregate::Min => "min",
            Aggregate::Max => "max",
        }
    }

    /// Column value classes this aggregate accepts from the catalog.
    fn accepts(self, ty: ValueType) -> bool {
        match self {
            Aggregate::Count => true,
            Aggregate::Sum | Aggregate::Avg => matches!(ty, ValueType::Int | ValueType::BigInt),
            // Min/Max are order statistics: numerics and ordered temporal
            // types. Text is excluded on purpose (alphabetical min/max of
            // labels is never what a report author means).
            Aggregate::Min | Aggregate::Max => matches!(
                ty,
                ValueType::Int | ValueType::BigInt | ValueType::Timestamp | ValueType::Date
            ),
        }
    }
}

/// A typed filter operand. The variant must match the catalog
/// [`ValueType`] of the filtered column (`Int` covers both Int and BigInt
/// columns) — enforced by [`ReportSpec::validate`], so a mismatched
/// operand can never reach a bind. The serde encoding is the externally
/// tagged slug (`{"text": "p1"}`, `{"bool": true}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FilterValue {
    Uuid(Uuid),
    Text(String),
    /// RFC 3339 instant (chrono serde encoding).
    Timestamp(DateTime<Utc>),
    /// `YYYY-MM-DD`.
    Date(NaiveDate),
    Int(i64),
    Bool(bool),
}

impl FilterValue {
    fn matches(&self, ty: ValueType) -> bool {
        matches!(
            (self, ty),
            (FilterValue::Uuid(_), ValueType::Uuid)
                | (FilterValue::Text(_), ValueType::Text)
                | (FilterValue::Timestamp(_), ValueType::Timestamp)
                | (FilterValue::Date(_), ValueType::Date)
                | (FilterValue::Int(_), ValueType::Int | ValueType::BigInt)
                | (FilterValue::Bool(_), ValueType::Bool)
        )
    }

    fn into_bind(self) -> Bind {
        match self {
            FilterValue::Uuid(u) => Bind::Uuid(u),
            FilterValue::Text(s) => Bind::Text(s),
            FilterValue::Timestamp(t) => Bind::Timestamp(t),
            FilterValue::Date(d) => Bind::Date(d),
            FilterValue::Int(i) => Bind::Int(i),
            FilterValue::Bool(b) => Bind::Bool(b),
        }
    }
}

/// One spec filter: catalog column + operator + typed operand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SpecFilter {
    /// Column name from the view's FILTERABLE inventory (the narrower v1
    /// allowlist — filtered-but-unselected columns surprise consumers).
    pub column: String,
    pub op: FilterOp,
    /// Required exactly when the operator takes a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<FilterValue>,
}

/// One aggregate measure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MeasureSpec {
    pub agg: Aggregate,
    /// `None` only with [`Aggregate::Count`] (`COUNT(*)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
}

impl MeasureSpec {
    /// Stable output column name: the aggregate slugged onto the source
    /// column (`sum_minutes`), bare `count` for `COUNT(*)`. The prefix
    /// scheme keeps measure names disjoint from projected dimension names
    /// (no catalog column carries an aggregate-prefix name).
    pub fn output_name(&self) -> String {
        match (&self.agg, &self.column) {
            (Aggregate::Count, None) => "count".to_string(),
            (agg, Some(c)) => format!("{}_{}", agg.slug(), c),
            (agg, None) => agg.slug().to_string(),
        }
    }
}

/// A stored report spec (TASKLIST R3). Every member is registry-validated;
/// see [`ReportSpec::validate`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportSpec {
    /// The versioned read-model view this report reads (the stored `kind`
    /// column). NEVER a physical table — the enum has no other variants.
    pub view: ViewId,
    #[serde(default)]
    pub filters: Vec<SpecFilter>,
    /// Group-by dimensions (any projected column except `org_id`). Empty
    /// means a single grand-total row.
    #[serde(default)]
    pub group_by: Vec<String>,
    /// Aggregate measures, one or more.
    pub measures: Vec<MeasureSpec>,
    /// Presentation hint (never affects the compiled SQL).
    pub viz: VizHint,
}

/// The jsonb body stored alongside the registry-backed `kind`/`viz`
/// columns: exactly the spec members that are NOT columns. Reassembled
/// through [`ReportSpec::from_parts`] on every load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecBody {
    #[serde(default)]
    pub filters: Vec<SpecFilter>,
    #[serde(default)]
    pub group_by: Vec<String>,
    pub measures: Vec<MeasureSpec>,
}

impl ReportSpec {
    /// Splits into the stored `kind` column value, the jsonb body, and the
    /// `viz` column value.
    #[must_use]
    pub fn to_parts(&self) -> (&'static str, SpecBody, &'static str) {
        (
            self.view.slug(),
            SpecBody {
                filters: self.filters.clone(),
                group_by: self.group_by.clone(),
                measures: self.measures.clone(),
            },
            self.viz.slug(),
        )
    }

    /// Reassembles and re-validates a spec from its stored representation.
    /// EVERY load passes through the registry: a stored spec that no longer
    /// validates (a definition hand-edited in place, or written by an older
    /// build with narrower rules) fails closed instead of compiling.
    ///
    /// # Errors
    /// [`ReportsError::InvalidSpec`] for unknown slugs or a malformed body
    /// document; registry violations surface from [`Self::validate`].
    pub fn from_parts(
        kind: &str,
        body: &serde_json::Value,
        viz: &str,
    ) -> Result<Self, ReportsError> {
        let view = ViewId::parse(kind).ok_or_else(|| ReportsError::InvalidSpec {
            field: "view".to_string(),
            reason: format!("unknown view slug {kind:?}"),
        })?;
        let hint = VizHint::parse(viz).ok_or_else(|| ReportsError::InvalidSpec {
            field: "viz".to_string(),
            reason: format!("unknown viz slug {viz:?}"),
        })?;
        let parsed: SpecBody =
            serde_json::from_value(body.clone()).map_err(|e| ReportsError::InvalidSpec {
                field: "spec".to_string(),
                reason: format!("body does not deserialize: {e}"),
            })?;
        let spec = ReportSpec {
            view,
            filters: parsed.filters,
            group_by: parsed.group_by,
            measures: parsed.measures,
            viz: hint,
        };
        spec.validate()?;
        Ok(spec)
    }

    /// Registry validation — the typed-registry gate every stored spec
    /// passes at write time AND load time. Pure; never touches SQL.
    ///
    /// # Errors
    /// [`ReportsError::InvalidSpec`] naming the offending spec member, or
    /// [`ReportsError::UnknownColumn`] when a column name resolves nowhere
    /// (the fail-closed identifier seam, shared with the prebuilt kinds).
    pub fn validate(&self) -> Result<(), ReportsError> {
        let invalid = |field: &str, reason: String| ReportsError::InvalidSpec {
            field: field.to_string(),
            reason,
        };

        // --- filters -------------------------------------------------------
        if self.filters.len() > MAX_FILTERS {
            return Err(invalid(
                "filters",
                format!(
                    "{} filters exceed the cap of {MAX_FILTERS}",
                    self.filters.len()
                ),
            ));
        }
        let mut seen: Vec<(&str, FilterOp)> = Vec::new();
        for f in &self.filters {
            let col = resolve_filter_column(self.view, &f.column)?;
            if f.op.takes_value() {
                let value = f.value.as_ref().ok_or_else(|| {
                    invalid(
                        "filters",
                        format!("operator {:?} on '{}' requires a value", f.op, f.column),
                    )
                })?;
                if !value.matches(col.value_type) {
                    return Err(invalid(
                        "filters",
                        format!(
                            "value type does not match column '{}' ({}-typed)",
                            f.column,
                            type_name(col.value_type)
                        ),
                    ));
                }
            } else if f.value.is_some() {
                return Err(invalid(
                    "filters",
                    format!("operator {:?} takes no value", f.op),
                ));
            }
            let key = (f.column.as_str(), f.op);
            if seen.contains(&key) {
                return Err(invalid(
                    "filters",
                    format!(
                        "duplicate filter on '{}' with operator {:?}",
                        f.column, f.op
                    ),
                ));
            }
            seen.push(key);
        }

        // --- group-by ------------------------------------------------------
        if self.group_by.len() > MAX_GROUP_BY {
            return Err(invalid(
                "group_by",
                format!(
                    "{} dimensions exceed the cap of {MAX_GROUP_BY}",
                    self.group_by.len()
                ),
            ));
        }
        for dim in &self.group_by {
            if dim == "org_id" {
                return Err(invalid(
                    "group_by",
                    "org_id is the tenancy seam, never a report dimension".to_string(),
                ));
            }
            lookup_column(self.view, dim).ok_or_else(|| ReportsError::UnknownColumn {
                view: self.view.qualified_name().to_string(),
                name: dim.clone(),
            })?;
        }
        let mut dims = self.group_by.clone();
        dims.sort();
        dims.dedup();
        if dims.len() != self.group_by.len() {
            return Err(invalid("group_by", "duplicate dimension".to_string()));
        }

        // --- measures ------------------------------------------------------
        if self.measures.is_empty() {
            return Err(invalid(
                "measures",
                "at least one measure is required".to_string(),
            ));
        }
        if self.measures.len() > MAX_MEASURES {
            return Err(invalid(
                "measures",
                format!(
                    "{} measures exceed the cap of {MAX_MEASURES}",
                    self.measures.len()
                ),
            ));
        }
        let mut names: Vec<String> = self.group_by.clone();
        for m in &self.measures {
            match &m.column {
                None if m.agg == Aggregate::Count => {}
                None => {
                    return Err(invalid(
                        "measures",
                        format!("aggregate {:?} requires a column", m.agg),
                    ));
                }
                Some(name) => {
                    if name == "org_id" {
                        return Err(invalid(
                            "measures",
                            "org_id is the tenancy seam, never a report column".to_string(),
                        ));
                    }
                    let col = lookup_column(self.view, name).ok_or_else(|| {
                        ReportsError::UnknownColumn {
                            view: self.view.qualified_name().to_string(),
                            name: name.clone(),
                        }
                    })?;
                    if !m.agg.accepts(col.value_type) {
                        return Err(invalid(
                            "measures",
                            format!(
                                "aggregate {:?} does not accept {}-typed column '{name}'",
                                m.agg,
                                type_name(col.value_type)
                            ),
                        ));
                    }
                }
            }
            let out = m.output_name();
            if names.contains(&out) {
                return Err(invalid(
                    "measures",
                    format!("duplicate output column '{out}'"),
                ));
            }
            names.push(out);
        }
        Ok(())
    }

    /// Compiles the spec into parameterized SQL over its view. Pure:
    /// identical inputs ⇒ identical output. The main query groups by the
    /// dimensions; [`CompiledReport::summary_sql`] mirrors the SAME
    /// filters (and RBAC narrowing) as whole-set totals.
    ///
    /// The RBAC seam matches the prebuilt kinds: `Scope::Self_(uid)` adds
    /// the view's personal-dimension equality predicate regardless of the
    /// spec's own filters, so self-scoping cannot be bypassed by spec
    /// content.
    ///
    /// # Errors
    /// [`ReportsError::InvalidSpec`] / [`ReportsError::UnknownColumn`]
    /// from validation; [`ReportsError::InvalidLimit`] for non-positive
    /// limits.
    pub fn compile(&self, scope: Scope, limit: i64) -> Result<CompiledReport, ReportsError> {
        self.validate()?;
        crate::params::validate_limit_raw(limit)?;

        let mut b = Binder::new();
        let conds = self.conditions(scope, &mut b);
        let where_main = if conds.is_empty() {
            String::new()
        } else {
            format!("\nWHERE {}", conds.join("\n  AND "))
        };

        let mut select: Vec<String> = Vec::new();
        for dim in &self.group_by {
            select.push(format!("{} AS \"{}\"", dim_expr(dim), dim));
        }
        for m in &self.measures {
            select.push(format!("{} AS \"{}\"", measure_expr(m), m.output_name()));
        }

        let mut sql = format!(
            "SELECT {}\nFROM {} r{}",
            select.join(",\n       "),
            self.view.qualified_name(),
            where_main,
        );
        if !self.group_by.is_empty() {
            let group: Vec<String> = self.group_by.iter().map(|d| dim_expr(d)).collect();
            sql.push_str(&format!("\nGROUP BY {}", group.join(", ")));
        }
        let mut order: Vec<String> = Vec::new();
        if let Some(first) = self.measures.first() {
            order.push(format!("\"{}\" DESC", first.output_name()));
        }
        for dim in &self.group_by {
            order.push(format!("{} ASC", dim_expr(dim)));
        }
        if !order.is_empty() {
            sql.push_str(&format!("\nORDER BY {}", order.join(", ")));
        }
        let lim = b.ph(Bind::Int(limit));
        sql.push_str(&format!("\nLIMIT {lim}"));

        // Summary: whole-set totals under the same conditions — one fresh
        // binder, identical predicate assembly (parity by construction).
        let mut sb = Binder::new();
        let s_conds = self.conditions(scope, &mut sb);
        let s_where = if s_conds.is_empty() {
            String::new()
        } else {
            format!("\nWHERE {}", s_conds.join("\n  AND "))
        };
        let mut s_select: Vec<String> = vec!["COUNT(*)::bigint AS rows_total".to_string()];
        for m in &self.measures {
            s_select.push(format!(
                "{} AS \"total_{}\"",
                measure_expr(m),
                m.output_name()
            ));
        }
        let summary_sql = format!(
            "SELECT {}\nFROM {} r{}",
            s_select.join(",\n       "),
            self.view.qualified_name(),
            s_where,
        );

        let mut columns: Vec<OutputColumn> = Vec::new();
        for dim in &self.group_by {
            let ty = lookup_column(self.view, dim)
                .map(|c| out_type(c.value_type))
                .unwrap_or(OutType::Text);
            columns.push(OutputColumn {
                name: dim.clone(),
                ty,
            });
        }
        for m in &self.measures {
            let ty = match (&m.agg, &m.column) {
                (Aggregate::Count, _) | (Aggregate::Sum, _) => OutType::BigInt,
                (Aggregate::Avg, _) => OutType::Float,
                (Aggregate::Min | Aggregate::Max, Some(name)) => out_type(
                    lookup_column(self.view, name)
                        .map(|c| c.value_type)
                        .unwrap_or(ValueType::Text),
                ),
                (Aggregate::Min | Aggregate::Max, None) => OutType::BigInt,
            };
            columns.push(OutputColumn {
                name: m.output_name(),
                ty,
            });
        }

        Ok(CompiledReport {
            sql,
            binds: b.binds,
            summary_sql,
            summary_binds: sb.binds,
            columns,
            cursor_shape: CursorShape::None,
            view: self.view,
        })
    }

    /// One predicate assembly shared by main query and summary. Bind order
    /// is minted in push order per binder, so each statement's
    /// placeholders stay contiguous and drift-free.
    fn conditions(&self, scope: Scope, b: &mut Binder) -> Vec<String> {
        let mut conds: Vec<String> = Vec::new();

        // RBAC narrowing FIRST — mirrors the prebuilt-kind precedence.
        if let Scope::Self_(uid) = scope {
            if let Some(dim) = self.view.personal_dim() {
                let n = b.ph(Bind::Uuid(uid));
                conds.push(format!("{} = {n}", dim_expr(dim)));
            }
        }

        for f in &self.filters {
            let col_expr = dim_expr(&f.column);
            if !f.op.takes_value() {
                conds.push(format!("{col_expr} IS {}", null_word(f.op)));
                continue;
            }
            // Validation guarantees presence and typing; the expect is
            // inert behind the same-validation gate above.
            let value = f.value.clone().expect("validated filter value");
            let n = b.ph(value.into_bind());
            conds.push(format!("{col_expr} {} {n}", f.op.fragment()));
        }
        conds
    }
}

// --- SQL fragment helpers (constants + catalog names only) ------------------

/// Alias-qualified column reference. `r` is the fixed row alias of every
/// spec-compiled statement; `name` originates from the validated spec and
/// resolved through the catalog before it ever reaches here.
fn dim_expr(name: &str) -> String {
    format!("r.{name}")
}

fn null_word(op: FilterOp) -> &'static str {
    match op {
        FilterOp::IsNull => "NULL",
        FilterOp::IsNotNull => "NOT NULL",
        _ => unreachable!("null_word called on a value operator"),
    }
}

/// Aggregate SQL fragment over a validated (existing, type-compatible)
/// column. Aliased by the caller.
fn measure_expr(m: &MeasureSpec) -> String {
    match (&m.agg, &m.column) {
        (Aggregate::Count, None) => "COUNT(*)::bigint".to_string(),
        (Aggregate::Count, Some(c)) => format!("COUNT({})::bigint", dim_expr(c)),
        (Aggregate::Sum, Some(c)) => format!("COALESCE(SUM({}), 0)::bigint", dim_expr(c)),
        (Aggregate::Avg, Some(c)) => format!("ROUND(AVG({})::numeric, 2)::float8", dim_expr(c)),
        (Aggregate::Min, Some(c)) => format!("MIN({})", dim_expr(c)),
        (Aggregate::Max, Some(c)) => format!("MAX({})", dim_expr(c)),
        (agg, None) => unreachable!("validated: {agg:?} requires a column"),
    }
}

fn out_type(ty: ValueType) -> OutType {
    match ty {
        ValueType::Uuid => OutType::Uuid,
        ValueType::Text => OutType::Text,
        ValueType::Timestamp => OutType::Timestamp,
        ValueType::Date => OutType::Date,
        ValueType::Int => OutType::Int,
        ValueType::BigInt => OutType::BigInt,
        ValueType::Bool => OutType::Bool,
    }
}

fn type_name(ty: ValueType) -> &'static str {
    match ty {
        ValueType::Uuid => "uuid",
        ValueType::Text => "text",
        ValueType::Timestamp => "timestamp",
        ValueType::Date => "date",
        ValueType::Int => "int",
        ValueType::BigInt => "bigint",
        ValueType::Bool => "bool",
    }
}

// --- bind bookkeeping (mirrors builder::Binder; placeholder numbers are
// minted strictly in push order) ---------------------------------------------

struct Binder {
    binds: Vec<Bind>,
}

impl Binder {
    fn new() -> Self {
        Self { binds: Vec::new() }
    }

    fn ph(&mut self, b: Bind) -> String {
        self.binds.push(b);
        format!("${}", self.binds.len())
    }
}

// --- structural safety tests ------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ReportsError;
    use serde_json::json;

    const PAYLOADS: &[&str] = &[
        "' OR '1'='1",
        "1; DROP TABLE tickets; --",
        "user_id IS NULL OR 1=1 --",
        "started_at > now() - interval '10 years'",
        "te.user_id = gen_random_uuid() UNION SELECT 1",
        "\"group_id\"",
        "$$ dollar-quoted $$",
        "${jndi}",
        "/* stealth */ minutes",
        "org_id",
    ];

    fn spec(view: ViewId) -> ReportSpec {
        ReportSpec {
            view,
            filters: vec![],
            group_by: vec![],
            measures: vec![MeasureSpec {
                agg: Aggregate::Count,
                column: None,
            }],
            viz: VizHint::Bar,
        }
    }

    fn filter(column: &str, op: FilterOp, value: Option<FilterValue>) -> SpecFilter {
        SpecFilter {
            column: column.to_string(),
            op,
            value,
        }
    }

    #[test]
    fn every_view_compiles_a_grand_total_spec_under_both_scopes() {
        for view in ViewId::ALL {
            for scope in [Scope::OrgWide, Scope::Self_(Uuid::nil())] {
                let compiled = spec(view)
                    .compile(scope, 100)
                    .unwrap_or_else(|e| panic!("{view:?}/{scope:?}: {e}"));
                assert_eq!(compiled.view, view);
                assert_eq!(compiled.cursor_shape, CursorShape::None);
                assert_statement_shape(&compiled.sql);
                assert_statement_shape(&compiled.summary_sql);
                assert_placeholders_contiguous(&compiled.sql, compiled.binds.len());
                assert_placeholders_contiguous(&compiled.summary_sql, compiled.summary_binds.len());
            }
        }
    }

    #[test]
    fn self_scope_adds_the_personal_dimension_predicate() {
        for view in ViewId::ALL {
            let dim = view.personal_dim().unwrap();
            let compiled = spec(view).compile(Scope::Self_(Uuid::nil()), 10).unwrap();
            assert!(
                compiled.sql.contains(&format!("r.{dim} = $1")),
                "{view:?}: personal narrowing missing: {}",
                compiled.sql
            );
            // Summary mirrors the same predicate.
            assert!(compiled.summary_sql.contains(&format!("r.{dim} = $1")));
            // Binds: the scope uuid + the LIMIT.
            assert_eq!(compiled.binds.len(), 2);
        }
    }

    #[test]
    fn filters_narrow_and_bind_in_push_order() {
        let mut s = spec(ViewId::TimeEntries);
        s.filters = vec![
            filter("billable", FilterOp::Eq, Some(FilterValue::Bool(true))),
            filter(
                "day_utc",
                FilterOp::GtEq,
                Some(FilterValue::Date(
                    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                )),
            ),
        ];
        s.group_by = vec!["user_id".to_string()];
        s.measures = vec![MeasureSpec {
            agg: Aggregate::Sum,
            column: Some("minutes".to_string()),
        }];
        let c = s.compile(Scope::OrgWide, 50).unwrap();
        assert!(c.sql.contains("r.billable = $1"));
        assert!(c.sql.contains("r.day_utc >= $2"));
        assert!(c.sql.contains("r.user_id"));
        assert!(c.sql.contains("GROUP BY r.user_id"));
        assert!(c.sql.contains("\"sum_minutes\" DESC"));
        assert!(c.sql.contains("LIMIT $3"));
        // Summary carries the same two filters (no group-by, no limit).
        assert!(c.summary_sql.contains("r.billable = $1"));
        assert!(c.summary_sql.contains("r.day_utc >= $2"));
        assert!(c.summary_sql.contains("total_sum_minutes"));
        assert_statement_shape(&c.sql);
        assert_statement_shape(&c.summary_sql);
        assert_placeholders_contiguous(&c.sql, c.binds.len());
        assert_placeholders_contiguous(&c.summary_sql, c.summary_binds.len());
    }

    #[test]
    fn null_operators_bind_no_value() {
        let mut s = spec(ViewId::Tickets);
        s.filters = vec![filter("client_id", FilterOp::IsNull, None)];
        let c = s.compile(Scope::OrgWide, 10).unwrap();
        assert!(c.sql.contains("r.client_id IS NULL"));
        // Only the LIMIT bind remains.
        assert_eq!(c.binds.len(), 1);
    }

    #[test]
    fn injection_payloads_fail_closed_on_every_surface() {
        for payload in PAYLOADS {
            for view in ViewId::ALL {
                // Filter columns go through the allowlist seam.
                let mut s = spec(view);
                s.filters = vec![filter(
                    payload,
                    FilterOp::Eq,
                    Some(FilterValue::Text("x".into())),
                )];
                assert!(
                    s.validate().is_err(),
                    "{payload:?} accepted as a filter on {view:?}"
                );
                // Group-by goes through the projection seam.
                let mut g = spec(view);
                g.group_by = vec![payload.to_string()];
                g.measures = vec![MeasureSpec {
                    agg: Aggregate::Count,
                    column: None,
                }];
                assert!(g.validate().is_err(), "{payload:?} accepted as a dimension");
                // Measure columns go through the projection + type seams.
                let mut m = spec(view);
                m.measures = vec![MeasureSpec {
                    agg: Aggregate::Count,
                    column: Some(payload.to_string()),
                }];
                assert!(m.validate().is_err(), "{payload:?} accepted as a measure");
            }
            // Stored-part reassembly rejects hostile kind/viz slugs.
            let body = serde_json::json!({"measures": [{"agg": "count"}]});
            assert!(ReportSpec::from_parts(payload, &body, "bar").is_err());
            assert!(ReportSpec::from_parts("tickets", &body, payload).is_err());
        }
    }

    #[test]
    fn registry_type_mismatches_are_rejected() {
        // Bool value on a uuid column.
        let mut s = spec(ViewId::Tickets);
        s.filters = vec![filter(
            "client_id",
            FilterOp::Eq,
            Some(FilterValue::Bool(true)),
        )];
        assert!(matches!(
            s.validate(),
            Err(ReportsError::InvalidSpec { field, .. }) if field == "filters"
        ));

        // Sum over a text column.
        let mut m = spec(ViewId::Tickets);
        m.measures = vec![MeasureSpec {
            agg: Aggregate::Sum,
            column: Some("priority".to_string()),
        }];
        assert!(matches!(
            m.validate(),
            Err(ReportsError::InvalidSpec { field, .. }) if field == "measures"
        ));

        // Sum over a timestamp column (numeric-only rule).
        let mut t = spec(ViewId::Tickets);
        t.measures = vec![MeasureSpec {
            agg: Aggregate::Avg,
            column: Some("created_at".to_string()),
        }];
        assert!(t.validate().is_err());

        // Min/Max over dates IS allowed.
        let mut d = spec(ViewId::Tickets);
        d.measures = vec![MeasureSpec {
            agg: Aggregate::Min,
            column: Some("due_at".to_string()),
        }];
        assert!(d.validate().is_ok());

        // Non-count aggregate without a column.
        let mut e = spec(ViewId::Tickets);
        e.measures = vec![MeasureSpec {
            agg: Aggregate::Sum,
            column: None,
        }];
        assert!(e.validate().is_err());

        // Empty measures.
        let mut z = spec(ViewId::Tickets);
        z.measures = vec![];
        assert!(z.validate().is_err());
    }

    #[test]
    fn shape_caps_and_duplicates_are_enforced() {
        // Filter cap.
        let mut f = spec(ViewId::Tickets);
        for i in 0..=MAX_FILTERS {
            f.filters.push(filter(
                "priority",
                if i == 0 {
                    FilterOp::Eq
                } else {
                    FilterOp::NotEq
                },
                Some(FilterValue::Text(format!("p{i}"))),
            ));
        }
        assert!(f.validate().is_err());

        // Duplicate (column, op).
        let mut d = spec(ViewId::Tickets);
        d.filters = vec![
            filter(
                "priority",
                FilterOp::Eq,
                Some(FilterValue::Text("p1".into())),
            ),
            filter(
                "priority",
                FilterOp::Eq,
                Some(FilterValue::Text("p2".into())),
            ),
        ];
        assert!(d.validate().is_err());

        // Group-by cap + duplicates + org_id ban.
        let mut g = spec(ViewId::Tickets);
        g.group_by = vec![
            "priority".into(),
            "category".into(),
            "client_id".into(),
            "board_id".into(),
        ];
        assert!(g.validate().is_err());
        g.group_by = vec!["priority".into(), "priority".into()];
        assert!(g.validate().is_err());
        g.group_by = vec!["org_id".into()];
        assert!(g.validate().is_err());

        // Measure cap + duplicate output names.
        let mut m = spec(ViewId::Tickets);
        for _ in 0..=MAX_MEASURES {
            m.measures.push(MeasureSpec {
                agg: Aggregate::Count,
                column: None,
            });
        }
        assert!(m.validate().is_err());
        m.measures = vec![
            MeasureSpec {
                agg: Aggregate::Count,
                column: None,
            },
            MeasureSpec {
                agg: Aggregate::Count,
                column: None,
            },
        ];
        assert!(
            m.validate().is_err(),
            "duplicate COUNT(*) collides on 'count'"
        );
    }

    #[test]
    fn parts_round_trip_through_jsonb_shaped_documents() {
        let mut s = spec(ViewId::SlaOutcomes);
        s.group_by = vec!["policy_id".to_string()];
        s.measures = vec![
            MeasureSpec {
                agg: Aggregate::Count,
                column: None,
            },
            MeasureSpec {
                agg: Aggregate::Avg,
                column: Some("paused_business_secs".to_string()),
            },
        ];
        s.viz = VizHint::Pie;

        let (kind, body, viz) = s.to_parts();
        // jsonb normalizes key order; simulate by re-serializing.
        let doc = serde_json::to_value(&body).unwrap();
        let rebuilt = ReportSpec::from_parts(kind, &doc, viz).unwrap();
        assert_eq!(rebuilt, s);

        // Malformed bodies and unknown slugs fail closed.
        assert!(ReportSpec::from_parts("nope", &doc, viz).is_err());
        assert!(ReportSpec::from_parts(kind, &doc, "hologram").is_err());
        assert!(
            ReportSpec::from_parts(kind, &serde_json::json!({"measures": []}), viz).is_err(),
            "an empty-measures body must not revalidate"
        );
    }

    #[test]
    fn viz_hints_round_trip_strictly() {
        for viz in VizHint::ALL {
            assert_eq!(VizHint::parse(viz.slug()), Some(viz));
            let json = serde_json::to_string(&viz).unwrap();
            assert_eq!(json, format!("\"{}\"", viz.slug()));
        }
        for bad in ["", "BAR", "scatter", "' OR 1=1 --"] {
            assert_eq!(VizHint::parse(bad), None);
        }
    }

    #[test]
    fn filter_values_use_lowercase_slugs() {
        let cases: Vec<(FilterValue, serde_json::Value)> = vec![
            (FilterValue::Text("p1".into()), json!({"text": "p1"})),
            (FilterValue::Bool(true), json!({"bool": true})),
            (
                FilterValue::Uuid(Uuid::nil()),
                json!({"uuid": Uuid::nil().to_string()}),
            ),
            (FilterValue::Int(42), json!({"int": 42})),
            (
                FilterValue::Date(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap()),
                json!({"date": "2026-01-02"}),
            ),
            (
                FilterValue::Timestamp(DateTime::<Utc>::UNIX_EPOCH),
                json!({"timestamp": "1970-01-01T00:00:00Z"}),
            ),
        ];
        for (value, encoded) in cases {
            assert_eq!(serde_json::to_value(&value).unwrap(), encoded);
            assert_eq!(
                serde_json::from_value::<FilterValue>(encoded).unwrap(),
                value
            );
        }
        // Unknown variants fail closed.
        assert!(serde_json::from_value::<FilterValue>(json!({"txt": "x"})).is_err());
        assert!(serde_json::from_value::<FilterValue>(json!("p1")).is_err());
    }

    #[test]
    fn aggregate_output_names_are_stable() {
        let m = |agg: Aggregate, column: Option<&str>| MeasureSpec {
            agg,
            column: column.map(str::to_string),
        };
        assert_eq!(m(Aggregate::Count, None).output_name(), "count");
        assert_eq!(
            m(Aggregate::Sum, Some("minutes")).output_name(),
            "sum_minutes"
        );
        assert_eq!(
            m(Aggregate::Avg, Some("minutes")).output_name(),
            "avg_minutes"
        );
        assert_eq!(m(Aggregate::Min, Some("day")).output_name(), "min_day");
    }

    /// Statement-shape guard, mirroring builder.rs: only SELECT/WITH entry
    /// points, no data-mutating keywords, no terminators/comments, and
    /// only `ops.v1_` relation references.
    #[track_caller]
    fn assert_statement_shape(sql: &str) {
        let lower = sql.to_ascii_lowercase();
        assert!(
            lower.starts_with("select") || lower.starts_with("with"),
            "spec reports must be reads only; got {sql}"
        );
        for banned in [
            "insert ",
            "update ",
            "delete ",
            "drop ",
            "alter ",
            "truncate ",
            "grant ",
            "revoke ",
            ";",
            "--",
            "/*",
        ] {
            assert!(
                !lower.contains(banned),
                "forbidden token '{banned}' in: {sql}"
            );
        }
        let mut rest = lower.as_str();
        while let Some(pos) = rest.find("ops.") {
            let tail = &rest[pos..pos + 10.min(rest.len() - pos)];
            assert!(
                tail.starts_with("ops.v1_"),
                "non-versioned or non-ops reference near '{tail}'"
            );
            rest = &rest[pos + "ops.".len()..];
        }
    }

    /// Placeholder continuity: first appearances run exactly 1..=binds.len().
    #[track_caller]
    fn assert_placeholders_contiguous(sql: &str, n_binds: usize) {
        let mut nums: Vec<usize> = Vec::new();
        let bytes = sql.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end].is_ascii_digit() {
                    end += 1;
                }
                let num: usize = sql[start..end].parse().expect("placeholder number");
                if !nums.contains(&num) {
                    nums.push(num);
                }
                i = end;
            } else {
                i += 1;
            }
        }
        let expected: Vec<usize> = (1..=n_binds).collect();
        assert_eq!(nums.len(), n_binds, "placeholder count drift in: {sql}");
        assert_eq!(nums, expected, "placeholder sequence drift in: {sql}");
    }

    proptest::proptest! {
        /// Property: ANY spec assembled from registry-resolvable members
        /// compiles to a structurally safe statement under both scopes —
        /// the compiler cannot be driven off the allowlist by input shape.
        #[test]
        fn registry_specs_always_compile_safely(
            view_idx in 0usize..5,
            n_filters in 0usize..=3,
            n_dims in 0usize..=3,
            n_measures in 1usize..=3,
            self_scoped in proptest::bool::ANY,
        ) {
            let view = ViewId::ALL[view_idx];
            let mut s = spec(view);

            // Filters from the view's filterable inventory, uniquely.
            let filterable = view.filterable();
            for i in 0..n_filters.min(filterable.len()) {
                let name = filterable[i * 2 % filterable.len()];
                let def = lookup_column(view, name).unwrap();
                let value = match def.value_type {
                    ValueType::Uuid => FilterValue::Uuid(Uuid::nil()),
                    ValueType::Text => FilterValue::Text("x".into()),
                    ValueType::Timestamp => FilterValue::Timestamp(DateTime::<Utc>::UNIX_EPOCH),
                    ValueType::Date => {
                        FilterValue::Date(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap())
                    }
                    ValueType::Int | ValueType::BigInt => FilterValue::Int(1),
                    ValueType::Bool => FilterValue::Bool(true),
                };
                let op = match def.value_type {
                    ValueType::Bool => FilterOp::Eq,
                    _ if i % 2 == 0 => FilterOp::Eq,
                    _ => FilterOp::NotEq,
                };
                let f = filter(name, op, Some(value));
                if !s.filters.contains(&f) {
                    s.filters.push(f);
                }
            }

            // Dimensions from the full projection (org_id excluded).
            let dims: Vec<String> = view
                .columns()
                .iter()
                .filter(|c| c.name != "org_id")
                .map(|c| c.name.to_string())
                .collect();
            for i in 0..n_dims {
                s.group_by.push(dims[i % dims.len()].clone());
            }
            s.group_by.sort();
            s.group_by.dedup();

            // Measures: count plus at most one numeric aggregate.
            s.measures.clear();
            for i in 0..n_measures {
                let m = match i {
                    0 => MeasureSpec { agg: Aggregate::Count, column: None },
                    _ => {
                        let numeric = view
                            .columns()
                            .iter()
                            .find(|c| matches!(c.value_type, ValueType::Int | ValueType::BigInt));
                        match numeric {
                            Some(c) if i == 1 => MeasureSpec {
                                agg: Aggregate::Sum,
                                column: Some(c.name.to_string()),
                            },
                            Some(c) => MeasureSpec {
                                agg: Aggregate::Min,
                                column: Some(c.name.to_string()),
                            },
                            None => MeasureSpec { agg: Aggregate::Count, column: None },
                        }
                    }
                };
                if !s.measures.contains(&m) {
                    s.measures.push(m);
                }
            }

            let scope = if self_scoped {
                Scope::Self_(Uuid::nil())
            } else {
                Scope::OrgWide
            };
            let compiled = match s.compile(scope, 100) {
                Ok(c) => c,
                // Only legitimate rejection: duplicate output names when
                // the strategy collapses to identical measures.
                Err(ReportsError::InvalidSpec { .. }) => return Ok(()),
                Err(e) => panic!("unexpected rejection: {e}"),
            };
            assert_statement_shape(&compiled.sql);
            assert_statement_shape(&compiled.summary_sql);
            assert_placeholders_contiguous(&compiled.sql, compiled.binds.len());
            assert_placeholders_contiguous(&compiled.summary_sql, compiled.summary_binds.len());
            assert_eq!(compiled.columns.len(), s.group_by.len() + s.measures.len());
        }
    }
}
