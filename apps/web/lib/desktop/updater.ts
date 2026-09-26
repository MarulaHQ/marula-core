// Desktop updater seam (Wave 34 lane 6, `p34-open-gaps-collector`).
//
// Rides the Wave 20 auto-update channel: the Tauri shell owns the check
// and the apply (endpoint + minisign pubkey live in tauri.conf.json and
// the Rust commands; the web app can neither redirect the channel nor
// skip verification). This module is only the typed wire — it maps the
// shell's answers into states the UI renders, and on plain web it answers
// "unavailable" instead of pretending a check ran.
//
// Non-purpose: no version comparison, no channel configuration, no
// download handling — the shell's updater plugin does all three.

import { isDesktopShell, tauriInvoke } from "./shell";

/** Wire shape of the shell's `check_desktop_update` result. */
export interface DesktopUpdateInfo {
  /** Version the channel announces. */
  version: string;
  /** Version this build runs. */
  currentVersion: string;
  /** Publish date string from the channel manifest, when present. */
  date?: string | null;
  /** Release notes from the channel manifest, when present. */
  notes?: string | null;
}

export type UpdateCheckResult =
  /** Not the desktop shell — no update machinery exists here. */
  | { kind: "unavailable" }
  /** The running version is current. */
  | { kind: "current" }
  | { kind: "available"; update: DesktopUpdateInfo }
  /** The check itself failed (e.g. channel not reachable). */
  | { kind: "error"; message: string };

export type UpdateInstallResult =
  | { kind: "restarting" }
  | { kind: "unsupported" }
  | { kind: "error"; message: string };

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * True when `value` is shaped like the shell's update payload. The invoke
 * boundary is untyped at runtime, so the UI never trusts the shape.
 */
function isUpdateInfo(value: unknown): value is DesktopUpdateInfo {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return typeof v.version === "string" && typeof v.currentVersion === "string";
}

/**
 * Asks the shell to check the update channel. `current` and `error` are
 * distinct on purpose: "up to date" is a fact, "couldn't check" is not.
 */
export async function checkDesktopUpdate(): Promise<UpdateCheckResult> {
  if (!isDesktopShell()) return { kind: "unavailable" };
  const pending = tauriInvoke("check_desktop_update");
  if (pending === null) return { kind: "unavailable" };
  try {
    const result = await pending;
    if (result === null || result === undefined) return { kind: "current" };
    if (!isUpdateInfo(result)) {
      return { kind: "error", message: "The shell returned an unexpected update payload." };
    }
    return { kind: "available", update: result };
  } catch (error) {
    return { kind: "error", message: errorMessage(error) };
  }
}

/**
 * Asks the shell to download, verify, install, and relaunch. The shell
 * restarts the app on success, so `restarting` normally means this window
 * goes away — render it as the final state, never as a spinner that must
 * resolve.
 */
export async function installDesktopUpdate(): Promise<UpdateInstallResult> {
  if (!isDesktopShell()) return { kind: "unsupported" };
  const pending = tauriInvoke("install_desktop_update");
  if (pending === null) return { kind: "unsupported" };
  try {
    await pending;
    return { kind: "restarting" };
  } catch (error) {
    return { kind: "error", message: errorMessage(error) };
  }
}
