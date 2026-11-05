//! "Describe it" → a report DRAFT (the private build tracker Wave 46 lane 3,
//! `p46-authoring-draft`).
//!
//! One natural-language entry point that emits the SAME `ReportSpec` +
//! `CardSpec` the builder and the card contract already speak, validated by the
//! SAME functions the save and run routes call, so a described report is never
//! a privileged path around the registry.
//!
//! # What this is, and what it is not
//!
//! - **Same boundary, not a second parser.** The draft is deserialised into
//!   `core_reports::ReportSpec` and `core_reports::CardSpec`, then run through
//!   `ReportSpec::validate`, `CardSpec::validate` and the SAME
//!   `ReportSpec::compile_windowed` the run routes execute. There is no
//!   report-shaped validator in this module and none on the client: a draft
//!   that does not compile here would not compile at run time either.
//! - **A draft, never a write.** This endpoint persists NOTHING. It returns the
//!   proposed spec + card, a plain-English read-back, the typed issues, and a
//!   structural diff against the caller's current draft. Saving is the caller's
//!   job through `POST /api/report-definitions` (or its PUT), which
//!   re-validates. The response says `saved: false` because that is always true.
//! - **No draft row, no migration.** Wave 46 gives this row no migration, so
//!   the "draft row" the close-out UI shows is this response object plus client
//!   state with a status. Nothing about a draft survives the request.
//! - **A transcript, never audio.** The caller supplies text. Voice-note
//!   capture is its own lane; this module never sees a media type, a duration
//!   or an upload.
//! - **Catalogue, not schemas (prompt shape).** The system prompt carries the
//!   compact catalogue — each view's slug and what it holds, the card shape
//!   prose, the card rules, the span/height vocabulary — and NO column lists.
//!   The model fetches the columns it actually needs through the
//!   [`VIEW_COLUMNS_TOOL`] tool. Same posture as Wave 43 lane 17's
//!   `get_node_schema`: the prompt stays roughly constant as the catalogue
//!   grows instead of growing with it.
//! - **The existing admission path.** The model call rides
//!   `ai_policy::authorize_surface_call` with the completion + tool-calling
//!   edges and `chat_runner::resolve_transport` — the same caps, grants,
//!   allowlist and sealed credential the chat runner uses. There is no second
//!   AI path and no report-specific surface row; `surface_key` is
//!   caller-supplied and the org grants it.
//!
//! # Reused from Wave 43 lane 17
//!
//! The loop, the tool-call wire round-trip, the lenient JSON extraction and the
//! usage/provenance ledger are `workflow_authoring`'s. This module supplies the
//! report catalogue, its own one tool, its own issue vocabulary and a
//! spec-shaped diff. Reuse over rebuild: two prose paths that parse JSON
//! differently would be two bugs.

use axum::{extract::State, http::Extensions, routing::post, Json, Router};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::routes::ai_policy::{authorize_surface_call, org_bound_tx, AiAdmissionAsk, AiEdge};
use crate::routes::chat_runner::{
    self, ChatModelTransport, LoopMessage, ModelCallRequest, ToolCallReq,
};
use crate::routes::report_defs::CatalogColumn;
use crate::routes::workflow_authoring::{
    extract_json_object, tool_calls_wire, AuthorModelInfo, AuthorUsage, AuthoringToolCall,
    MAX_AUTHORING_STEPS,
};
use crate::routes::ApiGeneration;
use crate::state::AppState;
use core_db::ai_policy::{record_usage, AiUsageNew};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::policy::{Action, Module, Permission};
use core_reports::catalog::{FilterOp, ValueType};
use core_reports::{
    CardShape, CardSpan, CardSpec, MeasureSpec, ReportSpec, ReportsError, RunWindow, SpecFilter,
    ViewId,
};

/// The name of the on-demand column tool. Stable: it is the tool id the model
/// calls back on, and the only tool this loop answers.
pub const VIEW_COLUMNS_TOOL: &str = "get_view_columns";

/// The longest transcript one authoring request accepts. A description of a
/// report is a paragraph or two; a pasted document is a caller confusing this
/// surface with a chat, and it would spend the org's tokens on a prompt the
/// catalogue already bounds. Refused typed rather than truncated, because a
/// silently shortened transcript would author something nobody asked for.
pub const MAX_TRANSCRIPT_CHARS: usize = 8_000;

// ---------------------------------------------------------------------------
// The compact catalogue (prompt: views, shapes, rules — never columns)
// ---------------------------------------------------------------------------

/// One line of prose per view: what it holds, in the words an author reads.
///
/// This is prompt prose, not a contract. The contract is the column registry in
/// `core_reports::catalog`, which the compiler enforces and the
/// [`VIEW_COLUMNS_TOOL`] serves. `core-reports` carries no view descriptions
/// (that crate is the card contract's, not this lane's), so the blurbs live
/// here, and `every_view_has_a_blurb` fails if a view is added without one.
pub fn view_blurb(view: ViewId) -> &'static str {
    match view {
        ViewId::Tickets => {
            "one row per ticket, with its board, client, site, priority, \
                            category, assignee, status and the timestamps a service desk \
                            reports on"
        }
        ViewId::TimeEntries => {
            "one row per time entry, with the user, ticket, billable flag \
                               and minutes — the view behind utilisation and effort reports"
        }
        ViewId::SlaOutcomes => {
            "one row per SLA outcome, with the policy that applied and \
                               whether the target was met — the view behind breach counts and \
                               compliance rates"
        }
        ViewId::GapDays => {
            "one row per user per day that has a coverage gap, so a report over \
                            it counts where cover was missing rather than how much work happened"
        }
        ViewId::FirstResponse => {
            "one row per first response, with the ticket, the assignee and \
                                  the minutes it took — the view behind response-time medians and \
                                  slowest-first lists"
        }
    }
}

/// The human word for a view slug. Derived, never a second name table: a new
/// view reads correctly the day it ships, as `gap_days` already does.
fn view_words(view: ViewId) -> String {
    view.slug().replace('_', " ")
}

