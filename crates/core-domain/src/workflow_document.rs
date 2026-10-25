//! The declarative workflow document and its pure engine (the private build tracker Wave 43
//! lane 15, `p43-workflow-executor`).
//!
//! Purpose: the ONE place the workflow document's shape, its author-time
//! validation and the DAG walk that runs it live. Everything here is pure:
//! no I/O, no database, no clock. The worker's executor (lane 15) owns the
//! effects; this module owns the decision of WHAT runs next and whether a
//! document is legal to run at all.
//!
//! # The document is data, never code (ADR 0016 rule (a))
//!
//! A workflow is a list of nodes (each naming a registry node type and its
//! JSON params) and a list of edges (a DAG, with one bounded exception:
//! `flow.foreach`'s body). There is no script, no expression string and no
//! eval anywhere in it. [`WorkflowDocument::validate`] enforces that
//! mechanically — [`find_embedded_code`] refuses a document carrying
//! anything that looks like an embedded script — so the boundary cannot
//! erode as nodes are added, exactly as rule (a) requires.
//!
//! # Loop protection is static and free (spec item D)
//!
//! Each node declares the event types it can produce (`NodeType::emits` /
//! `DomainOp::emitted_event`, Wave 43 lane 4E). So a `flow.waitFor` whose
//! matched event a reachable node in the SAME document can emit is refused
//! at author time through lane 13's [`node_registry::check_loop_safety`] —
//! a validation error, never a runtime surprise. `loop.maxDepth` on the
//! document bounds how many times a resumed run may re-enter, default 1.
//!
//! # The cursor is the resume state
//!
//! [`WorkflowCursor`] is the executor's opaque continuation. It serializes
//! straight into `workflow_runs.resume_cursor` (lane 14's ledger), so a
//! parked run carries exactly the state it needs to continue and no more.
//! The walk is a pure function over `(document, cursor)`:
//! [`WorkflowCursor::next_ready`] names the next node to execute, and the
//! executor records the node's outcome back into the cursor.
//!
//! # What is here and what is not
//!
//! Here: the document model, author-time validation, the cursor and the
//! walker. Not here: node EXECUTION (the worker), the run ledger writes
//! (`core_db::workflow_runs`, lane 14), the event stream and the queue. The
//! executor drives this module; this module never drives the executor.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::node_registry::{self, DomainOp, ExecutorPath, NodeRegistryError, PluginNodeKind};
use crate::policy::{self, Action, Module, Permission};
use crate::settings::WORKFLOW_DISABLED_NODE_KINDS;

/// The document version this build understands. Bumped when the shape
/// changes in a way an older reader could misread.
pub const DOCUMENT_VERSION: &str = "1";

/// The default `flow.foreach` iteration cap when the node sets no
/// `maxItems`. The org can raise it up to [`MAX_FOREACH_CAP`]; an author
/// cannot. Small enough that a workflow cannot become an outage generator
/// by default, large enough for the "tickets on this board today" case.
pub const DEFAULT_FOREACH_CAP: i64 = 500;

/// The absolute ceiling on a `flow.foreach`. The catalogue's own schema
/// caps `maxItems` here; this constant is the engine's copy so a document
/// hand-edited past the schema is still refused.
pub const MAX_FOREACH_CAP: i64 = 10_000;

/// The default re-entry bound for a resumed run.
pub const DEFAULT_LOOP_MAX_DEPTH: u32 = 1;

// --- the control node ids the walker and validator react to ----------------

pub const FLOW_JOIN: &str = "flow.join";
pub const FLOW_PARALLEL: &str = "flow.parallel";
pub const FLOW_FOREACH: &str = "flow.foreach";
pub const FLOW_BRANCH: &str = "flow.branch";
pub const FLOW_SWITCH: &str = "flow.switch";
pub const FLOW_STOP: &str = "flow.stop";
pub const FLOW_FAIL: &str = "flow.fail";
pub const FLOW_ABANDON: &str = "flow.abandon";
pub const FLOW_DELAY: &str = "flow.delay";
pub const FLOW_WAIT_UNTIL: &str = "flow.waitUntil";
pub const FLOW_DEADLINE: &str = "flow.deadline";
pub const FLOW_WAIT_FOR: &str = "flow.waitFor";
pub const FLOW_WAIT_FOR_ALL: &str = "flow.waitForAll";
pub const FLOW_AWAIT_APPROVAL: &str = "flow.awaitApproval";

/// The edge label a `flow.foreach`'s body root is entered by.
pub const BODY_LABEL: &str = "body";
/// The edge label a `flow.branch`'s true arm carries when the author names
/// none. The registry allows `trueLabel`; this is the fallback.
pub const DEFAULT_TRUE_LABEL: &str = "true";
pub const DEFAULT_FALSE_LABEL: &str = "false";
/// The edge label an error `route` policy follows, and the label a
/// `flow.errorHandler` converges error arms on.
pub const ERROR_LABEL: &str = "error";

// ---------------------------------------------------------------------------
// The document
// ---------------------------------------------------------------------------

/// The declarative workflow document: nodes and edges, no code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDocument {
    /// The document shape version.
    pub version: String,
    /// Every node, in authored order. The order is the deterministic tie
    /// break the walker uses when several nodes are ready at once.
    pub nodes: Vec<WorkflowNode>,
    /// Every directed edge. An edge may carry a `label` selecting a branch
    /// arm, a switch case, a fan-out port or a foreach body.
    #[serde(default)]
    pub edges: Vec<WorkflowEdge>,
    /// How many times a resumed run may re-enter. `1` (default) means a run
    /// resumes at most once; a workflow that wants to re-arm itself says so
    /// at author time (spec item D).
    #[serde(default = "default_loop_depth")]
    pub loop_max_depth: u32,
}

fn default_loop_depth() -> u32 {
    DEFAULT_LOOP_MAX_DEPTH
}

/// One node: a registry node type plus its validated params. `onError` is a
/// property ON the node (spec item E), read from [`Self::on_error`], so an
/// author cannot forget to attach a policy — it is already there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNode {
    /// The author-assigned stable id, unique within the document.
    pub id: String,
    /// The registry node type id, e.g. `ticket.update`.
    pub node_type: String,
    /// The node's params, validated against the registry's input schema.
    #[serde(default)]
    pub params: Value,
}

impl WorkflowNode {
    /// The per-step failure policy, read from the node's `onError` param.
    /// Absent means `halt`, the safe default.
    pub fn on_error(&self) -> OnError {
        match self.params.get("onError").and_then(Value::as_str) {
            Some("continue") => OnError::Continue,
            Some("route") => OnError::Route,
            _ => OnError::Halt,
        }
    }
}

/// What a step does when it fails (spec item E). A policy on the node, so
/// it cannot be forgotten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnError {
    /// Stop the run and record the failure. The default.
    Halt,
    /// Record the failure and continue along the node's normal outgoing
    /// edges — a best-effort step.
    Continue,
    /// Send the run down the node's `error`-labelled edge.
    Route,
}

/// One directed edge. `label` is `None` for a plain edge, and names the
/// branch arm, switch case, fan-out port or foreach body otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl WorkflowEdge {
    /// The edge's stable key, used in the cursor's completed set. The label
    /// is part of the identity, so two labelled edges between the same pair
    /// never collide.
    pub fn key(&self) -> String {
        edge_key(&self.from, &self.to, self.label.as_deref())
    }
}

/// The stable key for an edge, with an optional iteration scope for a body
/// edge running inside a `flow.foreach`.
pub fn edge_key(from: &str, to: &str, label: Option<&str>) -> String {
    let label = label.unwrap_or("");
    format!("{from}\u{1}{to}\u{1}{label}")
}

// ---------------------------------------------------------------------------
// Validation errors
// ---------------------------------------------------------------------------

