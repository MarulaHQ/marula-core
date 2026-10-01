/**
 * Which work session the timer bar follows.
 *
 * The normal case, and today the only rule: the bar is visible while a ticket
 * is being worked. A work session exists from the moment a person starts a
 * timer on a ticket until they stop it, so an open session is the signal.
 * A running session wins; when the person has paused, the most recently
 * started open session is the one shown, so a paused timer is never stranded.
 * With no open session the bar does not render.
 *
 * Other surfaces will be able to surface the bar for their own actions later
 * (the workflow builder testing a run is the named case). That is deliberately
 * not built yet; this module holds the one rule the app uses now.
 *
 * Non-purpose: no timer maths and no UI. The server owns segments and the dock
 * owns rendering.
 */

interface SessionLike {
  stopped_at?: string | null;
  segments?: Array<{ started_at?: string | null }> | null;
}

function latestSegmentStart(session: SessionLike): string {
  const last = (session.segments ?? []).slice(-1)[0];
  return last?.started_at ?? "";
}

/** The session the timer bar should follow, or null to hide it. */
export function selectWorkSession<T extends SessionLike>(
  active: T | null,
  sessions: T[] | null | undefined,
): T | null {
  if (active) return active;
  const open = (sessions ?? []).filter((s) => s.stopped_at === null);
  open.sort((a, b) => latestSegmentStart(b).localeCompare(latestSegmentStart(a)));
  return open[0] ?? null;
}
