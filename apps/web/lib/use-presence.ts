// usePresenceRoster — the ticket detail's ephemeral presence face
// (Wave 13 lane 9).
//
// Purpose: keep THIS viewer's heartbeat on the open ticket (30s cadence
// via POST /api/v1/presence/heartbeat, first beat immediately on mount)
// and render the honest roster: who is viewing, who is editing, and who
// has gone idle (past the server's 90s TTL). Heartbeats run ONLY while
// the ticket is open and the tab is visible — unmount and tab-hidden
// both stop them, so presence reflects real attention. Roster updates
// arrive two ways: the scoped stream's `type: "presence"` frames (live)
// and each heartbeat's response (self-sync). Presence is EPHEMERAL — it
// never touches the event trail or the reconcile path.
//
// Non-purpose: no rendering (components/instant/presence-roster.tsx), no
// contract-event handling.

"use client";

import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import type { PresenceViewerJson } from "./instant/events";
import {
  rosterView,
  type RosterEntry,
} from "./instant/presence";
import { instantRuntime } from "./instant/runtime";

/** How long after the last input event the heartbeat reports editing. */
const EDITING_WINDOW_MS = 15_000;
const HEARTBEAT_MS = 30_000;
const AGE_TICK_MS = 15_000;

export function usePresenceRoster(ticketId: string): {
  viewers: RosterEntry[];
} {
  const runtime = instantRuntime();
  const [viewersRaw, setViewersRaw] = useState<PresenceViewerJson[]>([]);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const editingUntil = useRef(0);

  // Scoped stream: the ticket's presence frames (and its events —
  // overlapping org delivery dedupes by frame identity in the store).
  useEffect(() => {
    if (ticketId === "") return;
    const offTicket = runtime.stream.subscribeTicket(ticketId);
    const offPresence = runtime.stream.onPresence((update) => {
      if (update.entity !== "ticket" || update.entity_id !== ticketId) return;
      setViewersRaw(update.viewers);
    });
    return () => {
      offTicket();
      offPresence();
    };
  }, [runtime, ticketId]);

  // Editing signal: recent typing inside an editable element anywhere on
  // the page (the detail workspace). Delegated + time-windowed — no
  // per-component wiring, honest "actively editing" semantics.
  useEffect(() => {
    if (ticketId === "") return;
    const markEditing = (ev: Event) => {
      const target = ev.target as HTMLElement | null;
      if (target === null) return;
      const tag = target.tagName;
      const editable =
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "SELECT" ||
        target.isContentEditable;
      if (editable) editingUntil.current = Date.now() + EDITING_WINDOW_MS;
    };
    document.addEventListener("input", markEditing, true);
    return () => document.removeEventListener("input", markEditing, true);
  }, [ticketId]);

  // Heartbeat loop: immediate first beat, then every 30s; paused while
  // the tab is hidden, resumed (with an immediate beat) when visible.
  useEffect(() => {
    if (ticketId === "") return;
    let stopped = false;
    let timer: ReturnType<typeof setInterval> | null = null;
    const beat = () => {
      if (stopped) return;
      void api.api
        .POST("/api/v1/presence/heartbeat", {
          body: {
            entity: "ticket",
            entity_id: ticketId,
            editing: Date.now() < editingUntil.current,
          },
        })
        .then(({ data }) => {
          if (!stopped && data !== undefined && data !== null) {
            setViewersRaw(data.viewers);
          }
        })
        .catch(() => undefined);
    };
    const start = () => {
      if (timer !== null) return;
      beat();
      timer = setInterval(beat, HEARTBEAT_MS);
    };
    const stop = () => {
      if (timer !== null) {
        clearInterval(timer);
        timer = null;
      }
    };
    const onVisibility = () => {
      if (document.visibilityState === "visible") start();
      else stop();
    };
    start();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      stopped = true;
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [ticketId]);

  // Aging ticker: re-render periodically so TTL-past entries flip to
  // idle without waiting for the next frame.
  useEffect(() => {
    if (ticketId === "") return;
    const tick = setInterval(() => setNowMs(Date.now()), AGE_TICK_MS);
    return () => clearInterval(tick);
  }, [ticketId]);

  return { viewers: rosterView(viewersRaw, nowMs) };
}
