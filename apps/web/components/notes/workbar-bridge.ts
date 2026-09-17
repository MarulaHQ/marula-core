// Workbar-panel request bridge (Wave 17 lane 11).
//
// Purpose: TimerDock is shell-level chrome; the Notes & Tasks panel needs
// the ticket page's data when the gate's Option B wants the tasks strip
// pre-scrolled to a specific unassigned row. The workbar bridge mirrors
// wrap-up-bridge: one module-level slot, registered by any surface that
// hosts the panel (the workbar shell component itself, or ticket pages
// hosting the panel pre-scroll target).
//
// One slot only: the panel is per-session, session-keyed by
// current.ticket_id — a second registration replaces the first while its
// owner is mounted, never leaves a stale target.

export interface WorkbarOpenRequest {
  ticketId: string;
  /** Close-gate Option B: pre-scroll to this unassigned checklist row
   * with its action cluster revealed and the row highlighted. */
  focusItemId?: string;
  /** Gate Option B opens with the TASKS section emphasized. */
  intent?: "tasks";
}

type WorkbarTarget = (request: WorkbarOpenRequest) => void;

let target: WorkbarTarget | null = null;

export function registerWorkbarTarget(fn: WorkbarTarget | null): void {
  target = fn;
}

/** Returns false when no mounted surface can open the panel. */
export function requestWorkbarPanel(request: WorkbarOpenRequest): boolean {
  if (target === null) return false;
  target(request);
  return true;
}
