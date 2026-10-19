// Timer read-model for the docked bar and Today strip.
//
// Purpose: one hook owning the caller's working set (running first),
// live-reconciled off the org-wide SSE stream — every `timer.*` event
// triggers a REST refetch so the server stays the single source of truth
// (§4.4: render optimistically, reconcile via snapshot + events). The
// stream owns reconnection. A 1s local ticker derives display elapsed from
// the SERVER-recorded segments; drift self-corrects on every event or
// refetch.
//
// Non-purpose: no mutations — components POST start/pause/resume/stop via
// the SDK and call `refresh()`; no notification concerns (use-notifications
// owns its own subscription).
"use client";

import { api, useAuth } from "./api";
import { instantRuntime } from "./instant/runtime";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useState } from "react";

export type TimerSession = components["schemas"]["TimerSession"];

function segmentElapsed(
  s: { started_at: string; ended_at?: string | null },
  nowMs: number,
): number {
  const start = new Date(s.started_at).getTime();
  const end = s.ended_at != null ? new Date(s.ended_at).getTime() : nowMs;
  return Math.max(0, Math.floor((end - start) / 1000));
}

/// Elapsed seconds for a session at time `nowMs` (server segments only).
export function sessionElapsedSecs(session: TimerSession, nowMs: number): number {
  return (session.segments ?? []).reduce(
    (total, seg) => total + segmentElapsed(seg, nowMs),
    0,
  );
}

export function formatElapsed(totalSecs: number): string {
  const h = Math.floor(totalSecs / 3600);
  const m = Math.floor((totalSecs % 3600) / 60);
  const s = totalSecs % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(h)}:${pad(m)}:${pad(s)}`;
}

/// The latest recorded pause/switch reason on a session, if any.
export function lastSegmentReason(session: TimerSession): string | null {
  const ended = (session.segments ?? []).filter((seg) => seg.ended_at != null);
  const last = ended[ended.length - 1];
  return last?.reason ?? last?.end_kind ?? null;
}

export function useTimer() {
  const auth = useAuth();
  const [sessions, setSessions] = useState<TimerSession[] | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());

  const refresh = useCallback(async () => {
    const ws = await api.api.GET("/api/v1/timers/working-set");
    if (ws.data) setSessions(ws.data);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  // Live leg: any timer.* event on the org-wide SSE stream reconciles the
  // set. The stream owns reconnection and gap replay.
  const runtime = instantRuntime();
  useEffect(() => {
    if (auth.status !== "authed" || !auth.me) return;
    const offEvents = runtime.bus.subscribe((event) => {
      if (event.type.startsWith("timer.")) void refresh();
    });
    const offScope = runtime.stream.subscribeOrg();
    return () => {
      offEvents();
      offScope();
    };
  }, [auth.status, auth.me, refresh, runtime]);

  // Display ticker: only while something is running.
  const sessionList = Array.isArray(sessions) ? sessions : [];
  const running = sessionList.find((s) => s.stopped_at === null) ?? null;
  useEffect(() => {
    if (running === null) return;
    const t = setInterval(() => setNowMs(Date.now()), 1000);
    return () => clearInterval(t);
  }, [running !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  return {
    sessions: sessionList,
    active: running,
    nowMs,
    refresh,
  };
}
