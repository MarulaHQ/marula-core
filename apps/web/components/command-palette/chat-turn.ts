// The palette's Ask leg.
//
// Purpose: one turn on the general_chat surface, streamed into the shared
// store so the palette (K02) and the panel (K03) render the same conversation.
// Sources come from the frames the runner actually sends: the screen context
// the turn was given, plus each tool_invocation. A proposal frame becomes a
// dashed row that links to the agent platform's approval queue — this module
// never approves anything.
//
// Non-purpose: no second approval path, no transcript persistence, no model
// picker (the runner resolves the org/user default for the surface).

"use client";

import { announce } from "../a11y/live-region";
import { runChatTurn, type ChatFrame } from "../../lib/agent-chat";
import { contextPrompt, type ScreenContext } from "./screen-context";
import { actions, chatSessionId, getSnapshot } from "./store";

const OUTCOME_WORDS: Record<string, string> = {
  step_limit_reached: "it reached the step limit",
  hard_cap_reached: "the org's hard cap was reached",
  verification_held: "verification held the answer",
  proposal_failed: "the proposal could not be filed",
  tool_execution_failed: "a tool call failed",
};

function stringOf(value: unknown): string | null {
  return typeof value === "string" && value !== "" ? value : null;
}

export async function sendChatTurn(text: string, context: ScreenContext): Promise<void> {
  const trimmed = text.trim();
  if (trimmed === "" || getSnapshot().busy) return;

  const history = getSnapshot().messages
    .filter((message) => message.role === "user" || message.text !== "")
    .map((message) => ({
      role: message.role === "user" ? "user" : "assistant",
      content: message.text,
    }));

  actions.pushUserMessage(trimmed);
  const assistant = actions.startAssistant();
  announce("Assistant is answering", "polite");

  // The context chip's facts are a source too: the turn was given them.
  const rowCount = context.rowCount;
  if (rowCount !== undefined) {
    actions.addSource(assistant.id, {
      label: `Using ${rowCount} ${rowCount === 1 ? "row" : "rows"}${context.view !== undefined ? ` from ${context.view}` : ""}`,
      detail: null,
    });
  }

  let completedText: string | null = null;
  let stopped: string | null = null;

  const onFrame = (frame: ChatFrame): void => {
    if (frame.event === "assistant_delta") {
      const delta = stringOf(frame.data.delta);
      if (delta !== null) actions.appendDelta(assistant.id, delta);
      return;
    }
    if (frame.event === "tool_invocation") {
      const tool = stringOf(frame.data.tool) ?? stringOf(frame.data.capability) ?? "a tool";
      const outcome = stringOf(frame.data.outcome);
      actions.addSource(assistant.id, {
        label: `Used ${tool}`,
        detail: outcome === null ? null : outcome.replaceAll("_", " "),
      });
      return;
    }
    if (frame.event === "assist_proposed") {
      const proposalId = stringOf(frame.data.proposal_id) ?? undefined;
      const ticketId = stringOf(frame.data.ticket_id) ?? undefined;
      const title = stringOf(frame.data.title) ?? "A change is proposed";
      actions.addProposal(assistant.id, {
        proposalId,
        ticketId,
        title,
        detail: "Waits for your approval. Nothing is sent.",
      });
      return;
    }
    if (frame.event === "assist_held" || frame.event === "assist_verification") {
      const reasons = Array.isArray(frame.data.reasons)
        ? frame.data.reasons.filter((reason): reason is string => typeof reason === "string")
        : [];
      actions.addSource(assistant.id, {
        label: "A proposed change was held for verification",
        detail: reasons.length === 0 ? null : reasons.join(", "),
      });
      return;
    }
    if (frame.event === "turn_completed") {
      const text = stringOf(frame.data.text);
      if (text !== null) completedText = text;
      const outcome = stringOf(frame.data.outcome);
      if (outcome !== null && outcome !== "completed") {
        stopped = OUTCOME_WORDS[outcome] ?? `it stopped at ${outcome.replaceAll("_", " ")}`;
      }
    }
  };

  try {
    const result = await runChatTurn(
      chatSessionId(),
      {
        surface_key: "general_chat",
        session_opener: history.length === 0,
        messages: [
          { role: "system", content: contextPrompt(context) },
          ...history,
          { role: "user", content: trimmed },
        ],
      },
      onFrame,
    );

    if (result.refusal !== undefined) {
      actions.failAssistant(
        assistant.id,
        result.refusalMessage ?? "The assistant is not available for this surface.",
      );
      announce("The assistant is not available", "assertive");
      return;
    }

    actions.finishAssistant(assistant.id, completedText ?? result.text);
    if (stopped !== null) {
      actions.failAssistant(assistant.id, `The assistant stopped early: ${stopped}.`);
      announce("The assistant stopped early", "polite");
      return;
    }
    if (getSnapshot().messages.some((m) => m.id === assistant.id && m.text === "")) {
      actions.failAssistant(assistant.id, "No answer came back. Try again.");
      announce("No answer came back", "assertive");
      return;
    }
    const firstLine = (completedText ?? result.text).split("\n")[0] ?? "";
    announce(firstLine, "polite");
  } catch (error) {
    actions.failAssistant(
      assistant.id,
      error instanceof Error ? error.message : "The turn did not complete.",
    );
    announce("The turn did not complete", "assertive");
  }
}