/// One catalogue shape as the prompt sees it. Mirrors
/// `report_defs::CatalogShape` deliberately: the prose here is *served* from the
/// same `core_reports::shape_catalogue` the catalogue endpoint uses, so the two
/// cannot disagree about what a chart needs.
fn shape_lines() -> Vec<String> {
    core_reports::shape_catalogue()
        .iter()
        .map(|info| {
            format!(
                "- {} — produced by viz {}; needs {}; default {} wide, {} tall. {}\n  refused when: {}",
                info.shape.slug(),
                info.viz
                    .iter()
                    .map(|v| v.slug())
                    .collect::<Vec<_>>()
                    .join(", "),
                info.data
                    .iter()
                    .map(|d| d.slug())
                    .collect::<Vec<_>>()
                    .join(", "),
                info.default_size.span.slug(),
                info.default_size.height.slug(),
                info.agent_description,
                info.refuses_when
            )
        })
        .collect()
}

/// The OpenAI-format tool definition for the on-demand column fetch. Mirrors
/// `workflow_authoring::node_schema_tool`'s shape: `{name, description,
/// parameters}`; the transport wraps it as `{"type":"function","function":…}`.
pub fn view_columns_tool() -> Value {
    json!({
        "name": VIEW_COLUMNS_TOOL,
        "description": "Return the exact projected and filterable columns of one report view, \
                        by its slug. Call this before writing a spec against a view you have \
                        not used — the column lists are deliberately NOT in this prompt, so \
                        fetch only the views you need.",
        "parameters": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "view": {
                    "type": "string",
                    "description": "A view slug from the catalogue, e.g. tickets or time_entries."
                }
            },
            "required": ["view"]
        }
    })
}

/// The system prompt: the authoring contract plus the compact catalogue. No
/// column lists — their absence is the point.
pub fn report_authoring_system_prompt() -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You author Marula reports from a caller's transcript.\n\
         \n\
         Emit ONLY a JSON object, no prose and no code fences, shaped:\n\
         {\"name\": <string>,\n\
          \"spec\": {\"view\": <view slug>, \"filters\": [{\"column\": <string>, \
         \"op\": <operator>, \"value\": <typed operand, omitted when the operator takes none>}], \
         \"group_by\": [<column>], \"measures\": [{\"agg\": <aggregate>, \"column\": <string, \
         omitted for count(*)>}], \"viz\": <viz slug>},\n\
          \"card\": {\"size\": {\"span\": <span>, \"height\": <height>}, \
         \"window_days\": <integer>, \"refresh\": {\"every_minutes\": <integer>, \
         \"reach_days\": <integer>}}}\n\
         \n\
         Rules the validator enforces (a draft that breaks one comes back rejected):\n\
         - `spec.view` must be one of the view slugs in the catalogue below.\n\
         - At most 8 filters. Each names a column from that view's FILTERABLE list \
           (call get_view_columns to see it). Operators: eq, not_eq, lt, lt_eq, gt, gt_eq \
           take a value; is_null and is_not_null take none. The value's type must match the \
           column: a uuid filter takes a uuid string, a text column a string, a date column \
           YYYY-MM-DD, a timestamp an RFC 3339 instant, an int column an integer, a bool \
           true or false. One filter per (column, operator).\n\
         - At most 3 group-by dimensions, any COLUMN of the view except org_id, which is \
           the tenancy seam and never a report dimension. No dimension twice. An empty \
           group_by means one grand-total row.\n\
         - Between 1 and 4 measures. `count` may omit its column; sum, avg, min and max \
           require one, and only accept the types the catalogue allows (sum and avg need a \
           number; min and max need a number or a date/timestamp). No two measures may \
           produce the same output column.\n\
         - `viz` is one of: bar, line, pie, stat, table. The card SHAPE is derived from the \
           spec, never sent: a chart needs a group-by dimension, a stat needs exactly one \
           measure and no dimension, a table is what is left.\n\
         - A `stat` is refused if it carries a second measure or any dimension.\n\
         - `card.window_days` is 1 to 366. `card.refresh.every_minutes` is 5 to 43200. The \
           window may not exceed the refresh reach, where `reach_days: 0` means the whole \
           window. The refresh may not be slower than the window it displays.\n\
         - The card must draw something. A chart or table whose filters resolve to zero rows \
           is refused, so prefer a filter the data can satisfy or a stat whose number is a \
           legitimate zero.\n\
         \n\
         When you are unsure which columns a view has, call get_view_columns(view) first; you \
         may call it for several views before answering.\n\
         \n",
    );

    prompt.push_str("VIEW CATALOGUE (slug — what it holds):\n");
    for view in ViewId::ALL {
        prompt.push_str(&format!("- {} — {}\n", view.slug(), view_blurb(view)));
    }

    prompt.push_str("\nCARD SHAPES (shape — viz — what the reader gets):\n");
    for line in shape_lines() {
        prompt.push_str(&line);
        prompt.push('\n');
    }

    prompt.push_str("\nCARD RULES:\n");
    for rule in core_reports::rules() {
        prompt.push_str(&format!("- {rule}\n"));
    }

    prompt.push_str(&format!(
        "\nSIZES: spans {}; heights {}.\n",
        CardSpan::ALL
            .iter()
            .map(|s| s.slug())
            .collect::<Vec<_>>()
            .join(", "),
        core_reports::CardHeight::ALL
            .iter()
            .map(|h| h.slug())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    prompt
}

/// The value class slug for a column type (`uuid | text | timestamp | date |
/// int | bool`), the same slug the builder and the catalogue endpoint publish.
fn value_type_slug(ty: ValueType) -> &'static str {
    match ty {
        ValueType::Uuid => "uuid",
        ValueType::Text => "text",
        ValueType::Timestamp => "timestamp",
        ValueType::Date => "date",
        ValueType::Int | ValueType::BigInt => "int",
        ValueType::Bool => "bool",
    }
}

