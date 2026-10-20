//! "Describe it" → a valid workflow (the private build tracker Wave 43 lane 17,
//! `p43-workflow-ai-authoring`).
//!
//! One natural-language entry point that emits the SAME workflow JSON the
//! canvas edits, through the SAME validated boundary, so a described
//! workflow is never a privileged path around the schema.
//!
//! # What this is, and what it is not
//!
//! - **Same JSON, same boundary.** The model emits a workflow document; it
//!   is parsed and validated by `core_domain::workflow_document::WorkflowDocument`
//!   — the exact function lane 16's save route (`workflow_documents::create` /
//!   `update`) calls before storage. A draft that does not validate comes
//!   back with typed issues, never a half-written row.
//! - **A draft, never a write (spec D).** This endpoint persists NOTHING. It
//!   returns the proposed document, the issues, and a structural diff against
//!   the current document. Saving is the caller's job, through lane 16's route,
//!   which re-validates.
//! - **Schemas on demand (spec B).** The system prompt carries a COMPACT
//!   catalogue — each node's id, label, agent-oriented description, `idempotent`
//!   flag and emitted events — and NO JSON Schema. The model calls the
//!   `get_node_schema` tool for the specific nodes it needs. This is Windmill's
//!   `get_runnable_details` pattern: the prompt stays roughly constant as the
//!   catalogue grows, instead of growing with it.
//! - **Repeat safety (spec C).** `agent_description` and `idempotent` ride the
//!   catalogue (ADR 0016 decision 8, Activepieces' `aiMetadata`). `idempotent`
//!   is the same flag lane 15's executor consumes for retry refusal
//!   (`node_registry::retry_refusal`, ADR 0017 §Dependencies), so the model
//!   reasons about repeat safety with the same fact the engine enforces.
//! - **The existing admission path (spec F).** The model call rides
//!   `ai_policy::authorize_surface_call` with the completion + tool-calling
//!   edges — the same caps, grants, allowlist and sealed credential the chat
//!   runner uses, through `chat_runner::resolve_transport`. There is no second
//!   AI path; `surface_key` is caller-supplied and the org grants it, exactly
//!   as `general_chat` works.
//!
//! # Shared seam
//!
//! Lane 17b (`p43-automation-builder`) consumes this endpoint for the simple
//! tier rather than building a second prose path; the request/response shape is
//! the frozen contract posted on the board before this module was written.

use axum::{extract::State, http::Extensions, Json};
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
use crate::state::AppState;
use core_db::ai_policy::{record_usage, AiUsageNew};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::node_registry::{self, NodeFamily, NodeType};
use core_domain::policy::{Action, Module, Permission};
use core_domain::workflow_document::{WorkflowDocError, WorkflowDocument};

/// The most model turns one authoring request may take. A tool call and a
/// final answer fit comfortably; a runaway that keeps asking for schemas is a
/// bug, not a strategy (same posture as the chat runner's loop bound).
pub const MAX_AUTHORING_STEPS: usize = 6;

/// The name of the on-demand schema tool. Stable: it is the tool id the model
/// calls back on.
pub const NODE_SCHEMA_TOOL: &str = "get_node_schema";

/// The document version new drafts are emitted against. Mirrors
/// `core_domain::workflow_document::DOCUMENT_VERSION`.
pub const AUTHORED_DOCUMENT_VERSION: &str = "1";

// ---------------------------------------------------------------------------
// The registry-derived compact catalogue (spec B + C)
// ---------------------------------------------------------------------------

/// One node as the authoring prompt sees it: NO schemas. Schemas are fetched
/// per node, on demand, through [`NODE_SCHEMA_TOOL`]. `idempotent` and
/// `agent_description` ride here so the model can reason about repeat safety
/// before it ever fetches a schema.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CatalogueNode {
    pub id: String,
    pub label: String,
    pub agent_description: String,
    pub family: String,
    pub tier: String,
    pub idempotent: bool,
    pub emits: Vec<String>,
    /// Whether the executor can run this node today (Wave 43 lane 20, item
    /// C). The authoring model is told, so it does not place a node the
    /// validator will refuse.
    pub executable: bool,
}

