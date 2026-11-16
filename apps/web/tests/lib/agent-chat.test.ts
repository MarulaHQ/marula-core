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

  it("names a successful response that carried no stream as a transport refusal", async () => {
    // A read that returns 2xx but no body is not an answer: without the typed
    // refusal the surface would report "No answer came back" for a transport
    // fault the person cannot act on.
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({ status: 200, body: null }) as unknown as Response),
    );

    const result = await runChatTurn(SESSION, {
      surface_key: "general_chat",
      messages: [{ role: "user", content: "hello" }],
    });

    expect(result.refusal).toBe("transport");
    expect(result.refusalMessage).toBe("no stream");
    expect(result.frames).toEqual([]);
  });

  it("returns no frames and no text when the stream closes without a turn", async () => {
    const empty = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.close();
      },
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(empty, { status: 200 })),
    );

    const result = await runChatTurn(SESSION, {
      surface_key: "general_chat",
      messages: [{ role: "user", content: "hello" }],
    });

    expect(result).toEqual({ frames: [], text: "" });
  });
});
