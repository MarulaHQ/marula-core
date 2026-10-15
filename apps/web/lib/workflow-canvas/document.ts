// The workflow document, in TypeScript — the same JSON the executor runs
// (`crates/core-domain/src/workflow_document.rs`).
//
// Purpose: the canvas edits exactly this shape and nothing else. Field names
// are the Rust serde names (`node_type`, `loop_max_depth`) because the wire is
// the document — a camelCase mirror would be a second shape that drifts.
//
// A trigger is NOT a node. `WorkflowDocument::validate` refuses a trigger node
// in the body: the trigger lives on the rule row (`trigger_event`), and the
// document is the body from its single entry node onward. The canvas draws the
// trigger as a fixed entry, not a draggable node.
//
// Non-purpose: no executor. `validateDocument` mirrors the author-time refusals
// that matter for live feedback; the server PATCH re-runs the authoritative
// `WorkflowDocument::validate` before it saves, so this is a preview, not the
// boundary.

export const DOCUMENT_VERSION = "1";
export const DEFAULT_LOOP_MAX_DEPTH = 1;
export const MAX_FOREACH_CAP = 10_000;

export interface WorkflowNodeDoc {
  id: string;
  node_type: string;
  params: Record<string, unknown>;
}

export interface WorkflowEdgeDoc {
  from: string;
  to: string;
  label?: string;
}

export interface WorkflowDocumentDoc {
  version: string;
  nodes: WorkflowNodeDoc[];
  edges: WorkflowEdgeDoc[];
  loop_max_depth: number;
}

export interface DocumentIssue {
  /** The node or edge the issue attaches to, for a canvas badge. */
  nodeId?: string;
  edgeKey?: string;
  message: string;
  severity: "error" | "warning";
}

export function emptyDocument(): WorkflowDocumentDoc {
  return {
    version: DOCUMENT_VERSION,
    nodes: [{ id: "entry", node_type: "flow.stop", params: {} }],
    edges: [],
    loop_max_depth: DEFAULT_LOOP_MAX_DEPTH,
  };
}

export function isWorkflowDocument(value: unknown): value is WorkflowDocumentDoc {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.version === "string" &&
    Array.isArray(v.nodes) &&
    Array.isArray(v.edges)
  );
}

/** Parses an untrusted value into a document, filling serde defaults. */
export function parseDocument(value: unknown): WorkflowDocumentDoc | null {
  if (!isWorkflowDocument(value)) return null;
  const nodes = (value.nodes as unknown[])
    .filter((n): n is Record<string, unknown> => typeof n === "object" && n !== null)
    .map((n) => ({
      id: String(n.id ?? ""),
      node_type: String(n.node_type ?? ""),
      params: (typeof n.params === "object" && n.params !== null
        ? (n.params as Record<string, unknown>)
        : {}) as Record<string, unknown>,
    }));
  const edges = (value.edges as unknown[])
    .filter((e): e is Record<string, unknown> => typeof e === "object" && e !== null)
    .map((e) => {
      const edge: WorkflowEdgeDoc = { from: String(e.from ?? ""), to: String(e.to ?? "") };
      if (typeof e.label === "string") edge.label = e.label;
      return edge;
    });
  return {
    version: value.version,
    nodes,
    edges,
    loop_max_depth:
      typeof value.loop_max_depth === "number" ? value.loop_max_depth : DEFAULT_LOOP_MAX_DEPTH,
  };
}

export function edgeKey(from: string, to: string, label?: string | null): string {
  return `${from}\u0001${to}\u0001${label ?? ""}`;
}

export function stableEdgeKey(edge: WorkflowEdgeDoc): string {
  return edgeKey(edge.from, edge.to, edge.label ?? null);
}

const TERMINALS = new Set(["flow.stop", "flow.fail", "flow.abandon"]);

export function isTerminalNodeType(nodeType: string): boolean {
  return TERMINALS.has(nodeType);
}

/**
 * The author-time refusals the canvas surfaces live, mirroring the subset of
 * `WorkflowDocument::validate` a UI can compute without the registry:
 * no nodes, duplicate ids, dangling/self/duplicate edges, embedded code,
 * unreachability, and a `route` policy with no error-labelled edge. Registry
 * conformance (unknown node type, params) is reported separately by the caller
 * from the loaded catalogue, and the server re-checks everything on save.
 */