impl From<&NodeType> for CatalogueNode {
    fn from(n: &NodeType) -> Self {
        Self {
            id: n.id.to_string(),
            label: n.label.to_string(),
            agent_description: n.agent_description.to_string(),
            family: n.family.as_str().to_string(),
            tier: n.tier.as_str().to_string(),
            idempotent: n.idempotent,
            emits: n.emits.iter().map(|e| e.to_string()).collect(),
            executable: n.executable,
        }
    }
}

/// The body catalogue: every node the model may place in a document BODY.
/// Triggers are excluded because a trigger is a document ENTRY
/// (`WorkflowDocument::validate` refuses a trigger node in the body) — the
/// trigger is the rule's `trigger_event`, so it is surfaced separately by
/// [`trigger_event_types`].
pub fn body_catalogue() -> Vec<CatalogueNode> {
    let mut nodes: Vec<CatalogueNode> = node_registry::agent_catalogue()
        .into_iter()
        .filter(|n| n.family != NodeFamily::Trigger)
        // Only nodes the executor can actually run (Wave 43 lane 20, item
        // C): offering the model a node that `validate` refuses would be a
        // guaranteed rejection, not a choice.
        .filter(|n| n.executable)
        .map(CatalogueNode::from)
        .collect();
    nodes.sort_by(|a, b| a.family.cmp(&b.family).then_with(|| a.id.cmp(&b.id)));
    nodes
}

/// Every trigger node the model may start from, with the event types it can
/// subscribe to, derived from the trigger node's own input schema
/// (`properties.eventType.enum`). One source of truth: the registry.
pub fn trigger_event_types() -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for node in node_registry::agent_catalogue()
        .into_iter()
        .filter(|n| n.family == NodeFamily::Trigger)
    {
        let events = node_registry::input_schema(node.id)
            .and_then(|schema| {
                schema
                    .get("properties")
                    .and_then(|p| p.get("eventType"))
                    .and_then(|e| e.get("enum"))
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
            })
            .unwrap_or_default();
        out.push((node.id.to_string(), events));
    }
    out
}

// ---------------------------------------------------------------------------
// The prompt (compact) and the on-demand tool
// ---------------------------------------------------------------------------

/// The OpenAI-format tool definition for the on-demand schema fetch. Mirrors
/// the shape `chat_runner::run_turn` passes (`{name, description, parameters}`);
/// the transport wraps it as `{"type":"function","function":…}`.
pub fn node_schema_tool() -> Value {
    json!({
        "name": NODE_SCHEMA_TOOL,
        "description": "Return the exact input and output JSON Schema for one workflow \
                        node type by its id. Call this before placing a node whose params \
                        you are unsure of — the schemas are deliberately NOT included in \
                        this prompt, so fetch only the nodes you need.",
        "parameters": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "node_id": {
                    "type": "string",
                    "description": "A node type id from the catalogue, e.g. ticket.update or flow.branch."
                }
            },
            "required": ["node_id"]
        }
    })
}

/// The system prompt: the authoring contract plus the compact catalogue. No
/// schemas — their absence is the point (spec B).
pub fn authoring_system_prompt() -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You author Marula workflows from a plain-English description.\n\
         \n\
         Emit ONLY a JSON object, no prose and no code fences, shaped:\n\
         {\"name\": <string>, \"trigger_event\": <string>, \"document\": {\n\
           \"version\": \"1\",\n\
           \"nodes\": [{\"id\": <string>, \"node_type\": <catalogue id>, \"params\": <object>}],\n\
           \"edges\": [{\"from\": <node id>, \"to\": <node id>, \"label\": <string, optional>}]\n\
         }}\n\
         \n\
         Rules the validator enforces (a draft that breaks one comes back rejected):\n\
         - `document.nodes` must NOT contain a trigger node; the trigger is \
           `trigger_event`.\n\
         - Every node id is unique; every edge endpoint must name a node; no \
           self-loops.\n\
         - Exactly one entry node (a node with no incoming edge); every node \
           reachable from it.\n\
         - Params must validate against the node's input schema. Call \
           get_node_schema for any node whose params you are not certain of.\n\
         - The document is DATA, never code: no expression strings, no scripts.\n\
         - Every node you place must be one the executor can run. The \
           catalogue below lists ONLY runnable nodes; a node outside it is \
           refused with `node_not_executable`.\n\
         - To feed one step's output into a later step's input, put a value \
           reference in the slot: {\"$ref\": {\"node\": <upstream node id>, \
           \"path\": <dotted output field>}}. The target must be UPSTREAM and \
           its output schema must declare the path.\n\
         - `idempotent: false` nodes are not safe to repeat — do not build a \
           retry loop around them.\n\
         \n\
         When unsure of a node's exact params, call get_node_schema(node_id) \
         first; you may call it for several nodes before answering.\n\
         \n",
    );

    prompt.push_str("BODY NODE CATALOGUE (id — label — repeat-safety — what it does):\n");
    for node in body_catalogue() {
        prompt.push_str(&format!(
            "- {} — {} — {} — {}\n",
            node.id,
            node.label,
            if node.idempotent {
                "idempotent"
            } else {
                "NOT idempotent"
            },
            node.agent_description
        ));
    }

    prompt.push_str("\nTRIGGER EVENT TYPES (pick one for `trigger_event`):\n");
    for (node, events) in trigger_event_types() {
        if events.is_empty() {
            continue;
        }
        prompt.push_str(&format!("- {}: {}\n", node, events.join(", ")));
    }
    prompt
}

