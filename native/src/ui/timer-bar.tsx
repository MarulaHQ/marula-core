// Running-timer bar — the mockup's pinned strip above the tab bar.
//
// Purpose: a timer is a server-backed entity (GET /timers/active +
// start/pause/resume/stop mutations), never mobile-private state. This bar
// renders the server's session, ticks its elapsed time locally between
// reconciliations, and sends pause/resume through the offline queue so an
// offline pause replays in order on the same entity.
//
// Non-purpose: no local timer creation — starting happens on a ticket.

import { useEffect, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { router } from "expo-router";

import { MODELS, fetchActiveTimer, type TimerSession } from "@/core/models";
import { pauseTimer, resumeTimer } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
import { useLiveTimer } from "@/live/use-live-timer";
import { shape, type, useTheme } from "./tokens";

export function isPaused(session: TimerSession): boolean {
  const segments = session.segments ?? [];
  const last = segments[segments.length - 1];
  return last !== undefined && last.ended_at != null;
}

export function elapsedMs(session: TimerSession, now: number): number {
  let total = 0;
  for (const segment of session.segments ?? []) {
    const start = Date.parse(segment.started_at);
    if (Number.isNaN(start)) continue;
    const end = segment.ended_at != null ? Date.parse(segment.ended_at) : now;
    if (Number.isNaN(end)) continue;
    total += Math.max(0, end - start);
  }
  return total;
}

export function clock(totalMs: number): string {
  const seconds = Math.floor(totalMs / 1000);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = seconds % 60;
  return [hours, minutes, secs]
    .map((part) => String(part).padStart(2, "0"))
    .join(":");
}

export function TimerBar() {
  const { tokens } = useTheme();
  const timer = useModel(MODELS.activeTimer, fetchActiveTimer);
  const { mutate } = useMutation();
  // The bar is the one always-mounted timer surface, so it also drives the
  // lock-screen mirror (one truth: the same server session).
  const live = useLiveTimer();
  const [now, setNow] = useState(() => Date.now());
  const session = timer.data ?? null;
  const paused = session !== null && isPaused(session);

  useEffect(() => {
    if (session === null || paused) return;
    const interval = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(interval);
  }, [session, paused]);

  const label = useMemo(() => {
    if (session === null) return "";
    const total = elapsedMs(session, now);
    return `${paused ? "Paused" : "Running"} · ${clock(total)}`;
  }, [session, now, paused]);

  if (session === null) return null;

  return (
    <View
      testID="timer-bar"
      style={[styles.bar, { backgroundColor: tokens.surface, borderColor: tokens.border }]}
    >
      <Pressable
        onPress={() => router.push(`/ticket/${session.ticket_id}`)}
        style={styles.info}
        accessibilityRole="button"
        accessibilityLabel={`Timer ${label}, open ticket`}
      >
        <View
          style={[
            styles.dot,
            { backgroundColor: paused ? tokens.warning.text : tokens.success.text },
          ]}
        />
        <Text style={[styles.label, { color: tokens.text }]}>{label}</Text>
      </Pressable>
      {live.supported && live.enabled && live.activeSessionId !== null ? (
        <View
          testID="timer-live"
          style={[styles.live, { borderColor: tokens.border }]}
        >
          <Text style={[styles.liveText, { color: tokens.textMuted }]}>LIVE</Text>
        </View>
      ) : null}
      <Pressable
        testID={paused ? "timer-resume" : "timer-pause"}
        onPress={() => {
          void mutate(paused ? resumeTimer : pauseTimer, {
            sessionId: session.id,
            ...(session.ticket_id !== undefined && session.ticket_id !== null
              ? { ticketId: session.ticket_id }
              : {}),
          });
        }}
        hitSlop={8}
        style={({ pressed }) => [
          styles.control,
          {
            borderColor: tokens.border,
            backgroundColor: tokens.surfaceHover,
            opacity: pressed ? 0.85 : 1,
          },
        ]}
        accessibilityRole="button"
        accessibilityLabel={paused ? "Resume timer" : "Pause timer"}
      >
        <Text style={[styles.controlText, { color: tokens.text }]}>
          {paused ? "▶" : "❚❚"}
        </Text>
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  bar: {
    flexDirection: "row",
    alignItems: "center",
    borderTopWidth: 1,
    borderBottomWidth: 1,
    paddingHorizontal: 12,
    minHeight: 48,
    gap: 12,
  },
  info: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 44,
  },
  dot: {
    width: 8,
    height: 8,
    borderRadius: 4,
  },
  label: {
    fontFamily: type.mono,
    fontSize: 14,
    fontVariant: ["tabular-nums"],
  },
  control: {
    minWidth: 44,
    minHeight: 36,
    borderRadius: shape.radiusControl,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  controlText: {
    fontFamily: type.sansSemiBold,
    fontSize: 12,
  },
  live: {
    borderWidth: 1,
    borderRadius: shape.radiusPill,
    paddingHorizontal: 6,
    paddingVertical: 3,
  },
  liveText: {
    fontFamily: type.mono,
    fontSize: 9,
    letterSpacing: 0.5,
  },
});
