// Deep-link parsing for the `marula://` scheme.
//
// Purpose: turn inbound links into typed destinations. The printed-QR
// vocabulary (marula://stock-item/<uuid>, …) is frozen by the Wave 32
// lane 5 scan contract; `marula://scan-result?text=…` is the dev
// injector's deterministic harness; push taps land on `marula://ticket/<id>`
// (or the notification center when the event carries no ticket).
//
// Non-purpose: no navigation (expo-router renders the routes) and no
// resolver calls — a QR link is resolved through POST /scan/resolve by the
// screen that receives it, exactly like a camera scan.

export const MARULA_SCHEME = "marula";

export type MarulaLink =
  | { kind: "scan-result"; text: string }
  | {
      kind: "entity";
      entity: "stock-item" | "stock-location" | "stock-serial" | "device-asset";
      id: string;
    }
  | { kind: "ticket"; id: string }
  | { kind: "notifications" }
  | { kind: "unknown"; url: string };

const UUID = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;

export function parseMarulaLink(rawUrl: string): MarulaLink | null {
  const url = rawUrl.trim();
  if (url === "") return null;
  const withoutScheme = stripScheme(url);
  if (withoutScheme === null) return null;
  const [pathPart, queryPart = ""] = splitOnce(withoutScheme, "?");
  const segments = pathPart.split("/").filter((segment) => segment.length > 0);
  const query = new URLSearchParams(queryPart);

  if (segments[0] === "scan-result") {
    const text = query.get("text") ?? "";
    return { kind: "scan-result", text: text.trim() };
  }
  if (segments[0] === "ticket" && segments[1] !== undefined && UUID.test(segments[1])) {
    return { kind: "ticket", id: segments[1] };
  }
  if (segments[0] === "notifications") return { kind: "notifications" };
  if (
    (segments[0] === "stock-item" ||
      segments[0] === "stock-location" ||
      segments[0] === "stock-serial" ||
      segments[0] === "device-asset") &&
    segments[1] !== undefined &&
    UUID.test(segments[1])
  ) {
    return { kind: "entity", entity: segments[0], id: segments[1] };
  }
  return { kind: "unknown", url };
}

function stripScheme(url: string): string | null {
  const schemePrefix = `${MARULA_SCHEME}://`;
  if (url.startsWith(schemePrefix)) {
    // Tolerate the conventional empty-authority form marula:///path.
    return url.slice(schemePrefix.length).replace(/^\/+/, "");
  }
  // Expo web renders deep links as root-relative paths.
  if (url.startsWith("/")) return url.replace(/^\/+/, "");
  return null;
}

function splitOnce(value: string, separator: string): [string, string?] {
  const index = value.indexOf(separator);
  if (index === -1) return [value];
  return [value.slice(0, index), value.slice(index + separator.length)];
}

/** Where a push notification tap lands. */
export function notificationRoute(notification: {
  ticket_id?: string | null;
}): string {
  return notification.ticket_id != null && notification.ticket_id !== ""
    ? `/ticket/${notification.ticket_id}`
    : "/notifications";
}

/** A data payload carried by a push message (adapted in src/push). */
export interface PushPayload {
  ticket_id?: string;
  notification_id?: string;
}

export function routeForPushPayload(payload: PushPayload | undefined): string {
  if (payload?.ticket_id !== undefined && UUID.test(payload.ticket_id)) {
    return `/ticket/${payload.ticket_id}`;
  }
  return "/notifications";
}