/// Runs the on-demand schema tool for one call, returning the tool-result
/// string the loop feeds back to the model. Pure and synchronous — the schema
/// is a static registry declaration, so there is no I/O (and no second HTTP
/// hop for what the process already holds).
pub fn run_node_schema_tool(args: &Value) -> String {
    let id = args
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if id.is_empty() {
        return json!({ "found": false, "error": "node_id is required" }).to_string();
    }
    match node_registry::get(id) {
        Some(node) => {
            let view = crate::routes::workflow_nodes::NodeView::from(node);
            json!({ "found": true, "node": view }).to_string()
        }
        None => json!({
            "found": false,
            "error": format!("unknown node type {id:?}; use an id from the catalogue")
        })
        .to_string(),
    }
}

// ---------------------------------------------------------------------------
// Output parsing — lenient extraction, strict validation (spec A)
// ---------------------------------------------------------------------------

/// Extracts the first JSON value from raw model content: a fenced code block
/// if present, else the whole content, else the first balanced `{…}` run. The
/// extraction is lenient on purpose — the VALIDATION is not. Pure.
pub fn extract_json_object(content: &str) -> Option<Value> {
    // 1. Fenced blocks first: ```json … ``` or ``` … ```.
    if let Some(rest) = content.split("```").nth(1) {
        let inner = rest
            .split("```")
            .next()
            .unwrap_or(rest)
            .trim_start_matches("json")
            .trim();
        if let Ok(value) = serde_json::from_str::<Value>(inner) {
            return Some(value);
        }
    }
    // 2. The whole content verbatim.
    let trimmed = content.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    // 3. The first balanced brace run (skips leading/trailing prose, and
    //    respects strings + escapes so a `}` inside a string never ends it).
    let bytes = content.as_bytes();
    let start = bytes.iter().position(|b| *b == b'{')?;
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, b) in bytes.iter().enumerate().skip(start) {
        if escaped {
            escaped = false;
            continue;
        }
        match *b {
            b'\\' if in_string => escaped = true,
            b'"' => in_string = !in_string,
            b'{' if !in_string => depth += 1,
            b'}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    let slice = &content[start..=i];
                    return serde_json::from_str::<Value>(slice).ok();
                }
            }
            _ => {}
        }
    }
    None
}

/// The authoring envelope the model answers with. `document` is the only
/// validated field; `name` and `trigger_event` are advisory suggestions the
/// caller passes to lane 16's save route.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthoringEnvelope {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub trigger_event: Option<String>,
    #[serde(default)]
    pub document: Option<Value>,
}

/// Splits a model answer into (envelope, document value). Lenient about shape:
/// if the object has no `document` key but looks like a document itself
/// (`nodes` array present), it is treated as the document.
pub fn split_envelope(value: Value) -> (AuthoringEnvelope, Option<Value>) {
    let has_document = value.get("document").is_some();
    let looks_like_document = value.get("nodes").is_some() && value.get("document").is_none();
    if looks_like_document {
        return (AuthoringEnvelope::default(), Some(value));
    }
    if has_document {
        let envelope: AuthoringEnvelope = serde_json::from_value(value).unwrap_or_default();
        let doc = envelope.document.clone();
        return (envelope, doc);
    }
    (AuthoringEnvelope::default(), None)
}

