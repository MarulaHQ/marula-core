// Timer pill — the pack's 56px ink pill above the tab bar, in every tab.
//
// Purpose: one server-backed truth (the active timer session) drawn as the
// mockups' `.timer`: ink surface in every theme, a gold pause disc, the
// gold tabular clock, a one-line context, and a stop disc that jumps to the
// Time tab's log sheet. Ticks locally between server reconciliations; the
// time entry itself is written only through the log sheet.
//
// Non-purpose: no local timer creation — starting happens on a ticket.

import { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { useRouter } from "expo-router";
import Ionicons from "@expo/vector-icons/Ionicons";

import { MODELS, fetchActiveTimer } from "@/core/models";
import { pauseTimer, resumeTimer } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
import { isPaused } from "@/ui/timer-bar";
import { elapsedMs, clockText } from "@/ui/clock-format";import { pack } from "@/ui/pack";
import { type , chrome } from "@/ui/tokens";

const GOLD = chrome.gold;
const INK = chrome.ink;
const INK_SOFT = chrome.surfaceHoverDark;
const CREAM = chrome.cream;

/** Where "Stop" sends its owner: the Time tab owns stop-and-log. */
export const TIME_TAB_HREF = "/(tabs)/time" as const;

export function TimePill() {
  const router = useRouter();
  const { mutate } = useMutation();
  const timer = useModel(MODELS.activeTimer, fetchActiveTimer);
  const session = timer.data ?? null;
  const paused = session !== null && isPaused(session);
  const seconds = useSecondTick(session !== null && !paused);

  if (session === null) return null;

  return (
    <View
      testID="timer-pill"
      accessibilityLabel="Running timer"
      style={styles.pill}
    >
      <Pressable
        testID="timer-pill-pause"
        onPress={() => {
          void mutate(paused ? resumeTimer : pauseTimer, {
            sessionId: session.id,
            ...(session.ticket_id !== null ? { ticketId: session.ticket_id } : {}),
          });
        }}
        accessibilityRole="button"
        accessibilityLabel={paused ? "Resume timer" : "Pause timer"}
        style={({ pressed }) => [styles.disc, { backgroundColor: GOLD, opacity: pressed ? 0.85 : 1 }]}
      >
        <Ionicons name={paused ? "play" : "pause"} size={18} color={INK} />
      </Pressable>
      <Text testID="timer-pill-clock" style={[styles.clock, { color: GOLD }]}>
        {clockText(elapsedMs(session, seconds))}
      </Text>
      <Pressable
        onPress={() => router.push(TIME_TAB_HREF)}
        accessibilityRole="button"
        accessibilityLabel="Running timer, open the log"
        style={styles.context}
      >
        <Text style={[styles.contextText, { color: CREAM }]} numberOfLines={1}>
          Running
        </Text>
      </Pressable>
      <Pressable
        testID="timer-pill-stop"
        onPress={() => router.push(TIME_TAB_HREF)}
        accessibilityRole="button"
        accessibilityLabel="Stop and log"
        style={({ pressed }) => [
          styles.disc,
          { backgroundColor: INK_SOFT, opacity: pressed ? 0.85 : 1 },
        ]}
      >
        <Ionicons name="stop" size={18} color={CREAM} />
      </Pressable>
    </View>
  );
}

/** A one-second clock that ticks only while `active` (the pill is the one
 * always-mounted timer surface, so it owns the render tick). */
export function useSecondTick(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const interval = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(interval);
  }, [active]);
  return now;
}

const styles = StyleSheet.create({
  pill: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    height: pack.timerPillHeight,
    marginHorizontal: 12,
    marginBottom: 6,
    borderRadius: 16,
    backgroundColor: INK,
    paddingHorizontal: 8,
  },
  disc: {
    width: 40,
    height: 40,
    borderRadius: 999,
    alignItems: "center",
    justifyContent: "center",
  },
  clock: {
    fontFamily: type.sansSemiBold,
    fontSize: 18,
    fontVariant: ["tabular-nums"],
  },
  context: {
    flex: 1,
    minHeight: 44,
    justifyContent: "center",
  },
  contextText: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
});