/// Runs the on-demand column tool for one call, returning the tool-result string
/// the loop feeds back to the model. Pure and synchronous: the catalog is a
/// static Rust declaration, so there is no I/O and no second HTTP hop.
pub fn run_view_columns_tool(args: &Value) -> String {
    let slug = args
        .get("view")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if slug.is_empty() {
        return json!({ "found": false, "error": "view is required" }).to_string();
    }
    let Some(view) = ViewId::parse(slug) else {
        return json!({
            "found": false,
            "error": format!("unknown view {slug:?}; use a slug from the catalogue")
        })
        .to_string();
    };
    let column = |name: &str| {
        core_reports::lookup_column(view, name).map(|def| CatalogColumn {
            name: def.name.to_string(),
            value_type: value_type_slug(def.value_type).to_string(),
        })
    };
    let columns: Vec<CatalogColumn> = view
        .columns()
        .iter()
        .map(|c| CatalogColumn {
            name: c.name.to_string(),
            value_type: value_type_slug(c.value_type).to_string(),
        })
        .collect();
    let filterable: Vec<CatalogColumn> = view
        .filterable()
        .iter()
        .filter_map(|name| column(name))
        .collect();
    let anchor = view
        .time_anchor()
        .map(|(name, ty)| json!({ "column": name, "value_type": value_type_slug(ty) }));
    json!({
        "found": true,
        "view": view.slug(),
        "columns": columns,
        "filterable": filterable,
        "time_anchor": anchor,
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Typed issues
// ---------------------------------------------------------------------------

/// One author-time refusal, pointed and stable so a UI can render it. The field
/// names match `workflow_authoring::AuthoringIssue` so one review component can
/// draw either surface's issues; the vocabulary is this module's because a
/// report fails in report-shaped ways (`unknown_column`, `window_wider_…`).
#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct ReportAuthoringIssue {
    /// A stable machine code (`unknown_column`, `invalid_card`, …).
    pub code: String,
    /// The offending spec or card member, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub message: String,
}

impl ReportAuthoringIssue {
    fn new(code: &str, path: Option<String>, message: String) -> Self {
        Self {
            code: code.to_string(),
            path,
            message,
        }
    }
}

/// Maps a registry error to a typed issue. Every [`ReportsError`] variant gets a
/// stable code, so a caller branches instead of parsing prose.
pub fn issue_from_reports_error(error: &ReportsError) -> ReportAuthoringIssue {
    let (code, path): (&str, Option<String>) = match error {
        ReportsError::UnknownColumn { name, .. } => ("unknown_column", Some(name.clone())),
        ReportsError::InvalidSpec { field, .. } => ("invalid_spec", Some(field.clone())),
        ReportsError::InvalidCard { field, .. } => ("invalid_card", Some(field.clone())),
        ReportsError::WindowWiderThanRefresh { .. } => ("window_wider_than_refresh", None),
        ReportsError::RefreshSlowerThanWindow { .. } => ("refresh_slower_than_window", None),
        ReportsError::EmptyCard { shape, failure } => (
            "empty_card",
            Some(format!("{}/{}", shape.slug(), failure.slug())),
        ),
        ReportsError::InvalidWindow(_) => ("invalid_window", None),
        ReportsError::InvalidLimit(_) => ("invalid_limit", None),
        ReportsError::InvalidCursor => ("invalid_cursor", None),
    };
    ReportAuthoringIssue::new(code, path, error.to_string())
}

// ---------------------------------------------------------------------------
// The draft and its validation
// ---------------------------------------------------------------------------

/// A validated report draft: the spec, its card, the shape the pair derives, and
/// the plain-English read-back. `shape` is derived by the contract's one
/// derivation (`CardShape::of`), never sent by the model.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportDraft {
    /// Advisory name from the model's answer envelope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub spec: ReportSpec,
    pub card: CardSpec,
    /// `chart` | `stat` | `table`, from `CardShape::of(&spec)`.
    pub shape: String,
    /// One sentence saying what the report shows. Derived from the spec and the
    /// card, so it cannot drift from the data the way a model-written summary
    /// can.
    pub reads: String,
}

/// The model's answer envelope. `spec` and `card` are the accepted fields; `name`
/// is advisory and passes to the save route unchanged.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ReportAuthoringEnvelope {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub spec: Option<Value>,
    #[serde(default)]
    pub card: Option<Value>,
}

/// The result of parsing one raw model answer: the draft (present only when the
/// pair validated), and any typed issues.
#[derive(Debug, Clone)]
pub struct ParsedReportDraft {
    pub draft: Option<ReportDraft>,
    pub issues: Vec<ReportAuthoringIssue>,
}

/// The caller's current draft, when editing. A report definition stores a spec
/// and no card (`core_db::report_defs::DefinitionFields` has no card member), so
/// `card` is optional here: a report has a spec from the day it is saved, and a
/// card only once a surface that displays it has decided on one.
#[derive(Debug, Clone)]
pub struct CurrentDraft {
    pub name: Option<String>,
    pub spec: ReportSpec,
    pub card: Option<CardSpec>,
}

/// Validates a candidate spec + card through the SAME boundary the save and run
/// routes use: registry validation for both, then the shared compile path. Pure.
///
/// `scope` is the caller's, so the compile check is the compile the caller would
/// actually run. `now` anchors the window; a base with no card is compiled over
/// the default thirty days.
pub fn validate_candidate(
    spec: &ReportSpec,
    card: Option<&CardSpec>,
    scope: core_reports::Scope,
    now: chrono::DateTime<Utc>,
) -> Vec<ReportAuthoringIssue> {
    let mut issues = Vec::new();
    if let Err(error) = spec.validate() {
        issues.push(issue_from_reports_error(&error));
    }
    if let Some(card) = card {
        if let Err(error) = card.validate() {
            issues.push(issue_from_reports_error(&error));
        }
    }
    if !issues.is_empty() {
        return issues;
    }
    // The same compile the run routes execute. A spec that validates but cannot
    // compile (an identifier the registry refuses later in the path) is a draft
    // the reviewer must not be told is ready.
    let window = RunWindow {
        start: now - Duration::days(i64::from(card.map_or(30, |c| c.window_days))),
        end: now,
    };
    if let Err(error) = spec.compile_windowed(scope, core_reports::PAGE_LIMIT_DEFAULT, window) {
        issues.push(issue_from_reports_error(&error));
    }
    issues
}

