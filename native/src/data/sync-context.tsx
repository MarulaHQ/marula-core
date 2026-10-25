// Sync provider and data hooks — the React binding of the sync engine.
//
// Purpose: create the engine for the signed-in session, bind durable storage
// to that session's user, wire connectivity transitions and token refresh into
// it, and on sign-out tear it down and await the device clear before another
// user's engine can hydrate. Also exposes the read/mutation/queue hooks
// screens use.
//
// Non-purpose: no UI. Screens never see the stream, stores, or the queue.

import * as Crypto from "expo-crypto";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";

import { useAuth } from "@/auth/auth-context";
import type { ModelState, MutationResult } from "@/core/sync";
import { SyncEngine } from "@/core/sync";
import type { QueuedOp } from "@/core/queue";
import { createSseSource } from "@/core/sse";
import type { WriteOperation } from "@/core/writes";
import { SSE_URL } from "@/config";
import { useConnectivity } from "@/data/connectivity";
import { asyncStore, clearUserStorage, setStorageOwner } from "@/data/storage";

export interface MutationHandle {
  mutate<A, B extends Record<string, unknown>>(
    operation: WriteOperation<A, B>,
    args: A,
  ): Promise<MutationResult>;
}

interface SyncContextValue {
  engine: SyncEngine | null;
  /** Re-render trigger for useSyncExternalStore consumers. */
  revision: number;
}

const SyncContext = createContext<SyncContextValue>({ engine: null, revision: 0 });

/** The sign-out teardown currently running. A new engine awaits it, so a
 * second user's cache can never hydrate over an unfinished clear. */
let pendingTeardown: Promise<void> | null = null;

export function SyncProvider({ children }: { children: ReactNode }) {
  const { client, status, ensureToken, accessToken, me } = useAuth();
  const { online } = useConnectivity();
  const [engine, setEngine] = useState<SyncEngine | null>(null);
  const engineRef = useRef<SyncEngine | null>(null);
  const onlineRef = useRef(online);
  // Read at engine construction, never a dependency: a profile edit must not
  // rebuild the engine (and drop the queue with it).
  const meRef = useRef(me);

  useEffect(() => {
    onlineRef.current = online;
  }, [online]);

  useEffect(() => {
    meRef.current = me;
  }, [me]);

  // The engine is an external system bound to the auth lifecycle; creating it
  // and publishing it to context is exactly an effect-owned side effect.
  /* eslint-disable react-hooks/set-state-in-effect -- engine lifecycle is effect-owned */
  useEffect(() => {
    if (status !== "signedIn") {
      const existing = engineRef.current;
      engineRef.current = null;
      setEngine(null);
      if (existing !== null) existing.stop();
      // Only a definite end clears the device. "loading" has not decided yet
      // and "biometric" is holding the previous session for an unlock, so
      // sweeping there would delete the cache the same user is about to
      // hydrate. A sign-out that never finished on the previous launch is
      // covered by the signed-out boot, which lands here.
      if (status !== "signedOut" && status !== "mfa") return;
      // Sign-out owns the whole device, not just the engine's three stores:
      // the engine's own state goes first (while the outgoing user is still
      // bound), then every namespace `asyncStore` hands out — cache, queue,
      // versions, the saved-scan shelf, the live token and preference — plus
      // the unprefixed keys older builds wrote. The next engine waits on it.
      // A storage failure must not wedge that wait: per-user keys already keep
      // the accounts apart, so a sweep that cannot run degrades to isolation
      // rather than stranding the next session without an engine.
      const teardown = (async () => {
        if (existing !== null) await existing.reset();
        await clearUserStorage();
      })().catch(() => undefined);
      pendingTeardown = teardown;
      return;
    }
    if (engineRef.current !== null) return;
    let cancelled = false;
    void (async () => {
      const teardown = pendingTeardown;
      if (teardown !== null) {
        await teardown;
        if (pendingTeardown === teardown) pendingTeardown = null;
      }
      if (cancelled || engineRef.current !== null) return;
      // Every store this engine opens resolves its keys from the owner, so
      // the binding lands before the first hydrate reads anything.
      setStorageOwner(meRef.current?.user_id ?? null);
      const next = new SyncEngine({
        client,
        stores: {
          cache: asyncStore("cache"),
          queue: asyncStore("queue"),
          versions: asyncStore("versions"),
        },
        realtime: {
          url: () => SSE_URL,
          token: () => accessToken(),
          factory: (url) => createSseSource(url),
        },
        isOnline: () => onlineRef.current,
        userId: () => meRef.current?.user_id ?? null,
        ensureToken,
        uuid: () => Crypto.randomUUID(),
      });
      engineRef.current = next;
      setEngine(next);
      void next.start();
    })();
    return () => {
      // The engine lifecycle is bound to `status`; the cleanup for a
      // signed-in engine runs on sign-out above, and on unmount here.
      cancelled = true;
      engineRef.current?.stop();
    };
  }, [status, client, ensureToken, accessToken]);
  /* eslint-enable react-hooks/set-state-in-effect */

  // Connectivity transitions drive queue flush + reconciliation.
  useEffect(() => {
    if (engine === null) return;
    void engine.handleConnectivityChange(online);
  }, [engine, online]);

  const revision = useSyncExternalStore(
    useCallback(
      (listener: () => void) => {
        if (engine === null) return () => undefined;
        return engine.subscribe(listener);
      },
      [engine],
    ),
    () => engine?.getRevision() ?? 0,
    () => 0,
  );

  const value = useMemo<SyncContextValue>(() => ({ engine, revision }), [engine, revision]);

  return <SyncContext.Provider value={value}>{children}</SyncContext.Provider>;
}

