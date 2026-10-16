// Unit suite: the C2 projection (Wave 43 lane 17b).
//
// The prose leg hands back a lane-17 workflow DOCUMENT; this module is the
// only place that document becomes the simple editor's shape. Every test
// here is also a statement about the boundary: what projects, what comes
// back named as unmapped, and why a field name this editor cannot read is
// unmapped rather than guessed.
import { describe, expect, it } from "vitest";
import { projectDraft } from "../../app/automations/draft-projection";

function doc(nodes: unknown, edges: unknown = []): Record<string, unknown> {
  return { version: "1", nodes, edges };
}

describe("projection of a lane-17 draft", () => {
  it("carries the envelope's name and trigger, and never a trigger node", () => {
    const result = projectDraft(
      "Escalate a P1",
      "ticket.created",
      doc([{ id: "raise", node_type: "ticket.setPriority", params: { priority: "p1" } }]),
    );
    expect(result.name).toBe("Escalate a P1");
    expect(result.triggerEvent).toBe("ticket.created");
    expect(result.conditions).toEqual([]);
    expect(result.actions).toEqual([{ type: "set_priority", priority: "p1" }]);
    expect(result.unmapped).toEqual([]);
  });

  it("unfolds the ticket actions the simple editor can hold", () => {
    const result = projectDraft(
      null,
      "ticket.updated",
      doc([
        { id: "p", node_type: "ticket.setPriority", params: { priority: "p2" } },
        { id: "a", node_type: "ticket.assign", params: { userId: "00000000-0000-0000-0000-0000000000aa" } },
        { id: "n", node_type: "ticket.internalNote", params: { body: "picked up" } },
        {
          id: "m",
          node_type: "notification.send",
          params: {
            userIds: ["00000000-0000-0000-0000-0000000000bb"],
            kind: "escalation",
            title: "P2 arrived",
          },
        },
        { id: "pl", node_type: "plugin.action", params: { entryKey: "rmm:quarantine", params: { body: "done" } } },
      ]),
    );
    expect(result.actions).toEqual([
      { type: "set_priority", priority: "p2" },
      { type: "assign_user", user_id: "00000000-0000-0000-0000-0000000000aa" },
      { type: "add_internal_comment", body: "picked up" },
      {
        type: "notify_users",
        user_ids: ["00000000-0000-0000-0000-0000000000bb"],
        kind: "escalation",
        title: "P2 arrived",
      },
      { type: "plugin_action", entry_key: "rmm:quarantine", params: { body: "done" } },
    ]);
    expect(result.unmapped).toEqual([]);
  });

  it("projects only the conditions whose field names a fact this editor owns", () => {
    const result = projectDraft(null, "ticket.created", doc([
      { id: "a", node_type: "field.equals", params: { field: "priority", value: "p1" } },
      { id: "b", node_type: "field.notEquals", params: { field: "status_category", value: "closed" } },
      { id: "c", node_type: "field.contains", params: { field: "title", value: "vpn" } },
      { id: "d", node_type: "field.notEquals", params: { field: "assignee_id", value: "unassigned" } },
      { id: "e", node_type: "field.isEmpty", params: { field: "assignee_id" } },
    ]));
    expect(result.conditions).toEqual([
      { type: "priority_is", priority: "p1" },
      { type: "category_is_not", category: "closed" },
      { type: "title_contains", needle: "vpn" },
      { type: "assignee_is_set" },
      { type: "unassigned" },
    ]);
    expect(result.unmapped).toEqual([]);
  });

  it("names what it drops, by node, rather than pretending it can show it", () => {
    const result = projectDraft(null, "ticket.created", doc([
      { id: "loop", node_type: "flow.foreach", params: {} },
      { id: "wait", node_type: "flow.delay", params: { seconds: 300 } },
    ]));
    expect(result.conditions).toEqual([]);
    expect(result.actions).toEqual([]);
    expect(result.unmapped).toEqual([
      { nodeType: "flow.foreach", nodeId: "loop" },
      { nodeType: "flow.delay", nodeId: "wait" },
    ]);
  });

  it("drops a condition whose field or value the editor cannot carry", () => {
    const result = projectDraft(null, "ticket.created", doc([
      { id: "x", node_type: "field.equals", params: { field: "sla_remaining", value: "3600" } },
      { id: "y", node_type: "field.equals", params: { field: "priority", value: "p9" } },
      { id: "z", node_type: "field.equals", params: { field: "title" } },
    ]));
    expect(result.conditions).toEqual([]);
    expect(result.unmapped.map((u) => u.nodeId)).toEqual(["x", "y", "z"]);
  });

  it("drops an action whose params are not enough to build one", () => {
    const result = projectDraft(null, "ticket.updated", doc([
      { id: "p", node_type: "ticket.setPriority", params: { priority: "p9" } },
      { id: "a", node_type: "ticket.assign", params: {} },
      { id: "n", node_type: "ticket.internalNote", params: { body: "   " } },
      {
        id: "m",
        node_type: "notification.send",
        params: { userIds: "not-a-list", kind: "escalation", title: "hi" },
      },
    ]));
    expect(result.actions).toEqual([]);
    expect(result.unmapped).toHaveLength(4);
  });

  it("is defensive about a document that is not a document", () => {
    const result = projectDraft(null, null, "nope");
    expect(result.name).toBeNull();
    expect(result.triggerEvent).toBeNull();
    expect(result.conditions).toEqual([]);
    expect(result.actions).toEqual([]);
    expect(result.unmapped).toEqual([]);
  });
});
