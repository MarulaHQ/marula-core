// The push-token update leg — client half.
//
// ActivityKit can update a Live Activity without the app running, but only
// if the server holds the activity's push token. The client acquires the
// token (MarulaLiveActivity `onPushToken`) and remembers it per session
// here. The server side is blocked: there is no device-token registration
// route and the worker has no APNs transport (docs/rn-groundwork.md,
// finding 3), so the token cannot be handed over yet. This module keeps the
// token ready and states the gap in one place.

import { asyncStore } from "@/data/storage";

const store = asyncStore("live");

export const PUSH_TOKEN_SERVER_GAP =
  "The activity token is captured on this device. The server has no token-registration route or APNs transport yet, so background pushes cannot update the activity — it updates while the app runs.";

export async function rememberActivityPushToken(
  sessionId: string,
  token: string,
): Promise<void> {
  await store.set(`activity.push.${sessionId}`, token);
}

export async function activityPushToken(sessionId: string): Promise<string | null> {
  return store.get(`activity.push.${sessionId}`);
}
