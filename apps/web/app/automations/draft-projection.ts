/**
 * draft-projection — the bridge from a lane-17 workflow draft to this page's
 * simple editor.
 *
 * The brief's decision (2026-10-16): the simple tier stores an
 * `AutomationRule` and the power tier stores a `WorkflowDocument`. The prose
 * leg asks lane 17's endpoint (`POST /api/workflows/author`) for a DOCUMENT,
 * then this module projects its trigger, conditions and actions into the
 * simple editor as an editable draft. The projection is deliberately ONE-WAY
 * and deliberately LOSSY — a document can express steps this editor has no
 * control for, and pretending otherwise would show a draft that silently
 * does something else. What cannot be projected comes back in `unmapped`,
 * named by node, so the person sees exactly what they would be dropping and
 * the draft is never applied to a rule that cannot represent it.
 *
 * Field names: `field.equals` is generic over any registry-declared field, so
 * a projection here is a *match against this editor's own vocabulary*, not a
 * registry lookup. A condition that names a field this editor cannot read is
 * unmapped, same as any other node it cannot carry.
 */
import type { components } from "@marula-core/sdk";

export type AutomationCondition = components["schemas"]["AutomationCondition"];
export type AutomationAction = components["schemas"]["AutomationAction"];
export type Priority = components["schemas"]["Priority"];
export type TicketStatusCategory = components["schemas"]["TicketStatusCategory"];

export type PluginActionDraft = {
  type: "plugin_action";
  entry_key: string;
  params: Record<string, unknown>;
};

/** Anything the simple editor can hold as an effect. */
export type ProjectedAction = AutomationAction | PluginActionDraft;

export interface UnmappedNode {
  /** The catalogue id, e.g. `flow.foreach`. */
  nodeType: string;
  /** The document's own node id, for pointing at it in the draft. */
  nodeId: string;
}

export interface ProjectionResult {
  /** The model's advisory name, when the envelope carried one. */
  name: string | null;
  /** The trigger the envelope chose, verbatim — it is the document's entry. */
  triggerEvent: string | null;
  conditions: AutomationCondition[];
  actions: ProjectedAction[];
  /** Nodes the simple editor cannot show. A non-empty list blocks nothing but
   * is shown honestly, and the apply button says what will be dropped. */
  unmapped: UnmappedNode[];
}

const PRIORITIES = new Set<string>(["p1", "p2", "p3", "p4"]);
const CATEGORIES = new Set<string>(["open", "in_progress", "waiting", "closed"]);

/** The registry field names this editor's five facts travel under. */
const FIELD_ALIASES: Record<string, "priority" | "category" | "title" | "assignee"> = {
  priority: "priority",
  status_category: "category",
  category: "category",
  status: "category",
  title: "title",
  assignee_id: "assignee",
  assignee: "assignee",
};

