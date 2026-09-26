// Live timer status (dev) — what the lock screen is showing, and why.
//
// Purpose: one place to see whether the platform bridge is supported, which
// timer session is mirrored, the ActivityKit push token the device got, and
// the server-side gap that stops background updates. Manual start/end makes
// the Dynamic Island capture deterministic for the review flow.

import { router } from "expo-router";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import {
  MODELS,
  fetchActiveTimer,
  fetchTicket,
} from "@/core/models";
import { useModel } from "@/data/sync-context";
import { liveTimerActivity } from "@/live/activity";
import { PUSH_TOKEN_SERVER_GAP } from "@/live/activity-push";
import { liveTimerSnapshot } from "@/live/timer-snapshot";
import { useLiveTimerEnabled } from "@/live/live-pref";
import {
  Button,
  Card,
  EmptyState,
  Mono,
  Pill,
  Screen,
  SectionHeader,
  Toggle,
} from "@/ui/primitives";
import { type, useTheme } from "@/ui/tokens";

export default function DevLiveScreen() {
  const { tokens } = useTheme();
  const [supported, setSupported] = useState<boolean | null>(null);
  const [token, setToken] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [banner, setBanner] = useState<string | null>(null);
  const [enabled, setEnabled] = useLiveTimerEnabled();

  const timer = useModel(MODELS.activeTimer, fetchActiveTimer);
  const ticketId = timer.data?.ticket_id ?? null;
  const ticket = useModel(
    ticketId !== null ? MODELS.ticket(ticketId) : "ticket:missing",
    useCallback((client) => fetchTicket(client, ticketId ?? ""), [ticketId]),
  );
  const snapshot = useMemo(() => {
    if (timer.data === undefined || timer.data === null) return null;
    return liveTimerSnapshot(timer.data, {
      ticketNumber: ticket.data?.number ?? null,
      title: ticket.data?.title ?? null,
    });
  }, [timer.data, ticket.data]);

  useEffect(() => {
    void liveTimerActivity.isSupported().then(setSupported);
    void liveTimerActivity.pushToken().then(setToken);
    const unsubscribe = liveTimerActivity.subscribePushToken((next) => setToken(next));
    return unsubscribe;
  }, []);

  const act = async (action: "start" | "update" | "end") => {
    if (action === "end") {
      setBusy(true);
      await liveTimerActivity.end();
      setBusy(false);
      setBanner("Activity ended.");
      return;
    }
    if (snapshot === null) {
      setBanner("No timer is running — start one from a ticket first.");
      return;
    }
    setBusy(true);
    try {
      await (action === "start"
        ? liveTimerActivity.start(snapshot)
        : liveTimerActivity.update(snapshot));
    } catch (cause) {
      setBusy(false);
      setBanner(
        cause instanceof Error ? `The bridge rejected it: ${cause.message}` : "The bridge rejected it.",
      );
      return;
    }
    setBusy(false);
    setBanner(
      action === "start"
        ? "Activity started — check the Dynamic Island / lock screen."
        : "Activity updated from the current server session.",
    );
  };

  return (
    <Screen testID="dev-live-screen">
      <Pressable onPress={() => router.back()} hitSlop={8}>
        <Text style={[styles.back, { color: tokens.primary }]}>‹ Back</Text>
      </Pressable>
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>Live timer</Text>
        <Text style={[styles.subtitle, { color: tokens.textMuted }]}>
          One server-backed timer session, mirrored to the lock screen.
        </Text>
      </View>

      <SectionHeader title="Bridge" />
      <Card>
        <View style={styles.statusRow}>
          <Text style={[styles.statusLabel, { color: tokens.text }]}>Module support</Text>
          <Pill
            pair={supported === true ? tokens.success : tokens.warning}
            label={
              supported === null ? "CHECKING" : supported ? "SUPPORTED" : "UNSUPPORTED"
            }
          />
        </View>
        <View style={styles.statusRow}>
          <Text style={[styles.statusLabel, { color: tokens.text }]}>Mirror enabled</Text>
          <Toggle value={enabled} onValueChange={setEnabled} label="Mirror" />
        </View>
        <View style={styles.statusRow}>
          <Text style={[styles.statusLabel, { color: tokens.text }]}>Session</Text>
          <Mono muted>{snapshot?.sessionId ?? "none"}</Mono>
        </View>
        <View style={styles.statusRow}>
          <Text style={[styles.statusLabel, { color: tokens.text }]}>Push token</Text>
          <Mono muted>{token !== null ? `${token.slice(0, 12)}…` : "none yet"}</Mono>
        </View>
        <Text style={[styles.gap, { color: tokens.textMuted }]}>{PUSH_TOKEN_SERVER_GAP}</Text>
      </Card>

      <SectionHeader title="Manual control" />
      <Card>
        {snapshot === null ? (
          <EmptyState message="No timer is running. Start one from a ticket to mirror it." />
        ) : (
          <>
            <Mono muted>
              {snapshot.ticketNumber} · {snapshot.state} ·{" "}
              {Math.round(snapshot.totalSeconds)}s banked
            </Mono>
            <Button
              label="Start from the active session"
              testID="dev-live-start"
              fullWidth
              loading={busy}
              onPress={() => void act("start")}
            />
            <Button
              label="Update"
              variant="secondary"
              testID="dev-live-update"
              fullWidth
              loading={busy}
              onPress={() => void act("update")}
            />
          </>
        )}
        <Button
          label="End the activity"
          variant="ghost"
          testID="dev-live-end"
          fullWidth
          loading={busy}
          onPress={() => void act("end")}
        />
        {banner !== null ? <Mono muted>{banner}</Mono> : null}
      </Card>
    </Screen>
  );
}

const styles = StyleSheet.create({
  back: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  header: {
    paddingTop: 4,
    gap: 2,
  },
  title: {
    fontFamily: type.sansBold,
    fontSize: 26,
    letterSpacing: -0.4,
  },
  subtitle: {
    fontFamily: type.sans,
    fontSize: 13,
    lineHeight: 19,
  },
  statusRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
    minHeight: 44,
  },
  statusLabel: {
    fontFamily: type.sansMedium,
    fontSize: 15,
  },
  gap: {
    fontFamily: type.sans,
    fontSize: 12,
    lineHeight: 17,
  },
});
