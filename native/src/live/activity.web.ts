// Live timer activity — web is a preview surface, not a device.

import { type LiveTimerSnapshot } from "./timer-snapshot";

export const liveTimerActivity = {
  async isSupported(): Promise<boolean> {
    return false;
  },
  async start(_snapshot: LiveTimerSnapshot): Promise<void> {
    return undefined;
  },
  async update(_snapshot: LiveTimerSnapshot): Promise<void> {
    return undefined;
  },
  async end(): Promise<void> {
    return undefined;
  },
  async pushToken(): Promise<string | null> {
    return null;
  },
  subscribePushToken(): () => void {
    return () => undefined;
  },
};
