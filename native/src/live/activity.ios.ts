// Live timer activity — iOS implementation over the local ActivityKit module.
//
// The module is compiled into the app by CNG (native/modules/
// meridian-live-activity) together with the widget extension target
// (native/targets/live-timer). When the module is not linked (Expo Go, web,
// tests), `requireOptionalNativeModule` returns null and every call reports
// unsupported instead of throwing.

import { requireOptionalNativeModule } from "expo-modules-core";

import { liveTimerPayload, type LiveTimerSnapshot } from "./timer-snapshot";

interface NativeLiveActivity {
  isSupported(): boolean;
  start(payload: ReturnType<typeof liveTimerPayload>): Promise<void>;
  update(payload: ReturnType<typeof liveTimerPayload>): Promise<void>;
  end(): Promise<void>;
  pushToken(): string | null;
  addListener(
    event: "onPushToken",
    listener: (event: { sessionId: string; token: string }) => void,
  ): { remove(): void };
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
    return native?.pushToken() ?? null;
  },
  subscribePushToken(listener: (token: string, sessionId: string) => void): () => void {
    if (native === null) return () => undefined;
    const subscription = native.addListener("onPushToken", (event) => {
      listener(event.token, event.sessionId);
    });
    return () => subscription.remove();
  },
};
