//! Unit tests for the report authoring mechanism. Pure functions are tested
//! directly; the loop is tested against the deterministic
//! `chat_runner::ScriptedTransport` (no HTTP, no database, no provider).

use super::*;
use crate::routes::chat_runner::{ModelCallRequest, ModelTurn, ScriptedTransport};
use core_reports::{Aggregate, CardRefresh, CardSize, VizHint};
use std::sync::Mutex;

fn ticket_bar() -> ReportSpec {
    ReportSpec {
        view: ViewId::Tickets,
        filters: Vec::new(),
        group_by: vec!["priority".to_string()],
        measures: vec![MeasureSpec {
            agg: Aggregate::Count,
            column: None,
        }],
        viz: VizHint::Bar,
    }
}

fn thirty_day_card() -> CardSpec {
    CardSpec {
        size: CardSize::default(),
        window_days: 30,
        refresh: CardRefresh::default(),
    }
}

fn envelope(spec: Value, card: Value) -> String {
    json!({ "name": "Tickets by priority", "spec": spec, "card": card }).to_string()
}

fn ticket_spec_json() -> Value {
    json!({
        "view": "tickets",
        "filters": [],
        "group_by": ["priority"],
        "measures": [{ "agg": "count" }],
        "viz": "bar"
    })
}

fn card_json() -> Value {
    json!({ "window_days": 30 })
}

fn now() -> chrono::DateTime<Utc> {
    Utc::now()
}

#[test]
fn the_prompt_catalogues_views_and_shapes_and_carries_no_columns() {
    let prompt = report_authoring_system_prompt();
    for view in ViewId::ALL {
        assert!(
            prompt.contains(view.slug()),
            "the prompt must name view {:?}",
            view.slug()
        );
        assert!(
            !view_blurb(view).is_empty(),
            "{:?} has no blurb",
            view.slug()
        );
    }
    for shape in core_reports::shape_catalogue() {
        assert!(
            prompt.contains(shape.shape.slug()),
            "the prompt must describe shape {:?}",
            shape.shape.slug()
        );
    }
    for rule in core_reports::rules() {
        assert!(
            prompt.contains(rule),
            "the prompt must carry every card rule"
        );
    }
    for size in CardSpan::ALL {
        assert!(
            prompt.contains(size.slug()),
            "span {:?} missing",
            size.slug()
        );
    }
    // The point of the posture: NO columns. A projection column appearing here
    // would mean the prompt grows with the registry instead of staying put.
    for column in [
        "assignee_id",
        "requester_id",
        "ticket_number",
        "user_display_name",
    ] {
        assert!(
            !prompt.contains(column),
            "the prompt must not inline column {column:?}"
        );
    }
    assert!(
        prompt.contains(VIEW_COLUMNS_TOOL),
        "the prompt must name the tool"
    );
}

#[test]
fn the_prompt_grows_with_shapes_not_with_columns() {
    let (prompt_chars, column_chars, views) = measure_prompt_footprint();
    assert_eq!(views, ViewId::ALL.len());
    // The inventory is real, and inlining it would grow the prompt by that
    // much — a share that climbs with every column the registry gains, while
    // this prompt stays where it is.
    assert!(column_chars > 0, "there are columns to inline");
    assert!(prompt_chars > 0);
    // The measurement is the one the review quotes: what the model already
    // reads, plus what the columns would add.
    let with_columns = prompt_chars + column_chars;
    assert!(with_columns > prompt_chars);
    // And the compact prompt carries none of them, which is what makes the
    // first number invariant. `org_id` is excluded on purpose: a card RULE
    // names it ("the tenancy seam, never a report dimension"), which is a
    // prohibition the model needs, not an inline column.
    let prompt = report_authoring_system_prompt();
    for column in [
        "assignee_id",
        "requester_id",
        "client_name",
        "ticket_number",
        "day_utc",
        "timer_session_id",
    ] {
        assert!(
            !prompt.contains(column),
            "the prompt must not inline column {column:?}"
        );
    }
    assert!(
        prompt.contains("org_id"),
        "the tenancy seam is named as a prohibition"
    );
}

