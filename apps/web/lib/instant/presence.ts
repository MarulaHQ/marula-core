// Presence roster logic (Wave 13 lane 9) — the pure half of the ephemeral
// presence surface.
//
// Purpose: classify one roster snapshot into honest viewer states. The
// server TTLs presence at 90s (crates/core-api realtime.rs): an entry
// whose heartbeat stopped ages past that window and renders "idle" on
// the client even before the server's next prune drops it — never faked
// as active. Non-purpose: no transport (the stream + heartbeat hook own
// that), no React, no rendering.

import type { PresenceViewerJson } from "./events";

/** The server-side presence TTL (realtime::presence, default 90s). */
export const PRESENCE_TTL_MS = 90_000;

export type RosterState = "editing" | "viewing" | "idle";

export interface RosterEntry {
  user_id: string;
  name: string;
  editing: boolean;
  state: RosterState;
  ageSeconds: number;
}

/** Classifies one roster snapshot against `nowMs`. */
export function rosterView(
  viewers: PresenceViewerJson[],
  nowMs: number,
): RosterEntry[] {
  // Pre-guard: the roster read model can emit undefined before the
  // ephemeral snapshot resolves (same crash family as the notification
  // dock's pre-guard race) — render as nobody viewing, not a crash.
  if (!Array.isArray(viewers)) return [];
  return viewers.map((v) => {
    const atMs = Date.parse(v.at);
    const ageMs = Number.isFinite(atMs) ? Math.max(0, nowMs - atMs) : 0;
    const state: RosterState =
      ageMs >= PRESENCE_TTL_MS ? "idle" : v.editing ? "editing" : "viewing";
    return {
      user_id: v.user_id,
      name: v.name,
      editing: v.editing,
      state,
      ageSeconds: Math.floor(ageMs / 1000),
    };
  });
}

/** True while `editingUntilMs` is in the future — the heartbeat hook's
 * "actively editing" signal (recent typing in an editable element). */
export function isEditing(editingUntilMs: number, nowMs: number): boolean {
  return nowMs < editingUntilMs;
}
