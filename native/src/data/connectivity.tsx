// Connectivity: real network reachability plus the dev-menu airplane
// toggle that proves the offline queue without touching OS settings.
//
// Purpose: one honest online/offline signal for the sync engine. Offline is
// a first-class state (the banner, the queue, and every mutation branch on
// it), and the dev override lets a deterministic harness force it.

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

  useEffect(() => {
    let cancelled = false;
    void Network.getNetworkStateAsync().then((state) => {
      if (!cancelled) setReachable(isReachable(state));
    });
    const subscription = Network.addNetworkStateListener((state) => {
      setReachable(isReachable(state));
    });
    return () => {
      cancelled = true;
      subscription.remove();
    };
  }, []);

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

function isReachable(state: Network.NetworkState): boolean {
  if (state.isConnected === false) return false;
  if (state.isInternetReachable === false) return false;
  return true;
}