export function validateDocument(
  doc: WorkflowDocumentDoc,
  opts: { knownNodeTypes?: Set<string> } = {},
): DocumentIssue[] {
  const issues: DocumentIssue[] = [];
  if (doc.nodes.length === 0) {
    issues.push({ message: "A workflow needs at least one step.", severity: "error" });
    return issues;
  }

  const seen = new Set<string>();
  for (const node of doc.nodes) {
    if (seen.has(node.id)) {
      issues.push({ nodeId: node.id, message: `Duplicate step id "${node.id}".`, severity: "error" });
    }
    seen.add(node.id);
    if (opts.knownNodeTypes && !opts.knownNodeTypes.has(node.node_type)) {
      issues.push({
        nodeId: node.id,
        message: `Unknown step type "${node.node_type}".`,
        severity: "error",
      });
    }
    const embedded = findEmbeddedCode(node.params);
    if (embedded) {
      issues.push({
        nodeId: node.id,
        message: `Embedded code refused at ${embedded.path}: ${embedded.detail} (a document is data, never code).`,
        severity: "error",
      });
    }
  }

  const keys = new Set<string>();
  const outgoing = new Map<string, WorkflowEdgeDoc[]>();
  for (const edge of doc.edges) {
    if (!seen.has(edge.from) || !seen.has(edge.to)) {
      issues.push({
        edgeKey: stableEdgeKey(edge),
        message: `Edge ${edge.from} → ${edge.to} names a step that does not exist.`,
        severity: "error",
      });
      continue;
    }
    if (edge.from === edge.to) {
      issues.push({
        edgeKey: stableEdgeKey(edge),
        message: `A step cannot connect to itself.`,
        severity: "error",
      });
    }
    const k = stableEdgeKey(edge);
    if (keys.has(k)) {
      issues.push({
        edgeKey: k,
        message: `Duplicate edge ${edge.from} → ${edge.to}.`,
        severity: "error",
      });
    }
    keys.add(k);
    const list = outgoing.get(edge.from) ?? [];
    list.push(edge);
    outgoing.set(edge.from, list);
  }

  // route-policy needs an error-labelled edge.
  for (const node of doc.nodes) {
    if (node.params.onError === "route" && !(outgoing.get(node.id) ?? []).some((e) => e.label === "error")) {
      issues.push({
        nodeId: node.id,
        message: `"Route on error" needs an edge labelled "error" out of this step.`,
        severity: "error",
      });
    }
  }

  // Reachability from the first node (the author's entry by declaration order).
  const entry = doc.nodes[0]?.id ?? "";
  const reachable = new Set<string>([entry]);
  const stack = [entry];
  while (stack.length) {
    const id = stack.pop() as string;
    for (const edge of outgoing.get(id) ?? []) {
      if (!reachable.has(edge.to)) {
        reachable.add(edge.to);
        stack.push(edge.to);
      }
    }
  }
  for (const node of doc.nodes) {
    if (!reachable.has(node.id)) {
      issues.push({
        nodeId: node.id,
        message: `This step is unreachable from the entry.`,
        severity: "warning",
      });
    }
  }

  return issues;
}

const BANNED_KEYS = new Set(["code", "script", "eval", "expression", "fn", "function"]);
const BANNED_VALUE_PATTERNS = ["${", "<%", "javascript:", "eval("];

function findEmbeddedCode(value: unknown, path = ""): { path: string; detail: string } | null {
  if (typeof value === "string") {
    for (const pat of BANNED_VALUE_PATTERNS) {
      if (value.includes(pat)) return { path: path || "(root)", detail: `contains ${pat}` };
    }
    return null;
  }
  if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i++) {
      const found = findEmbeddedCode(value[i], `${path}[${i}]`);
      if (found) return found;
    }
    return null;
  }
  if (typeof value === "object" && value !== null) {
    for (const [key, child] of Object.entries(value)) {
      const childPath = path === "" ? key : `${path}.${key}`;
      if (BANNED_KEYS.has(key)) return { path: childPath, detail: `${key} is not allowed` };
      const found = findEmbeddedCode(child, childPath);
      if (found) return found;
    }
  }
  return null;
}

/**
 * A one-line human summary of a node for its canvas card, read from its params
 * (never re-derived from the schema). Falls back to the palette label.
 */
export function summariseParams(node: WorkflowNodeDoc): string {
  const parts: string[] = [];
  for (const [key, value] of Object.entries(node.params)) {
    if (key === "onError") continue;
    if (value === undefined || value === null || value === "") continue;
    const label = typeof value === "string" ? value : JSON.stringify(value);
    parts.push(`${key}: ${label}`);
    if (parts.length === 2) break;
  }
  return parts.join(" · ");
}
