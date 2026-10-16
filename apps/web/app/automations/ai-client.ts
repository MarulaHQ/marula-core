/**
 * ai-client — the browser side of the rule builder's two AI legs.
 *
 * Both legs ride the SAME admission path as every other AI edge in this
 * product: an org grant on a native surface, behind the org master switch
 * (`ai.features_enabled`). The surface is `general_chat` — NOT the seeded
 * `automation_writing` key, which is a PLUGIN surface reserved for the
 * premium plugin (docs/planning/16 §141: core ships only the registry seam).
 *
 * C2 calls lane 17's endpoint (POST /api/v1/workflows/author), never a second
 * prose path, and it never saves: the endpoint itself is documented as
 * `saved: false, always`.
 *
 * C1 is SSE. openapi-fetch cannot stream, so this rides fetch() with the same
 * base URL + bearer token the typed client uses — the same posture as
 * `lib/agent-chat.ts`, whose frame parsing this mirrors.
 */
import { api, apiBaseUrl, loadTokens } from "../../lib/api";

// ---------------------------------------------------------------------------
// C1 — the live draft summary
// ---------------------------------------------------------------------------

export type SummaryFrameName = "summary_started" | "summary_delta" | "summary_completed" | "summary_failed";

export interface SummaryFrame {
  event: SummaryFrameName;
  data: Record<string, unknown>;
}

/** The draft facts the summary is written from. */
export interface DraftSummaryDraft {
  surface_key: string;
  name?: string | null;
  trigger_event: string;
  subject_kind?: string | null;
  scope?: string | null;
  conditions_text: string;
  actions_text: string[];
  incomplete_conditions?: number;
  provider?: string | null;
  model?: string | null;
}

export interface SummaryResult {
  /** The summary so far, as frames arrived. */
  text: string;
  frames: SummaryFrame[];
  modelInfo: { provider: string; model: string; resolution: string } | null;
  usage: { prompt_tokens: number; completion_tokens: number } | null;
  /** The typed refusal code when the call was refused pre-stream. */
  refusal?: string;
  refusalMessage?: string;
}

/**
 * One streamed draft summary. Resolves when the stream closes. `onDelta`
 * fires for every text fragment the server emits, so the caller renders the
 * sentence as it arrives rather than after a blocking wait.
 */
export async function streamDraftSummary(
  draft: DraftSummaryDraft,
  onDelta: (fragment: string) => void,
): Promise<SummaryResult> {
  const base = apiBaseUrl();
  const tokens = loadTokens();
  if (tokens === null) {
    return {
      text: "",
      frames: [],
      modelInfo: null,
      usage: null,
      refusal: "auth",
      refusalMessage: "not signed in",
    };
  }
  const response = await fetch(`${base}/api/v1/automation-rules/draft-summary`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      accept: "text/event-stream",
      authorization: `Bearer ${tokens.access_token}`,
    },
    body: JSON.stringify(draft),
  });
  // Refusals answer typed BEFORE any stream begins — the client branches on
  // the code and renders a not-available state, never a dead spinner. The
  // refusal envelope nests the machine reason under `error`
  // (`{error: {code, reason, message, surface}}`).
  if (response.status >= 400) {
    const body = (await response.json().catch(() => ({}))) as RefusalShape;
    const shape = body.error ?? body;
    const reason = typeof shape.reason === "string" ? shape.reason : undefined;
    const message =
      typeof shape.message === "string" ? shape.message : `AI refused (${response.status})`;
    return {
      text: "",
      frames: [],
      modelInfo: null,
      usage: null,
      refusal: response.status === 429 ? "rate_limited" : (reason ?? "ai_refused"),
      refusalMessage: reason === "ai_features_disabled" ? "AI is switched off for this org." : message,
    };
  }
  if (response.body === null) {
    return { text: "", frames: [], modelInfo: null, usage: null, refusal: "transport", refusalMessage: "no stream" };
  }

  const frames: SummaryFrame[] = [];
  const result: SummaryResult = { text: "", frames, modelInfo: null, usage: null };
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let currentEvent = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
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
      const frame: SummaryFrame = { event: currentEvent as SummaryFrameName, data: parsed };
      frames.push(frame);
      // `delta` is the house fragment key (chat runner's assistant_delta);
      // `text` is the whole-sentence key the completed frame carries.
      if (frame.event === "summary_delta" && typeof parsed["delta"] === "string") {
        result.text += parsed["delta"] as string;
        onDelta(parsed["delta"] as string);
      }
      if (frame.event === "summary_completed") {
        if (typeof parsed["text"] === "string") result.text = parsed["text"] as string;
        const usage = asRecord(parsed["usage"]);
        result.usage = {
          prompt_tokens: typeof usage["prompt_tokens"] === "number" ? (usage["prompt_tokens"] as number) : 0,
          completion_tokens:
            typeof usage["completion_tokens"] === "number" ? (usage["completion_tokens"] as number) : 0,
        };
      }
      if (frame.event === "summary_started") {
        const modelInfo = asRecord(parsed);
        result.modelInfo = {
          provider: typeof modelInfo["provider"] === "string" ? (modelInfo["provider"] as string) : "",
          model: typeof modelInfo["model"] === "string" ? (modelInfo["model"] as string) : "",
          resolution: typeof modelInfo["resolution"] === "string" ? (modelInfo["resolution"] as string) : "",
        };
      }
      if (frame.event === "summary_failed" && typeof parsed["reason"] === "string") {
        result.refusal = "stream_failed";
        result.refusalMessage = parsed["reason"] as string;
      }
      currentEvent = "";
    }
  }
  return result;
}

