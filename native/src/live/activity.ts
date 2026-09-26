// Live timer activity — the platform-neutral bridge (default: unsupported).
//
// Purpose: one typed API the app calls; the platform files pick the
// implementation. iOS maps it to an ActivityKit Live Activity, Android to a
// foreground-service notification, everything else (web, tests, a dev client
// without the module linked) to an honest no-op that reports unsupported.
//
// Non-purpose: no snapshot logic (timer-snapshot.ts) and no UI.

import { liveTimerPayload, type LiveTimerSnapshot } from "./timer-snapshot";

export interface LiveTimerActivity {
  isSupported(): Promise<boolean>;
  start(snapshot: LiveTimerSnapshot): Promise<void>;
  update(snapshot: LiveTimerSnapshot): Promise<void>;
  end(): Promise<void>;
  /** ActivityKit's per-activity push token (iOS); null elsewhere. */
  pushToken(): Promise<string | null>;
  subscribePushToken(
    listener: (token: string, sessionId: string) => void,
  ): () => void;
}

const unsupported: LiveTimerActivity = {
  isSupported: async () => false,
  start: async () => undefined,
  update: async () => undefined,
  end: async () => undefined,
  pushToken: async () => null,
  subscribePushToken: () => () => undefined,
};

export const liveTimerActivity: LiveTimerActivity = unsupported;

export { liveTimerPayload };
