// Instant runtime — one assembly of store + event bus + SSE stream +
// offline queue + SDK transport for the whole app.
//
// Purpose: lazily construct the instant-layer singletons and wire the
// flows: SSE stream frames (contract events) → bus → store deltas;
// `type: "presence"` frames stay OUT of the reconcile path and reach the
// presence rosters through the stream's own listeners; queue replay on
// reconnect and on browser-online; replay results reconcile the store
// through the entity ledger. A reconnect that carried a Last-Event-ID
// cursor runs one idempotent rehydration sweep (the resume fallback past
// the replay buffer). `__resetInstantRuntimeForTests` gives suites a
// clean slate (the queue is the only durable piece — it reloads from
// storage).
//
// Non-purpose: no React (use-instant.ts binds hooks), no page wiring.

import { api, loadTokens, resolveApiBaseUrl } from "../api";
import { EventBus } from "./event-bus";
import { InstantStore, type MutationMetaLike } from "./instant-store";
import {
  OfflineQueue,
  type InvokeResult,
  type QueuedWrite,
} from "./offline-queue";
import { InstantStream } from "./stream";
import type { InstantTransport, InstantTransportResult } from "./instant-write";

export interface Runtime {
  store: InstantStore;
  bus: EventBus;
  stream: InstantStream;
  queue: OfflineQueue;
  transport: InstantTransport;
}

/** SSE URL for one stream scope (null = org-wide), auth via `?token=` —
 * EventSource cannot set headers; the server accepts the query token on
 * the stream path only (same posture as the /ws upgrade). */
function streamUrl(scope: string | null): string | null {
  const tokens = loadTokens();
  if (tokens === null) return null;
  const base = resolveApiBaseUrl();
  const u = new URL(base);
  u.pathname = "/api/events/stream";
  u.searchParams.set("token", tokens.access_token);
  if (scope !== null) u.searchParams.set("ticket", scope);
  return u.toString();
}

/** The IF3 typed error body, wherever openapi-fetch parked it. */
function reasonOf(error: unknown): { reason: string; message?: string; details?: unknown } {
  const outer = error as
    | { error?: { reason?: string; message?: string }; reason?: string; message?: string }
    | null;
  const inner = outer?.error ?? outer;
  return {
    reason: typeof inner?.reason === "string" ? inner.reason : "conflict",
    message: typeof inner?.message === "string" ? inner.message : undefined,
    // Preserve the whole typed body so conflict extras (e.g. the target
    // statuses behind `board_move_status_conflict`) reach the renderer.
    details: inner,
  };
}

/** The IF2 mutation envelope of a successful response, when present. */
export function mutationOf(data: unknown): MutationMetaLike | null {
  const meta = (data as { mutation?: MutationMetaLike } | null)?.mutation;
  if (meta === undefined || meta === null || typeof meta.seq !== "number") return null;
  return meta;
}

async function sdkRequest(
  method: "POST" | "PATCH" | "PUT" | "DELETE",
  path: string,
  body: unknown,
): Promise<InstantTransportResult> {
  type RawResult = { data?: unknown; error?: unknown; response?: Response };
  let raw: RawResult;
  try {
    // One bounded boundary: openapi-fetch's per-path overloads collapse
    // here; paths come from typed write specs built against MeridianPaths.
    // A bounded timeout keeps a hung connection from wedging the replay
    // loop: the op stays queued and the next reconnect retries it. (A
    // raced timer instead of an AbortSignal: the signal rides Request
    // construction, which some embedded fetch stubs reject.)
    const opts = { body } as unknown as Parameters<typeof api.api.POST>[1];
    const call = api.api[method] as (
      p: string,
      o: typeof opts,
    ) => Promise<RawResult>;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const timeout = new Promise<never>((_, reject) => {
      timer = setTimeout(
        () => reject(new DOMException("request timed out", "TimeoutError")),
        REQUEST_TIMEOUT_MS,
      );
    });
    raw = await Promise.race([
      call(path, opts),
      timeout,
    ]).finally(() => {
      if (timer !== null) clearTimeout(timer);
    });
  } catch {
    return { outcome: "network" };
  }
  if (raw.error !== undefined) {
    return { outcome: "rejected", ...reasonOf(raw.error) };
  }
  return { outcome: "accepted", mutation: mutationOf(raw.data) };
}

/** Replay requests abandon after 15s (network outcome → op stays queued). */
const REQUEST_TIMEOUT_MS = 15_000;

function buildRuntime(): Runtime {
  const bus = new EventBus();
  const store = new InstantStore();
  const stream = new InstantStream({
    url: streamUrl,
    bus,
    onResync: () => store.rehydrateAll(),
  });

  const transport: InstantTransport = {
    request: (method, path, body) => sdkRequest(method, path, body),
  };

  // Queue invoker: every replay attempt reconciles the store from the
  // server's verdict — accepted snapshots apply through the ledger,
  // rejections restore the captured pre-optimistic entity.
  const invoke = async (op: QueuedWrite): Promise<InvokeResult> => {
    const result = await transport.request(op.method, op.path, op.body);
    if (result.outcome === "network") return { outcome: "network" };
    if (result.outcome === "rejected") {
      if (op.entityId !== undefined) {
        store.revertEntity(op.entityId, op.before);
        store.ledger.endPending(op.entityId);
      }
      return { outcome: "rejected", reason: result.reason ?? "conflict", message: result.message };
    }
    if (op.entityId !== undefined) {
      store.ledger.endPending(op.entityId);
      if (result.mutation !== undefined && result.mutation !== null) {
        store.applyMutation(op.entityId, result.mutation);
      }
    }
    return { outcome: "accepted", mutation: result.mutation };
  };

  const queue = new OfflineQueue({
    storage: typeof window === "undefined" ? memoryStorage() : window.localStorage,
    invoke,
  });

  // Live deltas: SSE contract-event frames land on the bus; the store
  // subscribes. Presence frames never touch the bus (ephemeral — the
  // stream's presence listeners own them).
  bus.subscribe((event) => store.applyEvent(event));
  // Reconnect triggers: replay the durable queue when the wire returns
  // or the browser regains network. replay() is reentrancy-guarded.
  stream.onStatus((status) => {
    if (status === "open") void queue.replay();
  });
  if (typeof window !== "undefined") {
    window.addEventListener("online", () => {
      stream.ensureConnected();
      void queue.replay();
    });
  }

  return { store, bus, stream, queue, transport };
}

/** Test seam: a storage stand-in for non-DOM contexts. */
function memoryStorage(): Pick<Storage, "getItem" | "setItem" | "removeItem"> {
  const map = new Map<string, string>();
  return {
    getItem: (k) => map.get(k) ?? null,
    setItem: (k, v) => void map.set(k, v),
    removeItem: (k) => void map.delete(k),
  };
}

let runtime: Runtime | null = null;

/** The app-wide instant runtime (lazy singleton). */
export function instantRuntime(): Runtime {
  if (runtime === null) runtime = buildRuntime();
  return runtime;
}

/** Suite seam: discard the current runtime; a fresh one builds lazily. */
export function __resetInstantRuntimeForTests(): void {
  runtime?.stream.stop();
  runtime = null;
}