/// The full parse+validate path one raw model answer takes: extract JSON →
/// split the envelope → validate the spec + card pair. Lenient about
/// extraction, strict about validation.
pub fn parse_report_output(
    raw: &str,
    scope: core_reports::Scope,
    now: chrono::DateTime<Utc>,
) -> ParsedReportDraft {
    let Some(value) = extract_json_object(raw) else {
        return ParsedReportDraft {
            draft: None,
            issues: vec![ReportAuthoringIssue::new(
                "not_json",
                None,
                "the model's answer contained no JSON object".to_string(),
            )],
        };
    };
    let envelope: ReportAuthoringEnvelope =
        serde_json::from_value(value.clone()).unwrap_or_default();
    let (Some(spec_value), Some(card_value)) = (envelope.spec, envelope.card) else {
        return ParsedReportDraft {
            draft: None,
            issues: vec![ReportAuthoringIssue::new(
                "no_draft",
                None,
                "the JSON object carried no `spec` and `card` pair".to_string(),
            )],
        };
    };
    let spec: ReportSpec = match serde_json::from_value(spec_value) {
        Ok(spec) => spec,
        Err(error) => {
            return ParsedReportDraft {
                draft: None,
                issues: vec![ReportAuthoringIssue::new(
                    "bad_spec_shape",
                    Some("spec".to_string()),
                    format!("the spec does not deserialize: {error}"),
                )],
            }
        }
    };
    let card: CardSpec = match serde_json::from_value(card_value) {
        Ok(card) => card,
        Err(error) => {
            return ParsedReportDraft {
                draft: None,
                issues: vec![ReportAuthoringIssue::new(
                    "bad_card_shape",
                    Some("card".to_string()),
                    format!("the card does not deserialize: {error}"),
                )],
            }
        }
    };
    let issues = validate_candidate(&spec, Some(&card), scope, now);
    let reads = describe_report(&spec, &card);
    let shape = CardShape::of(&spec).slug().to_string();
    let draft = issues.is_empty().then(|| ReportDraft {
        name: envelope.name,
        spec,
        card,
        shape,
        reads,
    });
    ParsedReportDraft { draft, issues }
}

// ---------------------------------------------------------------------------
// The plain-English read-back
// ---------------------------------------------------------------------------

fn measure_phrase(measure: &MeasureSpec) -> String {
    match (&measure.agg, &measure.column) {
        (core_reports::Aggregate::Count, None) => "a count of rows".to_string(),
        (core_reports::Aggregate::Count, Some(column)) => {
            format!("a count of {}", column.replace('_', " "))
        }
        (agg, Some(column)) => {
            let word = match agg {
                core_reports::Aggregate::Sum => "the sum of",
                core_reports::Aggregate::Avg => "the average of",
                core_reports::Aggregate::Min => "the lowest",
                core_reports::Aggregate::Max => "the highest",
                core_reports::Aggregate::Count => "a count of",
            };
            format!("{word} {}", column.replace('_', " "))
        }
        (agg, None) => agg.slug().to_string(),
    }
}

fn operator_words(op: FilterOp) -> &'static str {
    use FilterOp::*;
    match op {
        Eq => "is",
        NotEq => "is not",
        Lt => "is below",
        LtEq => "is at or below",
        Gt => "is above",
        GtEq => "is at or above",
        IsNull => "is empty",
        IsNotNull => "is set",
    }
}

/// The operand as it reads in a sentence. A typed operand renders plainly; a
/// date keeps its day, a timestamp its instant, text keeps its quotes so it is
/// visibly a value rather than a column name.
fn operand_words(value: &core_reports::FilterValue) -> String {
    use core_reports::FilterValue::*;
    match value {
        Text(text) => format!("\"{text}\""),
        Uuid(id) => id.to_string(),
        Timestamp(ts) => ts.to_rfc3339(),
        Date(day) => day.to_string(),
        Int(n) => n.to_string(),
        Bool(true) => "true".to_string(),
        Bool(false) => "false".to_string(),
    }
}

fn filter_phrase(filter: &SpecFilter) -> String {
    let column = filter.column.replace('_', " ");
    match &filter.value {
        Some(value) => format!(
            "{column} {} {}",
            operator_words(filter.op),
            operand_words(value)
        ),
        None => format!("{column} {}", operator_words(filter.op)),
    }
}

fn every_words(minutes: u32) -> String {
    if minutes % (24 * 60) == 0 {
        match minutes / (24 * 60) {
            1 => "every day".to_string(),
            days => format!("every {days} days"),
        }
    } else if minutes % 60 == 0 {
        match minutes / 60 {
            1 => "every hour".to_string(),
            hours => format!("every {hours} hours"),
        }
    } else {
        format!("every {minutes} minutes")
    }
}

fn plural(n: u32) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// One sentence saying what the report shows and how its card behaves. Derived
/// from the validated spec and card, so the read-back and the data cannot
/// disagree.
pub fn describe_report(spec: &ReportSpec, card: &CardSpec) -> String {
    let shape = CardShape::of(spec).slug();
    let measures = spec
        .measures
        .iter()
        .map(measure_phrase)
        .collect::<Vec<_>>()
        .join(", ");
    let mut sentence = if spec.group_by.is_empty() {
        format!(
            "A {shape} over {}, showing {measures}",
            view_words(spec.view)
        )
    } else {
        format!(
            "A {shape} over {} grouped by {}, showing {measures}",
            view_words(spec.view),
            spec.group_by
                .iter()
                .map(|d| d.replace('_', " "))
                .collect::<Vec<_>>()
                .join(" and ")
        )
    };
    if !spec.filters.is_empty() {
        sentence.push_str(", limited to rows where ");
        sentence.push_str(
            &spec
                .filters
                .iter()
                .map(filter_phrase)
                .collect::<Vec<_>>()
                .join(" and "),
        );
    }
    sentence.push_str(&format!(
        ". It spans the last {} day{}, refreshes {}, and sits {} wide and {} tall.",
        card.window_days,
        plural(card.window_days),
        every_words(card.refresh.every_minutes),
        card.size.span.slug(),
        card.size.height.slug()
    ));
    sentence
}

// ---------------------------------------------------------------------------
// The structural diff
// ---------------------------------------------------------------------------

