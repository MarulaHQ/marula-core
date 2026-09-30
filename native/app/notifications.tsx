// Notification center — the pack's `07-notifications` and `08-deep-linked-open`.
//
// Purpose: grouped by day (Today, Yesterday, Earlier this week), a gold
// unread dot per unseen row, and the single red icon on the SLA breach row —
// red only because a breach is red, always with words. Mark all read rides
// the offline queue. A ticket-bearing row opens the ticket with the green
// "From a notification" banner and a Back that returns to this list, not to
// the Board.
//
// Non-purpose: no per-kind preferences (More owns registration).

import { useEffect, useMemo, useState } from "react";
import { router } from "expo-router";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";

import { MODELS, fetchNotifications, type Notification } from "@/core/models";
import { markAllNotificationsRead, markNotificationRead } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
import { RequireAuth } from "@/ui/require-auth";
import { ErrorNote, Loading } from "@/ui/primitives";
import { PACK_ICONS, PackHeader, pack, statusPairFor } from "@/ui/pack";
import { type, useTheme, type ThemeTokens , chrome } from "@/ui/tokens";

const GOLD = chrome.gold;

function dayBucket(iso: string, now: number): "today" | "yesterday" | "earlier" {
  const then = new Date(iso);
  const today = new Date(now);
  const startToday = new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime();
  const startYesterday = startToday - 24 * 60 * 60 * 1000;
  const t = then.getTime();
  if (t >= startToday) return "today";
  if (t >= startYesterday) return "yesterday";
  return "earlier";
}

const BUCKETS: { key: "today" | "yesterday" | "earlier"; label: string }[] = [
  { key: "today", label: "Today" },
  { key: "yesterday", label: "Yesterday" },
  { key: "earlier", label: "Earlier this week" },
];

/** Per-kind icon; the breach pair is the only red on the screen. */
function kindVisual(kind: string, tokens: ThemeTokens): { icon: string; pair: { text: string; background: string } } {
  if (kind === "sla_breach") {
    return { icon: PACK_ICONS.alert, pair: tokens.danger };
  }
  if (kind === "sla_risk" || kind === "escalation" || kind === "missing_time") {
    return { icon: PACK_ICONS.alert, pair: tokens.warning };
  }
  if (kind === "mention" || kind === "assignment") {
    return { icon: PACK_ICONS.users, pair: tokens.neutral };
  }
  if (kind === "ticket_update") {
    return { icon: PACK_ICONS.kanban, pair: tokens.neutral };
  }
  return { icon: PACK_ICONS.bell, pair: tokens.neutral };
}

