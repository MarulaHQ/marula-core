import { describe, expect, it } from "vitest";
import {
  emptyDocument,
  edgeKey,
  parseDocument,
  validateDocument,
  type WorkflowDocumentDoc,
  type WorkflowNodeDoc,
} from "./document";
import { deriveDocumentFromRule } from "./derive";
import {
  schemaFields,
  seedParams,
  validateAgainstSchema,
  type JsonSchema,
} from "./schema";

describe("the declared JSON-Schema subset (mirror of the Rust validator)", () => {
  const schema: JsonSchema = {
    type: "object",
    additionalProperties: false,
    properties: {
      eventType: { type: "string", enum: ["ticket.created", "ticket.closed"] },
      boardId: { type: "string", format: "uuid" },
      count: { type: "integer", minimum: 1, maximum: 10 },
      note: { type: "string", maxLength: 300 },
    },
    required: ["eventType"],
  };

  it("accepts a valid instance", () => {
    expect(validateAgainstSchema(schema, { eventType: "ticket.created", count: 3 })).toEqual([]);
  });

  it("reports a missing required field by name", () => {
    const violations = validateAgainstSchema(schema, { count: 3 });
    expect(violations).toHaveLength(1);
    expect(violations[0]?.path).toBe("eventType");
    expect(violations[0]?.message).toContain("required");
  });

  it("honours enum, uuid, and numeric bounds", () => {
    expect(validateAgainstSchema(schema, { eventType: "nope" })[0]?.message).toContain("one of");
    expect(validateAgainstSchema(schema, { eventType: "ticket.created", boardId: "x" })[0]?.path).toBe(
      "boardId",
    );
    expect(
      validateAgainstSchema(schema, { eventType: "ticket.created", count: 11 })[0]?.message,
    ).toContain("at most");
  });

  it("refuses an unknown field when additionalProperties is false", () => {
    const violations = validateAgainstSchema(schema, { eventType: "ticket.created", extra: 1 });
    expect(violations.some((v) => v.path === "extra")).toBe(true);
  });

  it("renders fields in declaration order and seeds enum/const defaults", () => {
    expect(schemaFields(schema).map((f) => f.key)).toEqual(["eventType", "boardId", "count", "note"]);
    const seeded = seedParams({ type: "object", properties: { onError: { const: "halt" } } });
    expect(seeded).toEqual({ onError: "halt" });
  });
});

describe("the workflow document", () => {
  const condNode: WorkflowNodeDoc = {
    id: "a",
    node_type: "field.equals",
    params: { field: "priority", value: "p1" },
  };
  const stopNode: WorkflowNodeDoc = { id: "z", node_type: "flow.stop", params: {} };
  const known = new Set(["field.equals", "flow.stop", "ticket.update"]);
  const base: WorkflowDocumentDoc = {
    version: "1",
    nodes: [condNode, stopNode],
    edges: [{ from: "a", to: "z" }],
    loop_max_depth: 1,
  };

  it("parses a saved document and fills defaults", () => {
    const parsed = parseDocument({ version: "1", nodes: base.nodes, edges: base.edges });
    expect(parsed?.loop_max_depth).toBe(1);
    expect(parsed?.nodes[0]?.node_type).toBe("field.equals");
  });

  it("is clean for a well-formed graph", () => {
    expect(validateDocument(base, { knownNodeTypes: known })).toEqual([]);
  });

  it("flags a dangling edge", () => {
    const issues = validateDocument({ ...base, edges: [{ from: "a", to: "missing" }] }, { knownNodeTypes: known });
    expect(issues.some((i) => i.message.includes("does not exist"))).toBe(true);
  });

  it("flags a self-loop", () => {
    const issues = validateDocument({ ...base, edges: [{ from: "a", to: "a" }] }, { knownNodeTypes: known });
    expect(issues.some((i) => i.message.includes("itself"))).toBe(true);
  });

  it("flags a duplicate id", () => {
    const issues = validateDocument({ ...base, nodes: [condNode, condNode] }, { knownNodeTypes: known });
    expect(issues.some((i) => i.message.includes("Duplicate step id"))).toBe(true);
  });

  it("refuses embedded code anywhere in params (ADR 0016 rule (a))", () => {
    const issues = validateDocument(
      { ...base, nodes: [{ id: "a", node_type: "field.equals", params: { script: "eval(1)" } }, stopNode] },
      { knownNodeTypes: known },
    );
    expect(issues.some((i) => i.message.includes("Embedded code"))).toBe(true);
  });

  it("warns when a route policy has no error edge", () => {
    const issues = validateDocument(
      { ...base, nodes: [{ id: "a", node_type: "ticket.update", params: { onError: "route" } }, stopNode] },
      { knownNodeTypes: known },
    );
    expect(issues.some((i) => i.message.toLowerCase().includes("error"))).toBe(true);
  });

  it("detects a repeated edge by key, and labels are part of identity", () => {
    const issues = validateDocument({ ...base, edges: [{ from: "a", to: "z" }, { from: "a", to: "z" }] }, {
      knownNodeTypes: known,
    });
    expect(issues.some((i) => i.message.includes("Duplicate edge"))).toBe(true);
    expect(edgeKey("a", "z", undefined)).not.toBe(edgeKey("a", "z", "error"));
  });

  it("an empty document is a single stop node (never zero nodes)", () => {
    expect(emptyDocument().nodes).toHaveLength(1);
  });
});

describe("promoting a simple rule to the canvas (view toggle, not a migration)", () => {
  it("maps condition kinds to registry nodes", () => {
    const { document, issues } = deriveDocumentFromRule({
      trigger_event: "ticket.created",
      conditions: [
        { type: "priority_is", priority: "p1" },
        { type: "title_contains", needle: "vpn" },
      ],
      actions: [],
    });
    expect(document.nodes.map((n) => n.node_type)).toEqual(["field.equals", "field.contains"]);
    expect(document.nodes[0]?.params).toEqual({ field: "priority", value: "p1" });
    expect(document.nodes[1]?.params).toEqual({ field: "title", needle: "vpn" });
    expect(issues).toEqual([]);
  });

  it("maps a ticket-writing action without pinning a ticket (executor uses the triggering ticket)", () => {
    const { document, issues } = deriveDocumentFromRule({
      trigger_event: "ticket.created",
      conditions: [],
      actions: [{ type: "set_priority", priority: "p2" }],
    });
    expect(document.nodes[0]?.node_type).toBe("ticket.setPriority");
    expect(document.nodes[0]?.params).toEqual({ priority: "p2" });
    expect(issues).toEqual([]);
  });

  it("falls back to a single stop node for a rule with no steps", () => {
    const { document } = deriveDocumentFromRule({
      trigger_event: "ticket.created",
      conditions: [],
      actions: [],
    });
    expect(document.nodes).toHaveLength(1);
    expect(document.nodes[0]?.node_type).toBe("flow.stop");
  });
});
