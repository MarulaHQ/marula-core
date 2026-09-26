// Shared desktop-shell seam (Wave 34 lane 6).
//
// One detection point and one invoke wrapper for every web→Tauri app
// command (raw ZPL transport, update check/apply). The desktop shell
// injects `window.__TAURI__` via `withGlobalTauri` and gates custom
// commands through the app ACL (desktop/capabilities/desktop-web.json);
// on plain web the global is absent and callers get an honest
// "unsupported/unavailable" answer — never a fake success.

export type TauriGlobal = {
  core?: {
    invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  };
};

/** True only inside the Tauri desktop shell (web falls back honestly). */
export function isDesktopShell(): boolean {
  if (typeof window === "undefined") return false;
  const w = window as unknown as { __TAURI__?: TauriGlobal };
  return typeof w.__TAURI__?.core?.invoke === "function";
}

/**
 * Invokes a Tauri app command, or returns null when not in the desktop
 * shell (or the global is missing). Callers map null to their own typed
 * unsupported state; errors from the IPC itself propagate as thrown
 * values so each caller can shape the message.
 */
export function tauriInvoke(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<unknown> | null {
  if (typeof window === "undefined") return null;
  const w = window as unknown as { __TAURI__?: TauriGlobal };
  const core = w.__TAURI__?.core;
  if (core === undefined || typeof core.invoke !== "function") return null;
  return core.invoke(cmd, args);
}
