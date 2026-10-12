//! Registry conformance tests.
//!
//! These are the tests that keep the registry a single source of truth
//! rather than a long list: every declaration must be well-formed, every
//! schema must be inside the declared subset, every Tier A wrapper must
//! ride a real closed operation with the right idempotence and emit seam,
//! and the `AutomationAction` variants the rule builder already has must
//! not be able to drift away from their registry node.

use serde_json::json;

use super::*;

// ---------------------------------------------------------------------------
// Declaration conformance
// ---------------------------------------------------------------------------

#[test]
fn every_node_declares_the_full_contract() {
    for node in all() {
        assert!(!node.id.is_empty(), "{node:?} has no id");
        assert!(!node.label.is_empty(), "{node:?} has no label");
        assert!(
            node.agent_description.len() >= 20,
            "{node:?} has no agent-oriented description"
        );
        assert!(
            node.agent_description.len() >= 20,
            "{}'s description is too short to be useful to a model",
            node.id
        );
        // The declared schemas must be real JSON Schema, not prose.
        parse_schema(node.input_schema).expect("{} input schema must parse");
        parse_schema(node.output_schema).expect("{} output schema must parse");
    }
}

#[test]
fn node_ids_are_unique_and_namespaced() {
    let mut seen: Vec<&str> = Vec::new();
    for node in all() {
        assert!(
            !seen.contains(&node.id),
            "duplicate node id {:?} — the registry is keyed by id",
            node.id
        );
        seen.push(node.id);
        assert!(
            node.id.contains('.'),
            "{:?} is not namespaced; ids read as `family.operation`",
            node.id
        );
    }
}

#[test]
fn the_catalogue_covers_every_family_and_tier() {
    let families = [
        NodeFamily::Trigger,
        NodeFamily::Decision,
        NodeFamily::Condition,
        NodeFamily::Action,
        NodeFamily::Control,
        NodeFamily::Value,
    ];
    for family in families {
        assert!(
            all().iter().any(|n| n.family == family),
            "no {family:?} node is declared"
        );
    }
    for tier in [NodeTier::A, NodeTier::B, NodeTier::C] {
        assert!(
            all().iter().any(|n| n.tier == tier),
            "no {tier:?} node is declared"
        );
    }
}

#[test]
fn the_catalogue_is_narrow() {
    // The catalogue's spine is Tier A; Tier B exists to avoid a second
    // declaration per domain operation, and Tier C is the plugin slots. If
    // the count grows past the recorded shape, the catalogue is drifting
    // toward the 200-node list the reference document rejects.
    assert_eq!(all().len(), 125, "catalogue size changed — record why");
    assert_eq!(tier_a().len(), 121);
}

#[test]
fn get_and_require_resolve_the_same_node() {
    for node in all() {
        assert_eq!(get(node.id), Some(node));
        assert_eq!(require(node.id).unwrap().id, node.id);
    }
    assert!(get("nope").is_none());
    assert_eq!(
        require("nope"),
        Err(NodeRegistryError::UnknownNode { id: "nope".to_string() })
    );
}

// ---------------------------------------------------------------------------
// The declared JSON-Schema subset
// ---------------------------------------------------------------------------

#[test]
fn every_declared_schema_stays_inside_the_subset() {
    // Unsupported keywords are refused rather than ignored: a schema the
    // validator cannot fully honour must not ship as if it could. The
    // same gate runs on plugin entries (see the manifest validation).
    for node in all() {
        for (field, raw) in [("input", node.input_schema), ("output", node.output_schema)] {
            let schema = parse_schema(raw).expect("{} {field} must parse");
            validate_declared_schema(&schema).unwrap_or_else(|v| {
                panic!("{}::{field} declares an unsupported keyword: {v}", node.id)
            });
        }
    }
}

