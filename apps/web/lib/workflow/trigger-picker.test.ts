import { describe, expect, it } from "vitest";
import {
  eventLabel,
  hasTriggerValue,
  humanizeEventType,
  nodeEventTypes,
  registryTriggerGroups,
  triggerPhrase,
  triggerValues,
  type NodeView,
} from "./trigger-picker";

// The SDK types `input_schema`/`output_schema` as `Record<string, never>`, but on
// the wire they are arbitrary JSON Schema objects. The helper takes the real
// shape so a fixture can declare an `eventType` enum, then hands back a NodeView.
type NodeFixture = Partial<Omit<NodeView, "input_schema" | "output_schema">> &
  Pick<NodeView, "id" | "label"> & {
    input_schema?: Record<string, unknown>;
    output_schema?: Record<string, unknown>;
  };

function node(partial: NodeFixture): NodeView {
  return {
    agent_description: "",
    family: "trigger",
    tier: "a",
    idempotent: true,
    audience: "human",
    executor: "trigger",
    input_schema: { type: "object" },
    output_schema: { type: "object" },
    emits: [],
    ...partial,
  } as unknown as NodeView;
}

const ticketNode = node({
  id: "trigger.ticket",
  label: "Ticket lifecycle",
  agent_description: "Starts a run when something happens to a ticket.",
  input_schema: {
    type: "object",
    properties: { eventType: { type: "string", enum: ["ticket.created", "ticket.status_changed"] } },
  },
});

// A trigger node with no eventType enum (schedule / manual / webhook) must not
// contribute an empty, unselectable group.
const scheduleNode = node({
  id: "trigger.schedule",
  label: "Schedule",
  input_schema: { type: "object", properties: { scheduleId: { type: "string" } } },
});

describe("registry trigger groups", () => {
  it("builds one group per registry trigger node, in registry order", () => {
    const groups = registryTriggerGroups({ nodes: [ticketNode, scheduleNode] } as never);
    // The schedule node declares no event types, so it is skipped.
    expect(groups.map((g) => g.id)).toEqual(["trigger.ticket", "platform"]);
    expect(groups[0]!.label).toBe("Ticket lifecycle");
    expect(groups[0]!.options).toEqual([
      { value: "ticket.created", label: "Ticket created" },
      { value: "ticket.status_changed", label: "Ticket status changed" },
    ]);
  });

  it("keeps the platform's own envelopes available beside the nodes", () => {
    const groups = registryTriggerGroups({ nodes: [ticketNode] } as never);
    const platform = groups.find((g) => g.id === "platform");
    expect(platform?.options.map((o) => o.value)).toEqual(["webhook.ingested", "notification.created"]);
    expect(hasTriggerValue(groups, "webhook.ingested")).toBe(true);
  });

  it("offers only the platform group when the catalogue is not loaded", () => {
    expect(registryTriggerGroups(null).map((g) => g.id)).toEqual(["platform"]);
    expect(registryTriggerGroups(undefined).map((g) => g.id)).toEqual(["platform"]);
  });

  it("does not decide which events exist — a new node's enums arrive as-is", () => {
    const brandNew = node({
      id: "trigger.fleet",
      label: "Fleet telemetry",
      input_schema: { type: "object", properties: { eventType: { enum: ["device.offline"] } } },
    });
    const groups = registryTriggerGroups({ nodes: [brandNew] } as never);
    expect(triggerValues(groups)).toContain("device.offline");
    expect(groups[0]!.options[0]!.label).toBe("Device offline");
  });

  it("reads an eventType enum defensively", () => {
    expect(nodeEventTypes(node({ id: "x", label: "X", input_schema: { type: "object" } }))).toEqual([]);
    expect(nodeEventTypes(node({ id: "x", label: "X", input_schema: {} }))).toEqual([]);
    expect(
      nodeEventTypes(
        node({
          id: "x",
          label: "X",
          input_schema: { properties: { eventType: { enum: ["a.b", 7, null] } } },
        }),
      ),
    ).toEqual(["a.b"]);
  });
});

describe("trigger wording", () => {
  it("prefers the curated label and falls back to a humanised one", () => {
    expect(eventLabel("ticket.created")).toBe("Ticket created");
    expect(eventLabel("device.offline")).toBe("Device offline");
    // Acronyms keep their case rather than reading as a word.
    expect(eventLabel("csat.response_recorded")).toBe("CSAT response recorded");
  });

  it("humanises an unknown type instead of hiding it", () => {
    expect(humanizeEventType("invoice_run.completed")).toBe("Invoice run completed");
    expect(humanizeEventType("")).toBe("");
  });

  it("gives every event a clause for the read-back sentence", () => {
    expect(triggerPhrase("ticket.created")).toBe("a ticket is created");
    // No curated phrase: a grammatical clause is derived, never the raw string.
    expect(triggerPhrase("device.offline")).toBe("the device offline event fires");
  });
});
