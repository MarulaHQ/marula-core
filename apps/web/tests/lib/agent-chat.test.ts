// The chat transport's refusal mapping.
//
// A turn that answers 401 is an auth problem, not an empty answer: the surface
// must say "sign in again" instead of reporting that no answer came back.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { runChatTurn } from "../../lib/agent-chat";
import { seedAuthTokens } from "../helpers/mock-api";

const SESSION = "00000000-0000-4000-8000-000000000001";

beforeEach(() => {
  seedAuthTokens();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("runChatTurn", () => {
  it("maps a 401 to an auth refusal with sign-in copy", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({ status: 401 }) as unknown as Response),
    );

    const result = await runChatTurn(SESSION, {
      surface_key: "general_chat",
      messages: [],
    });

    expect(result.refusal).toBe("unauthenticated");
    expect(result.refusalMessage).toMatch(/sign in/i);
    expect(result.text).toBe("");
  });
});
