// Live timer preference — one device-scoped switch for the lock-screen /
// ongoing-notification timer.
//
// Purpose: whether the phone mirrors the running timer outside the app is a
// device choice, not org configuration. Persisted in the same prefs store
// the theme uses, and shared through a tiny external store so the hook and
// the More screen never disagree.

import { useEffect, useSyncExternalStore } from "react";

import { asyncStore } from "@/data/storage";

const prefs = asyncStore("prefs");
const KEY = "live.activity";

let enabled = true;
let hydrated = false;
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function hydrate(): void {
  if (hydrated) return;
  hydrated = true;
  void prefs.get(KEY).then((stored) => {
    if (stored === "0") {
      enabled = false;
      emit();
    }
  });
}

export function setLiveTimerEnabled(next: boolean): void {
  hydrate();
  enabled = next;
  void prefs.set(KEY, next ? "1" : "0");
  emit();
}

export function liveTimerEnabled(): boolean {
  return enabled;
}

export function useLiveTimerEnabled(): [boolean, (next: boolean) => void] {
  const value = useSyncExternalStore(subscribe, liveTimerEnabled, () => true);
  useEffect(() => {
    hydrate();
  }, []);
  return [value, setLiveTimerEnabled];
}