#[test]
fn the_subset_gate_refuses_a_schema_the_validator_cannot_honour() {
    // A keyword the validator would silently ignore is a contract that
    // lies, so declaring one is refused.
    let smuggled = json!({"type": "object", "patternProperties": {"^x": {"type": "string"}}});
    assert!(validate_declared_schema(&smuggled).is_err());
    let honest = json!({"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}}});
    assert!(validate_declared_schema(&honest).is_ok());
    // An undeclared `type` value is refused for the same reason.
    let wrong = json!({"type": "object", "properties": {"at": {"type": "datetime"}}});
    assert!(validate_declared_schema(&wrong).is_err());
}

#[test]
fn validate_params_accepts_a_well_formed_node_and_refuses_junk() {
    let ok = json!({"boardId": "3f2504e0-4f89-11d3-9a0c-0305e82c3301", "title": "Device offline"});
    assert!(validate_params("ticket.create", &ok).is_ok());

    let missing = json!({"title": "no board"});
    assert!(matches!(
        validate_params("ticket.create", &missing),
        Err(NodeRegistryError::SchemaViolation { .. })
    ));

    let wrong_type = json!({"boardId": 7, "title": "x"});
    assert!(validate_params("ticket.create", &wrong_type).is_err());

    // Rule (a) in force: an undeclared key is refused, so params cannot
    // smuggle a second channel.
    let smuggled = json!({"boardId": "3f2504e0-4f89-11d3-9a0c-0305e82c3301", "title": "x", "script": "rm -rf"});
    assert!(validate_params("ticket.create", &smuggled).is_err());

    assert!(matches!(
        validate_params("not.a.node", &json!({})),
        Err(NodeRegistryError::UnknownNode { .. })
    ));
}

#[test]
fn enums_and_formats_are_enforced() {
    assert!(validate_params("ticket.setPriority", &json!({"ticketId": "3f2504e0-4f89-11d3-9a0c-0305e82c3301", "priority": "p1"})).is_ok());
    assert!(validate_params("ticket.setPriority", &json!({"ticketId": "3f2504e0-4f89-11d3-9a0c-0305e82c3301", "priority": "p9"})).is_err());
    assert!(validate_params("ticket.setPriority", &json!({"ticketId": "not-a-uuid", "priority": "p1"})).is_err());
}

// ---------------------------------------------------------------------------
// Idempotence, the retry refusal and the emit seam
// ---------------------------------------------------------------------------

#[test]
fn every_node_declares_idempotence_explicitly() {
    // The point is that the flag is a decision, not a default: a model
    // needs to know whether repeating a step is safe, and the executor
    // needs it for retry semantics.
    for node in all() {
        let _ = node.idempotent;
        if let ExecutorPath::DomainWrapper(op) = node.executor {
            assert_eq!(
                node.idempotent,
                op.idempotent(),
                "{} disagrees with {}'s idempotence",
                node.id,
                op.as_str()
            );
        }
    }
}

#[test]
fn creates_sends_and_ledger_moves_are_not_idempotent() {
    for id in [
        "ticket.create",
        "ticket.comment",
        "stockItem.move",
        "email.send",
        "decision.ask",
        "ai.completion",
    ] {
        assert!(!get(id).unwrap().idempotent, "{id} must be unsafe to repeat");
    }
    for id in [
        "ticket.update",
        "ticket.transition",
        "ticket.setPriority",
        "domain.query",
        "flow.delay",
    ] {
        assert!(get(id).unwrap().idempotent, "{id} must be safe to repeat");
    }
}

#[test]
fn retry_refusal_names_the_node_it_refuses() {
    assert!(retry_refusal("ticket.update", &json!({})).is_ok());
    let refused = retry_refusal("decision.ask", &json!({}));
    assert_eq!(
        refused,
        Err(NodeRegistryError::RetryRefused {
            id: "decision.ask".to_string()
        })
    );
    // The refusal is per node, not a global panic.
    assert!(retry_refusal("flow.retry", &json!({"maxAttempts": 3})).is_ok());
}

#[test]
fn every_tier_a_action_emits_the_event_its_operation_produces() {
    let mut checked = 0;
    for node in all() {
        if let ExecutorPath::DomainWrapper(op) = node.executor {
            let expected = op.emitted_event();
            let declared = node.emits;
            if expected.is_empty() {
                assert!(
                    declared.is_empty(),
                    "{} declares emits for an operation with no event",
                    node.id
                );
            } else {
                assert!(
                    declared.contains(&expected),
                    "{} must emit {} but declares {:?}",
                    node.id,
                    expected,
                    declared
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 69, "every closed operation must have a wrapper");
}

#[test]
fn domain_update_resolves_its_seam_and_flag_per_operation() {
    // Tier B rides the same emit seam as Tier A — that is what makes the
    // loop refusal complete rather than approximate.
    let create = json!({"operation": "ticket.create"});
    assert_eq!(emitted_event_types("domain.update", &create), vec!["ticket.created"]);
    assert_eq!(retry_refusal("domain.update", &create), Err(NodeRegistryError::RetryRefused { id: "domain.update".to_string() }));

    let update = json!({"operation": "ticket.update"});
    assert_eq!(emitted_event_types("domain.update", &update), vec!["ticket.updated"]);
    assert!(retry_refusal("domain.update", &update).is_ok());

    // An operation with no declared event can never be statically linked
    // to a wait — the refusal stays conservative by construction.
    let sms = json!({"operation": "sms.send"});
    assert!(emitted_event_types("domain.update", &sms).is_empty());
}

#[test]
fn check_loop_safety_refuses_a_workflow_that_can_resume_itself() {
    let wait = json!({"eventTypes": ["ticket.status_changed"]});
    let notify = json!({"userIds": []});
    let safe = [LoopCheckNode::first_party("flow.waitFor", &wait), LoopCheckNode::first_party("notification.send", &notify)];
    assert!(check_loop_safety("flow.waitFor", &["ticket.status_changed".to_string()], &safe).is_ok());

    // Tier A, through its own declaration.
    let close = json!({"ticketId": "3f2504e0-4f89-11d3-9a0c-0305e82c3301", "status": "closed"});
    let refused = check_loop_safety(
        "flow.waitFor",
        &["ticket.status_changed".to_string()],
        &[LoopCheckNode::first_party("flow.waitFor", &wait), LoopCheckNode::first_party("ticket.transition", &close)],
    );
    assert_eq!(
        refused,
        Err(NodeRegistryError::LoopRefusal {
            waiting: "flow.waitFor".to_string(),
            event: "ticket.status_changed".to_string(),
            producing: "ticket.transition".to_string(),
        })
    );

    // Tier B, resolved through the selected operation.
    let refused_b = check_loop_safety(
        "flow.waitFor",
        &["ticket.status_changed".to_string()],
        &[
            LoopCheckNode::first_party("flow.waitFor", &wait),
            LoopCheckNode::first_party("domain.update", &json!({"operation": "ticket.transition"})),
        ],
    );
    assert!(refused_b.is_err());

    // Tier C: the caller supplies the entry's declared emits, and the
    // refusal sees them. This is the deadlock the catalogue names.
    let plugin_emits = vec!["ticket.status_changed".to_string()];
    let refused_c = check_loop_safety(
        "flow.waitFor",
        &["ticket.status_changed".to_string()],
        &[
            LoopCheckNode::first_party("flow.waitFor", &wait),
            LoopCheckNode {
                id: "plugin.acme.push_ticket",
                params: &json!({}),
                plugin_emits: &plugin_emits,
            },
        ],
    );
    assert_eq!(
        refused_c,
        Err(NodeRegistryError::LoopRefusal {
            waiting: "flow.waitFor".to_string(),
            event: "ticket.status_changed".to_string(),
            producing: "plugin.acme.push_ticket".to_string(),
        })
    );

    // An event nobody produces is never refused, and the waiting node
    // itself never counts as its own producer.
    let clock = json!({"duration": "2h"});
    assert!(check_loop_safety(
        "flow.waitFor",
        &["ticket.status_changed".to_string()],
        &[LoopCheckNode::first_party("flow.waitFor", &wait), LoopCheckNode::first_party("flow.delay", &clock)],
    )
    .is_ok());
}

// ---------------------------------------------------------------------------
// The Decision Engine posture
// ---------------------------------------------------------------------------

#[test]
fn the_internal_decision_surfaces_are_unnamable() {
    // The six user-facing surfaces are the only ones the enum can name;
    // the four internal ones and the two engine surfaces are refused by
    // structure rather than by a check someone has to remember.
    let schema = output_schema("decision.ask");
    assert!(schema.is_some());
    let declaration = get("decision.ask").unwrap().input_schema;
    for internal in [
        "decision_search_rerank",
        "decision_llm_verify",
        "decision_tool_routing",
        "decision_loop_termination",
        "decision_automation_conditions",
        "decision_workbench",
    ] {
        assert!(
            !declaration.contains(internal),
            "{internal} must not be nameable from decision.ask"
        );
    }
    for offered in [
        "decision_ticket_triage",
        "decision_alert_triage",
        "decision_dispatch",
        "decision_billing_audit",
        "decision_contract_scope",
        "decision_record_dedupe",
    ] {
        assert!(
            declaration.contains(offered),
            "{offered} is offered and must be nameable"
        );
    }
    assert!(validate_params("decision.ask", &json!({"surface": "decision_tool_routing", "questionSet": {"source": "workflow"}})).is_err());
    assert!(validate_params("decision.ask", &json!({"surface": "decision_ticket_triage", "questionSet": {"source": "workflow"}})).is_ok());
}

// ---------------------------------------------------------------------------
// Outputs are referenceable, which is what makes a node usable downstream
// ---------------------------------------------------------------------------

#[test]
fn actions_and_decisions_declare_referenceable_outputs() {
    for id in [
        "ticket.create",
        "domain.query",
        "domain.update",
        "decision.ask",
        "plugin.read",
        "value.math",
    ] {
        assert!(
            has_referenceable_output(id).unwrap(),
            "{id} must declare an output schema a downstream node can reference"
        );
    }
    // `value.literal` echoes whatever it is handed, so it has no declared
    // shape to reference — and `has_referenceable_output` says so honestly
    // rather than promising a field that is not there.
    assert!(has_referenceable_output("value.literal").unwrap());
    assert!(matches!(
        has_referenceable_output("nope"),
        Err(NodeRegistryError::UnknownNode { .. })
    ));
}

// ---------------------------------------------------------------------------
// The palette and the agent catalogue
// ---------------------------------------------------------------------------

#[test]
fn the_palette_is_tier_ordered_and_human_visible() {
    let palette = palette();
    assert_eq!(palette.len(), all().len(), "first-party nodes are all human-visible");
    let mut last = NodeTier::A;
    for node in palette {
        assert!(node.audience.includes_human());
        assert!(node.tier >= last, "the palette must list Tier A first");
        last = node.tier;
    }
}

#[test]
fn the_agent_catalogue_is_the_ai_visible_slice() {
    let agent = agent_catalogue();
    assert!(agent.iter().any(|n| n.id == "decision.ask"));
    assert!(agent.iter().any(|n| n.id == "ticket.create"));
    assert!(agent.iter().all(|n| n.audience.includes_ai()));
}

#[test]
fn audience_parsing_is_total() {
    assert_eq!(NodeAudience::parse("human"), Some(NodeAudience::Human));
    assert_eq!(NodeAudience::parse("ai"), Some(NodeAudience::Ai));
    assert_eq!(NodeAudience::parse("both"), Some(NodeAudience::Both));
    assert_eq!(NodeAudience::parse("everyone"), None);
    assert!(!NodeAudience::Human.includes_ai());
    assert!(NodeAudience::Both.includes_human());
}

// ---------------------------------------------------------------------------
// The plugin node contract (A2)
// ---------------------------------------------------------------------------

#[test]
fn plugin_node_kinds_map_from_the_surface_vocabulary_only() {
    assert_eq!(PluginNodeKind::from_surface("automation_action"), Some(PluginNodeKind::Action));
    assert_eq!(PluginNodeKind::from_surface("read"), Some(PluginNodeKind::Read));
    // The UI kinds are not workflow nodes and never become one.
    assert_eq!(PluginNodeKind::from_surface("widget"), None);
    assert_eq!(PluginNodeKind::from_surface("report_render"), None);
    assert_eq!(PluginNodeKind::from_surface("field_validator"), None);
}

#[test]
fn a_plugin_trigger_is_an_event_type_not_a_node() {
    // Two facts, deliberately not one. `PluginNodeKind::Trigger` is
    // REPRESENTABLE because lane 4 admitted the surface kind and a
    // contract that could not describe it would be a gap; but no
    // first-party path produces one, because a plugin trigger arrives
    // through lane 1's signed ingest as an event type lane 2's registry
    // makes subscribable. The second fact lives in the persistence layer:
    // `plugin_surface::NODE_SURFACES` omits `automation_trigger`, so the
    // read endpoint never hands out a trigger "node". If that list ever
    // grows to include it, this lane's decision was overturned and the
    // change should say so.
    assert_eq!(
        PluginNodeKind::from_surface("automation_trigger"),
        Some(PluginNodeKind::Trigger)
    );
    assert!(
        !all().iter().any(|n| n.id.starts_with("plugin.trigger")),
        "a plugin trigger must not be a first-party node"
    );
    // `trigger.webhook` is the first-party slot an external event rides,
    // not a plugin node.
    assert!(get("trigger.webhook").is_some());
}

#[test]
fn the_plugin_contract_defaults_point_the_conservative_way() {
    let definition = json!({"params_schema": {"type":"object","properties":{"deviceId":{"type":"string"}},"required":["deviceId"]}});
    let contract = PluginNodeContract::parse(
        "read",
        "plugin.acme-rmm.device",
        "Read device",
        &definition,
        Some(json!({"type":"object","properties":{"siteId":{"type":"string"}}})),
        false,
        &[],
        "human",
        None,
    )
    .unwrap();

    assert_eq!(contract.kind, PluginNodeKind::Read);
    assert!(!contract.idempotent, "an undeclared plugin action is retry-unsafe");
    assert_eq!(contract.audience, NodeAudience::Human, "a silent plugin stays off the model's view");
    assert!(contract.agent_description.is_none());
    // The human label stands in for a missing agent description, so the
    // catalogue never renders a blank row.
    assert_eq!(contract.as_node_type().agent_description, "Read device");

    let bad_params = json!({"deviceId": 4});
    assert!(contract.validate_params(&bad_params).is_err());
    let good_params = json!({"deviceId": "abc"});
    assert!(contract.validate_params(&good_params).is_ok());
}

#[test]
fn the_plugin_contract_refuses_a_ui_surface_and_a_bad_audience() {
    let definition = json!({});
    assert!(PluginNodeContract::parse("widget", "plugin.acme.widget", "Widget", &definition, None, true, &[], "human", None).is_err());
    assert!(PluginNodeContract::parse("automation_action", "plugin.acme.act", "Act", &definition, None, true, &[], "everyone", None).is_err());
}

#[test]
fn a_plugin_entry_becomes_a_registry_shaped_node() {
    let definition = json!({});
    let contract = PluginNodeContract::parse(
        "automation_action",
        "plugin.acme.push_ticket",
        "Push ticket",
        &definition,
        Some(json!({"type":"object","properties":{"remoteId":{"type":"string"}}})),
        true,
        &["rmm.ticket_pushed".to_string()],
        "both",
        Some("Pushes the ticket's summary to the vendor portal."),
    )
    .unwrap();
    let node = contract.declaration().into_owned();
    assert_eq!(node.id, "plugin.acme.push_ticket");
    assert_eq!(node.tier, NodeTier::C);
    assert_eq!(node.family, NodeFamily::Action);
    assert!(node.idempotent);
    assert_eq!(node.emits, ["rmm.ticket_pushed"]);
    // A plugin's emits reach the loop refusal through LoopCheckNode, the
    // seam the caller fills from the install at author time.
    let plugin_emits = vec!["rmm.ticket_pushed".to_string()];
    assert_eq!(
        emitted_event_types(node.id, &json!({})),
        Vec::<&str>::new()
    );
    assert!(check_loop_safety(
        "flow.waitFor",
        &["rmm.ticket_pushed".to_string()],
        &[LoopCheckNode {
            id: "plugin.acme.push_ticket",
            params: &json!({}),
            plugin_emits: &plugin_emits,
        }]
    )
    .is_err());
}

// ---------------------------------------------------------------------------
// The AutomationAction drift guard (B)
// ---------------------------------------------------------------------------

#[test]
fn the_catalogue_digest_is_stable_and_well_formed() {
    let first = catalogue_digest();
    assert_eq!(first.len(), 64, "a sha256 hex digest");
    assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(first, catalogue_digest(), "the digest must be deterministic");
    assert!(!CATALOGUE_VERSION.is_empty());
}

#[test]
fn automation_action_variants_cannot_drift_from_the_registry() {
    // The five built-in rule effects are Tier A nodes here, and the plugin
    // effect is the Tier C slot. Adding an `AutomationAction` variant
    // without an entry in AUTOMATION_ACTION_NODES fails this test.
    let variants = [
        "assign_user",
        "set_priority",
        "add_internal_comment",
        "notify_users",
        "plugin_action",
    ];
    assert_eq!(variants.len(), AUTOMATION_ACTION_NODES.len());
    for tag in variants {
        let node_id = automation_action_node(tag).expect("the rule engine has a node");
        assert!(
            get(node_id).is_some(),
            "{tag} maps onto {node_id}, which the registry does not declare"
        );
    }
    assert!(automation_action_node("delete_everything").is_none());
}
