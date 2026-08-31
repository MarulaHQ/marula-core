// Instant runtime — one assembly of store + event bus + socket + offline
// queue + SDK transport for the whole app.
//
// Purpose: lazily construct the instant-layer singletons and wire the
// flows: socket frames → bus → store deltas; queue replay on reconnect
// and on browser-online; replay results reconcile the store through the
// entity ledger. `__resetInstantRuntimeForTests` gives suites a clean
// slate (the queue is the only durable piece — it reloads from storage).
//
// Non-purpose: no React (use-instant.ts binds hooks), no page wiring.

import { api, loadTokens } from "../api";
import { EventBus } from "./event-bus";
import { InstantStore, type MutationMetaLike } from "./instant-store";
import {
  OfflineQueue,
  type InvokeResult,
  type QueuedWrite,
} from "./offline-queue";
import { InstantSocket } from "./socket";
import type { InstantTransport, InstantTransportResult } from "./instant-write";

export interface Runtime {
  store: InstantStore;
  bus: EventBus;
  socket: InstantSocket;
  queue: OfflineQueue;
  transport: InstantTransport;
}

function wsUrl(): string | null {
  const tokens = loadTokens();
  if (tokens === null) return null;
  if (process.env.NEXT_PUBLIC_WS_URL) {
    return `${process.env.NEXT_PUBLIC_WS_URL}?token=${tokens.access_token}`;
  }
  const base = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";
  const u = new URL(base);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  u.port = process.env.NEXT_PUBLIC_WS_PORT ?? "8081";
  u.pathname = "/ws";
  return `${u.toString()}?token=${tokens.access_token}`;
}

/** The IF3 typed error body, wherever openapi-fetch parked it. */
function reasonOf(error: unknown): { reason: string; message?: string } {
  const outer = error as
    | { error?: { reason?: string; message?: string }; reason?: string; message?: string }
    | null;
  const inner = outer?.error ?? outer;
  return {
    reason: typeof inner?.reason === "string" ? inner.reason : "conflict",
    message: typeof inner?.message === "string" ? inner.message : undefined,
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
    const opts = { body } as unknown as Parameters<typeof api.api.POST>[1];
    const call = api.api[method] as (
      p: string,
      o: typeof opts,
    ) => Promise<RawResult>;
    raw = await call(path, opts);
  } catch {
    return { outcome: "network" };
  }
  if (raw.error !== undefined) {
    return { outcome: "rejected", ...reasonOf(raw.error) };
  }
  return { outcome: "accepted", mutation: mutationOf(raw.data) };
}

function buildRuntime(): Runtime {
  const bus = new EventBus();
  const store = new InstantStore();
  const socket = new InstantSocket({ url: wsUrl, bus });

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

  // Live deltas: socket frames land on the bus; the store subscribes.
  bus.subscribe((event) => store.applyEvent(event));
  // Reconnect triggers: replay the durable queue when the wire returns
  // or the browser regains network. replay() is reentrancy-guarded.
  socket.onStatus((status) => {
    if (status === "open") void queue.replay();
  });
  if (typeof window !== "undefined") {
    window.addEventListener("online", () => {
      socket.ensureConnected();
      void queue.replay();
    });
  }

  return { store, bus, socket, queue, transport };
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
  runtime?.socket.stop();
  runtime = null;
}