/// One author-time refusal, pointed and stable so a UI can render it.
#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct AuthoringIssue {
    /// A stable machine code (`unknown_node_type`, `node_params`, …).
    pub code: String,
    /// The offending node id or a JSON path, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub message: String,
}

/// Maps a document validation error to a typed issue. Every
/// [`WorkflowDocError`] variant gets a stable code.
pub fn issue_from_error(error: &WorkflowDocError) -> AuthoringIssue {
    let (code, path, message): (&str, Option<String>, String) = match error {
        WorkflowDocError::NotAnObject => ("not_an_object", None, error.to_string()),
        WorkflowDocError::UnsupportedVersion(_) => ("unsupported_version", None, error.to_string()),
        WorkflowDocError::NoNodes => ("no_nodes", None, error.to_string()),
        WorkflowDocError::DuplicateNodeId(id) => {
            ("duplicate_node_id", Some(id.clone()), error.to_string())
        }
        WorkflowDocError::UnknownNodeType { id, .. } => {
            ("unknown_node_type", Some(id.clone()), error.to_string())
        }
        WorkflowDocError::NodeParams { id, .. } => {
            ("node_params", Some(id.clone()), error.to_string())
        }
        WorkflowDocError::UnknownEdgeEndpoint { from, to } => (
            "unknown_edge_endpoint",
            Some(format!("{from}→{to}")),
            error.to_string(),
        ),
        WorkflowDocError::DuplicateEdge(key) => {
            ("duplicate_edge", Some(key.clone()), error.to_string())
        }
        WorkflowDocError::SelfLoop(id) => ("self_loop", Some(id.clone()), error.to_string()),
        WorkflowDocError::EntryNodes(ids) => {
            ("entry_nodes", Some(ids.join(", ")), error.to_string())
        }
        WorkflowDocError::Unreachable(id) => ("unreachable", Some(id.clone()), error.to_string()),
        WorkflowDocError::EmbeddedCode { path, .. } => {
            ("embedded_code", Some(path.clone()), error.to_string())
        }
        WorkflowDocError::LoopRefusal(_) => ("loop_refusal", None, error.to_string()),
        WorkflowDocError::NestedForeach(id) => {
            ("nested_foreach", Some(id.clone()), error.to_string())
        }
        WorkflowDocError::ForeachCapExceeded { id, .. } => {
            ("foreach_cap_exceeded", Some(id.clone()), error.to_string())
        }
        WorkflowDocError::ForeachNoBody(id) => {
            ("foreach_no_body", Some(id.clone()), error.to_string())
        }
        WorkflowDocError::ForeachBodyLeaks { id, .. } => {
            ("foreach_body_leaks", Some(id.clone()), error.to_string())
        }
        WorkflowDocError::ForeachBodyParks(id) => {
            ("foreach_body_parks", Some(id.clone()), error.to_string())
        }
        WorkflowDocError::Cycle(id) => ("cycle", Some(id.clone()), error.to_string()),
        WorkflowDocError::RouteWithoutErrorEdge(id) => (
            "route_without_error_edge",
            Some(id.clone()),
            error.to_string(),
        ),
        WorkflowDocError::NodeNotExecutable { id, .. } => {
            ("node_not_executable", Some(id.clone()), error.to_string())
        }
        WorkflowDocError::BadReference { id } => {
            ("bad_reference", Some(id.clone()), error.to_string())
        }
        WorkflowDocError::ReferenceNotUpstream { id, .. } => (
            "reference_not_upstream",
            Some(id.clone()),
            error.to_string(),
        ),
        WorkflowDocError::ReferencePathUnknown { id, .. } => (
            "reference_path_unknown",
            Some(id.clone()),
            error.to_string(),
        ),
    };
    AuthoringIssue {
        code: code.to_string(),
        path,
        message,
    }
}

/// Validates a candidate document through the SAME boundary lane 16 uses.
/// Returns the typed issues; an empty vector means the document is runnable.
pub fn validate_candidate(document: &Value) -> Vec<AuthoringIssue> {
    match WorkflowDocument::parse(document) {
        Ok(_) => Vec::new(),
        Err(error) => vec![issue_from_error(&error)],
    }
}