/// Why a document was refused. Typed so an author-time surface can point at
/// the offending node or edge instead of printing a paragraph.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkflowDocError {
    #[error("a workflow document must be a JSON object")]
    NotAnObject,
    #[error("unsupported document version {0:?} (this build reads {DOCUMENT_VERSION:?})")]
    UnsupportedVersion(String),
    #[error("a workflow needs at least one node")]
    NoNodes,
    #[error("duplicate node id {0:?}")]
    DuplicateNodeId(String),
    #[error("node {id:?} uses unknown node type {node_type:?}")]
    UnknownNodeType { id: String, node_type: String },
    #[error("node {id:?} params are invalid: {detail}")]
    NodeParams { id: String, detail: String },
    #[error("edge {from:?} → {to:?} names a node that does not exist")]
    UnknownEdgeEndpoint { from: String, to: String },
    #[error("duplicate edge {0:?}")]
    DuplicateEdge(String),
    #[error("self-loop on node {0:?} — the only legal back-edge is a foreach body")]
    SelfLoop(String),
    #[error("a workflow needs exactly one entry node; found {0:?}")]
    EntryNodes(Vec<String>),
    #[error("node {0:?} is unreachable from the entry")]
    Unreachable(String),
    #[error("embedded code refused at {path:?}: {detail} (ADR 0016 rule (a): the document is data, never code)")]
    EmbeddedCode { path: String, detail: String },
    #[error("static loop refusal: {0}")]
    LoopRefusal(String),
    #[error(
        "node {0:?} is a flow.foreach inside another foreach body — nested iteration is refused"
    )]
    NestedForeach(String),
    #[error("node {id:?} declares maxItems {requested}, above the ceiling {ceiling}")]
    ForeachCapExceeded {
        id: String,
        requested: i64,
        ceiling: i64,
    },
    #[error("node {0:?} has no `body` edge — a foreach with no body is not iteration")]
    ForeachNoBody(String),
    #[error("node {id:?}'s body leaves the body via {edge:?}; a body node may only continue inside the body or to the foreach's non-body edges")]
    ForeachBodyLeaks { id: String, edge: String },
    #[error("node {0:?} parks inside a foreach body; a body may not park (the iteration cannot be resumed mid-item)")]
    ForeachBodyParks(String),
    #[error(
        "cycle detected through {0:?} — only a foreach body may loop, and only bounded by its cap"
    )]
    Cycle(String),
    #[error("node {0:?} declares onError=route but has no {ERROR_LABEL:?}-labelled outgoing edge")]
    RouteWithoutErrorEdge(String),
    #[error(
        "node {id:?} uses {node_type:?}, which this build's executor cannot run — the catalogue marks it unimplemented, so the document is refused at author time instead of failing at run time"
    )]
    NodeNotExecutable { id: String, node_type: String },
    #[error("node {id:?} has a value reference that is not an object of {{node, path}}")]
    BadReference { id: String },
    #[error("node {id:?} references node {target:?}, which is not upstream of it")]
    ReferenceNotUpstream { id: String, target: String },
    #[error("node {id:?} references {target:?} field {path:?}, which its output schema does not declare")]
    ReferencePathUnknown {
        id: String,
        target: String,
        path: String,
    },
    #[error(
        "node {id:?} uses {node_type:?}, a node kind this org has switched off \
         (org setting {WORKFLOW_DISABLED_NODE_KINDS})"
    )]
    NodeKindDisabled { id: String, node_type: String },
    #[error(
        "node {id:?} uses {node_type:?}, which requires the {permission} authority — \
         the author of this graph does not hold it"
    )]
    NodeAuthority {
        id: String,
        node_type: String,
        permission: String,
    },
    #[error("node {id:?} uses {node_type:?}, whose authority cannot be derived: {detail}")]
    AuthorityUnderivable {
        id: String,
        node_type: String,
        detail: String,
    },
}