/// One structural change between the caller's current draft and the proposal.
/// The caller renders these as the review diff; nothing is applied here.
#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct ReportDiffEntry {
    /// `add`, `remove` or `change`.
    pub op: String,
    /// A stable location, e.g. `view`, `group_by/priority`, `measures/count`,
    /// `filters/priority:eq`, `card/window_days`.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub after: Option<Value>,
}

fn filter_key(filter: &SpecFilter) -> String {
    // The operator's serde slug is its wire spelling (`eq`, `is_null`), so the
    // key matches what the client already renders.
    let op = serde_json::to_value(filter.op)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "op".to_string());
    format!("{}:{op}", filter.column)
}

fn keyed<'a, T, F>(items: &'a [T], key: F) -> Vec<(String, &'a T)>
where
    F: Fn(&'a T) -> String,
{
    items.iter().map(|item| (key(item), item)).collect()
}

fn push_entries(
    entries: &mut Vec<ReportDiffEntry>,
    section: &str,
    before: Option<&Value>,
    after: Option<&Value>,
) {
    match (before, after) {
        (None, Some(after)) => entries.push(ReportDiffEntry {
            op: "add".to_string(),
            path: section.to_string(),
            before: None,
            after: Some(after.clone()),
        }),
        (Some(before), None) => entries.push(ReportDiffEntry {
            op: "remove".to_string(),
            path: section.to_string(),
            before: Some(before.clone()),
            after: None,
        }),
        (Some(before), Some(after)) if before != after => entries.push(ReportDiffEntry {
            op: "change".to_string(),
            path: section.to_string(),
            before: Some(before.clone()),
            after: Some(after.clone()),
        }),
        _ => {}
    }
}

/// A structural diff between a base draft and a proposal, keyed by column and
/// output name so a re-ordering is not a change. Pure.
///
/// A missing base (a fresh description) diffs against nothing: every member of
/// the proposal is an `add`, which is what a reviewer reading "what will this
/// report be" needs to see. A base with no card — a saved report definition —
/// shows its card members as additions rather than pretending a card was there.
pub fn diff_drafts(base: Option<&CurrentDraft>, draft: &ReportDraft) -> Vec<ReportDiffEntry> {
    let mut entries = Vec::new();

    // Top-level spec members. `name` is advisory and has no base: the caller's
    // current definition name is not part of the spec being diffed, so a
    // proposed name always reads as an addition.
    let named = draft.name.clone().map(Value::String);
    for (path, before, after) in [
        (
            "view",
            base.map(|base| json!(base.spec.view.slug())),
            Some(json!(draft.spec.view.slug())),
        ),
        (
            "viz",
            base.map(|base| json!(base.spec.viz.slug())),
            Some(json!(draft.spec.viz.slug())),
        ),
        ("name", None, named.as_ref().cloned()),
    ] {
        push_entries(&mut entries, path, before.as_ref(), after.as_ref());
    }

    // Group-by dimensions, keyed by column.
    let base_dims: Vec<String> = base
        .map(|base| base.spec.group_by.clone())
        .unwrap_or_default();
    for dim in &draft.spec.group_by {
        if !base_dims.contains(dim) {
            entries.push(ReportDiffEntry {
                op: "add".to_string(),
                path: format!("group_by/{}", dim.replace('_', " ")),
                before: None,
                after: Some(json!(dim)),
            });
        }
    }
    for dim in &base_dims {
        if !draft.spec.group_by.contains(dim) {
            entries.push(ReportDiffEntry {
                op: "remove".to_string(),
                path: format!("group_by/{}", dim.replace('_', " ")),
                before: Some(json!(dim)),
                after: None,
            });
        }
    }

    // Measures, keyed by the output column they produce.
    let base_measures: Vec<(String, Value)> = base
        .map(|base| {
            keyed(&base.spec.measures, |m| m.output_name())
                .into_iter()
                .map(|(name, measure)| (name, serde_json::to_value(measure).unwrap_or(Value::Null)))
                .collect()
        })
        .unwrap_or_default();
    let draft_measures: Vec<(String, Value)> = keyed(&draft.spec.measures, |m| m.output_name())
        .into_iter()
        .map(|(name, measure)| (name, serde_json::to_value(measure).unwrap_or(Value::Null)))
        .collect();
    for (name, after) in &draft_measures {
        match base_measures.iter().find(|(k, _)| k == name) {
            None => push_entries(&mut entries, &format!("measures/{name}"), None, Some(after)),
            Some((_, before)) => push_entries(
                &mut entries,
                &format!("measures/{name}"),
                Some(before),
                Some(after),
            ),
        }
    }
    for (name, before) in &base_measures {
        if !draft_measures.iter().any(|(k, _)| k == name) {
            push_entries(
                &mut entries,
                &format!("measures/{name}"),
                Some(before),
                None,
            );
        }
    }

    // Filters, keyed by column + operator.
    let base_filters: Vec<(String, Value)> = base
        .map(|base| {
            keyed(&base.spec.filters, filter_key)
                .into_iter()
                .map(|(key, filter)| (key, serde_json::to_value(filter).unwrap_or(Value::Null)))
                .collect()
        })
        .unwrap_or_default();
    let draft_filters: Vec<(String, Value)> = keyed(&draft.spec.filters, filter_key)
        .into_iter()
        .map(|(key, filter)| (key, serde_json::to_value(filter).unwrap_or(Value::Null)))
        .collect();
    for (key, after) in &draft_filters {
        match base_filters.iter().find(|(k, _)| k == key) {
            None => push_entries(&mut entries, &format!("filters/{key}"), None, Some(after)),
            Some((_, before)) => push_entries(
                &mut entries,
                &format!("filters/{key}"),
                Some(before),
                Some(after),
            ),
        }
    }
    for (key, before) in &base_filters {
        if !draft_filters.iter().any(|(k, _)| k == key) {
            push_entries(&mut entries, &format!("filters/{key}"), Some(before), None);
        }
    }

    // Card members. The shape is derived from the spec, so it is not diffed;
    // the numbers a reviewer can actually change are. A base with no card shows
    // all five as additions, which is the truth about a saved report.
    let base_card = base.and_then(|base| base.card);
    for (path, before, after) in [
        (
            "card/window_days",
            base_card.map(|c| json!(c.window_days)),
            json!(draft.card.window_days),
        ),
        (
            "card/refresh.every_minutes",
            base_card.map(|c| json!(c.refresh.every_minutes)),
            json!(draft.card.refresh.every_minutes),
        ),
        (
            "card/refresh.reach_days",
            base_card.map(|c| json!(c.refresh.reach_days)),
            json!(draft.card.refresh.reach_days),
        ),
        (
            "card/size.span",
            base_card.map(|c| json!(c.size.span.slug())),
            json!(draft.card.size.span.slug()),
        ),
        (
            "card/size.height",
            base_card.map(|c| json!(c.size.height.slug())),
            json!(draft.card.size.height.slug()),
        ),
    ] {
        push_entries(&mut entries, path, before.as_ref(), Some(&after));
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.op.cmp(&b.op)));
    entries
}

// ---------------------------------------------------------------------------
// The authoring loop
// ---------------------------------------------------------------------------

/// The result of one authoring loop, transport-agnostic so it is unit-testable
/// without HTTP or a database.
#[derive(Debug, Clone)]
pub struct ReportAuthoringOutcome {
    /// The validated draft, when the answer carried a valid spec + card pair.
    pub draft: Option<ReportDraft>,
    /// Typed issues; empty means `draft` is present and saveable.
    pub issues: Vec<ReportAuthoringIssue>,
    pub tool_calls: Vec<AuthoringToolCall>,
    pub steps: usize,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// Set when the loop ended because of a transport failure, so a caller can
    /// report honestly instead of pretending the model answered.
    pub transport_error: Option<String>,
}

impl ReportAuthoringOutcome {
    pub fn valid(&self) -> bool {
        self.draft.is_some() && self.issues.is_empty()
    }
}

/// Builds the initial message list: the compact system prompt plus the
/// transcript (and the caller's current draft when editing).
pub fn build_messages(transcript: &str, base: Option<&CurrentDraft>) -> Vec<LoopMessage> {
    let mut messages = vec![LoopMessage {
        role: "system".to_string(),
        content: report_authoring_system_prompt(),
        tool_calls: None,
        tool_call_id: None,
    }];
    let mut user = String::new();
    user.push_str("Describe the report you want:\n");
    user.push_str(transcript);
    if let Some(base) = base {
        user.push_str("\n\nThe report currently looks like this — edit it, keeping what works:\n");
        let mut current = json!({ "name": base.name, "spec": base.spec });
        if let Some(card) = base.card {
            current["card"] = json!(card);
        }
        user.push_str(&current.to_string());
    }
    messages.push(LoopMessage {
        role: "user".to_string(),
        content: user,
        tool_calls: None,
        tool_call_id: None,
    });
    messages
}

/// The bounded authoring loop: model → tool-result feedback → repeat, then the
/// final answer is parsed and validated. Mirrors `chat_runner::run_turn`'s loop
/// shape (same transport seam, same tool protocol) without the SSE framing — the
/// same loop lane 17 runs, with this module's tool and validator.
pub async fn run_authoring_loop(
    transport: &dyn ChatModelTransport,
    model: &str,
    transcript: &str,
    base: Option<&CurrentDraft>,
    scope: core_reports::Scope,
    now: chrono::DateTime<Utc>,
    max_steps: usize,
) -> ReportAuthoringOutcome {
    let tools = vec![view_columns_tool()];
    let mut messages = build_messages(transcript, base);
    let mut steps = 0usize;
    let mut prompt_tokens = 0u32;
    let mut completion_tokens = 0u32;
    let mut tool_calls: Vec<AuthoringToolCall> = Vec::new();
    let mut final_content = String::new();
    let mut transport_error: Option<String> = None;

    loop {
        if steps >= max_steps.max(1) {
            break;
        }
        let ask = ModelCallRequest {
            model: Some(model.to_string()),
            messages: messages.clone(),
            tools: tools.clone(),
            response_format: Some(json!({ "type": "json_object" })),
        };
        let turn = match transport.call(&ask).await {
            Ok(turn) => turn,
            Err(error) => {
                transport_error = Some(error.to_string());
                break;
            }
        };
        steps += 1;
        prompt_tokens += turn.prompt_tokens;
        completion_tokens += turn.completion_tokens;

        if !turn.content.is_empty() {
            final_content = turn.content.clone();
        }
        messages.push(LoopMessage {
            role: "assistant".to_string(),
            content: turn.content.clone(),
            tool_calls: if turn.tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls_wire(&turn.tool_calls))
            },
            tool_call_id: None,
        });
        if turn.tool_calls.is_empty() {
            break;
        }
        for call in turn.tool_calls {
            let (result, outcome) = handle_tool_call(&call);
            tool_calls.push(AuthoringToolCall {
                name: call.name.clone(),
                argument: call
                    .arguments
                    .get("view")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                outcome: outcome.to_string(),
            });
            messages.push(LoopMessage {
                role: "tool".to_string(),
                content: result,
                tool_calls: None,
                tool_call_id: Some(call.id.clone()),
            });
        }
    }

    let parsed = if final_content.is_empty() {
        ParsedReportDraft {
            draft: None,
            issues: vec![ReportAuthoringIssue::new(
                "no_answer",
                None,
                "the model produced no final answer".to_string(),
            )],
        }
    } else {
        parse_report_output(&final_content, scope, now)
    };

    ReportAuthoringOutcome {
        draft: parsed.draft,
        issues: parsed.issues,
        tool_calls,
        steps,
        prompt_tokens,
        completion_tokens,
        transport_error,
    }
}

