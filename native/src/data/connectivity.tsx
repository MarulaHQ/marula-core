// Connectivity: real network reachability plus the dev-menu airplane
// toggle that proves the offline queue without touching OS settings.
//
// Purpose: one honest online/offline signal for the sync engine. Offline is
// a first-class state (the banner, the queue, and every mutation branch on
// it), and the dev override lets a deterministic harness force it.
//
// The interface flag alone is not the truth: a simulator (and a laptop on a
// VPN) can report connected-but-no-internet while the API answers fine. When
// the interface says "no internet", one short health probe decides — that is
// the same signal every queued write cares about.

import * as Network from "expo-network";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { API_URL } from "@/config";

interface ConnectivityValue {
  online: boolean;
  forcedOffline: boolean;
  setForcedOffline(value: boolean): void;
}

const ConnectivityContext = createContext<ConnectivityValue>({
  online: true,
  forcedOffline: false,
  setForcedOffline: () => undefined,
});

export function ConnectivityProvider({ children }: { children: ReactNode }) {
  const [reachable, setReachable] = useState(true);
  const [forcedOffline, setForcedOffline] = useState(false);

  const probe = useCallback(async (): Promise<boolean> => {
    try {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 4000);
      const response = await fetch(`${API_URL}/api/v1/health`, {
        signal: controller.signal,
      });
      clearTimeout(timer);
      return response.ok;
    } catch {
      return false;
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    const apply = async (state: Network.NetworkState) => {
      if (state.isConnected === false) {
        if (!cancelled) setReachable(false);
        return;
      }
      if (state.isInternetReachable === true) {
        if (!cancelled) setReachable(true);
        return;
      }
      const ok = await probe();
      if (!cancelled) setReachable(ok);
    };
    void Network.getNetworkStateAsync().then((state) => {
      if (!cancelled) void apply(state);
    });
    const subscription = Network.addNetworkStateListener((state) => {
      void apply(state);
    });
    return () => {
      cancelled = true;
      subscription.remove();
    };
  }, [probe]);

  const setForced = useCallback((value: boolean) => {
    setForcedOffline(value);
  }, []);

  const value = useMemo<ConnectivityValue>(
    () => ({
      online: reachable && !forcedOffline,
      forcedOffline,
      setForcedOffline: setForced,
    }),
    [reachable, forcedOffline, setForced],
  );

  return (
    <ConnectivityContext.Provider value={value}>{children}</ConnectivityContext.Provider>
  );
}

export function useConnectivity(): ConnectivityValue {
  return useContext(ConnectivityContext);
}
