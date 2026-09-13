// Thin React bindings over the instant runtime (IF4/IF5/IF6 client face).
//
// Purpose: useSyncExternalStore-backed cache reads with hydrate-once
// semantics (known data renders synchronously — no loading flash, no
// refetch), refcounted socket channel subscriptions, the optimistic write
// entry point, and the outbox state for surfacing per-op results.
//
// Non-purpose: no read-model definitions (read-models.ts), no transport.

"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useSyncExternalStore } from "react";
import type { ReadModelSpec } from "./instant/instant-store";
import type { WriteSpec, WriteResult } from "./instant/instant-write";
import { executeInstantWrite } from "./instant/instant-write";
import type { WriteOutcome } from "./instant/offline-queue";
import { instantRuntime } from "./instant/runtime";

export interface InstantCollection<T> {
  /** Synchronous cache read; undefined only before first hydration. */
  data: T | undefined;
  /**
   * True ONLY when there is no cached data yet. Hydrate-once keeps this
   * false on every later mount — known data never flashes a spinner.
   */
  loading: boolean;
  /** Explicit re-snapshot (bypasses hydrate-once). */
  refresh(): Promise<void>;
  /** Synchronous optimistic/authoritative patch of this collection. */
  patch(updater: (data: T) => T): void;
}

/**
 * Binds one read-model collection to the component tree. `spec` may be
 * rebuilt per render; the store keeps the first spec per cache key (its
 * closures match the key's ids by construction).
 */
export function useInstantCollection<T>(
  key: string,
  spec: ReadModelSpec<T>,
): InstantCollection<T> {
  const runtime = instantRuntime();
  const specRef = useRef(spec);
  specRef.current = spec;

  const subscribe = useCallback(
    (fn: () => void) => runtime.store.subscribe(fn),
    [runtime],
  );
  const getSnapshot = useCallback(() => runtime.store.peek<T>(key), [runtime, key]);
  const data = useSyncExternalStore(subscribe, getSnapshot, () => undefined);

  const refresh = useCallback((): Promise<void> => {
    return runtime.store.refresh(key, specRef.current).then(() => undefined);
  }, [runtime, key]);

  const patch = useCallback(
    (updater: (data: T) => T) => {
      runtime.store.patch(key, updater);
    },
    [runtime, key],
  );

  // Hydrate-once: cache hits already have `data` (no flash, no refetch);
  // misses hydrate in an effect, sharing the in-flight fetch via the store.
  useEffect(() => {
    void runtime.store.ensure(key, specRef.current).catch(() => {});
  }, [runtime, key]);

  return { data, loading: data === undefined, refresh, patch };
}

/**
 * Refcounted subscription to socket channels for this view (e.g.
 * `board:<id>`, `user:<me>`, `org:<org>`). Empty arrays are ignored so
 * pages can await ids discovered by hydration.
 */
export function useInstantChannels(channels: string[]): void {
  const runtime = instantRuntime();
  const joined = channels.filter((c) => c !== "").sort().join("|");
  useEffect(() => {
    if (joined === "") return;
    const offs = joined.split("|").map((c) => runtime.socket.subscribe(c));
    return () => {
      for (const off of offs) off();
    };
  }, [runtime, joined]);
}

/** The optimistic write entry point: mutate → render → reconcile. */
export interface InstantWrite {
  (spec: WriteSpec): Promise<WriteResult>;
}

/** The IF2 metadata needed to stamp a created entity's version cursor. */
export interface MutationStamp {
  seq: number;
  updated_at: string;
  version: number;
}

/**
 * The optimistic write entry point plus the create-stamp seam: when a
 * write CREATES an entity, the optimistic render used a temp id but the
 * envelope carries the server id + seq — stamp it so the NEXT write on
 * that entity carries its If-Match version.
 */
export function useInstantWrite(): {
  mutate: InstantWrite;
  stampEntity(entityId: string, meta: MutationStamp): void;
} {
  const runtime = instantRuntime();
  const mutate = useCallback<InstantWrite>(
    (spec) =>
      executeInstantWrite(runtime.store, runtime.queue, runtime.transport, spec),
    [runtime],
  );
  const stampEntity = useCallback(
    (entityId: string, meta: MutationStamp) => {
      runtime.store.applyMutation(
        entityId,
        { ...meta, after: null, before: null },
        { skipApply: true },
      );
    },
    [runtime],
  );
  return { mutate, stampEntity };
}

export interface OutboxState {
  pending: number;
  outcomes: WriteOutcome[];
}

/** Durable outbox state for the surfacing UI (IF6 per-op results). */
export function useOutbox(): OutboxState {
  const runtime = instantRuntime();
  const [state, setState] = useState<OutboxState>(() => ({
    pending: runtime.queue.pendingCount,
    outcomes: runtime.queue.recentOutcomes(),
  }));
  useEffect(() => {
    const update = () =>
      setState({
        pending: runtime.queue.pendingCount,
        outcomes: runtime.queue.recentOutcomes(),
      });
    const offState = runtime.queue.onState(update);
    const offOutcome = runtime.queue.onOutcome(update);
    return () => {
      offState();
      offOutcome();
    };
  }, [runtime]);
  return state;
}

/** Connection health for the recovery banner (Wave 13 lane 2).
 *
 * Reports `{ status, online, pending }` — `status` from the instant
 * socket's live leg, `online` from the browser's network state, `pending`
 * the durable queue depth. Components decide what to show from this trio;
 * the socket only connects once something subscribes, so pages without
 * instant channels naturally report a quiet `closed` here. */
export function useConnectionState(): {
  status: "connecting" | "open" | "closed";
  online: boolean;
  pending: number;
} {
  const runtime = instantRuntime();
  const [status, setStatus] = useState<"connecting" | "open" | "closed">(
    () => runtime.socket.statusNow(),
  );
  const [online, setOnline] = useState(() =>
    typeof navigator === "undefined" ? true : navigator.onLine,
  );
  const { pending } = useOutbox();
  useEffect(() => {
    const offStatus = runtime.socket.onStatus(setStatus);
    const goOnline = () => setOnline(true);
    const goOffline = () => setOnline(false);
    window.addEventListener("online", goOnline);
    window.addEventListener("offline", goOffline);
    return () => {
      offStatus();
      window.removeEventListener("online", goOnline);
      window.removeEventListener("offline", goOffline);
    };
  }, [runtime]);
  return { status, online, pending };
}