/// Dispatches one tool call. Only [`VIEW_COLUMNS_TOOL`] is understood; anything
/// else is answered typed (the model cannot invent a tool that does work).
fn handle_tool_call(call: &ToolCallReq) -> (String, &'static str) {
    if call.name == VIEW_COLUMNS_TOOL {
        let result = run_view_columns_tool(&call.arguments);
        let outcome = if result.contains("\"found\":true") {
            "ok"
        } else {
            "not_found"
        };
        (result, outcome)
    } else {
        (
            json!({ "error": format!("unknown tool {:?}", call.name) }).to_string(),
            "rejected",
        )
    }
}

// ---------------------------------------------------------------------------
// Measurement
// ---------------------------------------------------------------------------

/// One candidate model answer in the measurement corpus.
#[derive(Debug, Clone)]
pub struct AuthoringSample {
    pub name: String,
    pub candidate: String,
}

/// The measured rates. `valid_json_rate` is the share of candidates from which a
/// JSON object was extracted; `boundary_rejection_rate` is the share of those
/// the REAL registry refused. Both are computed against the live catalog and
/// compile path by [`parse_report_output`].
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AuthoringMeasurement {
    pub total: usize,
    pub json_extracted: usize,
    pub boundary_valid: usize,
    pub boundary_rejected: usize,
    pub valid_json_rate: f64,
    pub boundary_rejection_rate: f64,
}

