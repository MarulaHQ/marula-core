// Tour request bus — a tiny synchronous pub/sub for explicit tour
// starts (Wave 71).
//
// Purpose: let the help panel ask TourScheduler to start the CURRENT
// surface's tour without threading callbacks through the tree. Only
// explicit intent travels here: a request bypasses the auto-start
// switches (tours.disabled / tours.org_disabled) and the per-session
// cap. The scheduler enforces that on its side — the bus just delivers.
//
// Non-purpose: no buffering or replay of requests — a request reaches
// only the subscribers mounted when it is published.

export type TourRequestListener = (surface: string) => void;

const listeners = new Set<TourRequestListener>();

/** Register a listener for "take the tour" requests. Returns the
 * unsubscribe function. */
export function subscribeTourRequests(cb: TourRequestListener): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/** Ask the scheduler to start the tour for a tour surface. */
export function requestTourStart(surface: string): void {
  for (const cb of [...listeners]) cb(surface);
}