/// The result of parsing one raw model answer: the document (when present),
/// the advisory envelope, and any typed issues.
#[derive(Debug, Clone)]
pub struct ParsedAuthoring {
    pub document: Option<Value>,
    pub envelope: Option<AuthoringEnvelope>,
    pub issues: Vec<AuthoringIssue>,
}

/// The full parse+validate path one raw model answer takes: extract JSON →
/// split envelope → validate the document. Returns the issues and the
/// document value (present only when extraction found one).
pub fn parse_authoring_output(raw: &str) -> ParsedAuthoring {
    let Some(value) = extract_json_object(raw) else {
        return ParsedAuthoring {
            document: None,
            envelope: None,
            issues: vec![AuthoringIssue {
                code: "not_json".to_string(),
                path: None,
                message: "the model's answer contained no JSON object".to_string(),
            }],
        };
    };
    let (envelope, document) = split_envelope(value);
    match document {
        Some(document) => {
            let issues = validate_candidate(&document);
            ParsedAuthoring {
                document: Some(document),
                envelope: Some(envelope),
                issues,
            }
        }
        None => ParsedAuthoring {
            document: None,
            envelope: Some(envelope),
            issues: vec![AuthoringIssue {
                code: "no_document".to_string(),
                path: None,
                message: "the JSON object carried no `document` field and was not a document"
                    .to_string(),
            }],
        },
    }
}

// ---------------------------------------------------------------------------
// The structural diff (spec D)
// ---------------------------------------------------------------------------

/// One structural change between the current document and the draft. The
/// caller renders these as the review diff; nothing is applied here.
#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct DiffEntry {
    /// `add`, `remove` or `change`.
    pub op: String,
    /// A stable location, e.g. `nodes/triage`, `edges/a→b`, `version`.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub after: Option<Value>,
}

fn node_key(node: &Value) -> Option<String> {
    node.get("id").and_then(Value::as_str).map(str::to_string)
}

fn edge_key(edge: &Value) -> Option<String> {
    let from = edge.get("from").and_then(Value::as_str)?;
    let to = edge.get("to").and_then(Value::as_str)?;
    let label = edge.get("label").and_then(Value::as_str).unwrap_or("");
    Some(if label.is_empty() {
        format!("{from}→{to}")
    } else {
        format!("{from}→{to}[{label}]")
    })
}

fn keyed_items(
    document: &Value,
    field: &str,
    key: impl Fn(&Value) -> Option<String>,
) -> Vec<(String, Value)> {
    document
        .get(field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| key(item).map(|k| (k, item.clone())))
                .collect()
        })
        .unwrap_or_default()
}

/// A structural diff between a base document and a draft, keyed by node id and
/// edge endpoints so a re-ordering is not a change. Pure.
pub fn diff_documents(base: &Value, draft: &Value) -> Vec<DiffEntry> {
    let mut entries = Vec::new();

    // Top-level scalars.
    for field in ["version", "loop_max_depth"] {
        let before = base.get(field).cloned();
        let after = draft.get(field).cloned();
        if before != after {
            entries.push(DiffEntry {
                op: "change".to_string(),
                path: field.to_string(),
                before,
                after,
            });
        }
    }

    // Nodes keyed by id.
    let base_nodes = keyed_items(base, "nodes", node_key);
    let draft_nodes = keyed_items(draft, "nodes", node_key);
    let base_ids: std::collections::BTreeSet<&String> = base_nodes.iter().map(|(k, _)| k).collect();
    let draft_ids: std::collections::BTreeSet<&String> =
        draft_nodes.iter().map(|(k, _)| k).collect();
    for (id, node) in &draft_nodes {
        if !base_ids.contains(id) {
            entries.push(DiffEntry {
                op: "add".to_string(),
                path: format!("nodes/{id}"),
                before: None,
                after: Some(node.clone()),
            });
        }
    }
    for (id, node) in &base_nodes {
        if !draft_ids.contains(id) {
            entries.push(DiffEntry {
                op: "remove".to_string(),
                path: format!("nodes/{id}"),
                before: Some(node.clone()),
                after: None,
            });
        }
    }
    for (id, before) in &base_nodes {
        if let Some((_, after)) = draft_nodes.iter().find(|(k, _)| k == id) {
            if before != after {
                entries.push(DiffEntry {
                    op: "change".to_string(),
                    path: format!("nodes/{id}"),
                    before: Some(before.clone()),
                    after: Some(after.clone()),
                });
            }
        }
    }

    // Edges keyed by endpoints (+ label).
    let base_edges = keyed_items(base, "edges", edge_key);
    let draft_edges = keyed_items(draft, "edges", edge_key);
    let base_keys: std::collections::BTreeSet<&String> =
        base_edges.iter().map(|(k, _)| k).collect();
    let draft_keys: std::collections::BTreeSet<&String> =
        draft_edges.iter().map(|(k, _)| k).collect();
    for (key, edge) in &draft_edges {
        if !base_keys.contains(key) {
            entries.push(DiffEntry {
                op: "add".to_string(),
                path: format!("edges/{key}"),
                before: None,
                after: Some(edge.clone()),
            });
        }
    }
    for (key, edge) in &base_edges {
        if !draft_keys.contains(key) {
            entries.push(DiffEntry {
                op: "remove".to_string(),
                path: format!("edges/{key}"),
                before: Some(edge.clone()),
                after: None,
            });
        }
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.op.cmp(&b.op)));
    entries
}