/// Runs a corpus through the real parse+validate path and reports the rates.
/// Pure — no model, no I/O — so it is deterministic evidence, not a hope.
pub fn measure_corpus(
    samples: &[AuthoringSample],
    scope: core_reports::Scope,
    now: chrono::DateTime<Utc>,
) -> AuthoringMeasurement {
    let total = samples.len();
    let mut json_extracted = 0usize;
    let mut boundary_valid = 0usize;
    for sample in samples {
        let parsed = parse_report_output(&sample.candidate, scope, now);
        // A candidate that never reached the boundary (not JSON, no spec/card)
        // is not "extracted"; the codes below are the ones that mean the model
        // produced a draft-shaped answer.
        let reached_boundary = !matches!(
            parsed.issues.first().map(|i| i.code.as_str()),
            Some("not_json") | Some("no_draft")
        );
        if reached_boundary {
            json_extracted += 1;
            if parsed.issues.is_empty() {
                boundary_valid += 1;
            }
        }
    }
    let boundary_rejected = json_extracted - boundary_valid;
    let rate = |n: usize| {
        if total == 0 {
            0.0
        } else {
            n as f64 / total as f64
        }
    };
    AuthoringMeasurement {
        total,
        json_extracted,
        boundary_valid,
        boundary_rejected,
        valid_json_rate: rate(json_extracted),
        boundary_rejection_rate: rate(boundary_rejected),
    }
}

/// The prompt-footprint measurement that motivates on-demand columns. Returns
/// `(prompt_chars, column_inventory_chars, view_count)`:
///
/// - `prompt_chars` is what this lane sends — it does not change when a column
///   is added to a view.
/// - `column_inventory_chars` is what inlining every view's projected and
///   filterable columns would ADD to that prompt. That number grows with the
///   registry; the prompt does not. The honest comparison is
///   `prompt_chars + column_inventory_chars` against `prompt_chars`, not
///   "the catalogue is smaller than the columns", which at five views it is not
///   (the rules prose dominates).
pub fn measure_prompt_footprint() -> (usize, usize, usize) {
    let compact = report_authoring_system_prompt().len();
    let mut inline = String::new();
    for view in ViewId::ALL {
        inline.push_str(view.slug());
        inline.push_str(view_blurb(view));
        for column in view.columns() {
            inline.push_str(column.name);
            inline.push_str(value_type_slug(column.value_type));
        }
        inline.push_str(&view.filterable().join(","));
    }
    (compact, inline.len(), ViewId::ALL.len())
}