impl From<WorkflowDocError> for crate::error::DomainError {
    fn from(e: WorkflowDocError) -> Self {
        crate::error::DomainError::Validation(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Parsing + validation
// ---------------------------------------------------------------------------

impl WorkflowDocument {
    /// Parses and validates a raw document value. The single entry point an
    /// author-time boundary or the executor calls.
    pub fn parse(value: &Value) -> Result<Self, WorkflowDocError> {
        let doc: WorkflowDocument =
            serde_json::from_value(value.clone()).map_err(|_| WorkflowDocError::NotAnObject)?;
        doc.validate()?;
        Ok(doc)
    }

    /// Every author-time refusal, in one place. Called by [`Self::parse`]
    /// and by the authoring boundary (lane 17) before a document saves.
    pub fn validate(&self) -> Result<(), WorkflowDocError> {
        if self.version != DOCUMENT_VERSION {
            return Err(WorkflowDocError::UnsupportedVersion(self.version.clone()));
        }
        if self.nodes.is_empty() {
            return Err(WorkflowDocError::NoNodes);
        }

        // (1) Node identity and registry conformance.
        let mut ids: BTreeSet<&str> = BTreeSet::new();
        for node in &self.nodes {
            if !ids.insert(node.id.as_str()) {
                return Err(WorkflowDocError::DuplicateNodeId(node.id.clone()));
            }
            let declared = node_registry::get(&node.node_type).ok_or_else(|| {
                WorkflowDocError::UnknownNodeType {
                    id: node.id.clone(),
                    node_type: node.node_type.clone(),
                }
            })?;
            // A trigger is a document ENTRY, never a step: a document may
            // not carry a trigger node in its body.
            if declared.family == node_registry::NodeFamily::Trigger {
                return Err(WorkflowDocError::UnknownNodeType {
                    id: node.id.clone(),
                    node_type: node.node_type.clone(),
                });
            }
            node_registry::validate_params(&node.node_type, &node.params).map_err(|e| {
                WorkflowDocError::NodeParams {
                    id: node.id.clone(),
                    detail: e.to_string(),
                }
            })?;
            // Coverage (Wave 43 lane 20, item C): the catalogue declares
            // whether the executor can run a node today. One that cannot is
            // refused HERE, at author time — the alternative is a document
            // that saves cleanly and dies at step 0 of a run.
            if !declared.executable {
                return Err(WorkflowDocError::NodeNotExecutable {
                    id: node.id.clone(),
                    node_type: node.node_type.clone(),
                });
            }
        }

        // (2) Edge endpoints and identity.
        let mut edge_keys: BTreeSet<String> = BTreeSet::new();
        for edge in &self.edges {
            if !ids.contains(edge.from.as_str()) || !ids.contains(edge.to.as_str()) {
                return Err(WorkflowDocError::UnknownEdgeEndpoint {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                });
            }
            if edge.from == edge.to {
                return Err(WorkflowDocError::SelfLoop(edge.from.clone()));
            }
            if !edge_keys.insert(edge.key()) {
                return Err(WorkflowDocError::DuplicateEdge(edge.key()));
            }
        }

        // (2b) Value references ($ref) resolve against an upstream node's
        // output (Wave 43 lane 20, item F). Checked here, at author time:
        // the target must exist upstream, and the path must be one that
        // node's output schema actually declares.
        self.validate_references()?;

        // (3) Rule (a): the document is data, never code.
        if let Some((path, detail)) =
            find_embedded_code(&serde_json::to_value(self).unwrap_or_default())
        {
            return Err(WorkflowDocError::EmbeddedCode {
                path,
                detail: detail.to_string(),
            });
        }

        // (4) shape: one entry, everything reachable, no illegal cycle.
        self.validate_graph()?;

        // (5) foreach: cap, non-nested, no parking body.
        self.validate_foreach()?;

        // (6) static loop refusal for every event-wait.
        self.validate_loop_safety()?;

        // (7) error routing needs an error edge.
        for node in &self.nodes {
            if node.on_error() == OnError::Route
                && !self
                    .outgoing(&node.id)
                    .iter()
                    .any(|e| e.label.as_deref() == Some(ERROR_LABEL))
            {
                return Err(WorkflowDocError::RouteWithoutErrorEdge(node.id.clone()));
            }
        }

        Ok(())
    }

    /// Enforces the `$ref` value-reference contract (Wave 43 lane 20, item
    /// F): every reference names a node UPSTREAM of the referencing one and
    /// a dotted path that node's declared output schema actually carries.
    /// Without this check a typo would resolve to `null` at run time and
    /// silently feed a null into the next step.
    fn validate_references(&self) -> Result<(), WorkflowDocError> {
        for node in &self.nodes {
            let mut stack: Vec<&Value> = vec![&node.params];
            while let Some(value) = stack.pop() {
                if node_registry::is_param_ref(value) {
                    let Some((target, path)) = node_registry::param_ref_parts(value) else {
                        return Err(WorkflowDocError::BadReference {
                            id: node.id.clone(),
                        });
                    };
                    // The target must be a node in this document that runs
                    // BEFORE the referencing node.
                    if self.node(target).is_none()
                        || !self.reachable_from([target.to_string()]).contains(&node.id)
                    {
                        return Err(WorkflowDocError::ReferenceNotUpstream {
                            id: node.id.clone(),
                            target: target.to_string(),
                        });
                    }
                    // The path must be one the target's output declares. The
                    // ref names a node ID, so resolve its TYPE first.
                    let target_type = self
                        .node(target)
                        .map(|n| n.node_type.as_str())
                        .unwrap_or_default();
                    let output = node_registry::output_schema(target_type).unwrap_or(Value::Null);
                    if !schema_declares_path(&output, path) {
                        return Err(WorkflowDocError::ReferencePathUnknown {
                            id: node.id.clone(),
                            target: target.to_string(),
                            path: path.to_string(),
                        });
                    }
                    continue;
                }
                match value {
                    Value::Object(map) => stack.extend(map.values()),
                    Value::Array(items) => stack.extend(items.iter()),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// The node by id, if declared.
    pub fn node(&self, id: &str) -> Option<&WorkflowNode> {
        self.nodes.iter().find(|n| n.id == id)
    }
    /// Every edge leaving a node, in document order.
    pub fn outgoing(&self, id: &str) -> Vec<&WorkflowEdge> {
        self.edges.iter().filter(|e| e.from == id).collect()
    }

    /// Every edge arriving at a node, in document order.
    pub fn incoming(&self, id: &str) -> Vec<&WorkflowEdge> {
        self.edges.iter().filter(|e| e.to == id).collect()
    }

    /// The node ids reachable from a set of roots by following edges.
    fn reachable_from(&self, roots: impl IntoIterator<Item = String>) -> BTreeSet<String> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<String> = roots.into_iter().collect();
        while let Some(id) = stack.pop() {
            for edge in self.outgoing(&id) {
                if seen.insert(edge.to.clone()) {
                    stack.push(edge.to.clone());
                }
            }
        }
        seen
    }

    /// The body of a `flow.foreach`: the nodes reachable from its `body`
    /// edge, minus the nodes reachable from its non-`body` edges (the
    /// continuation), minus the foreach node itself. Computed, never
    /// stored, so it cannot drift from the edges.
    pub fn foreach_body(&self, foreach_id: &str) -> BTreeSet<String> {
        let mut body_roots: Vec<String> = Vec::new();
        let mut done_roots: Vec<String> = Vec::new();
        for edge in self.outgoing(foreach_id) {
            if edge.label.as_deref() == Some(BODY_LABEL) {
                body_roots.push(edge.to.clone());
            } else {
                done_roots.push(edge.to.clone());
            }
        }
        // The roots THEMSELVES are part of their closures (a body root is a
        // body node; a continuation root is the continuation).
        let mut body_reach = self.reachable_from(body_roots.clone());
        body_reach.extend(body_roots);
        let mut done_reach = self.reachable_from(done_roots.clone());
        done_reach.extend(done_roots);
        body_reach
            .into_iter()
            .filter(|id| id != foreach_id && !done_reach.contains(id))
            .collect()
    }

    /// The `flow.foreach` node ids in this document.
    pub fn foreach_nodes(&self) -> Vec<&WorkflowNode> {
        self.nodes
            .iter()
            .filter(|n| n.node_type == FLOW_FOREACH)
            .collect()
    }

    /// The main-graph nodes: every node that is not inside some foreach
    /// body. The top-level walker only ever executes these; body nodes run
    /// inside a nested cursor driven by the foreach node's executor.
    pub fn main_node_ids(&self) -> BTreeSet<String> {
        let bodies: BTreeSet<String> = self
            .foreach_nodes()
            .iter()
            .flat_map(|f| self.foreach_body(&f.id))
            .collect();
        self.nodes
            .iter()
            .map(|n| n.id.clone())
            .filter(|id| !bodies.contains(id))
            .collect()
    }

    fn validate_graph(&self) -> Result<(), WorkflowDocError> {
        let entries: Vec<String> = self
            .nodes
            .iter()
            .filter(|n| self.incoming(&n.id).is_empty())
            .map(|n| n.id.clone())
            .collect();
        if entries.len() != 1 {
            return Err(WorkflowDocError::EntryNodes(entries));
        }
        let entry = &entries[0];
        // Reachability from the entry, over all edges, ignoring the foreach
        // body's own reachability rules (a body node IS reachable through
        // the body edge, so it passes here).
        let mut reachable = self.reachable_from(vec![entry.clone()]);
        reachable.insert(entry.clone());
        for node in &self.nodes {
            if !reachable.contains(&node.id) {
                return Err(WorkflowDocError::Unreachable(node.id.clone()));
            }
        }
        // A cycle is legal only when it passes through a foreach node (the
        // bounded body loop). Depth-first search marking the stack.
        if let Some(cycle_at) = self.find_illegal_cycle(entry) {
            return Err(WorkflowDocError::Cycle(cycle_at));
        }
        Ok(())
    }

    /// Returns the id of a node on an ILLEGAL cycle, or `None` when every
    /// cycle is a legal foreach body loop.
    fn find_illegal_cycle(&self, entry: &str) -> Option<String> {
        // Compute which nodes belong to a foreach body; a cycle is legal
        // only if every node on it is inside one foreach body (the body's
        // back-edge to its foreach is a body→foreach edge, which is one of
        // the "non-body" edges of the foreach, so the foreach itself sits on
        // the cycle and must be allowed too).
        let legal: BTreeSet<String> = self
            .foreach_nodes()
            .iter()
            .flat_map(|f| {
                let mut set = self.foreach_body(&f.id);
                set.insert(f.id.clone());
                set
            })
            .collect();

        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            White,
            Grey,
            Black,
        }
        let mut mark: BTreeMap<String, Mark> = self
            .nodes
            .iter()
            .map(|n| (n.id.clone(), Mark::White))
            .collect();

        fn dfs(
            doc: &WorkflowDocument,
            id: &str,
            legal: &BTreeSet<String>,
            mark: &mut BTreeMap<String, Mark>,
        ) -> Option<String> {
            mark.insert(id.to_string(), Mark::Grey);
            for edge in doc.outgoing(id) {
                match mark.get(&edge.to).copied().unwrap_or(Mark::Black) {
                    Mark::Grey => {
                        // A cycle. Legal only if every node on it is legal
                        // (inside a foreach body or a foreach node).
                        if !legal.contains(&edge.to) || !legal.contains(id) {
                            return Some(edge.to.clone());
                        }
                    }
                    Mark::White => {
                        if let Some(c) = dfs(doc, &edge.to, legal, mark) {
                            return Some(c);
                        }
                    }
                    Mark::Black => {}
                }
            }
            mark.insert(id.to_string(), Mark::Black);
            None
        }

        // Deterministic order: iterate in declaration order.
        for node in &self.nodes {
            if mark.get(&node.id) == Some(&Mark::White) {
                if let Some(c) = dfs(self, &node.id, &legal, &mut mark) {
                    return Some(c);
                }
            }
        }
        let _ = entry;
        None
    }

    fn validate_foreach(&self) -> Result<(), WorkflowDocError> {
        let bodies: BTreeSet<String> = self
            .foreach_nodes()
            .iter()
            .flat_map(|f| self.foreach_body(&f.id))
            .collect();
        for f in self.foreach_nodes() {
            // Cap: the node's own maxItems, else the default. Refused above
            // the ceiling rather than silently truncated.
            let requested = f
                .params
                .get("maxItems")
                .and_then(Value::as_i64)
                .unwrap_or(DEFAULT_FOREACH_CAP);
            if requested > MAX_FOREACH_CAP {
                return Err(WorkflowDocError::ForeachCapExceeded {
                    id: f.id.clone(),
                    requested,
                    ceiling: MAX_FOREACH_CAP,
                });
            }
            if requested < 1 {
                return Err(WorkflowDocError::ForeachCapExceeded {
                    id: f.id.clone(),
                    requested,
                    ceiling: MAX_FOREACH_CAP,
                });
            }
            let body = self.foreach_body(&f.id);
            if body.is_empty() {
                return Err(WorkflowDocError::ForeachNoBody(f.id.clone()));
            }
            // No nested foreach.
            for id in &body {
                if let Some(n) = self.node(id) {
                    if n.node_type == FLOW_FOREACH {
                        return Err(WorkflowDocError::NestedForeach(n.id.clone()));
                    }
                    // No parking inside a body: the iteration cannot be
                    // resumed mid-item.
                    if is_parking_node(&n.node_type) {
                        return Err(WorkflowDocError::ForeachBodyParks(n.id.clone()));
                    }
                }
            }
            // Body stays inside the body (its non-continuation edges do not
            // jump out to an unrelated node).
            for id in &body {
                for edge in self.outgoing(id) {
                    let target_body = body.contains(&edge.to);
                    let target_is_foreach = edge.to == f.id;
                    let target_in_continuation = !target_body
                        && !target_is_foreach
                        && self.foreach_body(&f.id).contains(&edge.to);
                    // A body node's edge may go to another body node, or out
                    // to the continuation (the foreach's non-body edges).
                    let continuation = self
                        .outgoing(&f.id)
                        .iter()
                        .filter(|e| e.label.as_deref() != Some(BODY_LABEL))
                        .map(|e| e.to.clone())
                        .collect::<BTreeSet<_>>();
                    let leaves_scope =
                        !target_body && !target_is_foreach && !continuation.contains(&edge.to);
                    let _ = target_in_continuation;
                    if leaves_scope {
                        return Err(WorkflowDocError::ForeachBodyLeaks {
                            id: f.id.clone(),
                            edge: edge.key(),
                        });
                    }
                }
            }
        }
        let _ = bodies;
        Ok(())
    }

    fn validate_loop_safety(&self) -> Result<(), WorkflowDocError> {
        // Every node that could produce an event, as lane 13's refusal
        // reads them. A plugin node's emits are supplied by the authoring
        // boundary; a document parsed from JSON carries only first-party
        // nodes, whose emits the registry resolves itself.
        let nodes: Vec<node_registry::LoopCheckNode<'_>> = self
            .nodes
            .iter()
            .map(|n| node_registry::LoopCheckNode::first_party(&n.node_type, &n.params))
            .collect();
        for node in &self.nodes {
            let awaited = awaited_events(node);
            if awaited.is_empty() {
                continue;
            }
            node_registry::check_loop_safety(&node.id, &awaited, &nodes).map_err(|e| match e {
                NodeRegistryError::LoopRefusal { .. } => {
                    WorkflowDocError::LoopRefusal(e.to_string())
                }
                other => WorkflowDocError::LoopRefusal(other.to_string()),
            })?;
        }
        Ok(())
    }
}

/// The event types an event-based wait node parks on. Reads the node's
/// `eventTypes` array; a `flow.deadline` waits on `eventTypes` too, plus its
/// `until` expiry (which is not an event and so is not part of the loop
/// check).
pub fn awaited_events(node: &WorkflowNode) -> Vec<String> {
    match node.node_type.as_str() {
        FLOW_WAIT_FOR | FLOW_WAIT_FOR_ALL | FLOW_DEADLINE => node
            .params
            .get("eventTypes")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// True for the nodes that PARK the run rather than hold a worker.
pub fn is_parking_node(node_type: &str) -> bool {
    matches!(
        node_type,
        FLOW_DELAY
            | FLOW_WAIT_UNTIL
            | FLOW_DEADLINE
            | FLOW_WAIT_FOR
            | FLOW_WAIT_FOR_ALL
            | FLOW_AWAIT_APPROVAL
    )
}

/// True for the nodes that END the run.
pub fn is_terminal_node(node_type: &str) -> bool {
    matches!(node_type, FLOW_STOP | FLOW_FAIL | FLOW_ABANDON)
}

/// The executor path a node rides, resolved from the registry.
pub fn executor_path(node_type: &str) -> Option<ExecutorPath> {
    node_registry::get(node_type).map(|n| n.executor)
}

/// Whether a declared output schema carries a dotted path (Wave 43 lane 20,
/// item F). An open object (`{"type":"object"}` with no `properties`) or an
/// object with `additionalProperties` accepts the remaining path — the
/// registry's default schema is exactly that, and a node that declares no
/// fields cannot have its fields checked. Arrays descend into `items`.
fn schema_declares_path(schema: &Value, path: &str) -> bool {
    if path.is_empty() {
        return true;
    }
    let mut current = schema;
    for segment in path.split('.').filter(|s| !s.is_empty()) {
        if current.get("type").and_then(Value::as_str) == Some("array") {
            match current.get("items") {
                Some(items) => {
                    current = items;
                    continue;
                }
                None => return true,
            }
        }
        let Some(props) = current.get("properties") else {
            return current.get("type").and_then(Value::as_str) == Some("object")
                || current.get("additionalProperties").is_some();
        };
        match props.get(segment) {
            Some(next) => current = next,
            None => return false,
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Node authority: what a graph's author must hold to place a node kind
// ---------------------------------------------------------------------------
//
// A document is a list of node kinds its author CHOSE, and a published graph
// runs those kinds as its author. So the graph can only carry authority its
// author holds. Two rules, both enforced in this module because the publish
// boundary and the executor must not drift apart:
//
//   1. the org must have the kind ENABLED, and a privileged kind — one that
//      writes a record or calls an effect — additionally requires its author
//      to hold that kind's authority at publish time;
//   2. the same rule is re-applied when a published graph RUNS, so a later
//      permission change cannot leave a graph exercising authority nobody
//      holds now (`crates/worker/src/workflow.rs` calls it on every start and
//      every resume, and records the refusal on the org's audit trail).
//
// The authority is DERIVED from the registry's own vocabulary, never from a
// hand-kept list of "dangerous nodes": `NodeType::executor` already says what
// a kind does, and [`node_authority`] is the one function that reads it. A
// new node kind therefore inherits its gate the day it is declared.

/// What one node kind's own execution requires.
///
/// A read is deliberately unprivileged: `domain.query` asks a module a
/// question and writes nothing, so the ruling's "writes across modules or
/// calls an effect" does not reach it. Narrowing reads to their module's
/// `view` grant is a separate question, recorded rather than assumed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeAuthority {
    /// Runs with no authority of its own: triggers, conditions, values,
    /// control flow and record reads.
    Unprivileged,
    /// Writes a record or calls an effect (a model edge, a plugin action). The
    /// permission is what its author must hold.
    Privileged(Permission),
    /// The kind is not in this build's registry, or a generic node's operation
    /// selector does not resolve. Refused rather than assumed harmless.
    Underivable(&'static str),
}

/// The org's node-kind policy: the one fact the authority gate needs that the
/// registry cannot answer, because it is org data. Built by the caller from
/// the org-scope [`WORKFLOW_DISABLED_NODE_KINDS`] setting.
///
/// Defaults-first: no stored setting means every executable kind is enabled,
/// which is how the codebase treats every other org setting. An org switches a
/// kind off by naming it, and the refusal names the same node back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrgNodePolicy {
    /// Node type ids the org switched off org-wide.
    pub disabled_kinds: Vec<String>,
}

impl OrgNodePolicy {
    pub fn new(disabled_kinds: Vec<String>) -> Self {
        Self { disabled_kinds }
    }

    /// The policy an org with no stored switch has: every kind enabled.
    pub fn defaults() -> Self {
        Self::default()
    }

    /// Reads the policy out of the org setting's stored value.
    pub fn from_setting(raw: Option<&Value>) -> Self {
        Self::new(disabled_node_kinds_from_setting(raw))
    }

    /// True when the org switched this kind off.
    pub fn is_disabled(&self, node_type: &str) -> bool {
        self.disabled_kinds.iter().any(|kind| kind == node_type)
    }
}

/// Parses the stored `workflow.disabled_node_kinds` value.
///
/// The settings column holds a scalar, so the list arrives as a JSON array
/// string (the `appearance.data_surfaces` precedent) or as a comma-separated
/// string; both are accepted, a stored array value is accepted too, and an
/// absent or empty value disables nothing.
pub fn disabled_node_kinds_from_setting(raw: Option<&Value>) -> Vec<String> {
    fn from_array(items: &[Value]) -> Vec<String> {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
            .map(str::to_string)
            .collect()
    }

    match raw {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => from_array(items),
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Vec::new();
            }
            // A JSON array carried as text is the documented shape; anything
            // else is the comma-separated spelling.
            if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(trimmed) {
                return from_array(&items);
            }
            trimmed
                .split(',')
                .map(str::trim)
                .filter(|kind| !kind.is_empty())
                .map(str::to_string)
                .collect()
        }
        Some(_) => Vec::new(),
    }
}

/// The module a closed domain operation answers to.
///
/// [`DomainOp::entity`] is the code's own vocabulary for "what this operation
/// touches", so the mapping reads it rather than restating the 68 operations.
/// Entities with no module of their own — communication, assets, purchasing,
/// field ops — answer to the org operator grant (`Module::Admin`): those nodes
/// touch no module's records, they act on the org.
fn module_for_entity(entity: &str) -> Module {
    match entity {
        "ticket" => Module::Tickets,
        "time_entry" | "timer" => Module::Time,
        "client" | "contact" | "site" => Module::Clients,
        "quote" | "contract" => Module::Commercial,
        "invoice" | "credit_note" => Module::Invoicing,
        "appointment" | "dispatch" | "appointment_request" | "travel_draft" => Module::Scheduling,
        "document" | "attachment" => Module::Documents,
        "kb_article" => Module::Kb,
        "secret" => Module::Secrets,
        "project" | "project_phase" | "project_task" => Module::Projects,
        // email, notification, sms, csat, alert, sign_off, stock_item, tool,
        // loan, purchase_order — no module of their own — and any entity a
        // future operation introduces: the org's operator grant, never a
        // silently weaker one. The test
        // `every_domain_operation_answers_to_a_module_and_a_write_action`
        // fails the day an operation resolves to something else.
        _ => Module::Admin,
    }
}

/// The authority one closed domain operation requires: the module of the
/// entity it writes, and the action its own selector names.
///
/// The action rule is mechanical so it cannot drift: an operation whose
/// selector ends in `.create` brings a new record into existence and needs the
/// module's `create` grant; every other operation changes an existing record
/// and needs `edit`.
pub fn domain_op_authority(op: DomainOp) -> Permission {
    let action = if op.as_str().ends_with(".create") {
        Action::Create
    } else {
        Action::Edit
    };
    Permission::Matrix {
        module: module_for_entity(op.entity()),
        action,
    }
}

/// The org operator grant: the authority of a kind that acts on the org as a
/// whole rather than on one module's records (an AI edge that spends money, a
/// domain operation on an entity with no module of its own).
fn operator_grant() -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action: Action::Edit,
    }
}

/// What ONE node requires of the graph's author, derived from the registry's
/// executor path — the single place this mapping lives.
///
/// * conditions, values, control flow, triggers and reads ask nothing;
/// * a closed domain op requires its module's write grant, and the generic
///   `domain.update` resolves that from its own `operation` param;
/// * an AI edge calls a model (it spends money and leaves the org), and a
///   plugin action writes to the external system it belongs to, so both need
///   the operator/plugin grant. A plugin READ asks a question.
pub fn node_authority(node: &WorkflowNode) -> NodeAuthority {
    let Some(declared) = node_registry::get(&node.node_type) else {
        return NodeAuthority::Underivable("the node kind is not in this build's registry");
    };
    match declared.executor {
        ExecutorPath::Trigger
        | ExecutorPath::Condition
        | ExecutorPath::Value
        | ExecutorPath::Control
        | ExecutorPath::DomainQuery => NodeAuthority::Unprivileged,
        ExecutorPath::DomainWrapper(op) => NodeAuthority::Privileged(domain_op_authority(op)),
        ExecutorPath::DomainUpdate => match node
            .params
            .get("operation")
            .and_then(Value::as_str)
            .and_then(DomainOp::parse)
        {
            Some(op) => NodeAuthority::Privileged(domain_op_authority(op)),
            None => NodeAuthority::Underivable(
                "domain.update names no operation this build knows, so the entity it \
                 writes — and therefore its authority — cannot be derived",
            ),
        },
        ExecutorPath::Ai(_) => NodeAuthority::Privileged(operator_grant()),
        ExecutorPath::Plugin(PluginNodeKind::Action) => {
            NodeAuthority::Privileged(Permission::ManagePlugins)
        }
        ExecutorPath::Plugin(PluginNodeKind::Read | PluginNodeKind::Trigger) => {
            NodeAuthority::Unprivileged
        }
    }
}

impl WorkflowDocument {
    /// The authority gate (Wave 56, `p56-node-authority`): every node kind must
    /// be one the org enables, and every PRIVILEGED kind must be one its author
    /// holds. Called by the publish boundary (a document that fails is never
    /// stored) and by the executor at start and resume (a graph that fails is
    /// refused at run and recorded on the audit trail).
    ///
    /// Not part of [`WorkflowDocument::validate`], which is pure and
    /// principal-free: authority is a fact about an author and an org, so it
    /// takes its own inputs rather than smuggling them into the document shape.
    pub fn validate_authority(
        &self,
        author: &[Permission],
        org: &OrgNodePolicy,
    ) -> Result<(), WorkflowDocError> {
        for node in &self.nodes {
            if org.is_disabled(&node.node_type) {
                return Err(WorkflowDocError::NodeKindDisabled {
                    id: node.id.clone(),
                    node_type: node.node_type.clone(),
                });
            }
            match node_authority(node) {
                NodeAuthority::Unprivileged => {}
                NodeAuthority::Privileged(permission) => {
                    if !policy::check(author, permission).is_allowed() {
                        return Err(WorkflowDocError::NodeAuthority {
                            id: node.id.clone(),
                            node_type: node.node_type.clone(),
                            permission: permission.key(),
                        });
                    }
                }
                NodeAuthority::Underivable(detail) => {
                    return Err(WorkflowDocError::AuthorityUnderivable {
                        id: node.id.clone(),
                        node_type: node.node_type.clone(),
                        detail: detail.to_string(),
                    })
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Rule (a): find embedded code
// ---------------------------------------------------------------------------

/// Keys that may never appear anywhere in a workflow document. A node's
/// declared schema already forbids unknown keys, but the document also
/// carries free-form objects (`where`, `fields`, plugin `params`), and the
/// rule is about the DOCUMENT, not the schema — so the scan is over the
/// whole value.
pub const BANNED_KEYS: &[&str] = &[
    "script",
    "code",
    "eval",
    "exec",
    "expression",
    "lambda",
    "shell",
    "cmd",
    "run_command",
];

/// Substrings that, in a string VALUE, mean the author is embedding code:
/// JS/template interpolation and the two scheme-style sinks. `value.template`
/// uses single-brace `{field}` substitution, which none of these match.
pub const BANNED_VALUE_PATTERNS: &[&str] = &["${", "<%", "javascript:", "eval("];

/// Returns the JSON path and reason of the first embedded-code construct,
/// or `None` when the value is pure declarative data. This is the mechanical
/// enforcement of ADR 0016 rule (a): the executor refuses a document that
/// carries anything resembling a script or expression, so the boundary
/// cannot erode as nodes are added.
pub fn find_embedded_code(value: &Value) -> Option<(String, &'static str)> {
    fn walk(value: &Value, path: &str) -> Option<(String, &'static str)> {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let lowered = key.to_ascii_lowercase();
                    if BANNED_KEYS.contains(&lowered.as_str()) {
                        return Some((
                            child_path(path, key),
                            "the key names an executable construct",
                        ));
                    }
                    if let Some(hit) = walk(child, &child_path(path, key)) {
                        return Some(hit);
                    }
                }
                None
            }
            Value::Array(items) => {
                for (i, child) in items.iter().enumerate() {
                    if let Some(hit) = walk(child, &format!("{path}[{i}]")) {
                        return Some(hit);
                    }
                }
                None
            }
            Value::String(text) => {
                if BANNED_VALUE_PATTERNS.iter().any(|p| text.contains(p)) {
                    return Some((
                        path.to_string(),
                        "the string embeds an expression or an executable sink",
                    ));
                }
                None
            }
            _ => None,
        }
    }
    walk(value, "")
}

fn child_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

// ---------------------------------------------------------------------------
// The cursor: the executor's resume state
// ---------------------------------------------------------------------------

/// The executor's opaque continuation. Serializes straight into
/// `workflow_runs.resume_cursor`, so a parked run carries exactly the state
/// it needs and no more.
///
/// Nothing here is derived twice: `completed` holds the edge keys already
/// fired, `executed` the node ids already run, `outputs` the value each node
/// returned (what `value.reference` reads), and `index` the next step
/// position to record on the ledger. `depth` counts re-entries against the
/// document's `loop_max_depth`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowCursor {
    /// The next `step_index` to write on `workflow_run_steps`.
    #[serde(default)]
    pub index: i32,
    /// Node ids that have executed (at least once).
    #[serde(default)]
    pub executed: BTreeSet<String>,
    /// Edge keys ([`edge_key`]) that have fired.
    #[serde(default)]
    pub completed: BTreeSet<String>,
    /// The value each executed node produced under its output key. The
    /// binding a `value.reference` reads.
    #[serde(default)]
    pub outputs: BTreeMap<String, Value>,
    /// How many times this run has been resumed. Bounded by the document's
    /// `loop_max_depth` (spec item D).
    #[serde(default)]
    pub depth: u32,
    /// `flow.foreach` progress, when the run is mid-iteration. A body may
    /// not park, so this never spans a resume — it exists so a body's
    /// iterations are recorded and counted.
    #[serde(default)]
    pub foreach: BTreeMap<String, ForeachProgress>,
    /// The parking node the run is stopped at, when it is mid-wait. Set
    /// when the run parks, read when it resumes.
    #[serde(default)]
    pub parked_node: Option<String>,
    /// For `flow.waitForAll`: which declared event types have arrived so
    /// far, per parking node. A run waiting on three approvals resumes only
    /// when all three are present.
    #[serde(default)]
    pub received: BTreeMap<String, BTreeSet<String>>,
    /// The parked run's wake-at, as an RFC3339 instant, when it is time-
    /// based. The executor reads it back to enqueue the wake job and to
    /// write `workflow_runs.wake_at`.
    #[serde(default)]
    pub wake_at: Option<String>,
    /// The parked run's `wake_on`: a comma-separated list of event types
    /// when it is event-based (`flow.waitFor*` / `flow.deadline`).
    #[serde(default)]
    pub wake_on: Option<String>,
    /// For `flow.awaitApproval`: the approval row each gate filed, per gate
    /// node id. This is the binding a decision must name — a decision whose
    /// `proposal_id` is not this value satisfies a different gate.
    #[serde(default)]
    pub approvals: BTreeMap<String, String>,
}

/// Which outgoing edges of a node fire after it succeeds. Most nodes fire
/// all of them; a branch or switch fires exactly one labelled arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeOutcome {
    All,
    Labels(Vec<String>),
    None,
}

/// One `flow.foreach`'s progress: how many items it has completed and the
/// cap it ran under.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ForeachProgress {
    pub items: i64,
    pub done: i64,
    pub batches: i64,
}

impl WorkflowCursor {
    /// A fresh cursor at the start of a document.
    pub fn start() -> Self {
        Self::default()
    }

    /// Records a node as executed and its output.
    pub fn record_node(&mut self, node_id: &str, output: Value) {
        self.executed.insert(node_id.to_string());
        self.outputs.insert(node_id.to_string(), output);
    }

    /// Fires one edge.
    pub fn fire_edge(&mut self, edge: &WorkflowEdge) {
        self.completed.insert(edge.key());
    }

    /// Fires every outgoing edge of a node that carries no label filter.
    pub fn fire_all_outgoing(&mut self, doc: &WorkflowDocument, node_id: &str) {
        for edge in doc.outgoing(node_id) {
            self.completed.insert(edge.key());
        }
    }

    /// Fires the labelled outgoing edges of a node whose label matches.
    pub fn fire_labelled(
        &mut self,
        doc: &WorkflowDocument,
        node_id: &str,
        labels: &[&str],
    ) -> usize {
        let mut fired = 0;
        for edge in doc.outgoing(node_id) {
            if edge
                .label
                .as_deref()
                .map(|l| labels.contains(&l))
                .unwrap_or(false)
            {
                self.completed.insert(edge.key());
                fired += 1;
            }
        }
        fired
    }

    /// True when a node has executed.
    pub fn has_executed(&self, node_id: &str) -> bool {
        self.executed.contains(node_id)
    }

    /// True when an edge has fired.
    pub fn has_fired(&self, edge: &WorkflowEdge) -> bool {
        self.completed.contains(&edge.key())
    }

    /// The next node to execute in the MAIN graph, in document order, or
    /// `None` when no node is ready (the run has run out of work). The
    /// top-level walker never enters a foreach body: that is the foreach
    /// node's own nested run.
    pub fn next_ready<'a>(&self, doc: &'a WorkflowDocument) -> Option<&'a WorkflowNode> {
        let main = doc.main_node_ids();
        for node in &doc.nodes {
            if !main.contains(&node.id) {
                continue;
            }
            if self.has_executed(&node.id) {
                continue;
            }
            if self.node_ready(doc, node) {
                return Some(node);
            }
        }
        None
    }

    /// Readiness of one main-graph node.
    fn node_ready(&self, doc: &WorkflowDocument, node: &WorkflowNode) -> bool {
        let incoming = doc.incoming(&node.id);
        if incoming.is_empty() {
            // The entry node is ready only at the very start.
            return !self.has_executed(&node.id) && self.completed.is_empty();
        }
        match node.node_type.as_str() {
            FLOW_JOIN => self.join_ready(node, &incoming),
            _ => incoming.iter().any(|e| self.has_fired(e)),
        }
    }

    /// A join's readiness by mode: `all` waits for every incoming edge,
    /// `any` for the first, `n_of_m` for the declared count (default 1).
    fn join_ready(&self, node: &WorkflowNode, incoming: &[&WorkflowEdge]) -> bool {
        let fired = incoming.iter().filter(|e| self.has_fired(e)).count();
        match node.params.get("mode").and_then(Value::as_str) {
            Some("all") => fired == incoming.len(),
            Some("n_of_m") => {
                let want = node
                    .params
                    .get("count")
                    .and_then(Value::as_i64)
                    .unwrap_or(1)
                    .max(1) as usize;
                fired >= want.min(incoming.len())
            }
            // `any` (the default an unknown mode falls back to) resumes on
            // the first arrival.
            _ => fired >= 1,
        }
    }

    /// True when the run has no ready node and therefore ends clean; the
    /// executor distinguishes this from a terminal node's own outcome.
    pub fn is_complete(&self, doc: &WorkflowDocument) -> bool {
        self.next_ready(doc).is_none()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, node_type: &str, params: Value) -> WorkflowNode {
        WorkflowNode {
            id: id.to_string(),
            node_type: node_type.to_string(),
            params,
        }
    }

    fn edge(from: &str, to: &str, label: Option<&str>) -> WorkflowEdge {
        WorkflowEdge {
            from: from.to_string(),
            to: to.to_string(),
            label: label.map(str::to_string),
        }
    }

    /// The smallest legal document: entry → stop.
    fn minimal() -> WorkflowDocument {
        WorkflowDocument {
            version: DOCUMENT_VERSION.to_string(),
            nodes: vec![
                node("a", FLOW_BRANCH, json!({"condition": "c1"})),
                node("z", FLOW_STOP, json!({})),
            ],
            edges: vec![edge("a", "z", None)],
            loop_max_depth: 1,
        }
    }

    #[test]
    fn a_minimal_document_validates_and_walks() {
        let doc = minimal();
        doc.validate().expect("minimal is legal");
        let cursor = WorkflowCursor::start();
        assert_eq!(cursor.next_ready(&doc).map(|n| n.id.as_str()), Some("a"));
    }

    #[test]
    fn an_unknown_node_type_is_refused() {
        let mut doc = minimal();
        doc.nodes[0].node_type = "nope.nothing".into();
        assert!(matches!(
            doc.validate(),
            Err(WorkflowDocError::UnknownNodeType { .. })
        ));
    }

    #[test]
    fn a_node_the_executor_cannot_run_is_refused_at_author_time() {
        // Item C: `ticket.setTags` is declared in the catalogue but has no
        // executor arm, so a document that places it must not save.
        let mut doc = minimal();
        doc.nodes[0] = node("a", "ticket.setTags", json!({ "tags": [] }));
        assert!(matches!(
            doc.validate(),
            Err(WorkflowDocError::NodeNotExecutable { .. })
        ));
    }

    /// entry → step, with the step carrying the given params.
    fn with_step(step_type: &str, step_params: Value) -> WorkflowDocument {
        WorkflowDocument {
            version: DOCUMENT_VERSION.to_string(),
            nodes: vec![
                node("a", "ticket.update", json!({ "fields": {} })),
                node("b", step_type, step_params),
                node("z", FLOW_STOP, json!({})),
            ],
            edges: vec![edge("a", "b", None), edge("b", "z", None)],
            loop_max_depth: 1,
        }
    }

    #[test]
    fn a_value_reference_resolves_a_typed_slot_from_an_upstream_node() {
        // Item F: capture-to-file style handoff — a `$ref` satisfies the
        // typed `title` slot (a literal `{{…}}` could not).
        let doc = with_step(
            "notification.send",
            json!({
                "userIds": ["00000000-0000-0000-0000-000000000001"],
                "kind": "escalation",
                "title": { "$ref": { "node": "a", "path": "ticketId" } }
            }),
        );
        doc.validate().expect("a well-formed ref validates");
    }

    #[test]
    fn a_reference_to_a_non_upstream_node_is_refused() {
        let doc = with_step(
            "notification.send",
            json!({
                "userIds": ["00000000-0000-0000-0000-000000000001"],
                "kind": "escalation",
                "title": { "$ref": { "node": "z", "path": "ticketId" } }
            }),
        );
        assert!(matches!(
            doc.validate(),
            Err(WorkflowDocError::ReferenceNotUpstream { .. })
        ));
    }

    #[test]
    fn a_reference_to_an_undeclared_output_path_is_refused() {
        let doc = with_step(
            "notification.send",
            json!({
                "userIds": ["00000000-0000-0000-0000-000000000001"],
                "kind": "escalation",
                "title": { "$ref": { "node": "a", "path": "notAField" } }
            }),
        );
        assert!(matches!(
            doc.validate(),
            Err(WorkflowDocError::ReferencePathUnknown { .. })
        ));
        // A malformed ref body is refused too.
        let bad = with_step(
            "notification.send",
            json!({
                "userIds": ["00000000-0000-0000-0000-000000000001"],
                "kind": "escalation",
                "title": { "$ref": "a.ticketId" }
            }),
        );
        assert!(matches!(
            bad.validate(),
            Err(WorkflowDocError::BadReference { .. })
        ));
    }

    #[test]
    fn a_trigger_node_in_the_body_is_refused() {
        let mut doc = minimal();
        // `ticket.created` is the trigger node id in the catalogue.
        doc.nodes[0] = node("a", "ticket.created", json!({}));
        assert!(matches!(
            doc.validate(),
            Err(WorkflowDocError::UnknownNodeType { .. })
        ));
    }

    #[test]
    fn params_are_validated_against_the_registry_schema() {
        // flow.branch requires `condition`; omitting it must be refused.
        let mut doc = minimal();
        doc.nodes[0] = node("a", FLOW_BRANCH, json!({}));
        assert!(matches!(
            doc.validate(),
            Err(WorkflowDocError::NodeParams { .. })
        ));
    }

    #[test]
    fn an_edge_to_a_missing_node_is_refused() {
        let mut doc = minimal();
        doc.edges.push(edge("z", "ghost", None));
        assert!(matches!(
            doc.validate(),
            Err(WorkflowDocError::UnknownEdgeEndpoint { .. })
        ));
    }

    #[test]
    fn a_self_loop_is_refused() {
        let mut doc = minimal();
        doc.edges.push(edge("a", "a", Some("loop")));
        assert!(matches!(doc.validate(), Err(WorkflowDocError::SelfLoop(_))));
    }

    #[test]
    fn a_second_entry_is_refused() {
        let mut doc = minimal();
        doc.nodes.push(node("b", FLOW_STOP, json!({})));
        // `b` has no incoming edge, so there are two entries.
        assert!(matches!(
            doc.validate(),
            Err(WorkflowDocError::EntryNodes(_))
        ));
    }

    #[test]
    fn an_unreachable_node_is_refused() {
        let mut doc = minimal();
        let mut orphan = node("orphan", FLOW_STOP, json!({}));
        orphan.params = json!({});
        doc.nodes.push(orphan);
        assert!(matches!(
            doc.validate(),
            Err(WorkflowDocError::EntryNodes(_)) | Err(WorkflowDocError::Unreachable(_))
        ));
    }

    #[test]
    fn embedded_code_is_refused_mechanically() {
        // A banned key inside a FREE-FORM object param (the schema allows any
        // keys there, so validation reaches the document-level scan).
        let with_key = WorkflowDocument {
            version: DOCUMENT_VERSION.into(),
            nodes: vec![
                node(
                    "t",
                    "value.template",
                    json!({"template": "x", "fields": {"script": "rm -rf /"}}),
                ),
                node("z", FLOW_STOP, json!({})),
            ],
            edges: vec![edge("t", "z", None)],
            loop_max_depth: 1,
        };
        assert!(matches!(
            with_key.validate(),
            Err(WorkflowDocError::EmbeddedCode { .. })
        ));
        // A string value that embeds an expression.
        let with_expr = WorkflowDocument {
            version: DOCUMENT_VERSION.into(),
            nodes: vec![
                node(
                    "t",
                    "value.template",
                    json!({"template": "x", "fields": {"note": "${process.env.SECRET}"}}),
                ),
                node("z", FLOW_STOP, json!({})),
            ],
            edges: vec![edge("t", "z", None)],
            loop_max_depth: 1,
        };
        assert!(matches!(
            with_expr.validate(),
            Err(WorkflowDocError::EmbeddedCode { .. })
        ));
    }

    #[test]
    fn a_value_template_is_not_embedded_code() {
        // value.template is the one place string building is allowed, and
        // its single-brace substitution is not one of the banned sinks.
        let doc = WorkflowDocument {
            version: DOCUMENT_VERSION.into(),
            nodes: vec![
                node(
                    "t",
                    "value.template",
                    json!({"template": "Hello {name}", "fields": {}}),
                ),
                node("z", FLOW_STOP, json!({})),
            ],
            edges: vec![edge("t", "z", None)],
            loop_max_depth: 1,
        };
        doc.validate().expect("a template is not code");
    }

    #[test]
    fn a_wait_for_an_event_a_sibling_produces_is_refused() {
        // flow.waitFor(ticket.status_changed) alongside ticket.transition,
        // which emits ticket.status_changed: the workflow would resume
        // itself forever. The registry refuses it statically.
        let tid = "00000000-0000-0000-0000-000000000001";
        let doc = WorkflowDocument {
            version: DOCUMENT_VERSION.into(),
            nodes: vec![
                node(
                    "w",
                    FLOW_WAIT_FOR,
                    json!({"eventTypes": ["ticket.status_changed"]}),
                ),
                node(
                    "x",
                    "ticket.transition",
                    json!({"ticketId": tid, "status": "closed"}),
                ),
                node("z", FLOW_STOP, json!({})),
            ],
            edges: vec![edge("w", "x", None), edge("x", "z", None)],
            loop_max_depth: 1,
        };
        match doc.validate() {
            Err(WorkflowDocError::LoopRefusal(_)) => {}
            other => panic!("expected a static loop refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_wait_for_an_event_no_sibling_produces_is_allowed() {
        let doc = WorkflowDocument {
            version: DOCUMENT_VERSION.into(),
            nodes: vec![
                node(
                    "w",
                    FLOW_WAIT_FOR,
                    json!({"eventTypes": ["quote.accepted"]}),
                ),
                node("z", FLOW_STOP, json!({})),
            ],
            edges: vec![edge("w", "z", None)],
            loop_max_depth: 1,
        };
        doc.validate()
            .expect("nobody in this document emits quote.accepted");
    }

    #[test]
    fn a_branch_selects_one_arm_and_the_join_waits_for_both() {
        // a(branch) → t / f → join(all) → z
        let doc = WorkflowDocument {
            version: DOCUMENT_VERSION.into(),
            nodes: vec![
                node("a", FLOW_BRANCH, json!({"condition": "c"})),
                node("t", FLOW_STOP, json!({})),
                node("f", FLOW_STOP, json!({})),
                node("j", FLOW_JOIN, json!({"mode": "all"})),
                node("z", FLOW_STOP, json!({})),
            ],
            edges: vec![
                edge("a", "t", Some(DEFAULT_TRUE_LABEL)),
                edge("a", "f", Some(DEFAULT_FALSE_LABEL)),
                edge("t", "j", None),
                edge("f", "j", None),
                edge("j", "z", None),
            ],
            loop_max_depth: 1,
        };
        doc.validate().expect("a diamond is legal");
        let mut cursor = WorkflowCursor::start();
        // Entry is ready.
        assert_eq!(cursor.next_ready(&doc).map(|n| n.id.as_str()), Some("a"));
        cursor.record_node("a", json!({"branch": "true"}));
        cursor.fire_labelled(&doc, "a", &[DEFAULT_TRUE_LABEL]);
        // Only the true arm fires; the false arm never becomes ready.
        assert_eq!(cursor.next_ready(&doc).map(|n| n.id.as_str()), Some("t"));
        cursor.record_node("t", json!({}));
        cursor.fire_all_outgoing(&doc, "t");
        // The join needs BOTH arms; only one fired, so it is not ready and
        // no other main node is either.
        assert_eq!(cursor.next_ready(&doc).map(|n| n.id.as_str()), None);
        // Fire the false arm too (as a fan-out would): now the join is ready.
        cursor.record_node("f", json!({}));
        cursor.fire_all_outgoing(&doc, "f");
        assert_eq!(cursor.next_ready(&doc).map(|n| n.id.as_str()), Some("j"));
    }

    #[test]
    fn a_parallel_node_fires_all_its_arms() {
        let doc = WorkflowDocument {
            version: DOCUMENT_VERSION.into(),
            nodes: vec![
                node("p", FLOW_PARALLEL, json!({"branches": ["a", "b"]})),
                node("a", FLOW_STOP, json!({})),
                node("b", FLOW_STOP, json!({})),
            ],
            edges: vec![edge("p", "a", None), edge("p", "b", None)],
            loop_max_depth: 1,
        };
        doc.validate().expect("a fan-out is legal");
        let mut cursor = WorkflowCursor::start();
        assert_eq!(cursor.next_ready(&doc).map(|n| n.id.as_str()), Some("p"));
        cursor.record_node("p", json!({}));
        cursor.fire_all_outgoing(&doc, "p");
        // Both arms become ready; the walker yields them in document order.
        assert_eq!(cursor.next_ready(&doc).map(|n| n.id.as_str()), Some("a"));
        cursor.record_node("a", json!({}));
        cursor.fire_all_outgoing(&doc, "a");
        assert_eq!(cursor.next_ready(&doc).map(|n| n.id.as_str()), Some("b"));
    }

    #[test]
    fn a_foreach_body_iterates_and_is_capped() {
        // f(foreach) -body-> work -done-> z   (work continues to the join
        // that is also the foreach's non-body successor)
        let doc = WorkflowDocument {
            version: DOCUMENT_VERSION.into(),
            nodes: vec![
                node(
                    "f",
                    FLOW_FOREACH,
                    json!({"collection": "items", "maxItems": 3}),
                ),
                node("work", FLOW_STOP, json!({})),
                node("z", FLOW_STOP, json!({})),
            ],
            edges: vec![
                edge("f", "work", Some(BODY_LABEL)),
                edge("work", "z", None),
                edge("f", "z", None),
            ],
            loop_max_depth: 1,
        };
        doc.validate().expect("a capped foreach is legal");
        let body = doc.foreach_body("f");
        assert!(body.contains("work"), "work is in the body");
        assert!(!body.contains("z"), "z is the continuation, not the body");
        // The main walker never enters the body.
        let main = doc.main_node_ids();
        assert!(main.contains("f"));
        assert!(!main.contains("work"));
    }

    #[test]
    fn a_foreach_over_the_ceiling_is_refused() {
        let doc = WorkflowDocument {
            version: DOCUMENT_VERSION.into(),
            nodes: vec![
                node(
                    "f",
                    FLOW_FOREACH,
                    json!({"collection": "items", "maxItems": 999999}),
                ),
                node("z", FLOW_STOP, json!({})),
            ],
            edges: vec![edge("f", "z", Some(BODY_LABEL)), edge("f", "z", None)],
            loop_max_depth: 1,
        };
        // The declared input schema caps `maxItems` at the ceiling and the
        // engine keeps its own copy; either refusal is the ceiling doing its
        // job.
        match doc.validate() {
            Err(WorkflowDocError::ForeachCapExceeded { .. })
            | Err(WorkflowDocError::NodeParams { .. }) => {}
            other => panic!("expected a cap refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_unbounded_cycle_is_refused() {
        // e → a → b → a, with no foreach involved. `e` is the entry.
        let doc = WorkflowDocument {
            version: DOCUMENT_VERSION.into(),
            nodes: vec![
                node("e", FLOW_STOP, json!({})),
                node("a", FLOW_BRANCH, json!({"condition": "c"})),
                node("b", FLOW_STOP, json!({})),
            ],
            edges: vec![
                edge("e", "a", None),
                edge("a", "b", None),
                edge("b", "a", None),
            ],
            loop_max_depth: 1,
        };
        match doc.validate() {
            Err(WorkflowDocError::Cycle(_)) => {}
            other => panic!("expected a cycle refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_on_error_route_without_an_error_edge_is_refused() {
        let doc = WorkflowDocument {
            version: DOCUMENT_VERSION.into(),
            nodes: vec![
                node(
                    "a",
                    "ticket.update",
                    json!({"ticketId": "00000000-0000-0000-0000-000000000001", "fields": {}, "onError": "route"}),
                ),
                node("z", FLOW_STOP, json!({})),
            ],
            edges: vec![edge("a", "z", None)],
            loop_max_depth: 1,
        };
        match doc.validate() {
            Err(WorkflowDocError::RouteWithoutErrorEdge(_)) => {}
            other => panic!("expected a route refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_cursor_round_trips_through_json() {
        let mut cursor = WorkflowCursor::start();
        cursor.record_node("a", json!({"x": 1}));
        cursor.fire_edge(&edge("a", "z", None));
        cursor.index = 4;
        let raw = serde_json::to_value(&cursor).expect("serialize");
        let back: WorkflowCursor = serde_json::from_value(raw).expect("deserialize");
        assert_eq!(back, cursor);
        assert!(back.has_executed("a"));
        assert_eq!(back.outputs["a"], json!({"x": 1}));
        assert_eq!(back.index, 4);
    }

    #[test]
    fn parse_round_trips_a_document_value() {
        let doc = minimal();
        let value = serde_json::to_value(&doc).unwrap();
        let parsed = WorkflowDocument::parse(&value).expect("parses");
        assert_eq!(parsed, doc);
    }

    // --- Wave 56: node authority (p56-node-authority) ----------------------

    /// entry → step → stop, with one authored step of the given kind.
    fn one_step(step_type: &str, step_params: Value) -> WorkflowDocument {
        WorkflowDocument {
            version: DOCUMENT_VERSION.to_string(),
            nodes: vec![
                node("a", step_type, step_params),
                node("z", FLOW_STOP, json!({})),
            ],
            edges: vec![edge("a", "z", None)],
            loop_max_depth: 1,
        }
    }

    fn matrix(module: Module, action: Action) -> Permission {
        Permission::Matrix { module, action }
    }

    /// The authority a workflow authoring role carries by itself: the operator
    /// grant the publish route already requires, and nothing else. This is the
    /// role the audit record is about — it can reach the canvas, so only the
    /// node authority gate stops it from wiring another module's writes.
    fn operator_only() -> Vec<Permission> {
        vec![
            matrix(Module::Admin, Action::View),
            matrix(Module::Admin, Action::Create),
            matrix(Module::Admin, Action::Edit),
        ]
    }

    #[test]
    fn an_author_without_the_nodes_authority_cannot_publish_a_privileged_node() {
        let doc = one_step("ticket.update", json!({ "fields": {} }));
        doc.validate().expect("the document itself is legal");

        let refusal = doc
            .validate_authority(&operator_only(), &OrgNodePolicy::defaults())
            .expect_err("an admin-only author may not wire a ticket write");
        match &refusal {
            WorkflowDocError::NodeAuthority {
                id,
                node_type,
                permission,
            } => {
                assert_eq!(id, "a", "the refusal names the authored node id");
                assert_eq!(node_type, "ticket.update");
                assert_eq!(permission, "tickets:edit", "the kind's own module grant");
            }
            other => panic!("expected a node-authority refusal, got {other:?}"),
        }
        // The typed refusal is also a readable one: node id, kind and the
        // exact authority are all in the message an author sees.
        let text = refusal.to_string();
        assert!(text.contains("\"a\""), "{text}");
        assert!(text.contains("ticket.update"), "{text}");
        assert!(text.contains("tickets:edit"), "{text}");
    }

    #[test]
    fn an_author_holding_the_nodes_authority_can_publish_it() {
        let doc = one_step("ticket.update", json!({ "fields": {} }));
        let mut author = operator_only();
        author.push(matrix(Module::Tickets, Action::Edit));
        doc.validate_authority(&author, &OrgNodePolicy::defaults())
            .expect("tickets:edit is the authority ticket.update needs");
    }

    #[test]
    fn an_org_that_disables_a_node_kind_refuses_it_even_for_the_authority_holder() {
        let doc = one_step("ticket.update", json!({ "fields": {} }));
        let mut author = operator_only();
        author.push(matrix(Module::Tickets, Action::Edit));
        let org = OrgNodePolicy::new(vec!["ticket.update".to_string()]);

        match doc.validate_authority(&author, &org) {
            Err(WorkflowDocError::NodeKindDisabled { id, node_type }) => {
                assert_eq!(id, "a");
                assert_eq!(node_type, "ticket.update");
            }
            other => panic!("expected a disabled-kind refusal, got {other:?}"),
        }

        // The org switch is a switch over KINDS, not only privileged ones: a
        // control node the org switched off is refused the same way.
        let plain = one_step("value.template", json!({ "template": "x" }));
        let off = OrgNodePolicy::new(vec!["value.template".to_string()]);
        assert!(matches!(
            plain.validate_authority(&[], &off),
            Err(WorkflowDocError::NodeKindDisabled { .. })
        ));
    }

    #[test]
    fn a_document_of_unprivileged_kinds_needs_no_authority_at_all() {
        // A value node and a stop: nothing writes, nothing calls an effect, so
        // an author holding no permission may publish it.
        let doc = one_step(
            "value.template",
            json!({ "template": "Hi {name}", "fields": {} }),
        );
        doc.validate().expect("legal");
        doc.validate_authority(&[], &OrgNodePolicy::defaults())
            .expect("a graph of pure value/control nodes carries no authority");
        assert_eq!(node_authority(&doc.nodes[0]), NodeAuthority::Unprivileged);
        assert_eq!(node_authority(&doc.nodes[1]), NodeAuthority::Unprivileged);
    }

    #[test]
    fn the_generic_update_node_resolves_its_authority_from_its_own_operation() {
        let generic = |operation: &str| WorkflowNode {
            id: "u".to_string(),
            node_type: "domain.update".to_string(),
            params: json!({ "operation": operation }),
        };
        assert_eq!(
            node_authority(&generic("invoice.record_payment")),
            NodeAuthority::Privileged(matrix(Module::Invoicing, Action::Edit))
        );
        // The create/read rule is the selector's own shape: `.create` brings a
        // record into existence, everything else edits one.
        assert_eq!(
            node_authority(&generic("ticket.create")),
            NodeAuthority::Privileged(matrix(Module::Tickets, Action::Create))
        );
        assert_eq!(
            node_authority(&generic("secret.record")),
            NodeAuthority::Privileged(matrix(Module::Secrets, Action::Edit))
        );
        // An operation this build does not know cannot be assumed harmless.
        assert!(matches!(
            node_authority(&generic("nonsense.op")),
            NodeAuthority::Underivable(_)
        ));
    }

    #[test]
    fn a_generic_update_naming_no_known_operation_is_refused_rather_than_assumed_harmless() {
        let doc = one_step("domain.update", json!({ "operation": "nonsense.op" }));
        assert!(matches!(
            doc.validate_authority(&[Permission::ManagePlugins], &OrgNodePolicy::defaults()),
            Err(WorkflowDocError::AuthorityUnderivable { .. })
        ));
    }

    #[test]
    fn every_registered_kind_derives_its_authority_from_its_executor_path() {
        // The drift guard for the mapping: a new node kind inherits its gate the
        // day it is declared, because this test reads the registry, not a list.
        for declared in node_registry::all() {
            let candidate = WorkflowNode {
                id: "n".to_string(),
                node_type: declared.id.to_string(),
                params: json!({ "operation": "ticket.update" }),
            };
            let authority = node_authority(&candidate);
            match declared.executor {
                ExecutorPath::Trigger
                | ExecutorPath::Condition
                | ExecutorPath::Value
                | ExecutorPath::Control
                | ExecutorPath::DomainQuery => assert_eq!(
                    authority,
                    NodeAuthority::Unprivileged,
                    "{} touches no record and calls no effect",
                    declared.id
                ),
                ExecutorPath::DomainWrapper(op) => assert_eq!(
                    authority,
                    NodeAuthority::Privileged(domain_op_authority(op)),
                    "{} rides {}",
                    declared.id,
                    op.as_str()
                ),
                ExecutorPath::DomainUpdate => assert_eq!(
                    authority,
                    NodeAuthority::Privileged(domain_op_authority(DomainOp::TicketUpdate)),
                    "{} resolves per selected operation",
                    declared.id
                ),
                ExecutorPath::Ai(_) => assert_eq!(
                    authority,
                    NodeAuthority::Privileged(matrix(Module::Admin, Action::Edit)),
                    "{} calls a model, which spends money",
                    declared.id
                ),
                ExecutorPath::Plugin(PluginNodeKind::Action) => assert_eq!(
                    authority,
                    NodeAuthority::Privileged(Permission::ManagePlugins),
                    "{} writes to an external system",
                    declared.id
                ),
                ExecutorPath::Plugin(PluginNodeKind::Read | PluginNodeKind::Trigger) => {
                    assert_eq!(authority, NodeAuthority::Unprivileged, "{}", declared.id)
                }
            }
        }
    }

    #[test]
    fn every_domain_operation_answers_to_a_module_and_a_write_action() {
        for op in DomainOp::ALL {
            match domain_op_authority(op) {
                Permission::Matrix { module, action } => {
                    assert!(
                        Module::ALL.contains(&module),
                        "{} resolves to a real module",
                        op.as_str()
                    );
                    // The action rule is mechanical: only a `.create` selector
                    // brings a record into existence.
                    assert_eq!(
                        action == Action::Create,
                        op.as_str().ends_with(".create"),
                        "{} takes the action its own selector names",
                        op.as_str()
                    );
                }
                other => panic!("{} produced {other:?}, not a matrix grant", op.as_str()),
            }
        }
    }

    #[test]
    fn the_operation_mapping_follows_the_operations_own_entity() {
        // A visible pin on the module mapping, so a changed answer is a
        // changed test rather than a silent widening.
        let cases = [
            (DomainOp::TicketUpdate, Module::Tickets),
            (DomainOp::TimeEntryLog, Module::Time),
            (DomainOp::ClientCreate, Module::Clients),
            (DomainOp::QuoteSend, Module::Commercial),
            (DomainOp::InvoiceRecordPayment, Module::Invoicing),
            (DomainOp::AppointmentCancel, Module::Scheduling),
            (DomainOp::DocumentAttach, Module::Documents),
            (DomainOp::KbArticleCreate, Module::Kb),
            (DomainOp::SecretRecord, Module::Secrets),
            (DomainOp::ProjectTaskComplete, Module::Projects),
            // No module of their own: the org operator grant.
            (DomainOp::EmailSend, Module::Admin),
            (DomainOp::StockItemMove, Module::Admin),
            (DomainOp::PurchaseOrderCreate, Module::Admin),
        ];
        for (op, module) in cases {
            assert_eq!(
                domain_op_authority(op),
                matrix(
                    module,
                    if op.as_str().ends_with(".create") {
                        Action::Create
                    } else {
                        Action::Edit
                    }
                ),
                "{}",
                op.as_str()
            );
        }
    }

    #[test]
    fn the_org_switch_reads_every_stored_shape() {
        assert!(disabled_node_kinds_from_setting(None).is_empty());
        assert!(disabled_node_kinds_from_setting(Some(&json!(""))).is_empty());
        assert!(disabled_node_kinds_from_setting(Some(&json!("  "))).is_empty());
        assert!(disabled_node_kinds_from_setting(Some(&json!(7))).is_empty());
        assert_eq!(
            disabled_node_kinds_from_setting(Some(&json!("[\"ticket.update\",\"flow.delay\"]"))),
            vec!["ticket.update".to_string(), "flow.delay".to_string()]
        );
        assert_eq!(
            disabled_node_kinds_from_setting(Some(&json!("ticket.update, flow.delay"))),
            vec!["ticket.update".to_string(), "flow.delay".to_string()]
        );
        // A stored array (the settings column admits a scalar, so this shape is
        // not what the write route produces — reading it is still correct).
        assert_eq!(
            disabled_node_kinds_from_setting(Some(&json!(["ticket.update"]))),
            vec!["ticket.update".to_string()]
        );
    }
}
