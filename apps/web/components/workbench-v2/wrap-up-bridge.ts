// Wrap-up request bridge between the shell work bar and the workbench.
//
// Purpose: TimerDock is shell-level chrome; the wrap-up sheet needs the
// ticket page's data and mutations. The workbench registers itself as the
// active target while mounted; the work bar asks before falling back to
// its direct stop behavior. One module-level slot — a page owns it while
// it is mounted, so there is never a stale target.

export interface WrapUpRequest {
  ticketId: string;
  /** Header "Close" opens the sheet with the resolve step preselected. */
  intent?: "close";
}

type WrapUpTarget = (request: WrapUpRequest) => void;

let target: WrapUpTarget | null = null;

export function registerWrapUpTarget(fn: WrapUpTarget | null): void {
  target = fn;
}

/** Returns false when no ticket page is mounted to handle the request. */
export function requestWrapUp(request: WrapUpRequest): boolean {
  if (target === null) return false;
  target(request);
  return true;
}