// ---------------------------------------------------------------------------
// C2 — the prose leg, on lane 17's endpoint
// ---------------------------------------------------------------------------

export interface AuthoringIssue {
  message: string;
  [key: string]: unknown;
}

export interface DiffEntry {
  [key: string]: unknown;
}

export interface AuthorDraft {
  /** `valid` is true only when the draft validated against the catalogue. */
  valid: boolean;
  draft: Record<string, unknown> | null;
  name: string | null;
  trigger_event: string | null;
  issues: AuthoringIssue[];
  diff: DiffEntry[];
  usage: { prompt_tokens: number; completion_tokens: number; steps: number };
  model_info: { provider: string; model: string; resolution: string };
  catalogue: { version: string; digest: string; node_count: number };
  error?: string;
}

export type AuthorOutcome =
  | { ok: true; result: AuthorDraft; latencyMs: number }
  | { ok: false; refusal: string; message: string };
/** The refusal envelope the API returns for a typed AI refusal. */
interface RefusalShape {
  error?: { message?: unknown; reason?: unknown };
  message?: unknown;
  reason?: unknown;
}

/** Ask lane 17 for a whole-workflow draft from one sentence. Never saves. */
export async function authorWorkflowDraft(prompt: string): Promise<AuthorOutcome> {
  const started = performance.now();
  const { data, error, response } = await api.api.POST("/api/v1/workflows/author", {
    body: {
      surface_key: "general_chat",
      prompt,
      document: null,
    },
  } as never);
  const latencyMs = Math.round(performance.now() - started);
  if (error !== undefined || data === undefined) {
    const status = (response as unknown as { status?: number } | undefined)?.status ?? 0;
    // openapi-fetch hands back the whole refusal envelope, which nests the
    // machine reason under `error` — read it there, fall back to a flat one.
    const envelope = (error ?? {}) as RefusalShape;
    const shape = envelope.error ?? envelope;
    const reason = typeof shape.reason === "string" ? shape.reason : undefined;
    const message =
      typeof shape.message === "string"
        ? shape.message
        : reason === "ai_features_disabled"
          ? "AI is switched off for this org."
          : "The draft did not come back.";
    return { ok: false, refusal: status === 403 ? (reason ?? "ai_refused") : "request_failed", message };
  }
  return { ok: true, result: data as unknown as AuthorDraft, latencyMs };
}

function asRecord(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function safeJson(raw: string): Record<string, unknown> | null {
  try {
    return JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return null;
  }
}