export default function NotificationsRoute() {
  const { tokens } = useTheme();
  const { mutate } = useMutation();
  const notifications = useModel(MODELS.notifications, fetchNotifications, { maxAgeMs: 30_000 });
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const interval = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(interval);
  }, []);
  const rows = useMemo(() => notifications.data?.items ?? [], [notifications.data]);
  const unreadCount = rows.filter((row) => row.read_at == null).length;

  const grouped = useMemo(() => {
    const groups = new Map<"today" | "yesterday" | "earlier", Notification[]>();
    for (const row of rows) {
      const bucket = dayBucket(row.created_at, now);
      const list = groups.get(bucket) ?? [];
      list.push(row);
      groups.set(bucket, list);
    }
    return groups;
  }, [rows, now]);

  const open = (row: Notification) => {
    if (row.read_at == null) {
      void mutate(markNotificationRead, { notificationId: row.id });
    }
    if (row.ticket_id !== undefined && row.ticket_id !== null) {
      router.push(`/ticket/${row.ticket_id}?from=notification` as never);
    }
  };

  return (
    <RequireAuth>
      <View style={[styles.screen, { backgroundColor: tokens.background }]} testID="notifications-screen">
        <View style={styles.headerWrap}>
          <PackHeader
            center
            onBack={() => {
              if (router.canGoBack()) router.back();
              else router.replace("/(tabs)/more");
            }}
            backTestID="notifications-back"
            title={<Text style={[styles.title, { color: tokens.text }]}>Notifications</Text>}
            trailing={
              <Pressable
                testID="notifications-read-all"
                disabled={unreadCount === 0}
                onPress={() => void mutate(markAllNotificationsRead, {})}
                accessibilityRole="button"
                accessibilityLabel="Mark all read"
                style={styles.markAll}
              >
                <Text
                  style={[
                    styles.markAllText,
                    { color: unreadCount === 0 ? tokens.textMuted : tokens.text },
                  ]}
                >
                  Mark all read
                </Text>
              </Pressable>
            }
          />
        </View>
        {notifications.status === "error" && notifications.data === undefined ? (
          <ErrorNote
            text={notifications.error?.text ?? "Couldn't load notifications."}
            onRetry={notifications.refresh}
          />
        ) : null}
        {notifications.status === "loading" && notifications.data === undefined ? (
          <Loading label="Loading notifications" />
        ) : null}
        <View style={styles.list}>
          {BUCKETS.map((bucket) => {
            const group = grouped.get(bucket.key) ?? [];
            if (group.length === 0) return null;
            return (
              <View key={bucket.key}>
                <Text style={[styles.bucketLabel, { color: tokens.textSecondary }]}>
                  {bucket.label}
                </Text>
                {group.map((row) => {
                  const visual = kindVisual(row.kind, tokens);
                  const unread = row.read_at == null;
                  return (
                    <Pressable
                      key={row.id}
                      testID={`notification-${row.id}`}
                      onPress={() => open(row)}
                      style={({ pressed }) => [styles.row, { borderBottomColor: tokens.border, opacity: pressed ? 0.85 : 1 }]}
                    >
                      <View style={[styles.iconDisc, { backgroundColor: visual.pair.background }]}>
                        <Ionicons name={visual.icon as never} size={19} color={visual.pair.text} />
                      </View>
                      <View style={styles.rowBody}>
                        <Text style={[styles.rowTitle, { color: tokens.text }]} numberOfLines={2}>
                          {row.title}
                        </Text>
                        <Text style={[styles.rowMeta, { color: tokens.textSecondary }]} numberOfLines={1}>
                          {[row.body, clockStamp(row.created_at)].filter(Boolean).join(" · ")}
                        </Text>
                      </View>
                      {unread ? <View style={[styles.unreadDot, { backgroundColor: GOLD }]} /> : null}
                    </Pressable>
                  );
                })}
              </View>
            );
          })}
          {notifications.data !== undefined && rows.length === 0 ? (
            <Text style={[styles.empty, { color: tokens.textMuted }]}>
              Nothing here yet — mentions, assignments, and alerts land here.
            </Text>
          ) : null}
        </View>
      </View>
    </RequireAuth>
  );
}

function clockStamp(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

void statusPairFor;
void pack;

const styles = StyleSheet.create({
  screen: {
    flex: 1,
  },
  headerWrap: {
    paddingHorizontal: 8,
    paddingTop: 8,
  },
  title: {
    fontFamily: type.sansSemiBold,
    fontSize: 22,
  },
  markAll: {
    minHeight: 44,
    justifyContent: "center",
    paddingHorizontal: 8,
  },
  markAllText: {
    fontFamily: type.sansSemiBold,
    fontSize: 14,
    textDecorationLine: "underline",
  },
  list: {
    flex: 1,
    paddingHorizontal: 16,
    paddingTop: 4,
  },
  bucketLabel: {
    fontFamily: type.sansMedium,
    fontSize: 12,
    letterSpacing: 0.8,
    textTransform: "uppercase",
    marginTop: 18,
    marginBottom: 4,
  },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
    minHeight: 72,
    paddingVertical: 14,
    borderBottomWidth: 1,
  },
  iconDisc: {
    width: 40,
    height: 40,
    borderRadius: 999,
    alignItems: "center",
    justifyContent: "center",
  },
  rowBody: {
    flex: 1,
    minWidth: 0,
    gap: 3,
  },
  rowTitle: {
    fontFamily: type.sansMedium,
    fontSize: 14,
    lineHeight: 19,
  },
  rowMeta: {
    fontFamily: type.sans,
    fontSize: 12,
  },
  unreadDot: {
    width: 10,
    height: 10,
    borderRadius: 5,
    marginTop: 6,
  },
  empty: {
    fontFamily: type.sans,
    fontSize: 14,
    textAlign: "center",
    paddingTop: 40,
  },
});