#[test]
fn the_column_tool_serves_the_real_registry() {
    let served: Value =
        serde_json::from_str(&run_view_columns_tool(&json!({ "view": "tickets" }))).unwrap();
    assert_eq!(served["found"], json!(true));
    let columns = served["columns"].as_array().unwrap();
    assert!(columns.iter().any(|c| c["name"] == json!("priority")));
    assert!(columns
        .iter()
        .any(|c| c["name"] == json!("created_at") && c["value_type"] == json!("timestamp")));
    let filterable = served["filterable"].as_array().unwrap();
    assert!(filterable.iter().any(|c| c["name"] == json!("priority")));
    // The narrower filterable inventory never invents a column the projection
    // does not carry.
    for column in filterable {
        assert!(columns.iter().any(|c| c["name"] == column["name"]));
    }
    assert_eq!(served["time_anchor"]["column"], json!("created_at"));

    let unknown: Value =
        serde_json::from_str(&run_view_columns_tool(&json!({ "view": "nope" }))).unwrap();
    assert_eq!(unknown["found"], json!(false));
    let empty: Value = serde_json::from_str(&run_view_columns_tool(&json!({}))).unwrap();
    assert_eq!(empty["found"], json!(false));
}

#[test]
fn a_valid_answer_becomes_a_ready_draft() {
    let parsed = parse_report_output(
        &envelope(ticket_spec_json(), card_json()),
        core_reports::Scope::OrgWide,
        now(),
    );
    assert!(parsed.issues.is_empty(), "issues: {:?}", parsed.issues);
    let draft = parsed.draft.expect("a valid answer yields a draft");
    assert_eq!(draft.shape, "chart");
    assert_eq!(draft.name.as_deref(), Some("Tickets by priority"));
    assert_eq!(draft.card.window_days, 30);
    // The read-back is derived, so it says what the spec says.
    assert_eq!(
        draft.reads,
        "A chart over tickets grouped by priority, showing a count of rows. It spans \
         the last 30 days, refreshes every hour, and sits half wide and regular tall."
    );
}

#[test]
fn a_stat_is_described_as_a_headline_number() {
    let spec = json!({
        "view": "tickets",
        "group_by": [],
        "measures": [{ "agg": "count" }],
        "viz": "stat"
    });
    let parsed = parse_report_output(
        &envelope(spec, card_json()),
        core_reports::Scope::OrgWide,
        now(),
    );
    let draft = parsed.draft.expect("a stat draft validates");
    assert_eq!(draft.shape, "stat");
    assert!(draft
        .reads
        .starts_with("A stat over tickets, showing a count of rows."));
}

#[test]
fn a_bad_answer_comes_back_typed_never_a_draft() {
    let scope = core_reports::Scope::OrgWide;

    // Unknown column.
    let unknown_column = json!({
        "view": "tickets",
        "group_by": ["nonsense"],
        "measures": [{ "agg": "count" }],
        "viz": "bar"
    });
    let parsed = parse_report_output(&envelope(unknown_column, card_json()), scope, now());
    assert!(parsed.draft.is_none());
    assert_eq!(parsed.issues[0].code, "unknown_column");

    // A stat with a dimension is refused by the spec validator.
    let stat_with_dimension = json!({
        "view": "tickets",
        "group_by": ["priority"],
        "measures": [{ "agg": "count" }],
        "viz": "stat"
    });
    let parsed = parse_report_output(&envelope(stat_with_dimension, card_json()), scope, now());
    assert_eq!(parsed.issues[0].code, "invalid_spec");
    assert_eq!(parsed.issues[0].path.as_deref(), Some("viz"));

    // A card whose window reaches past one refresh.
    let parsed = parse_report_output(
        &envelope(
            ticket_spec_json(),
            json!({ "window_days": 30, "refresh": { "every_minutes": 60, "reach_days": 7 } }),
        ),
        scope,
        now(),
    );
    assert_eq!(parsed.issues[0].code, "window_wider_than_refresh");

    // No JSON at all.
    let parsed = parse_report_output("I can't build that.", scope, now());
    assert_eq!(parsed.issues[0].code, "not_json");

    // JSON with no spec/card pair.
    let parsed = parse_report_output(&json!({ "name": "x" }).to_string(), scope, now());
    assert_eq!(parsed.issues[0].code, "no_draft");

    // A spec that is not spec-shaped.
    let parsed = parse_report_output(&envelope(json!({ "view": 42 }), card_json()), scope, now());
    assert_eq!(parsed.issues[0].code, "bad_spec_shape");
    assert_eq!(parsed.issues[0].path.as_deref(), Some("spec"));
}

