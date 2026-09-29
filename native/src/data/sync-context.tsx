// Sync provider and data hooks — the React binding of the sync engine.
//
// Purpose: create the engine for the signed-in session, wire connectivity
// transitions and token refresh into it, tear it down (and drop the offline
// stores) on sign-out, and expose the read/mutation/queue hooks screens use.
//
// Non-purpose: no UI. Screens never see sockets, stores, or the queue.

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
import type { SocketLike } from "@/core/realtime";
import type { WriteOperation } from "@/core/writes";
import { WS_URL } from "@/config";
import { useConnectivity } from "@/data/connectivity";
import { asyncStore } from "@/data/storage";

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

export function SyncProvider({ children }: { children: ReactNode }) {
  const { client, status, ensureToken, accessToken } = useAuth();
  const { online } = useConnectivity();
  const [engine, setEngine] = useState<SyncEngine | null>(null);
  const engineRef = useRef<SyncEngine | null>(null);
  const onlineRef = useRef(online);

  useEffect(() => {
    onlineRef.current = online;
  }, [online]);

  // The engine is an external system bound to the auth lifecycle; creating it
  // and publishing it to context is exactly an effect-owned side effect.
  /* eslint-disable react-hooks/set-state-in-effect -- engine lifecycle is effect-owned */
  useEffect(() => {
    if (status !== "signedIn") {
      const existing = engineRef.current;
      engineRef.current = null;
      setEngine(null);
      if (existing !== null) {
        existing.stop();
        void existing.reset();
      }
      return;
    }
    if (engineRef.current !== null) return;
    const next = new SyncEngine({
      client,
      stores: {
        cache: asyncStore("cache"),
        queue: asyncStore("queue"),
        versions: asyncStore("versions"),
      },
      realtime: {
        url: () => WS_URL,
        token: () => accessToken(),
        socketFactory: (url) => new WebSocket(url) as unknown as SocketLike,
      },
      isOnline: () => onlineRef.current,
      ensureToken,
      uuid: () => Crypto.randomUUID(),
    });
    engineRef.current = next;
    setEngine(next);
    void next.start();
    return () => {
      // The engine lifecycle is bound to `status`; the cleanup for a
      // signed-in engine runs on sign-out above, and on unmount here.
      next.stop();
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
  fetcher: (client: import("@marula-core/sdk").MeridianClient) => Promise<T>,
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
