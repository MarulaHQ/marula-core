// Live timer hook — keeps the lock screen / ongoing notification on the ONE
// server-backed timer entity.
//
// Purpose: watch the active timer session (and its ticket labels), derive
// the snapshot, and tell the platform bridge to start/update/end. The
// ticking second is rendered by the platform; only pause/resume/switch and
// label changes send updates. Unsupported platforms (web, Expo Go, tests)
// report unsupported and nothing else happens.
//
// Non-purpose: no session control. Starting and stopping timers stays on
// the ticket surface.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  MODELS,
  fetchActiveTimer,
  fetchTicket,
} from "@/core/models";
import { useModel } from "@/data/sync-context";
import { liveTimerActivity } from "@/live/activity";
import { rememberActivityPushToken } from "@/live/activity-push";
import { useLiveTimerEnabled } from "@/live/live-pref";
import { liveTimerSnapshot } from "@/live/timer-snapshot";

export function useLiveTimer(): {
  supported: boolean;
  enabled: boolean;
  setEnabled: (next: boolean) => void;
  pushToken: string | null;
  activeSessionId: string | null;
} {
  const [enabled, setEnabled] = useLiveTimerEnabled();
  const timer = useModel(MODELS.activeTimer, fetchActiveTimer);
  const ticketId = timer.data?.ticket_id ?? null;
  const ticket = useModel(
    ticketId !== null ? MODELS.ticket(ticketId) : "ticket:missing",
    useCallback((client) => fetchTicket(client, ticketId ?? ""), [ticketId]),
  );

  const [supported, setSupported] = useState(false);
  const [pushToken, setPushToken] = useState<string | null>(null);
  const lastSignature = useRef<string | null>(null);

  const snapshot = useMemo(() => {
    if (timer.data === undefined || timer.data === null) return null;
    return liveTimerSnapshot(timer.data, {
      ticketNumber: ticket.data?.number ?? null,
      title: ticket.data?.title ?? null,
    });
  }, [timer.data, ticket.data]);

  const signature =
    snapshot !== null
      ? [
          snapshot.sessionId,
          snapshot.state,
          snapshot.segmentStartedAtIso,
          Math.round(snapshot.accumulatedSeconds),
          snapshot.ticketNumber,
          snapshot.title,
        ].join("|")
      : "";

  useEffect(() => {
    let alive = true;
    void liveTimerActivity.isSupported().then((value) => {
      if (alive) setSupported(value);
    });
    const unsubscribe = liveTimerActivity.subscribePushToken((token, sessionId) => {
      void rememberActivityPushToken(sessionId, token);
      if (alive) setPushToken(token);
    });
    return () => {
      alive = false;
      unsubscribe();
    };
  }, []);

  useEffect(() => {
    if (!supported) return;
    if (!enabled || snapshot === null) {
      if (lastSignature.current !== null) {
        lastSignature.current = null;
        void liveTimerActivity.end();
      }
      return;
    }
    if (signature === lastSignature.current) return;
    const isStart = lastSignature.current === null;
    lastSignature.current = signature;
    void (isStart
      ? liveTimerActivity.start(snapshot)
      : liveTimerActivity.update(snapshot)
    ).catch(() => {
      // The activity may be disabled by the user at the OS level; the bar
      // still works, so a failed mirror is not an error the tech must see.
      lastSignature.current = null;
    });
  }, [enabled, signature, snapshot, supported]);

  return {
    supported,
    enabled,
    setEnabled,
    pushToken,
    activeSessionId: snapshot?.sessionId ?? null,
  };
}
