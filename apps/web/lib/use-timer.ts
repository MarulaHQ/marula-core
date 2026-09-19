// Timer read-model for the docked bar and Today strip.
//
// Purpose: one hook owning the caller's working set (running first),
// live-reconciled over the `user:<me>` socket channel — every `timer.*`
// event triggers a REST refetch so the server stays the single source of
// truth (§4.4: render optimistically, reconcile via snapshot + events).
// A 1s local ticker derives display elapsed from the SERVER-recorded
// segments; drift self-corrects on every event or refetch.
//
// Non-purpose: no mutations — components POST start/pause/resume/stop via
// the SDK and call `refresh()`; no notification concerns (use-notifications
// owns its own socket).
"use client";

import { api, loadTokens, resolveApiBaseUrl, useAuth } from "./api";
import type { components } from "@meridian/sdk";
import { useCallback, useEffect, useRef, useState } from "react";

export type TimerSession = components["schemas"]["TimerSession"];

function wsUrl(): string {
  if (process.env.NEXT_PUBLIC_WS_URL) return process.env.NEXT_PUBLIC_WS_URL;
  const base = resolveApiBaseUrl();
  const u = new URL(base);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  u.port = process.env.NEXT_PUBLIC_WS_PORT ?? "8081";
  u.pathname = "/ws";
  return u.toString();
}

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
  const socketRef = useRef<WebSocket | null>(null);

  const refresh = useCallback(async () => {
    const ws = await api.api.GET("/api/v1/timers/working-set");
    if (ws.data) setSessions(ws.data);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  // Live leg: any timer.* event on the user channel reconciles the set.
  useEffect(() => {
    if (auth.status !== "authed" || !auth.me) return;
    let closed = false;
    let attempt = 0;
    let retry: ReturnType<typeof setTimeout> | undefined;

    const connect = () => {
      const tokens = loadTokens();
      if (!tokens || closed) return;
      const sock = new WebSocket(`${wsUrl()}?token=${tokens.access_token}`);
      socketRef.current = sock;
      sock.onopen = () => {
        attempt = 0;
        sock.send(
          JSON.stringify({ type: "subscribe", channel: `user:${auth.me!.user_id}` }),
        );
      };
      sock.onmessage = (m) => {
        try {
          const frame = JSON.parse(String(m.data)) as {
            type: string;
            event?: { type?: string };
          };
          if (frame.type === "event" && frame.event?.type?.startsWith("timer.")) {
            void refresh();
          }
        } catch {
          // ignore malformed frames; REST remains source of truth
        }
      };
      sock.onclose = () => {
        socketRef.current = null;
        if (closed) return;
        attempt += 1;
        retry = setTimeout(connect, Math.min(1000 * 2 ** attempt, 15000));
      };
    };
    connect();
    return () => {
      closed = true;
      if (retry) clearTimeout(retry);
      socketRef.current?.close();
      socketRef.current = null;
    };
  }, [auth.status, auth.me, refresh]);

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
