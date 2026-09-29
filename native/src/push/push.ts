// Push notification seam.
//
// Purpose: request permission, set the Android channel, acquire the device
// push token, and route notification taps into the app's deep links. The
// notification-center fan-out (`notification.created` over the realtime
// socket) already reaches every signed-in client; native push adds the
// background leg.
//
// Honest gap (recorded in the landing record): the server contract has no
// device-token registration endpoint and the worker's fan-out has no push
// transport today, so a token acquired here cannot be registered yet. This
// module surfaces that state instead of pretending — a contract follow-up
// lane owns the endpoint + APNs/FCM transport.

import * as Notifications from "expo-notifications";
import { router } from "expo-router";
import { useEffect } from "react";
import { Platform } from "react-native";

import { routeForPushPayload, type PushPayload } from "@/core/links";
import { lightTokens } from "@/ui/tokens";

export type PushStatus =
  | "unsupported"
  | "not_configured"
  | "denied"
  | "registered"
  | "error";

export interface PushState {
  status: PushStatus;
  token?: string;
  detail: string;
  /** True once the server can accept device tokens (future contract). */
  serverRegistration: boolean;
}

const NOT_CONFIGURED_DETAIL =
  "Native push needs an EAS project id and a server device-registration endpoint. The in-app notification center works today over the live socket.";

export async function registerForPush(projectId: string): Promise<PushState> {
  if (Platform.OS === "web") {
    return {
      status: "unsupported",
      detail: "Push registration is a device concern; the web build uses the in-app inbox.",
      serverRegistration: false,
    };
  }
  try {
    if (Platform.OS === "android") {
      await Notifications.setNotificationChannelAsync("default", {
        name: "Marula",
        importance: Notifications.AndroidImportance.DEFAULT,
        lightColor: lightTokens.primary,
      });
    }
    const existing = await Notifications.getPermissionsAsync();
    let granted = existing.granted;
    if (!granted && existing.canAskAgain) {
      const asked = await Notifications.requestPermissionsAsync();
      granted = asked.granted;
    }
    if (!granted) {
      return {
        status: "denied",
        detail: "Notifications are off for Marula — turn them on in system settings.",
        serverRegistration: false,
      };
    }
    if (projectId === "") {
      const deviceToken = await Notifications.getDevicePushTokenAsync().catch(() => null);
      return {
        status: "not_configured",
        ...(deviceToken !== null ? { token: deviceToken.data } : {}),
        detail: NOT_CONFIGURED_DETAIL,
        serverRegistration: false,
      };
    }
    const token = await Notifications.getExpoPushTokenAsync({ projectId });
    return {
      status: "registered",
      token: token.data,
      detail: "This device can receive pushes once the server registration endpoint ships.",
      serverRegistration: false,
    };
  } catch (error) {
    return {
      status: "error",
      detail: error instanceof Error ? error.message : "Push registration failed.",
      serverRegistration: false,
    };
  }
}

/** Routes notification taps (and cold-start responses) into the app. */
export function useNotificationObserver(): void {
  useEffect(() => {
    if (Platform.OS === "web") return;
    const handle = (response: Notifications.NotificationResponse) => {
      const data = response.notification.request.content.data as PushPayload;
      router.push(routeForPushPayload(data) as never);
    };
    const subscription = Notifications.addNotificationResponseReceivedListener(handle);
    void Notifications.getLastNotificationResponseAsync().then((response) => {
      if (response !== null) handle(response);
    });
    return () => subscription.remove();
  }, []);
}

/** Foreground presentation: the OS banner plus the in-app inbox. */
export function configureForegroundPresentation(): void {
  if (Platform.OS === "web") return;
  Notifications.setNotificationHandler({
    handleNotification: async () => ({
      shouldShowBanner: true,
      shouldShowList: true,
      shouldPlaySound: false,
      shouldSetBadge: false,
    }),
  });
}
