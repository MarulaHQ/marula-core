/**
 * The wave41 D2 AI-trace call, checked. Lane 15 (`p41-ui-audit`) runs this
 * suite as its deterministic check, so the tickets list cannot drift back to
 * the mockup's no-grant sparkle.
 */
import { describe, expect, it } from "vitest";
import {
  isAgentOrigin,
  mayRenderAgentTrace,
  resolveAgentTrace,
  type ActivityOrigin,
} from "./ai-trace";

describe("AI-trace rule", () => {
  it("renders a live AI affordance only with a grant", () => {
    expect(mayRenderAgentTrace("affordance", true)).toBe(true);
    expect(mayRenderAgentTrace("affordance", false)).toBe(false);
  });

  it("renders recorded history with or without a grant", () => {
    expect(mayRenderAgentTrace("history", true)).toBe(true);
    expect(mayRenderAgentTrace("history", false)).toBe(true);
  });

  it("keeps the recorded text of an agent entry on a no-grant screen", () => {
    const draw = resolveAgentTrace("agent_action", false);
    expect(draw.kind).toBe("history");
    expect(draw.showText).toBe(true);
  });

  it("drops the sparkle glyph on a no-grant screen, including for a recorded suggestion", () => {
    expect(resolveAgentTrace("agent_recommendation", false).showGlyph).toBe(false);
    expect(resolveAgentTrace("agent_action", false).showGlyph).toBe(false);
  });

  it("shows the sparkle glyph only for an agent entry when a grant exists", () => {
    expect(resolveAgentTrace("agent_recommendation", true).showGlyph).toBe(true);
    expect(resolveAgentTrace("agent_action", true).showGlyph).toBe(true);
    for (const origin of ["human", "system"] as const) {
      expect(resolveAgentTrace(origin, true).showGlyph).toBe(false);
    }
  });

  it("treats only the two agent origins as agent-caused", () => {
    const agent: ActivityOrigin[] = ["agent_recommendation", "agent_action"];
    const notAgent: ActivityOrigin[] = ["human", "system"];
    expect(agent.every(isAgentOrigin)).toBe(true);
    expect(notAgent.some(isAgentOrigin)).toBe(false);
  });
});