export function useEngine(): SyncEngine | null {
  return useContext(SyncContext).engine;
}
export interface ModelResult<T> extends ModelState<T> {
  refresh(): void;
}

/** Read a cached public read model; refreshes when missing, stale, or after
 * the max age, and re-renders on every cache/event/queue change. */
export function useModel<T>(
  key: string,
  fetcher: (client: import("@marula-core/sdk").MarulaClient) => Promise<T>,
  options?: { maxAgeMs?: number },
): ModelResult<T> {
  const { engine } = useContext(SyncContext);
  useSyncExternalStore(
    useCallback(
      (listener: () => void) => {
        if (engine === null) return () => undefined;
        return engine.subscribe(listener);
      },
      [engine],
    ),
    () => engine?.getRevision() ?? 0,
    () => 0,
  );

  const maxAgeMs = options?.maxAgeMs;
  useEffect(() => {
    if (engine === null) return;
    const detach = engine.registerModel(key, fetcher);
    void engine.refresh(key, fetcher, maxAgeMs !== undefined ? { maxAgeMs } : undefined);
    return detach;
  }, [engine, key, fetcher, maxAgeMs]);

  const state =
    engine?.getModelState<T>(key) ??
    ({ status: "idle", data: undefined, stale: false, offline: false } as ModelState<T>);

  const refresh = useCallback(() => {
    void engine?.refresh(key, fetcher, { force: true });
  }, [engine, key, fetcher]);

  return { ...state, refresh };
}

/** Queue snapshot for the offline banner, the synced indicator, and the
 * settings queue inspector. `revision` forces the snapshot to recompute on
 * every engine change. */
export function useQueue(): {
  pending: QueuedOp[];
  rejected: QueuedOp[];
  pendingCount: number;
  revision: number;
} {
  const { engine, revision } = useContext(SyncContext);
  return useMemo(
    () => ({
      pending: engine?.queue.pending() ?? [],
      rejected: engine?.queue.rejected() ?? [],
      pendingCount: engine?.queue.pendingCount() ?? 0,
      revision,
    }),
    [engine, revision],
  );
}

export function useMutation(): MutationHandle {
  const { engine } = useContext(SyncContext);
  return useMemo<MutationHandle>(
    () => ({
      async mutate(operation, args) {
        if (engine === null) {
          return {
            ok: false,
            queued: false,
            opId: "",
            failure: { text: "Sign in again to make changes." },
          };
        }
        return engine.mutate(operation, args);
      },
    }),
    [engine],
  );
}
