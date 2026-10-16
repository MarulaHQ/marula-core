//! Unit tests for the workflow authoring mechanism. Pure functions are tested
//! directly; the loop is tested against the deterministic
//! `chat_runner::ScriptedTransport` (no HTTP, no database, no provider).

use super::*;
use crate::routes::chat_runner::{ModelTurn, ScriptedTransport, ToolCallReq};
use serde_json::json;

/// A minimal document that validates: one `flow.stop` entry node.
fn minimal_document() -> Value {
    json!({
        "version": "1",
        "nodes": [{ "id": "end", "node_type": "flow.stop", "params": {} }],
        "edges": []
    })
}

/// A realistic two-node document: raise priority, then notify.
fn two_node_document() -> Value {
    json!({
        "version": "1",
        "nodes": [
            { "id": "raise", "node_type": "ticket.setPriority", "params": { "priority": "p1" } },
            { "id": "notify", "node_type": "notification.send", "params": {
                "userIds": ["00000000-0000-0000-0000-000000000001"],
                "kind": "escalation",
                "title": "P1 ticket raised"
            } }
        ],
        "edges": [{ "from": "raise", "to": "notify" }]
    })
}

fn envelope(document: Value) -> String {
    json!({
        "name": "P1 escalation",
        "trigger_event": "ticket.created",
        "document": document
    })
    .to_string()
}

#[test]
fn minimal_and_two_node_documents_validate_against_the_real_catalogue() {
    assert!(validate_candidate(&minimal_document()).is_empty());
    assert!(validate_candidate(&two_node_document()).is_empty());
}

#[test]
fn extract_handles_fences_verbatim_and_surrounding_prose() {
    let doc = minimal_document();
    // Fenced.
    let fenced = format!("Here you go:\n```json\n{}\n```\nThanks!", doc);
    assert_eq!(extract_json_object(&fenced), Some(doc.clone()));
    // Verbatim.
    assert_eq!(extract_json_object(&doc.to_string()), Some(doc.clone()));
    // Prose around a bare object.
    let prose = format!("Sure. {} That's the workflow.", doc);
    assert_eq!(extract_json_object(&prose), Some(doc));
    // No JSON at all.
    assert_eq!(extract_json_object("I cannot build that."), None);
}

#[test]
fn extract_ignores_braces_inside_strings() {
    let value = json!({ "title": "a } brace inside", "n": 1 });
    let raw = format!("prefix {value} suffix");
    assert_eq!(extract_json_object(&raw), Some(value));
}

#[test]
fn split_envelope_accepts_a_bare_document() {
    let (envelope, document) = split_envelope(minimal_document());
    assert!(envelope.name.is_none());
    assert_eq!(document, Some(minimal_document()));
}

#[test]
fn split_envelope_reads_name_trigger_and_document() {
    let value = json!({
        "name": "n",
        "trigger_event": "ticket.created",
        "document": minimal_document()
    });
    let (envelope, document) = split_envelope(value);
    assert_eq!(envelope.name.as_deref(), Some("n"));
    assert_eq!(envelope.trigger_event.as_deref(), Some("ticket.created"));
    assert_eq!(document, Some(minimal_document()));
}

#[test]
fn parse_output_reports_typed_issues() {
    // Valid → no issues.
    let parsed = parse_authoring_output(&envelope(minimal_document()));
    assert!(parsed.document.is_some());
    assert!(parsed.issues.is_empty());

    // Unknown node type → unknown_node_type, named.
    let bad = json!({
        "document": { "version": "1",
            "nodes": [{ "id": "a", "node_type": "ticket.doesNotExist", "params": {} }],
            "edges": [] }
    });
    let parsed = parse_authoring_output(&bad.to_string());
    assert_eq!(parsed.issues.len(), 1);
    assert_eq!(parsed.issues[0].code, "unknown_node_type");
    assert_eq!(parsed.issues[0].path.as_deref(), Some("a"));

    // Missing required param → node_params.
    let bad_params = json!({
        "document": { "version": "1",
            "nodes": [{ "id": "a", "node_type": "ticket.setPriority", "params": {} }],
            "edges": [] }
    });
    let parsed = parse_authoring_output(&bad_params.to_string());
    assert_eq!(parsed.issues[0].code, "node_params");

    // Trigger in the body → refused (a trigger is a document entry, not a step).
    let trigger_body = json!({
        "document": { "version": "1",
            "nodes": [{ "id": "a", "node_type": "trigger.ticket", "params": { "eventType": "ticket.created" } }],
            "edges": [] }
    });
    let parsed = parse_authoring_output(&trigger_body.to_string());
    assert_eq!(parsed.issues[0].code, "unknown_node_type");

    // No JSON → not_json.
    let parsed = parse_authoring_output("no json here");
    assert_eq!(parsed.issues[0].code, "not_json");
}

#[test]
fn diff_reports_added_changed_and_removed_keyed_by_identity() {
    let base = two_node_document();
    let mut draft = two_node_document();
    // Change the priority param, add a third node, drop nothing.
    draft["nodes"][0]["params"]["priority"] = json!("p2");
    draft["nodes"].as_array_mut().unwrap().push(json!({
        "id": "end", "node_type": "flow.stop", "params": {}
    }));
    draft["edges"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "from": "notify", "to": "end" }));

    let diff = diff_documents(&base, &draft);
    // raise: changed; end: added; the notify→end edge: added.
    assert!(diff
        .iter()
        .any(|d| d.op == "change" && d.path == "nodes/raise"));
    assert!(diff.iter().any(|d| d.op == "add" && d.path == "nodes/end"));
    assert!(diff
        .iter()
        .any(|d| d.op == "add" && d.path == "edges/notify→end"));

    // Re-ordering is not a change.
    let mut reordered = two_node_document();
    reordered["nodes"].as_array_mut().unwrap().reverse();
    reordered["edges"].as_array_mut().unwrap().reverse();
    assert!(diff_documents(&two_node_document(), &reordered).is_empty());
}