fn empty_document() -> Value {
    json!({ "version": AUTHORED_DOCUMENT_VERSION, "nodes": [], "edges": [] })
}

// ---------------------------------------------------------------------------
// The authoring loop
// ---------------------------------------------------------------------------

/// One tool call the model made, as the response reports it.
#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct AuthoringToolCall {
    pub name: String,
    /// The `node_id` argument for `get_node_schema`, echoed for the trace.
    pub argument: String,
    /// `ok` or `not_found` / `rejected`.
    pub outcome: String,
}

/// The result of one authoring loop, transport-agnostic so it is unit-testable
/// without HTTP or a database.
#[derive(Debug, Clone)]
pub struct AuthoringOutcome {
    /// The model's final answer text.
    pub raw_content: String,
    /// The extracted document value, when the answer carried one.
    pub document: Option<Value>,
    /// Advisory name/trigger from the answer envelope.
    pub envelope: AuthoringEnvelope,
    /// Typed validation issues; empty means the document is runnable.
    pub issues: Vec<AuthoringIssue>,
    pub tool_calls: Vec<AuthoringToolCall>,
    pub steps: usize,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// Set when the loop ended because of a transport failure, so a caller can
    /// report honestly instead of pretending the model answered.
    pub transport_error: Option<String>,
}

impl AuthoringOutcome {
    pub fn valid(&self) -> bool {
        self.document.is_some() && self.issues.is_empty()
    }
}