#[test]
fn the_boundary_is_the_one_the_save_route_calls() {
    // Everything the spec validator refuses here, `report_definitions_create`
    // refuses too: both call `ReportSpec::validate`. This test pins the
    // agreement on the two failure classes a model actually produces.
    for spec in [
        // over the filter cap is the validator's business, not ours
        json!({ "view": "tickets", "group_by": [], "measures": [], "viz": "table" }),
        json!({ "view": "tickets", "group_by": ["org_id"], "measures": [{ "agg": "count" }], "viz": "bar" }),
    ] {
        let parsed = parse_report_output(
            &envelope(spec.clone(), card_json()),
            core_reports::Scope::OrgWide,
            now(),
        );
        assert!(parsed.draft.is_none(), "spec must be refused: {spec}");
        let candidates: Vec<ReportAuthoringIssue> = parsed.issues.clone();
        assert_eq!(candidates[0].code, "invalid_spec");
    }
}

#[test]
fn the_diff_keys_by_column_and_output_name() {
    let base_spec = ticket_bar();
    let base_card = thirty_day_card();
    let current = CurrentDraft {
        name: None,
        spec: base_spec.clone(),
        card: Some(base_card),
    };
    let mut draft = ReportDraft {
        name: Some("Tickets by priority and status".to_string()),
        spec: base_spec,
        card: base_card,
        shape: "chart".to_string(),
        reads: String::new(),
    };
    draft.spec.group_by = vec!["priority".to_string(), "status_name".to_string()];
    draft.spec.measures = vec![MeasureSpec {
        agg: Aggregate::Sum,
        column: Some("breached".to_string()),
    }];
    draft.spec.filters = vec![SpecFilter {
        column: "priority".to_string(),
        op: FilterOp::Eq,
        value: Some(core_reports::FilterValue::Text("p1".to_string())),
    }];
    draft.card.window_days = 7;

    let diff = diff_drafts(Some(&current), &draft);
    let at = |path: &str| diff.iter().find(|e| e.path == path);

    assert_eq!(at("name").expect("name added").op, "add");
    assert_eq!(
        at("group_by/status name")
            .expect("second dimension added")
            .op,
        "add"
    );
    assert_eq!(
        at("measures/count").expect("old measure removed").op,
        "remove"
    );
    assert_eq!(
        at("measures/sum_breached").expect("new measure added").op,
        "add"
    );
    assert_eq!(at("filters/priority:eq").expect("filter added").op, "add");
    let window = at("card/window_days").expect("window changed");
    assert_eq!(window.op, "change");
    assert_eq!(window.before, Some(json!(30)));
    assert_eq!(window.after, Some(json!(7)));
    // Unchanged members are absent: a diff that lists everything is not a diff.
    assert!(at("view").is_none());
    assert!(at("card/size.span").is_none());
    // Deterministically ordered by path.
    let paths: Vec<&str> = diff.iter().map(|e| e.path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(paths, sorted);
}

#[test]
fn a_base_with_no_card_diffs_its_card_as_additions() {
    // A saved report definition has a spec and no card. The honest diff says
    // so: nothing on the card side is already decided.
    let current = CurrentDraft {
        name: Some("Tickets by priority".to_string()),
        spec: ticket_bar(),
        card: None,
    };
    let draft = ReportDraft {
        name: Some("Tickets by priority".to_string()),
        spec: ticket_bar(),
        card: thirty_day_card(),
        shape: "chart".to_string(),
        reads: String::new(),
    };
    let diff = diff_drafts(Some(&current), &draft);
    assert!(
        !diff.iter().any(|e| e.path.starts_with("group_by/")),
        "the base spec already grouped by priority: {diff:?}"
    );
    for path in [
        "card/window_days",
        "card/refresh.every_minutes",
        "card/size.span",
        "card/size.height",
    ] {
        let entry = diff
            .iter()
            .find(|e| e.path == path)
            .unwrap_or_else(|| panic!("{path} must be an addition"));
        assert_eq!(entry.op, "add", "{path}");
        assert_eq!(entry.before, None, "{path} had no before");
    }
}

#[test]
fn a_fresh_description_diffs_against_nothing_as_additions() {
    let draft = ReportDraft {
        name: Some("Open P1s".to_string()),
        spec: ticket_bar(),
        card: thirty_day_card(),
        shape: "chart".to_string(),
        reads: String::new(),
    };
    let diff = diff_drafts(None, &draft);
    assert!(!diff.is_empty());
    assert!(diff.iter().all(|e| e.op == "add"));
    assert!(diff.iter().any(|e| e.path == "view"));
    assert!(diff.iter().any(|e| e.path == "group_by/priority"));
    assert!(diff.iter().any(|e| e.path == "measures/count"));
    assert!(diff.iter().any(|e| e.path == "card/window_days"));
}

/// The loop must answer the column tool and then validate the final answer —
/// the mechanism, end to end, with no provider and no database.
#[tokio::test]
async fn the_loop_answers_the_column_tool_and_validates_the_answer() {
    let transport = ScriptedTransport::new(vec![
        ModelTurn {
            content: String::new(),
            tool_calls: vec![ToolCallReq {
                id: "c1".to_string(),
                name: VIEW_COLUMNS_TOOL.to_string(),
                arguments: json!({ "view": "tickets" }),
            }],
            prompt_tokens: 40,
            completion_tokens: 8,
        },
        ModelTurn {
            content: envelope(ticket_spec_json(), card_json()),
            tool_calls: Vec::new(),
            prompt_tokens: 120,
            completion_tokens: 60,
        },
    ]);
    let outcome = run_authoring_loop(
        &transport,
        "marula-test",
        AuthoringRun {
            transcript: "Tickets by priority, as a bar chart, last thirty days.",
            base: None,
            scope: core_reports::Scope::OrgWide,
            now: now(),
            max_steps: MAX_AUTHORING_STEPS,
            verification: Verification::none(),
        },
    )
    .await;
    assert!(outcome.valid(), "issues: {:?}", outcome.issues);
    assert_eq!(outcome.steps, 2);
    assert_eq!(outcome.tool_calls.len(), 1);
    assert_eq!(outcome.tool_calls[0].name, VIEW_COLUMNS_TOOL);
    assert_eq!(outcome.tool_calls[0].argument, "tickets");
    assert_eq!(outcome.tool_calls[0].outcome, "ok");
    assert_eq!(outcome.prompt_tokens, 160);
    assert_eq!(outcome.completion_tokens, 68);
    assert!(outcome.transport_error.is_none());
}

/// An unknown tool is answered typed: the model cannot invent a tool that works.
#[tokio::test]
async fn an_unknown_tool_is_refused_and_the_loop_still_bounds_itself() {
    let transport = ScriptedTransport::new(vec![ModelTurn {
        content: String::new(),
        tool_calls: vec![ToolCallReq {
            id: "c1".to_string(),
            name: "drop_all_tables".to_string(),
            arguments: json!({}),
        }],
        prompt_tokens: 10,
        completion_tokens: 2,
    }]);
    let outcome = run_authoring_loop(
        &transport,
        "marula-test",
        AuthoringRun {
            transcript: "anything",
            base: None,
            scope: core_reports::Scope::OrgWide,
            now: now(),
            max_steps: 2,
            verification: Verification::none(),
        },
    )
    .await;
    assert_eq!(
        outcome.steps, 1,
        "the script ran out of turns, not the bound"
    );
    assert_eq!(outcome.tool_calls[0].outcome, "rejected");
    assert_eq!(outcome.issues[0].code, "no_answer");
}

#[test]
fn the_corpus_measurement_separates_extraction_from_validation() {
    let realistic = vec![
        AuthoringSample {
            name: "chart".into(),
            candidate: envelope(ticket_spec_json(), card_json()),
        },
        AuthoringSample {
            name: "stat".into(),
            candidate: envelope(
                json!({
                    "view": "time_entries",
                    "group_by": [],
                    "measures": [{ "agg": "sum", "column": "minutes" }],
                    "viz": "stat"
                }),
                card_json(),
            ),
        },
        AuthoringSample {
            name: "fenced".into(),
            candidate: format!(
                "```json\n{}\n```",
                envelope(ticket_spec_json(), card_json())
            ),
        },
    ];
    let boundary = vec![
        AuthoringSample {
            name: "unknown-view".into(),
            candidate: envelope(
                json!({ "view": "not_a_view", "group_by": [], "measures": [{ "agg": "count" }], "viz": "stat" }),
                card_json(),
            ),
        },
        AuthoringSample {
            name: "prose-only".into(),
            candidate: "I am not able to build that.".into(),
        },
    ];
    let scope = core_reports::Scope::OrgWide;
    let realistic_measure = measure_corpus(&realistic, scope, now());
    let boundary_measure = measure_corpus(&boundary, scope, now());

    assert_eq!(realistic_measure.total, 3);
    assert_eq!(
        realistic_measure.boundary_valid, 3,
        "every realistic draft passes the boundary: {realistic_measure:?}"
    );
    assert_eq!(boundary_measure.total, 2);
    assert!(
        boundary_measure.boundary_rejected >= 1,
        "a malformation must be refused: {boundary_measure:?}"
    );
    assert_eq!(
        boundary_measure.valid_json_rate, 0.5,
        "prose is not extracted"
    );
}

// ---------------------------------------------------------------------------
// Verification: the data check, the repair cap, and the finding that survives
// ---------------------------------------------------------------------------

/// Scripted turns plus a transcript of what the loop sent, so a test can read
/// the repair prompt the model actually received.
struct RecordingTransport {
    turns: Mutex<Vec<ModelTurn>>,
    seen: Mutex<Vec<ModelCallRequest>>,
}

impl RecordingTransport {
    fn new(turns: Vec<ModelTurn>) -> Self {
        Self {
            turns: Mutex::new(turns),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// The tool names the loop offered, from the first ask.
    fn tools(&self) -> Vec<String> {
        self.seen
            .lock()
            .expect("recording lock poisoned")
            .first()
            .map(|req| {
                req.tools
                    .iter()
                    .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every prompt the transport was asked with, rendered as one string.
    fn prompts(&self) -> Vec<String> {
        self.seen
            .lock()
            .expect("recording lock poisoned")
            .iter()
            .map(|req| {
                req.messages
                    .iter()
                    .map(|message| message.content.clone())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl ChatModelTransport for RecordingTransport {
    async fn call(
        &self,
        req: &ModelCallRequest,
    ) -> Result<ModelTurn, crate::routes::chat_runner::ChatTransportError> {
        self.seen
            .lock()
            .expect("recording lock poisoned")
            .push(req.clone());
        let mut turns = self.turns.lock().expect("turns lock poisoned");
        if turns.is_empty() {
            return Err(
                crate::routes::chat_runner::ChatTransportError::Unavailable {
                    reason: "scripted transport exhausted (test bug)".to_string(),
                },
            );
        }
        Ok(turns.remove(0))
    }

    fn kind(&self) -> &str {
        "recording_test"
    }
}

/// A verifier that answers from a script instead of a database: one entry per
/// verification call, the last entry repeating once the script runs out.
struct FakeVerifier {
    script: Mutex<Vec<Vec<DraftFinding>>>,
    calls: Mutex<usize>,
}

impl FakeVerifier {
    fn always(findings: Vec<DraftFinding>) -> Self {
        Self {
            script: Mutex::new(vec![findings]),
            calls: Mutex::new(0),
        }
    }

    fn scripted(script: Vec<Vec<DraftFinding>>) -> Self {
        Self {
            script: Mutex::new(script),
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().expect("calls lock poisoned")
    }
}

impl DraftVerifier for FakeVerifier {
    fn verify<'a>(
        &'a self,
        _spec: &'a ReportSpec,
        _card: &'a CardSpec,
        _scope: core_reports::Scope,
        _window: RunWindow,
        _card_index: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Vec<DraftFinding>> + Send + 'a>> {
        Box::pin(async move {
            *self.calls.lock().expect("calls lock poisoned") += 1;
            let mut script = self.script.lock().expect("script lock poisoned");
            if script.is_empty() {
                return Vec::new();
            }
            if script.len() == 1 {
                return script[0].clone();
            }
            script.remove(0)
        })
    }
}

/// Proves the contract's refusals need no run: calling it is a test bug.
struct NeverCalledVerifier;

impl DraftVerifier for NeverCalledVerifier {
    fn verify<'a>(
        &'a self,
        _spec: &'a ReportSpec,
        _card: &'a CardSpec,
        _scope: core_reports::Scope,
        _window: RunWindow,
        _card_index: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Vec<DraftFinding>> + Send + 'a>> {
        Box::pin(async move {
            panic!("a draft the contract already refused must not reach the run");
        })
    }
}

fn no_rows(index: Option<usize>) -> DraftFinding {
    DraftFinding::new(
        "no_rows",
        index,
        format!(
            "{}has no rows at this window.",
            finding_subject(index, "chart")
        ),
    )
}

fn answer_turn() -> ModelTurn {
    ModelTurn {
        content: envelope(ticket_spec_json(), card_json()),
        tool_calls: Vec::new(),
        prompt_tokens: 100,
        completion_tokens: 40,
    }
}

/// The model may check its own work before it answers: the tool runs the
/// candidate through the injected verifier and answers typed.
#[tokio::test]
async fn the_verification_tool_runs_the_candidate_and_answers_typed() {
    let verifier = FakeVerifier::always(Vec::new());
    let transport = RecordingTransport::new(vec![
        ModelTurn {
            content: String::new(),
            tool_calls: vec![ToolCallReq {
                id: "v1".to_string(),
                name: VERIFY_DRAFT_TOOL.to_string(),
                arguments: json!({ "spec": ticket_spec_json(), "card": card_json() }),
            }],
            prompt_tokens: 40,
            completion_tokens: 8,
        },
        answer_turn(),
    ]);
    let outcome = run_authoring_loop(
        &transport,
        "marula-test",
        AuthoringRun {
            transcript: "Tickets by priority, as a bar chart.",
            base: None,
            scope: core_reports::Scope::OrgWide,
            now: now(),
            max_steps: MAX_AUTHORING_STEPS,
            verification: Verification::new(&verifier),
        },
    )
    .await;

    assert_eq!(outcome.tool_calls.len(), 1);
    assert_eq!(outcome.tool_calls[0].name, VERIFY_DRAFT_TOOL);
    assert_eq!(outcome.tool_calls[0].argument, "tickets");
    assert_eq!(outcome.tool_calls[0].outcome, "clean");
    assert_eq!(
        verifier.calls(),
        2,
        "the tool ran, then the answer was checked"
    );
    assert!(outcome.valid(), "issues: {:?}", outcome.issues);
    // Both tools ride the ask, and no others: the model cannot invent one.
    let tools = transport.tools();
    assert_eq!(
        tools,
        vec![VIEW_COLUMNS_TOOL.to_string(), VERIFY_DRAFT_TOOL.to_string()]
    );
}

/// A zero-row card is caught by the verifier and arrives at the model as a
/// typed reason, not a complaint: the next prompt names the class.
#[tokio::test]
async fn a_zero_row_card_becomes_a_typed_reason_in_the_next_prompt() {
    let verifier = FakeVerifier::scripted(vec![vec![no_rows(None)], Vec::new()]);
    let transport = RecordingTransport::new(vec![answer_turn(), answer_turn()]);
    let outcome = run_authoring_loop(
        &transport,
        "marula-test",
        AuthoringRun {
            transcript: "Tickets by priority, as a bar chart.",
            base: None,
            scope: core_reports::Scope::OrgWide,
            now: now(),
            max_steps: MAX_AUTHORING_STEPS,
            verification: Verification::new(&verifier),
        },
    )
    .await;

    assert!(outcome.valid(), "the repair is clean: {:?}", outcome.issues);
    assert_eq!(outcome.verification.attempts.len(), 2);
    assert_eq!(outcome.verification.attempts[0].findings[0].kind, "no_rows");
    assert!(outcome.verification.attempts[1].findings.is_empty());
    assert_eq!(outcome.verification.findings.len(), 0);
    assert_eq!(outcome.verification.repaired, vec![no_rows(None)]);
    let prompts = transport.prompts();
    assert_eq!(prompts.len(), 2);
    assert!(
        prompts[1].contains("no_rows") && prompts[1].contains("has no rows at this window"),
        "the repair prompt carries the typed reason: {}",
        prompts[1]
    );
    assert!(
        prompts[1].contains("corrected JSON object"),
        "the repair prompt asks for a corrected answer"
    );
}

/// On exhaustion the draft comes back WITH the failure named, and `valid()` is
/// false: a draft the data check could not draw is never presented as clean.
#[tokio::test]
async fn on_exhaustion_the_finding_survives_and_the_draft_is_not_clean() {
    let verifier = FakeVerifier::always(vec![no_rows(None)]);
    let transport = RecordingTransport::new(vec![
        answer_turn(),
        answer_turn(),
        answer_turn(),
        answer_turn(),
    ]);
    let outcome = run_authoring_loop(
        &transport,
        "marula-test",
        AuthoringRun {
            transcript: "Tickets by priority, as a bar chart.",
            base: None,
            scope: core_reports::Scope::OrgWide,
            now: now(),
            max_steps: MAX_AUTHORING_STEPS,
            verification: Verification::with_rounds(&verifier, 2),
        },
    )
    .await;

    assert!(outcome.draft.is_some(), "the draft is still handed back");
    assert!(!outcome.valid(), "a surviving finding is not valid");
    // Round 0 is the check, rounds 1 and 2 are the two repairs the cap allows.
    assert_eq!(outcome.verification.attempts.len(), 3);
    assert_eq!(outcome.verification.findings, vec![no_rows(None)]);
    assert_eq!(outcome.issues.len(), 1);
    assert_eq!(outcome.issues[0].code, "no_rows");
    assert_eq!(verifier.calls(), 3, "the cap is spent, not exceeded");
    assert_eq!(outcome.steps, 3, "three answers, one per verification");
}

/// A dashboard verifies each card through the same function and reports the
/// finding against the card's own index.
#[tokio::test]
async fn a_dashboard_reports_findings_per_card_index() {
    let cards = vec![
        DashboardCard {
            spec: ticket_bar(),
            card: thirty_day_card(),
        },
        DashboardCard {
            spec: ReportSpec {
                view: ViewId::TimeEntries,
                filters: Vec::new(),
                group_by: vec!["user_id".to_string()],
                measures: vec![MeasureSpec {
                    agg: Aggregate::Count,
                    column: None,
                }],
                viz: VizHint::Bar,
            },
            card: thirty_day_card(),
        },
    ];
    // The port does not decide which card fails; this fake adds the index the
    // way the production verifier is given it.
    struct SecondCardFails;
    impl DraftVerifier for SecondCardFails {
        fn verify<'a>(
            &'a self,
            spec: &'a ReportSpec,
            _card: &'a CardSpec,
            _scope: core_reports::Scope,
            _window: RunWindow,
            card_index: Option<usize>,
        ) -> Pin<Box<dyn Future<Output = Vec<DraftFinding>> + Send + 'a>> {
            let fails = spec.view == ViewId::TimeEntries;
            Box::pin(async move {
                if fails {
                    vec![no_rows(card_index)]
                } else {
                    Vec::new()
                }
            })
        }
    }
    let findings = verify_dashboard(
        &SecondCardFails,
        &cards,
        core_reports::Scope::OrgWide,
        now(),
    )
    .await;
    assert_eq!(
        findings.len(),
        1,
        "one failing card, one finding: {findings:?}"
    );
    assert_eq!(findings[0].card_index, Some(1));
    assert!(
        findings[0].message.starts_with("Card 2's chart"),
        "the finding names its card: {}",
        findings[0].message
    );
}

/// The classes that need no run are caught before it: a chart hint with no
/// dimension, and a window wider than one refresh reaches.
#[tokio::test]
async fn the_contract_classes_are_caught_without_a_run() {
    let mut no_dimension = ticket_bar();
    no_dimension.group_by.clear();
    let findings = verify_candidate(
        &NeverCalledVerifier,
        &no_dimension,
        &thirty_day_card(),
        core_reports::Scope::OrgWide,
        now(),
        None,
    )
    .await;
    assert_eq!(findings[0].kind, "viz_contradicts_data", "{findings:?}");

    let mut narrow_reach = thirty_day_card();
    narrow_reach.refresh = CardRefresh {
        every_minutes: 60,
        reach_days: 7,
    };
    let findings = verify_candidate(
        &NeverCalledVerifier,
        &ticket_bar(),
        &narrow_reach,
        core_reports::Scope::OrgWide,
        now(),
        None,
    )
    .await;
    assert_eq!(
        findings[0].kind, "window_wider_than_refresh",
        "{findings:?}"
    );
}

/// The empty-card refusal keeps the contract's own slug and names the card.
#[test]
fn the_card_contract_refusal_becomes_a_finding_with_its_own_slug() {
    let error = ReportsError::EmptyCard {
        shape: CardShape::Chart,
        failure: core_reports::CardFailure::NoRows,
    };
    let finding = finding_from_reports_error(Some(2), &error);
    assert_eq!(finding.kind, "no_rows");
    assert_eq!(finding.card_index, Some(2));
    assert!(
        finding.message.starts_with("Card 3's chart has no rows"),
        "{}",
        finding.message
    );
}

/// The measurement reads the loop's own record: one failed draft, repaired in
/// one round, counted by class.
#[test]
fn the_verification_measurement_counts_what_the_loop_recorded() {
    let failure = no_rows(None);
    let repaired = ReportAuthoringOutcome {
        draft: None,
        issues: Vec::new(),
        tool_calls: Vec::new(),
        steps: 2,
        prompt_tokens: 0,
        completion_tokens: 0,
        transport_error: None,
        verification: DraftVerification {
            ran: true,
            checked: verification_checks(),
            attempts: vec![
                VerificationAttempt {
                    round: 0,
                    findings: vec![failure.clone()],
                },
                VerificationAttempt {
                    round: 1,
                    findings: Vec::new(),
                },
            ],
            repaired: vec![failure],
            findings: Vec::new(),
        },
    };
    let unfixed = ReportAuthoringOutcome {
        draft: None,
        issues: Vec::new(),
        tool_calls: Vec::new(),
        steps: 1,
        prompt_tokens: 0,
        completion_tokens: 0,
        transport_error: None,
        verification: DraftVerification {
            ran: true,
            checked: verification_checks(),
            attempts: vec![VerificationAttempt {
                round: 0,
                findings: vec![no_rows(Some(1))],
            }],
            repaired: Vec::new(),
            findings: vec![no_rows(Some(1))],
        },
    };
    let measured = measure_verification(&[repaired, unfixed]);
    assert_eq!(measured.drafts, 2);
    assert_eq!(measured.failed, 2);
    assert_eq!(measured.repaired, 1);
    assert_eq!(measured.unrepairable, 1);
    assert_eq!(measured.by_class.get("no_rows"), Some(&2));
    assert_eq!(measured.rounds_to_repair, vec![2]);
}