#[test]
fn catalogue_is_compact_and_carries_repeat_safety() {
    let catalogue = body_catalogue();
    assert!(!catalogue.is_empty());
    // No trigger nodes in the body catalogue.
    assert!(catalogue.iter().all(|n| n.family != "trigger"));
    // Every entry carries an agent description.
    assert!(catalogue
        .iter()
        .all(|n| !n.agent_description.trim().is_empty()));
    // The compact catalogue's serialized form carries NO schema text.
    let serialized = serde_json::to_string(&catalogue).unwrap();
    assert!(!serialized.contains("input_schema"));
    assert!(!serialized.contains("\"properties\""));
    // A known non-idempotent node is present with its flag.
    let notify = catalogue
        .iter()
        .find(|n| n.id == "notification.send")
        .unwrap();
    assert!(
        !notify.idempotent,
        "notification.send must be flagged not idempotent"
    );
}

#[test]
fn trigger_event_types_are_derived_from_the_registry() {
    let triggers = trigger_event_types();
    assert!(triggers.iter().any(|(id, _)| id == "trigger.ticket"));
    let ticket = triggers
        .iter()
        .find(|(id, _)| id == "trigger.ticket")
        .unwrap();
    assert!(ticket.1.iter().any(|e| e == "ticket.created"));
}

#[test]
fn node_schema_tool_returns_schema_for_known_and_typed_gap_for_unknown() {
    let ok = run_node_schema_tool(&json!({ "node_id": "ticket.update" }));
    let parsed: Value = serde_json::from_str(&ok).unwrap();
    assert_eq!(parsed["found"], json!(true));
    assert!(parsed["node"]["input_schema"].is_object());
    assert!(parsed["node"]["output_schema"].is_object());

    let missing = run_node_schema_tool(&json!({ "node_id": "does.notExist" }));
    let parsed: Value = serde_json::from_str(&missing).unwrap();
    assert_eq!(parsed["found"], json!(false));

    let empty = run_node_schema_tool(&json!({}));
    assert!(empty.contains("\"found\":false"));
}

#[tokio::test]
async fn loop_fetches_a_schema_then_emits_a_valid_draft() {
    let transport = ScriptedTransport::new(vec![
        ModelTurn {
            content: String::new(),
            tool_calls: vec![ToolCallReq {
                id: "call-1".to_string(),
                name: NODE_SCHEMA_TOOL.to_string(),
                arguments: json!({ "node_id": "ticket.setPriority" }),
            }],
            prompt_tokens: 40,
            completion_tokens: 8,
        },
        ModelTurn {
            content: envelope(two_node_document()),
            tool_calls: vec![],
            prompt_tokens: 120,
            completion_tokens: 90,
        },
    ]);
    let outcome =
        run_authoring_loop(&transport, "test-model", "raise a P1 and notify", None, 4).await;
    assert!(outcome.valid(), "issues: {:?}", outcome.issues);
    assert_eq!(outcome.steps, 2);
    assert_eq!(outcome.tool_calls.len(), 1);
    assert_eq!(outcome.tool_calls[0].name, NODE_SCHEMA_TOOL);
    assert_eq!(outcome.tool_calls[0].outcome, "ok");
    assert_eq!(outcome.prompt_tokens, 160);
    assert_eq!(outcome.completion_tokens, 98);
    assert_eq!(
        outcome.envelope.trigger_event.as_deref(),
        Some("ticket.created")
    );
}

#[tokio::test]
async fn loop_surfaces_a_transport_failure_honestly() {
    // No scripted turns → the transport reports Unavailable on the first call.
    let transport = ScriptedTransport::new(vec![]);
    let outcome = run_authoring_loop(&transport, "test-model", "anything", None, 3).await;
    assert!(!outcome.valid());
    assert!(outcome.transport_error.is_some());
}

#[test]
fn measurement_reports_valid_json_and_schema_rejection_rates() {
    let samples = vec![
        AuthoringSample {
            name: "valid".into(),
            candidate: envelope(minimal_document()),
        },
        AuthoringSample {
            name: "valid-fenced".into(),
            candidate: format!("```json\n{}\n```", envelope(two_node_document())),
        },
        AuthoringSample {
            name: "unknown-node".into(),
            candidate: json!({ "document": { "version": "1",
                "nodes": [{ "id": "a", "node_type": "ticket.nope", "params": {} }], "edges": [] } })
            .to_string(),
        },
        AuthoringSample {
            name: "prose-only".into(),
            candidate: "I cannot do that.".into(),
        },
    ];
    let m = measure_corpus(&samples);
    assert_eq!(m.total, 4);
    assert_eq!(m.json_extracted, 3);
    assert_eq!(m.schema_valid, 2);
    assert_eq!(m.schema_rejected, 1);
    assert!((m.valid_json_rate - 0.75).abs() < 1e-9);
    assert!((m.schema_rejection_rate - 0.25).abs() < 1e-9);
}

#[test]
fn prompt_footprint_shows_the_on_demand_saving() {
    let (compact, inline, nodes) = measure_prompt_footprint();
    assert!(nodes > 0);
    assert!(compact > 0);
    assert!(
        inline > compact,
        "inlining every schema must cost more than the compact catalogue (compact={compact}, inline={inline})"
    );
    // The compact catalogue must be less than the inlined schemas alone.
    assert!(compact < inline);
}