/// Builds the initial message list: the compact system prompt plus the user's
/// description (and the current document when editing).
pub fn build_messages(prompt: &str, base: Option<&Value>) -> Vec<LoopMessage> {
    let mut messages = vec![LoopMessage {
        role: "system".to_string(),
        content: authoring_system_prompt(),
        tool_calls: None,
        tool_call_id: None,
    }];
    let mut user = String::new();
    user.push_str("Describe the workflow you want:\n");
    user.push_str(prompt);
    if let Some(base) = base {
        user.push_str(
            "\n\nThe workflow currently looks like this — edit it, keeping what works:\n",
        );
        user.push_str(&serde_json::to_string(base).unwrap_or_default());
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
/// shape (same transport seam, same tool protocol) without the SSE framing.
pub async fn run_authoring_loop(
    transport: &dyn ChatModelTransport,
    model: &str,
    prompt: &str,
    base: Option<&Value>,
    max_steps: usize,
) -> AuthoringOutcome {
    let tools = vec![node_schema_tool()];
    let mut messages = build_messages(prompt, base);
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
                    .get("node_id")
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
        ParsedAuthoring {
            document: None,
            envelope: None,
            issues: vec![AuthoringIssue {
                code: "no_answer".to_string(),
                path: None,
                message: "the model produced no final answer".to_string(),
            }],
        }
    } else {
        parse_authoring_output(&final_content)
    };

    AuthoringOutcome {
        raw_content: final_content,
        document: parsed.document,
        envelope: parsed.envelope.unwrap_or_default(),
        issues: parsed.issues,
        tool_calls,
        steps,
        prompt_tokens,
        completion_tokens,
        transport_error,
    }
}

/// Dispatches one tool call. Only [`NODE_SCHEMA_TOOL`] is understood; anything
/// else is answered typed (the model cannot invent a tool that does work).
fn handle_tool_call(call: &ToolCallReq) -> (String, &'static str) {
    if call.name == NODE_SCHEMA_TOOL {
        let result = run_node_schema_tool(&call.arguments);
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

/// The provider-wire `tool_calls` round-trip for an assistant message (the
/// same shape the chat runner sends back).
fn tool_calls_wire(calls: &[ToolCallReq]) -> Value {
    json!(calls
        .iter()
        .map(|c| json!({
            "id": c.id,
            "type": "function",
            "function": {
                "name": c.name,
                "arguments": serde_json::to_string(&c.arguments).unwrap_or_else(|_| "{}".to_string()),
            }
        }))
        .collect::<Vec<_>>())
}

// ---------------------------------------------------------------------------
// Measurement (spec E)
// ---------------------------------------------------------------------------

/// One candidate model answer in the measurement corpus.
#[derive(Debug, Clone)]
pub struct AuthoringSample {
    pub name: String,
    pub candidate: String,
}

/// The measured rates. `valid_json_rate` is the share of candidates from
/// which a JSON object was extracted; `schema_rejection_rate` is the share of
/// EXTRACTED documents the real catalogue refused. Both are computed against
/// the live registry by [`parse_authoring_output`].
#[derive(Debug, Clone, Serialize, ToSchema, PartialEq)]
pub struct AuthoringMeasurement {
    pub total: usize,
    pub json_extracted: usize,
    pub schema_valid: usize,
    pub schema_rejected: usize,
    pub valid_json_rate: f64,
    pub schema_rejection_rate: f64,
}

/// Runs a corpus through the real parse+validate path and reports the rates.
/// Pure — no model, no I/O — so it is deterministic evidence, not a hope.
pub fn measure_corpus(samples: &[AuthoringSample]) -> AuthoringMeasurement {
    let total = samples.len();
    let mut json_extracted = 0usize;
    let mut schema_valid = 0usize;
    for sample in samples {
        let parsed = parse_authoring_output(&sample.candidate);
        if parsed.document.is_some() {
            json_extracted += 1;
            if parsed.issues.is_empty() {
                schema_valid += 1;
            }
        }
    }
    let schema_rejected = json_extracted - schema_valid;
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
        schema_valid,
        schema_rejected,
        valid_json_rate: rate(json_extracted),
        schema_rejection_rate: rate(schema_rejected),
    }
}

/// The prompt-footprint measurement that motivates on-demand schemas: the
/// compact catalogue (what this lane sends) versus the full catalogue with
/// every schema inlined (the naive alternative). Returns
/// `(compact_chars, inline_chars, node_count)`.
pub fn measure_prompt_footprint() -> (usize, usize, usize) {
    let compact = authoring_system_prompt().len();
    let mut inline = String::new();
    let nodes = body_catalogue();
    for node in &nodes {
        inline.push_str(&node.id);
        inline.push_str(&node.label);
        inline.push_str(&node.agent_description);
        inline.push_str(
            &node_registry::input_schema(&node.id)
                .unwrap_or_default()
                .to_string(),
        );
        inline.push_str(
            &node_registry::output_schema(&node.id)
                .unwrap_or_default()
                .to_string(),
        );
    }
    (compact, inline.len(), nodes.len())
}

// ---------------------------------------------------------------------------
// The HTTP endpoint
// ---------------------------------------------------------------------------

/// Admin-surface permission helper, matching the workflow document routes.
fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthorWorkflowRequest {
    /// The AI surface the call is admitted on (e.g. `general_chat`). The org
    /// grants it like any other; there is no workflow-specific surface and no
    /// new admission path.
    pub surface_key: String,
    /// The plain-English description of the workflow to author.
    pub prompt: String,
    /// The current document, when editing an existing workflow. Validated
    /// first, so the returned diff is against a real document.
    #[schema(value_type = Object, nullable = true)]
    pub document: Option<Value>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_steps: Option<u32>,
}

/// Model-provenance summary for the response.
#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorModelInfo {
    pub provider: String,
    pub model: String,
    pub resolution: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub steps: usize,
}

/// The registry provenance the draft was validated against.
#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorCatalogueInfo {
    pub version: String,
    pub digest: String,
    pub node_count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorWorkflowResponse {
    /// True when `draft` is present and validated against the catalogue.
    pub valid: bool,
    /// Always false — this endpoint never writes (spec D).
    pub saved: bool,
    /// The validated document, when the model produced one.
    #[schema(value_type = Object, nullable = true)]
    pub draft: Option<Value>,
    /// Advisory name from the model's answer envelope.
    pub name: Option<String>,
    /// Advisory trigger event type from the envelope.
    pub trigger_event: Option<String>,
    /// Typed validation issues; empty means `draft` is runnable.
    pub issues: Vec<AuthoringIssue>,
    /// The changes the draft makes against `document` (or against an empty
    /// document when generating fresh).
    pub diff: Vec<DiffEntry>,
    pub tool_calls: Vec<AuthoringToolCall>,
    pub usage: AuthorUsage,
    pub model_info: AuthorModelInfo,
    pub catalogue: AuthorCatalogueInfo,
    /// Set when the model transport failed; the caller reports this honestly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// POST /api/workflows/author — author a workflow draft from a sentence.
///
/// Admin-gated (Edit). NOT a write: it returns a draft, its issues and the
/// diff, and persists nothing. The completion rides the shared AI admission
/// path with the completion + tool-calling edges.
#[utoipa::path(operation_id = "workflow_author", post, path = "/api/workflows/author", tag = "workflows", security(("bearer" = [])),
    request_body = AuthorWorkflowRequest,
    responses((status = 200, body = AuthorWorkflowResponse),
              (status = 422, description = "the request or the base document did not validate"),
              (status = 403, description = "AI admission refused (typed reason in the envelope)")))]
