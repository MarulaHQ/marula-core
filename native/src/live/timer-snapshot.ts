// Live timer snapshot — the ONE mapping from the server's timer entity to
// every glanceable surface (iOS Live Activity, Android foreground-service
// notification, and the in-app timer bar's own clock).
//
// Purpose: pure derivation. The server session (GET /timers/active) carries
// segments; this module folds them into the small typed state the platform
// widgets render, so the phone, the lock screen, and a future watch face
// cannot disagree. No platform code, no effects — unit tested.
//
// Non-purpose: no persistence, no transport, no timers of its own.

import type { TimerSession } from "@/core/models";

export interface LiveTimerSnapshot {
  sessionId: string;
  ticketId: string;
  ticketNumber: string;
  title: string;
  state: "running" | "paused";
  /** Current segment start (or the last segment's start when paused). */
  segmentStartedAtIso: string;
  /** Seconds banked before the current segment. */
  accumulatedSeconds: number;
  /** Authoritative elapsed seconds at `updatedAtIso`. */
  totalSeconds: number;
  updatedAtIso: string;
}

/** Fold the session's segments into running/paused + elapsed totals. */
export function liveTimerSnapshot(
  session: TimerSession,
  labels: { ticketNumber?: string | null; title?: string | null },
  now: number = Date.now(),
): LiveTimerSnapshot {
  const segments = session.segments ?? [];
  let accumulated = 0;
  let total = 0;
  let lastStart: string = session.started_at;
  let running = false;

  for (const segment of segments) {
    const start = Date.parse(segment.started_at);
    if (Number.isNaN(start)) continue;
    lastStart = segment.started_at;
    if (segment.ended_at != null) {
      const end = Date.parse(segment.ended_at);
      if (!Number.isNaN(end)) {
        const seconds = Math.max(0, (end - start) / 1000);
        accumulated += seconds;
        total += seconds;
      }
      running = false;
    } else {
      const seconds = Math.max(0, (now - start) / 1000);
      total += seconds;
      running = true;
    }
  }

  if (segments.length === 0) {
    // Defensive: a session always has a first segment, but an empty shape
    // should still render something honest and ticking.
    const start = Date.parse(session.started_at);
    const seconds = Number.isNaN(start) ? 0 : Math.max(0, (now - start) / 1000);
    total = seconds;
    running = session.stopped_at == null;
  }

  return {
    sessionId: session.id,
    ticketId: session.ticket_id,
    ticketNumber: labels.ticketNumber ?? "",
    title: labels.title ?? "",
    state: running ? "running" : "paused",
    segmentStartedAtIso: lastStart,
    accumulatedSeconds: running ? accumulated : total,
    totalSeconds: total,
    updatedAtIso: new Date(now).toISOString(),
  };
}

/** True when two snapshots would render identically (pause/resume and
 * session changes update; the ticking second does not). */
export function sameLiveTimerSnapshot(
  a: LiveTimerSnapshot | null,
  b: LiveTimerSnapshot | null,
): boolean {
  if (a === null || b === null) return a === b;
  return (
    a.sessionId === b.sessionId &&
    a.state === b.state &&
    a.segmentStartedAtIso === b.segmentStartedAtIso &&
    Math.round(a.accumulatedSeconds) === Math.round(b.accumulatedSeconds) &&
    a.ticketNumber === b.ticketNumber &&
    a.title === b.title
  );
}

/** The wire shape both platform bridges accept (ISO strings + epoch millis,
 * so Swift and Kotlin parse the same payloads). */
export interface LiveTimerPayload {
  sessionId: string;
  ticketId: string;
  ticketNumber: string;
  title: string;
  state: string;
  segmentStartedAt: string;
  segmentStartedAtMillis: number;
  accumulatedSeconds: number;
  totalSeconds: number;
  updatedAt: string;
}

export function liveTimerPayload(snapshot: LiveTimerSnapshot): LiveTimerPayload {
  return {
    sessionId: snapshot.sessionId,
    ticketId: snapshot.ticketId,
    ticketNumber: snapshot.ticketNumber,
    title: snapshot.title,
    state: snapshot.state,
    segmentStartedAt: snapshot.segmentStartedAtIso,
    segmentStartedAtMillis: Date.parse(snapshot.segmentStartedAtIso) || Date.now(),
    accumulatedSeconds: snapshot.accumulatedSeconds,
    totalSeconds: snapshot.totalSeconds,
    updatedAt: snapshot.updatedAtIso,
  };
}
