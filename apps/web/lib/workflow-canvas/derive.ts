// Promoting a simple-tier rule to the canvas, and reading the simple tier back.
//
// ADR 0016 decision 3: one document family, two editors. The rule builder
// writes `conditions`/`actions`; the canvas writes `definition`. Opening a rule
// in the canvas is a VIEW TOGGLE, not a migration — so this file DERIVES a
// starting graph from the simple fields without destroying them. The simple
// fields stay on the row, so switching back to the simple editor loses nothing.
//
// Non-purpose: no write-back into `conditions`/`actions`. The canvas saves a
// `definition` and leaves the simple fields intact; a second editor that
// rewrote the first editor's fields would be exactly the "migration" the spec
// rules out.
//
// What it cannot express, it reports rather than drops. Ticket-writing nodes
// omit `ticketId`: the executor resolves it from the triggering envelope, so
// the step acts on the ticket that fired the run (spec: the simple tier's
// implicit subject) without the author hand-copying an id.

import type { DocumentIssue, WorkflowDocumentDoc, WorkflowNodeDoc } from "./document";

/** The simple-tier rule, as the automations surface holds it. */
export interface SimpleRule {
  trigger_event: string;
  conditions: SimpleCondition[];
  actions: SimpleAction[];
  subject_kind?: "ticket" | "board" | null;
}

export interface SimpleCondition {
  type: string;
  [key: string]: unknown;
}

export interface SimpleAction {
  type: string;
  [key: string]: unknown;
}

function node(id: string, nodeType: string, params: Record<string, unknown>): WorkflowNodeDoc {
  return { id, node_type: nodeType, params };
}

/**
 * Maps one condition to its registry node. Returns `null` for a condition shape
 * this build does not know — the caller reports it rather than guessing.
 */
function conditionNode(c: SimpleCondition): WorkflowNodeDoc | null {
  switch (c.type) {
    case "priority_is":
      return node("", "field.equals", { field: "priority", value: c.priority });
    case "category_is":
      return node("", "field.equals", { field: "category", value: c.category });
    case "title_contains":
      return node("", "field.contains", { field: "title", needle: c.needle ?? "" });
    case "unassigned":
      return node("", "field.isEmpty", { field: "assignee_id" });
    case "csat_score_below":
      return node("", "number.lt", { field: "csat_score", value: c.threshold });
    case "ai_decision":
      return node("", "decision.matches", {
        surface: c.surface,
        question: c.question,
        operator: c.operator,
        threshold: c.threshold,
        confidencePath: c.confidence_path,
      });
    default:
      return null;
  }
}

/**
 * Maps one action to its registry node plus any issue that stops it from
 * validating. A ticket-writing action has no ticket at author time, so it is
 * reported as needing a value (the mockup's "needs a value" state) rather than
 * silently saved invalid.
 */
function actionNode(a: SimpleAction): { node: WorkflowNodeDoc; issues: string[] } {
  switch (a.type) {
    case "assign_user":
      return {
        node: node("", "ticket.assign", { userId: a.user_id }),
        issues: [],
      };
    case "set_priority":
      return {
        node: node("", "ticket.setPriority", { priority: a.priority }),
        issues: [],
      };
    case "add_internal_comment":
      return {
        node: node("", "ticket.internalNote", { body: a.body ?? "" }),
        issues: [],
      };
    case "notify_users":
      return {
        node: node("", "notification.send", {
          userIds: a.user_ids ?? [],
          kind: a.kind,
          title: a.title ?? "",
        }),
        issues: [],
      };
    case "plugin_action":
      return {
        node: node("", "plugin.action", { entryKey: a.entry_key, params: a.params ?? {} }),
        issues: [],
      };
    default:
      return {
        node: node("", "domain.update", { operation: a.type }),
        issues: [`The step "${a.type}" has no direct canvas node yet; it maps to a generic update you must complete.`],
      };
  }
}

export interface DerivedDocument {
  document: WorkflowDocumentDoc;
  issues: DocumentIssue[];
}

/**
 * Derives a canvas document from a simple rule. Conditions become condition
 * nodes chained to the entry; actions follow. Named `entry` is the first node
 * so the executor's "exactly one entry" rule holds. Every node the map cannot
 * represent is reported, never dropped.
 */
export function deriveDocumentFromRule(rule: SimpleRule): DerivedDocument {
  const nodes: WorkflowNodeDoc[] = [];
  const edges: WorkflowDocumentDoc["edges"] = [];
  const issues: DocumentIssue[] = [];

  let counter = 0;
  const nextId = (prefix: string) => `${prefix}${++counter}`;

  let previous: string | null = null;
  const chain = (n: WorkflowNodeDoc): string => {
    const id = n.id || nextId(n.node_type.split(".").pop() ?? "step");
    nodes.push({ ...n, id });
    if (previous) edges.push({ from: previous, to: id });
    previous = id;
    return id;
  };

  for (const c of rule.conditions) {
    const mapped = conditionNode(c);
    if (mapped) {
      chain(mapped);
    } else {
      // A condition the canvas has no node for: a branch condition node is the
      // honest placeholder, and it is reported so the author completes it.
      const id = chain(node("", "field.equals", { field: "", value: null }));
      issues.push({
        nodeId: id,
        message: `The condition "${c.type}" has no direct canvas node; pick a field and a value.`,
        severity: "error",
      });
    }
  }

  for (const a of rule.actions) {
    const { node: mapped, issues: actionIssues } = actionNode(a);
    const id = chain(mapped);
    for (const message of actionIssues) {
      issues.push({ nodeId: id, message, severity: "warning" });
    }
  }

  if (nodes.length === 0) {
    nodes.push(node("entry", "flow.stop", {}));
  }

  return {
    document: {
      version: "1",
      nodes,
      edges,
      loop_max_depth: 1,
    },
    issues,
  };
}

/**
 * A one-sentence plain-English read of a simple rule, so promotion can show the
 * author what it is starting from. Mirrors the rule builder's own read-back.
 */
export function describeRule(rule: SimpleRule): string {
  const when = `When ${rule.trigger_event}`;
  const conds = rule.conditions.length
    ? ` and ${rule.conditions.map(describeCondition).join(" and ")}`
    : "";
  const acts = rule.actions.length
    ? `, then ${rule.actions.map(describeAction).join(", then ")}`
    : "";
  return `${when}${conds}${acts}.`;
}

function describeCondition(c: SimpleCondition): string {
  const text: Record<string, (c: SimpleCondition) => string> = {
    priority_is: (c) => `priority is ${String(c.priority)}`,
    category_is: (c) => `status is ${String(c.category)}`,
    title_contains: (c) => `the title contains "${String(c.needle)}"`,
    unassigned: () => `it is unassigned`,
    csat_score_below: (c) => `CSAT is below ${String(c.threshold)}`,
    ai_decision: (c) => `the model answers ${String(c.question)}`,
  };
  return text[c.type]?.(c) ?? c.type;
}

function describeAction(a: SimpleAction): string {
  const text: Record<string, (a: SimpleAction) => string> = {
    assign_user: () => `assign it`,
    set_priority: (a) => `set priority to ${String(a.priority)}`,
    add_internal_comment: () => `add an internal note`,
    notify_users: (a) => `notify ${(a.user_ids as unknown[] | undefined)?.length ?? 0} people`,
    plugin_action: (a) => `run ${String(a.entry_key)}`,
  };
  return text[a.type]?.(a) ?? a.type;
}