function asRecord(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function asList(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

/** `field.equals` value kinds this editor can carry. `string | string[]`. */
function conditionValue(value: unknown): string | null {
  const raw = typeof value === "string" ? value : Array.isArray(value) ? value.find(asString) : null;
  return raw ?? null;
}

/**
 * One body node in the draft. Lane 17's envelope keeps the trigger OUTSIDE
 * the document (`document.nodes` must not contain a trigger node), so a
 * projection never has to un-triggers a body.
 */
interface BodyNode {
  nodeId: string;
  nodeType: string;
  params: Record<string, unknown>;
}

export function projectDraft(
  name: unknown,
  triggerEvent: unknown,
  document: unknown,
): ProjectionResult {
  const body = asRecord(document);
  const nodes: BodyNode[] = asList(body["nodes"]).map((n) => {
    const node = asRecord(n);
    return {
      nodeId: asString(node["id"]) ?? "",
      nodeType: asString(node["node_type"]) ?? "",
      params: asRecord(node["params"]),
    };
  });

  const conditions: AutomationCondition[] = [];
  const actions: ProjectedAction[] = [];
  const unmapped: UnmappedNode[] = [];

  for (const node of nodes) {
    const mapped = projectCondition(node) ?? projectAction(node);
    if (mapped === null) {
      unmapped.push({ nodeType: node.nodeType || "(unnamed)", nodeId: node.nodeId });
      continue;
    }
    if (mapped.kind === "condition") conditions.push(mapped.condition);
    else actions.push(mapped.action);
  }

  return {
    name: asString(name),
    triggerEvent: asString(triggerEvent),
    conditions,
    actions,
    unmapped,
  };
}

type Projected =
  | { kind: "condition"; condition: AutomationCondition }
  | { kind: "action"; action: ProjectedAction };

/** Conditions, only where the field names a fact this editor owns. */
function projectCondition(node: BodyNode): Projected | null {
  if (node.nodeType === "field.isEmpty") {
    // "no value at all" is only expressible here for the one nullable fact
    // the rule vocabulary carries — the assignee.
    return FIELD_ALIASES[asString(node.params["field"]) ?? ""] === "assignee"
      ? { kind: "condition", condition: { type: "unassigned" } }
      : null;
  }
  if (node.nodeType !== "field.equals" && node.nodeType !== "field.notEquals" && node.nodeType !== "field.contains") {
    return null;
  }
  const field = asString(node.params["field"]);
  const value = conditionValue(node.params["value"]);
  if (field === null || value === null) return null;
  const fact = FIELD_ALIASES[field];
  const isNegation = node.nodeType === "field.notEquals";

  if (fact === "priority") {
    if (!PRIORITIES.has(value)) return null;
    if (isNegation) return { kind: "condition", condition: { type: "priority_is_not", priority: value as Priority } };
    return { kind: "condition", condition: { type: "priority_is", priority: value as Priority } };
  }
  if (fact === "category") {
    if (!CATEGORIES.has(value)) return null;
    if (isNegation) {
      return { kind: "condition", condition: { type: "category_is_not", category: value as TicketStatusCategory } };
    }
    return { kind: "condition", condition: { type: "category_is", category: value as TicketStatusCategory } };
  }
  if (fact === "title" && node.nodeType === "field.contains" && value.trim() !== "") {
    return { kind: "condition", condition: { type: "title_contains", needle: value } };
  }
  if (fact === "assignee" && node.nodeType === "field.notEquals" && value === "unassigned") {
    return { kind: "condition", condition: { type: "assignee_is_set" } };
  }
  return null;
}

/** Actions, only the five this editor's THEN step can hold. */
function projectAction(node: BodyNode): Projected | null {
  switch (node.nodeType) {
    case "ticket.setPriority": {
      const priority = asString(node.params["priority"]);
      if (priority === null || !PRIORITIES.has(priority)) return null;
      return { kind: "action", action: { type: "set_priority", priority: priority as Priority } };
    }
    case "ticket.assign": {
      const userId = asString(node.params["userId"]);
      if (userId === null) return null;
      return { kind: "action", action: { type: "assign_user", user_id: userId } };
    }
    case "ticket.internalNote": {
      const body = asString(node.params["body"]);
      // A note of whitespace is not a note: the editor's own validation
      // refuses it, so the projection refuses it instead of handing over a
      // draft the save button would bounce.
      if (body === null || body.trim() === "") return null;
      return { kind: "action", action: { type: "add_internal_comment", body } };
    }
    case "notification.send": {
      const userIds = asList(node.params["userIds"]).filter(isUuidString);
      const kind = asString(node.params["kind"]);
      const title = asString(node.params["title"]);
      if (userIds.length === 0 || kind === null || title === null) return null;
      return {
        kind: "action",
        action: {
          type: "notify_users",
          user_ids: userIds,
          kind: kind as components["schemas"]["NotificationKind"],
          title,
        },
      };
    }
    case "plugin.action": {
      const entryKey = asString(node.params["entryKey"]);
      if (entryKey === null) return null;
      return {
        kind: "action",
        action: { type: "plugin_action", entry_key: entryKey, params: asRecord(node.params["params"]) },
      };
    }
    default:
      return null;
  }
}

function isUuidString(value: unknown): value is string {
  return typeof value === "string" && /^[0-9a-f-]{36}$/i.test(value);
}
