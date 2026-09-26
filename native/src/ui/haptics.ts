// Haptics — the field-ops touch feedback, in one place.
//
// Purpose: a scan that lands, a stage tick, and a committed movement each
// fire the same haptic everywhere (mobile-native doctrine: one haptic per
// user action, same frame as the visual, never the only feedback).
//
// Non-purpose: no UI. Callers keep their visual feedback independent; a
// device with haptics off must lose nothing but the tap.

import * as Haptics from "expo-haptics";
import { Platform } from "react-native";

function canBuzz(): boolean {
  return Platform.OS === "ios" || Platform.OS === "android";
}

/** A scan resolved and was staged. */
export function scanStaged(): void {
  if (!canBuzz()) return;
  void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(() => undefined);
}

/** A scan was rejected (wrong kind, unknown, ambiguous candidate picked). */
export function scanRejected(): void {
  if (!canBuzz()) return;
  void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Error).catch(() => undefined);
}

/** A movement / checkout / return was committed (or queued). */
export function commitConfirmed(): void {
  if (!canBuzz()) return;
  void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success).catch(
    () => undefined,
  );
}

/** A checklist tick or a picker detent. */
export function tick(): void {
  if (!canBuzz()) return;
  void Haptics.selectionAsync().catch(() => undefined);
}
