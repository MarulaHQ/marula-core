/**
 * The AI-trace rule (wave41 D2, owned by lane 20, checked by lane 15).
 *
 * The standing rule is "no grant, no trace of AI". Wave 41's mockup review
 * found the rule pulled two ways on the tickets list: the no-grant state must
 * show no trace of AI, yet a ticket whose last activity was an agent
 * suggestion shows "Assistant suggestion · 15m" in its activity column. The
 * two are not in conflict once the word "trace" is split:
 *
 *   - An AFFORDANCE offers, hints at, or leaves a gap for an AI action. It
 *     renders only when the person holds a grant for that surface. This is
 *     what "no trace" governs: no sparkle control, no "ask the assistant"
 *     row, no empty slot where a suggestion would sit.
 *   - HISTORY is a record of what already happened. An agent action that
 *     took place is a fact about the ticket, and hiding it would make the
 *     audit trail lie. Its text renders whether or not a grant exists.
 *
 * THE CALL, made here so lane 15 has one thing to check: a recorded agent
 * entry in the activity column is history, so its text renders with or
 * without a grant; the sparkle glyph is an affordance mark, so it renders
 * only with a grant. The mockup drew both on a no-grant screen; the glyph is
 * the part the rule removes.
 *
 * Non-purpose: this file decides what an agent entry may draw. It does not
 * fetch grants, own the tickets list, or style anything.
 */

/** Whether a trace is an offer of AI, or a record that AI already acted. */
export type AgentTraceKind = "affordance" | "history";

/** What an activity-column entry was, before it is drawn. */
export type ActivityOrigin = "human" | "system" | "agent_recommendation" | "agent_action";

/** The drawing decision for one activity entry. */
export interface AgentTraceDraw {
  /** Activity entries are always history: they record what happened. */
  kind: "history";
  /** The entry's own text, for example "Assistant suggestion · 15m". */
  showText: boolean;
  /** The AI affordance glyph (the sparkle) that marks a live AI offer. */
  showGlyph: boolean;
}

/** Whether a trace of this kind may render for this person at all. A live
 *  affordance needs the grant; history always renders. */
export function mayRenderAgentTrace(trace: AgentTraceKind, granted: boolean): boolean {
  return trace === "history" || granted;
}

/** Whether an activity entry came from the assistant. */
export function isAgentOrigin(origin: ActivityOrigin): boolean {
  return origin === "agent_recommendation" || origin === "agent_action";
}

/**
 * The one decision the "Last activity" cell makes, so the tickets list and
 * its test agree. The text of a recorded entry always renders; the sparkle is
 * an affordance mark and follows the grant.
 */
export function resolveAgentTrace(origin: ActivityOrigin, granted: boolean): AgentTraceDraw {
  return {
    kind: "history",
    showText: true,
    showGlyph: isAgentOrigin(origin) && granted,
  };
}
