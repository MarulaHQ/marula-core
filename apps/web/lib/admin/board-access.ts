// Shared client-side gating predicates for admin configuration screens.
//
// Purpose: mirror the server's authorization verdicts closely enough to
// render read-only/disabled states BEFORE the API rejects — never a
// security boundary, only UX honesty (the backend stays authoritative).
// Non-purpose: no fetching; callers pass already-loaded state.
import type { components } from "@marula-core/sdk";

export type BoardNavEntry = components["schemas"]["BoardNavEntry"];

/** Org-level special permission that gates board configuration server-side. */
export const MANAGE_BOARDS_PERMISSION = "special:manage_boards";

/**
 * True when this user may edit the given board's workflow:
 * an explicit board-admin membership row always configures; without a
 * membership row, org-level `special:manage_boards` passes through
 * (defaults-first doctrine). Any narrower membership denies.
 */
export function canConfigureBoard(
  entry: Pick<BoardNavEntry, "my_role">,
  permissions: readonly string[],
): boolean {
  if (entry.my_role === "admin") return true;
  if (entry.my_role === null || entry.my_role === undefined) {
    return permissions.includes(MANAGE_BOARDS_PERMISSION);
  }
  return false;
}
