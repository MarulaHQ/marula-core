// ChatRunner client — the live general chat surface on the agent loop.
//
// Purpose: ONE browser-side client for POST
// /api/v1/ai/chat/sessions/{session_id}/turns. The runner is SSE and
// openapi-fetch cannot stream, so this rides fetch() with the same base
// URL + bearer token the typed client uses; the SSE loop-frame contract
// (turn_started / assistant_delta / tool_invocation / assist_proposed /
// assist_held / turn_completed) is parsed exactly.
//
// Posture: typed refusals answer BEFORE any stream (401 unauthenticated, 403
// codes) — clients branch on the machine reason and render typed
// not-available states, never a dead spinner.

import { apiBaseUrl, authReady, loadTokens } from "./api";

export type ChatFrameName =
  | "turn_started"
  | "assistant_delta"
  | "tool_invocation"
  | "assist_proposed"
  | "assist_verification"
  | "assist_held"
  | "turn_completed";

export interface ChatFrame {
  event: ChatFrameName;
  data: Record<string, unknown>;
}

export interface ChatTurnResult {
  frames: ChatFrame[];
  /** The final assistant text (accumulated deltas or turn payload). */
  text: string;
  /** The typed refusal code when the turn was refused pre-stream. */
  refusal?: string;
  refusalMessage?: string;
}

/** A stream that sends nothing for this long is treated as finished, so a
 *  stalled connection cannot hold the surface's busy state forever. */
const STREAM_IDLE_MS = 30_000;

/** One bounded model turn; resolves when the SSE stream closes. `onFrame`
 *  receives each parsed frame as it arrives, which is what lets a chat surface
 *  render deltas, sources and proposals while the turn runs. */
export async function runChatTurn(
  sessionId: string,
  turn: {
    surface_key: string;
    messages: Array<{ role: string; content: string }>;
    provider?: string | null;
    model?: string | null;
    session_opener?: boolean;
  },
  onFrame?: (frame: ChatFrame) => void,
): Promise<ChatTurnResult> {
  const base = apiBaseUrl();
  // A full page load restores the token from the session cookie. Wait for
  // that only when there is no token yet; a seeded test session proceeds.
  if (loadTokens() === null) await authReady();
  const tokens = loadTokens();
  const response = await fetch(
    `${base}/api/v1/ai/chat/sessions/${sessionId}/turns`,
    {
      method: "POST",
      credentials: "include",
      headers: {
        "content-type": "application/json",
        accept: "text/event-stream",
        ...(tokens === null ? {} : { authorization: `Bearer ${tokens.access_token}` }),
      },
      body: JSON.stringify(turn),
    },
  );
  // The turns route reads the bearer header only, so a request sent while the
  // token is missing comes back 401 with no stream at all. Left unhandled it
  // reads as an empty answer ("No answer came back") instead of a session that
  // needs signing in again.
  if (response.status === 401) {
    return {
      frames: [],
      text: "",
      refusal: "unauthenticated",
      refusalMessage: "Your session has expired. Sign in again to use the assistant.",
    };
  }
  if (response.status === 403 || response.status === 400 || response.status === 429) {
    const body = (await response.json().catch(() => ({}))) as Record<string, unknown>;
    return {
      frames: [],
      text: "",
      refusal: "ai_refused",
      refusalMessage:
        typeof body["message"] === "string"
          ? (body["message"] as string)
          : `refused (${response.status})`,
    };
  }
  if (response.body === null) {
    return { frames: [], text: "", refusal: "transport", refusalMessage: "no stream" };
  }
  const frames: ChatFrame[] = [];
  let text = "";
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let currentEvent = "";
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  const armIdleTimer = (): void => {
    if (idleTimer !== undefined) clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
      // Cancelling resolves the pending read with `done`, which takes the
      // normal completion path instead of leaving the turn open.
      void reader.cancel();
    }, STREAM_IDLE_MS);
  };
  try {
    armIdleTimer();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      armIdleTimer();
      buffer += decoder.decode(value, { stream: true });
      let sep: number = buffer.indexOf("\n\n");
      while (sep !== -1) {
        const chunk = buffer.slice(0, sep);
        buffer = buffer.slice(sep + 2);
        sep = buffer.indexOf("\n\n");
        let data = "";
        for (const line of chunk.split("\n")) {
          if (line.startsWith("event:")) currentEvent = line.slice(6).trim();
          else if (line.startsWith("data:")) data += line.slice(5).trim();
        }
        if (currentEvent === "" || data === "") continue;
        const parsed = safeJson(data);
        if (parsed === null) continue;
        const frame: ChatFrame = { event: currentEvent as ChatFrameName, data: parsed };
        frames.push(frame);
        onFrame?.(frame);
        if (currentEvent === "assistant_delta" && typeof parsed.delta === "string") {
          text += parsed.delta;
        } else if (currentEvent === "turn_completed" && typeof parsed.text === "string") {
          text = parsed.text;
        }
        currentEvent = "";
      }
    }
  } finally {
    if (idleTimer !== undefined) clearTimeout(idleTimer);
  }
  return { frames, text };
}

function safeJson(raw: string): Record<string, unknown> | null {
  try {
    return JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return null;
  }
}


