// Live timer activity — Android implementation over the local
// foreground-service module (an ongoing notification with a chronometer is
// Android's equivalent of a Live Activity).

import { requireOptionalNativeModule } from "expo-modules-core";

import { liveTimerPayload, type LiveTimerSnapshot } from "./timer-snapshot";

interface NativeLiveActivity {
  isSupported(): boolean;
  start(payload: ReturnType<typeof liveTimerPayload>): Promise<void>;
  update(payload: ReturnType<typeof liveTimerPayload>): Promise<void>;
  end(): Promise<void>;
  pushToken(): string | null;
}

const native = requireOptionalNativeModule<NativeLiveActivity>("MeridianLiveActivity");

export const liveTimerActivity = {
  async isSupported(): Promise<boolean> {
    if (native === null) return false;
    try {
      return native.isSupported();
    } catch {
      return false;
    }
  },
  async start(snapshot: LiveTimerSnapshot): Promise<void> {
    await native?.start(liveTimerPayload(snapshot));
  },
  async update(snapshot: LiveTimerSnapshot): Promise<void> {
    await native?.update(liveTimerPayload(snapshot));
  },
  async end(): Promise<void> {
    await native?.end();
  },
  async pushToken(): Promise<string | null> {
    return null;
  },
  subscribePushToken(): () => void {
    return () => undefined;
  },
};