// ---------------------------------------------------------------------------
// The HTTP endpoint
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthorReportRequest {
    /// The AI surface the call is admitted on (e.g. `general_chat`). The org
    /// grants it like any other; there is no report-specific surface and no new
    /// admission path.
    pub surface_key: String,
    /// The transcript to author from. TEXT ONLY: voice-note capture is its own
    /// lane, and this endpoint has no audio member to send one through.
    pub transcript: String,
    /// The spec of the report being edited. Validated first, so the returned
    /// diff is against a spec the reviewer could have saved.
    #[serde(default)]
    pub spec: Option<ReportSpec>,
    /// The card of the report being edited, when the surface that displays it
    /// has one. A stored report definition carries a spec and no card, so this
    /// is optional: without it the diff shows the card members as additions
    /// rather than pretending a card was already there.
    #[serde(default)]
    pub card: Option<CardSpec>,
    /// The current definition's name, when editing. Advisory: it seeds the diff
    /// so a rename is visible, and the model may propose its own.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_steps: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorReportCatalogue {
    /// How many views the prompt catalogued. A draft is validated against the
    /// same registry, so this is also how much of the registry it could name.
    pub views: usize,
    /// How many card shapes the prompt described.
    pub shapes: usize,
    /// How many card rules the prompt carried.
    pub rules: usize,
    /// The size of the system prompt in characters — the honest measure of what
    /// the model was told, and the number the on-demand-columns posture keeps
    /// small as columns grow.
    pub prompt_chars: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorReportResponse {
    /// True when `draft` is present and passed the save/run boundary.
    pub valid: bool,
    /// Always false — this endpoint never writes.
    pub saved: bool,
    /// The validated draft, when the model produced one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<ReportDraft>,
    /// Typed validation issues; empty means `draft` is present and saveable.
    pub issues: Vec<ReportAuthoringIssue>,
    /// The changes the draft makes against the caller's current draft (or an
    /// empty base when generating fresh).
    pub diff: Vec<ReportDiffEntry>,
    pub tool_calls: Vec<AuthoringToolCall>,
    pub usage: AuthorUsage,
    pub model_info: AuthorModelInfo,
    /// The catalogue the draft was validated against.
    pub catalogue: AuthorReportCatalogue,
    /// Set when the model transport failed; the caller reports this honestly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The report-definitions slice of the authoring surface, built per generation
/// so both mounts (`/api/v1` canonical + `/api` legacy alias) register the route
/// through one merge line in `routes::mod`.
pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    Router::new().route(&format!("{}/reports/author", gen.base()), post(author))
}

/// POST /api/reports/author — author a report draft from a transcript.
///
/// Reporting-create gated: the permission the save route needs is the
/// permission a draft needs, so a viewer can never be handed a proposal they
/// cannot accept. NOT a write: it returns a draft, its issues, a read-back and
/// the diff, and persists nothing. The completion rides the shared AI admission
/// path with the completion + tool-calling edges.
#[utoipa::path(
    operation_id = "report_author",
    post,
    path = "/api/reports/author",
    tag = "reports",
    security(("bearer" = [])),
    request_body = AuthorReportRequest,
    responses(
        (status = 200, body = AuthorReportResponse),
        (status = 422, description = "the request or the base draft did not validate"),
        (status = 403, description = "AI admission refused (typed reason in the envelope)")
    )
)]
pub async fn author(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<AuthorReportRequest>,
) -> ApiResult<Json<AuthorReportResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Reporting,
            action: Action::Create,
        },
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let surface_key = req.surface_key.trim().to_string();
    if surface_key.is_empty() {
        return Err(ApiError::Validation("surface_key is required".into()));
    }
    let transcript = req.transcript.trim().to_string();
    if transcript.is_empty() {
        return Err(ApiError::Validation("transcript is required".into()));
    }
    if transcript.chars().count() > MAX_TRANSCRIPT_CHARS {
        return Err(ApiError::Validation(format!(
            "transcript must be at most {MAX_TRANSCRIPT_CHARS} characters"
        )));
    }

    // The compile check runs with the caller's own scope, so a draft that would
    // be narrowed differently for this user is checked as this user.
    let scope = if authed.principal.is_owner_or_admin() {
        core_reports::Scope::OrgWide
    } else {
        core_reports::Scope::Self_(user_id)
    };
    let now = Utc::now();

    // A base draft must itself be saveable, so the diff is meaningful. A report
    // definition stores a spec and no card, so the card half is optional: half a
    // card with no spec is not a report and is refused as a caller bug.
    let base = match (req.spec.clone(), req.card) {
        (Some(spec), card) => {
            let issues = validate_candidate(&spec, card.as_ref(), scope, now);
            if !issues.is_empty() {
                return Err(ApiError::Validation(format!(
                    "the current draft does not validate: {}",
                    issues
                        .iter()
                        .map(|i| i.message.clone())
                        .collect::<Vec<_>>()
                        .join("; ")
                )));
            }
            Some(CurrentDraft {
                name: req.name.clone(),
                spec,
                card,
            })
        }
        (None, Some(_)) => {
            return Err(ApiError::Validation(
                "a card without a spec is not a report; send the spec too".into(),
            ))
        }
        (None, None) => None,
    };

    // --- admission, on the existing path -----------------------------------
    let mut tx = org_bound_tx(&state, org_id, user_id).await?;
    let call = authorize_surface_call(
        &state,
        &mut tx,
        AiAdmissionAsk {
            org_id,
            user_id,
            surface_key: &surface_key,
            provider_ask: req.provider.as_deref(),
            model_ask: req.model.as_deref(),
            required_capabilities: &[AiEdge::Completion, AiEdge::ToolCall],
            internal_core_call: false,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;

    // --- the loop, on the shared transport seam ----------------------------
    let transport = chat_runner::resolve_transport(org_id, &call);
    let max_steps = req
        .max_steps
        .map(|n| n as usize)
        .unwrap_or(MAX_AUTHORING_STEPS)
        .min(MAX_AUTHORING_STEPS);
    let outcome = run_authoring_loop(
        transport.as_ref(),
        &call.model,
        &transcript,
        base.as_ref(),
        scope,
        now,
        max_steps,
    )
    .await;

    // Usage ledger (caps bind across calls, exactly as the chat runner's do).
    record_authoring_usage(&state, &call, org_id, user_id, &outcome).await;

    let diff = match &outcome.draft {
        Some(draft) => diff_drafts(base.as_ref(), draft),
        None => Vec::new(),
    };
    let response = AuthorReportResponse {
        valid: outcome.valid(),
        saved: false,
        draft: outcome.draft.clone(),
        issues: outcome.issues.clone(),
        diff,
        tool_calls: outcome.tool_calls.clone(),
        usage: AuthorUsage {
            prompt_tokens: outcome.prompt_tokens,
            completion_tokens: outcome.completion_tokens,
            steps: outcome.steps,
        },
        model_info: AuthorModelInfo {
            provider: call.provider.clone(),
            model: call.model.clone(),
            resolution: call.resolution.to_string(),
        },
        catalogue: AuthorReportCatalogue {
            views: ViewId::ALL.len(),
            shapes: core_reports::shape_catalogue().len(),
            rules: core_reports::rules().len(),
            prompt_chars: report_authoring_system_prompt().len(),
        },
        error: outcome.transport_error.clone(),
    };
    Ok(Json(response))
}

/// Writes the usage ledger + a provenance audit row for the authoring call.
/// Best-effort: a failure to record never fails the response (the draft is
/// already produced), and the admission hard cap remains the primary gate.
async fn record_authoring_usage(
    state: &AppState,
    call: &crate::routes::ai_policy::AuthorizedAiCall,
    org_id: Uuid,
    user_id: Uuid,
    outcome: &ReportAuthoringOutcome,
) {
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => {
            tracing::warn!(%error, "report authoring usage ledger begin failed");
            return;
        }
    };
    if core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .is_err()
    {
        return;
    }
    let _ = record_usage(
        &mut tx,
        &AiUsageNew {
            id: Uuid::now_v7(),
            org_id,
            user_id,
            surface: call.surface_key.clone(),
            provider: call.provider.clone(),
            model: call.model.clone(),
            prompt_tokens: outcome.prompt_tokens as i32,
            completion_tokens: outcome.completion_tokens as i32,
            cost_weight: call.cost_weight,
            request_id: Some(format!("report_author:{}", Uuid::now_v7())),
        },
    )
    .await;
    let _ = core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Agent(format!("report_authoring/user:{user_id}")),
            action: "ai.report_author".into(),
            entity_type: Some("report_definition".into()),
            entity_id: None,
            detail: json!({
                "surface": call.surface_key,
                "provider": call.provider,
                "model": call.model,
                "steps": outcome.steps,
                "valid": outcome.valid(),
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await;
    let _ = tx.commit().await;
}

#[cfg(test)]
mod tests;