pub async fn author(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<AuthorWorkflowRequest>,
) -> ApiResult<Json<AuthorWorkflowResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let surface_key = req.surface_key.trim().to_string();
    if surface_key.is_empty() {
        return Err(ApiError::Validation("surface_key is required".into()));
    }
    let prompt = req.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(ApiError::Validation("prompt is required".into()));
    }
    // A base document must itself be valid, so the diff is meaningful.
    if let Some(base) = &req.document {
        let issues = validate_candidate(base);
        if !issues.is_empty() {
            return Err(ApiError::Validation(format!(
                "the base document does not validate: {}",
                issues
                    .iter()
                    .map(|i| i.message.clone())
                    .collect::<Vec<_>>()
                    .join("; ")
            )));
        }
    }

    // --- admission, on the existing path (spec F) --------------------------
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
        &prompt,
        req.document.as_ref(),
        max_steps,
    )
    .await;

    // Usage ledger (caps bind across calls, exactly as the chat runner's do).
    record_authoring_usage(&state, &call, org_id, user_id, &outcome).await;

    let base = req.document.clone().unwrap_or_else(empty_document);
    let diff = match &outcome.document {
        Some(document) => diff_documents(&base, document),
        None => Vec::new(),
    };
    let catalogue = node_registry::agent_catalogue();
    let response = AuthorWorkflowResponse {
        valid: outcome.valid(),
        saved: false,
        draft: outcome.document.clone(),
        name: outcome.envelope.name.clone(),
        trigger_event: outcome.envelope.trigger_event.clone(),
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
        catalogue: AuthorCatalogueInfo {
            version: node_registry::CATALOGUE_VERSION.to_string(),
            digest: node_registry::catalogue_digest(),
            node_count: catalogue.len(),
        },
        error: outcome.transport_error.clone(),
    };
    Ok(Json(response))
}

/// Writes the usage ledger + a provenance audit row for the authoring call.
/// Best-effort: a failure to record never fails the response (the draft is
/// already produced), but a hard cap is still checked MID-loop by the caller
/// having bounded steps — the admission hard cap is the primary gate.
async fn record_authoring_usage(
    state: &AppState,
    call: &crate::routes::ai_policy::AuthorizedAiCall,
    org_id: Uuid,
    user_id: Uuid,
    outcome: &AuthoringOutcome,
) {
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => {
            tracing::warn!(%error, "authoring usage ledger begin failed");
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
            request_id: Some(format!("workflow_author:{}", Uuid::now_v7())),
        },
    )
    .await;
    let _ = core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Agent(format!("workflow_authoring/user:{user_id}")),
            action: "ai.workflow_author".into(),
            entity_type: Some("workflow_document".into()),
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
