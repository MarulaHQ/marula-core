// One Ask turn, two sessions.
//
// Purpose: pin what a reset means for a turn that is still in flight. When the
// signed-in account changes, the store is cleared and the next account's turn
// gets its own messages. The old turn must then neither clear the new turn's
// `busy` flag nor announce the old account's answer through the live region.
//
// Non-purpose: the SSE frames themselves, which `lib/agent-chat` owns.
import { beforeEach, describe, expect, it, vi } from "vitest";

const announce = vi.hoisted(() => vi.fn());
const runChatTurn = vi.hoisted(() => vi.fn());

vi.mock("../../components/a11y/live-region", () => ({ announce }));
vi.mock("../../lib/agent-chat", () => ({ runChatTurn }));

import { actions, getSnapshot } from "../../components/command-palette/store";
import { sendChatTurn } from "../../components/command-palette/chat-turn";

beforeEach(() => {
  actions.resetSession();
  announce.mockClear();
  runChatTurn.mockReset();
});

describe("a turn that outlives its session", () => {
  it("does not clear the next session's busy flag", () => {
    const oldTurn = actions.startAssistant();
    actions.resetSession();
    const freshTurn = actions.startAssistant();
    expect(getSnapshot().busy).toBe(true);

    // The old message is gone with the reset, so finishing it is a no-op.
    expect(actions.finishAssistant(oldTurn.id, "Old account's answer")).toBe(false);
    expect(getSnapshot().busy).toBe(true);
    expect(getSnapshot().messages.map((message) => message.id)).toEqual([freshTurn.id]);

    // Ids stay monotonic, so the old id can never collide with a new message.
    const seq = (id: string) => Number(id.split("-")[1]);
    expect(seq(freshTurn.id)).toBeGreaterThan(seq(oldTurn.id));

    expect(actions.finishAssistant(freshTurn.id, "The current answer")).toBe(true);
    expect(getSnapshot().busy).toBe(false);
  });

  it("does not announce the previous account's answer after a reset", async () => {
    runChatTurn.mockImplementation(
      async (
        _sessionId: string,
        _turn: unknown,
        onFrame?: (frame: { event: string; data: Record<string, unknown> }) => void,
      ) => {
        onFrame?.({ event: "assistant_delta", data: { delta: "Old account's answer" } });
        // The account changes while the answer is still streaming.
        actions.resetSession();
        return { frames: [], text: "Old account's answer" };
      },
    );

    await sendChatTurn("What did I spend this month?", { label: "Billing" });

    const spoken = announce.mock.calls.map((call) => call[0]);
    expect(spoken).toEqual(["Assistant is answering"]);
    expect(getSnapshot().messages).toHaveLength(0);
    expect(getSnapshot().busy).toBe(false);
  });
});
