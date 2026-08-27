// Docked ACTIVE TIMER bar (TASKLIST 2.9).
//
// Purpose: the bottom-docked working-set bar per design/mockups/today-*
// — current session (running first), elapsed in mono digits, the preserved
// pause-reason chip, Pause/Resume and Switch actions, and a thin Ember top
// edge only while the timer is live. Server-authoritative: state comes
// from useTimer (REST snapshot reconciled by timer.* WS events).
//
// Non-purpose: no timer math (server owns segments) and no ticket queue —
// Switch deep-links to /today where per-ticket start lives.
"use client";

import { Button, StatusTag } from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { api, useAuth } from "../lib/api";
import {
  formatElapsed,
  lastSegmentReason,
  sessionElapsedSecs,
  useTimer,
} from "../lib/use-timer";

type Ticket = components["schemas"]["Ticket"];

function PauseIcon() {
  return (
    <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12" fill="none">
      <path d="M4 2.5v7M8 2.5v7" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}

function PlayIcon() {
  return (
    <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12" fill="none">
      <path d="M3.5 2.2v7.6L9.5 6 3.5 2.2Z" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
    </svg>
  );
}

function SwitchIcon() {
  return (
    <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12" fill="none">
      <path d="M2 4h6.5L7 2.5M10 8H3.5L5 9.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function TimerDock() {
  const auth = useAuth();
  const router = useRouter();
  const { active, sessions, nowMs, refresh } = useTimer();
  // The dock shows the current focus: the running session, or — when
  // everything is paused — the most recently touched open session.
  const current =
    active ??
    [...(sessions ?? [])]
      .filter((s) => s.stopped_at === null)
      .sort((a, b) => {
        const la = (a.segments ?? []).slice(-1)[0]?.started_at ?? "";
        const lb = (b.segments ?? []).slice(-1)[0]?.started_at ?? "";
        return lb.localeCompare(la);
      })[0] ??
    null;
  const [ticket, setTicket] = useState<Ticket | null>(null);

  useEffect(() => {
    if (auth.status !== "authed" || current === null) {
      setTicket(null);
      return;
    }
    let cancelled = false;
    void api.api
      .GET("/api/v1/tickets/{id}", { params: { path: { id: current.ticket_id } } })
      .then(({ data }) => {
        if (!cancelled) setTicket(data ?? null);
      });
    return () => {
      cancelled = true;
    };
  }, [auth.status, current?.ticket_id]); // eslint-disable-line react-hooks/exhaustive-deps

  if (auth.status !== "authed" || current === null) return null;

  const live = current.stopped_at === null && current.segments?.slice(-1)[0]?.ended_at === null;
  const elapsed = sessionElapsedSecs(current, nowMs);
  const reason = lastSegmentReason(current);

  async function pauseOrResume() {
    if (live) {
      await api.api.POST("/api/v1/timers/pause", { body: { session_id: current!.id } });
    } else {
      await api.api.POST("/api/v1/timers/resume", { body: { session_id: current!.id } });
    }
    void refresh();
  }

  return (
    <div
      role="region"
      aria-label="Active timer"
      className="fixed inset-x-0 bottom-0 z-40 border-t"
      style={{
        borderTop: live
          ? "2px solid var(--m-accent)"
          : "1px solid var(--m-border)",
        background: "var(--m-surface)",
        boxShadow: "var(--m-shadow-strong, 0 -4px 16px rgba(0,0,0,0.18))",
      }}
    >
      <div className="mx-auto flex max-w-[1400px] flex-wrap items-center gap-x-6 gap-y-2 px-4 py-3">
        <span
          className="text-[10px] font-medium uppercase tracking-[0.14em]"
          style={{ color: "var(--m-text-muted)" }}
        >
          Active timer
        </span>

        <button
          type="button"
          onClick={() => router.push(`/tickets/${current.ticket_id}`)}
          className="flex min-w-0 items-baseline gap-3 text-left"
        >
          <span
            className="font-mono text-lg font-semibold tracking-tight"
            style={{ color: "var(--m-text)" }}
          >
            {ticket?.number ?? current.ticket_id.slice(0, 8)}
          </span>
          <span
            className="truncate text-[13px]"
            style={{ color: "var(--m-text-secondary)" }}
          >
            {ticket?.title ?? "…"}
          </span>
        </button>

        <div className="flex items-baseline gap-2">
          <span
            className="text-[10px] font-medium uppercase tracking-[0.14em]"
            style={{ color: "var(--m-text-muted)" }}
          >
            Elapsed
          </span>
          <span
            data-testid="timer-elapsed"
            className="font-mono text-3xl font-semibold tabular-nums leading-none"
            style={{ color: "var(--m-text)" }}
          >
            {formatElapsed(elapsed)}
          </span>
        </div>

        <div className="flex items-center gap-2">
          <span
            className="text-[10px] font-medium uppercase tracking-[0.14em]"
            style={{ color: "var(--m-text-muted)" }}
          >
            Status
          </span>
          {reason !== null ? (
            <StatusTag tone={live ? "neutral" : "warning"}>{reason}</StatusTag>
          ) : (
            <StatusTag tone="success">tracking</StatusTag>
          )}
        </div>

        <div className="ml-auto flex items-center gap-2">
          <Button size="sm" variant="secondary" onClick={() => void pauseOrResume()}>
            {live ? <PauseIcon /> : <PlayIcon />}
            {live ? "Pause" : "Resume"}
          </Button>
          <Button size="sm" variant="secondary" onClick={() => router.push("/today")}>
            <SwitchIcon />
            Switch
          </Button>
        </div>
      </div>
    </div>
  );
}
