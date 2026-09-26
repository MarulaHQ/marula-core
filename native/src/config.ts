// Runtime configuration and the lane's port convention.
//
// Port convention (Wave 33, mirrors the private high-port pattern):
//   Metro / Expo dev server   8181   (NOT 8081 — that is the realtime hub)
//   Expo web fast-iteration   8182
//   core-api                  per-run from scripts/e2e-stack.sh (random high)
//   realtime WS               per-run from scripts/e2e-stack.sh (random high)
//
// Boot the stack with `scripts/e2e-stack.sh up`, read `e2e/.e2e-stack.env`,
// then point the app at it:
//   EXPO_PUBLIC_API_URL=http://localhost:<API_PORT> \
//   EXPO_PUBLIC_WS_URL=ws://localhost:<WS_PORT>/ws \
//   pnpm --filter @meridian/native dev
//
// iOS Simulator shares the host network, so localhost works. The Android
// emulator maps the host at 10.0.2.2 — the defaults below handle both.
// Physical devices need the host's LAN IP in both variables.

import { Platform } from "react-native";

const devHost = Platform.select({
  ios: "localhost",
  android: "10.0.2.2",
  default: "localhost",
});

/** Env URLs come from operators and stack scripts, where a trailing slash is
 * easy to leave on. A double slash 404s every request, so normalize here:
 * the app always concatenates paths, never re-parses the base. */
function withoutTrailingSlash(value: string): string {
  return value.endsWith("/") ? value.slice(0, -1) : value;
}

export const API_URL = withoutTrailingSlash(
  process.env.EXPO_PUBLIC_API_URL ?? `http://${devHost}:8080`,
);

export const WS_URL = withoutTrailingSlash(
  process.env.EXPO_PUBLIC_WS_URL ?? `ws://${devHost}:8081/ws`,
);

/** EAS project id — absent until an operator registers one (see README). */
export const EAS_PROJECT_ID = process.env.EXPO_PUBLIC_EAS_PROJECT_ID ?? "";

/** Dev-menu surfaces (airplane toggle, scan injector, queue shortcuts, the
 * API/WS readout). On in a dev client, off in release builds unless the
 * verification build opts in (CI Maestro runs a Release build with this set
 * to "1" so the deterministic offline path stays testable). */
export const DEV_TOOLS_ENABLED =
  __DEV__ || process.env.EXPO_PUBLIC_ENABLE_DEV_TOOLS === "1";

/** Deep-link scheme, frozen by the scan contract. */
export const APP_SCHEME = "meridian";

export const APP_VERSION = "0.1.0";
