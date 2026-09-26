// Notification center — the fan-out read model with live badge sync.
//
// Purpose: the caller's notifications hydrate from the REST read model and
// live through `notification.created` / `notification.read` events on the
// user channel, so the badge and the list move together across devices.
// Marking read goes through the offline queue (it is a mutation like any
// other) and routes to the ticket when one is attached.

import { router } from "expo-router";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { MODELS, fetchNotifications, type Notification } from "@/core/models";
import { markAllNotificationsRead, markNotificationRead } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
import { RequireAuth } from "@/ui/require-auth";
import {
  Button,
  Card,
  EmptyState,
  ErrorNote,
  Loading,
  Mono,
  Pill,
  Screen,
} from "@/ui/primitives";
import { infoPair, type, useTheme } from "@/ui/tokens";

export default function NotificationsRoute() {
  const { tokens } = useTheme();
  const { mutate } = useMutation();
  const notifications = useModel(MODELS.notifications, fetchNotifications, { maxAgeMs: 30_000 });
  const rows = notifications.data?.items ?? [];
  const unread = rows.filter((row) => row.read_at == null);

  return (
    <RequireAuth>
      <Screen testID="notifications-screen">
        <Pressable
          testID="notifications-back"
          onPress={() => {
            if (router.canGoBack()) router.back();
            else router.replace("/");
          }}
          hitSlop={8}
        >
          <Text style={[styles.back, { color: tokens.primary }]}>‹ Back</Text>
        </Pressable>
        <View style={styles.header}>
          <Text style={[styles.title, { color: tokens.text }]}>Notifications</Text>
          <Text style={[styles.subtitle, { color: tokens.textMuted }]}>
            {unread.length === 0 ? "You're caught up." : `${unread.length} unread`}
          </Text>
        </View>
        <View style={styles.actions}>
          <Button
            label="Mark all read"
            variant="secondary"
            testID="notifications-read-all"
            disabled={unread.length === 0}
            onPress={() => void mutate(markAllNotificationsRead, {})}
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
        {notifications.data !== undefined && rows.length === 0 ? (
          <EmptyState message="Nothing here yet — mentions, assignments, and alerts land here." />
        ) : null}
        {rows.map((row: Notification) => (
          <NotificationRow
            key={row.id}
            notification={row}
            onPress={() => {
              if (row.read_at == null) {
                void mutate(markNotificationRead, { notificationId: row.id });
              }
              if (row.ticket_id !== undefined && row.ticket_id !== null) {
                router.push(`/ticket/${row.ticket_id}`);
              }
            }}
          />
        ))}
      </Screen>
    </RequireAuth>
  );
}

function NotificationRow({
  notification,
  onPress,
}: {
  notification: Notification;
  onPress: () => void;
}) {
  const { tokens } = useTheme();
  const unread = notification.read_at == null;
  const hasTicket = notification.ticket_id !== undefined && notification.ticket_id !== null;
  return (
    <Card testID={`notification-${notification.id}`} onPress={onPress} style={styles.card}>
      <View style={styles.rowHeader}>
        <Text
          style={[styles.rowTitle, unread ? styles.rowTitleUnread : null, { color: tokens.text }]}
          numberOfLines={2}
        >
          {notification.title}
        </Text>
        {unread ? <Pill pair={infoPair(tokens)} label="NEW" /> : null}
      </View>
      {notification.body !== null && notification.body !== "" ? (
        <Text style={[styles.rowBody, { color: tokens.textSecondary }]} numberOfLines={3}>
          {notification.body}
        </Text>
      ) : null}
      <View style={styles.rowFooter}>
        <Pill pair={tokens.neutral} label={kindLabel(notification.kind)} />
        {hasTicket ? (
          <Text style={[styles.affordance, { color: tokens.primary }]}>Open ticket →</Text>
        ) : unread ? (
          <Text style={[styles.affordance, { color: tokens.textMuted }]}>Tap to mark read</Text>
        ) : null}
        <View style={styles.footerSpacer} />
        <Mono muted>
          {new Date(notification.created_at).toLocaleString(undefined, {
            month: "short",
            day: "numeric",
            hour: "2-digit",
            minute: "2-digit",
          })}
        </Mono>
      </View>
    </Card>
  );
}

function kindLabel(kind: string): string {
  return kind.replace(/_/g, " ").toUpperCase();
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
  },
  subtitle: {
    fontFamily: type.sans,
    fontSize: 14,
  },
  actions: {
    flexDirection: "row",
  },
  card: {
    gap: 6,
  },
  rowHeader: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 8,
  },
  rowTitle: {
    flex: 1,
    fontFamily: type.sansMedium,
    fontSize: 15,
  },
  rowTitleUnread: {
    fontFamily: type.sansSemiBold,
  },
  rowBody: {
    fontFamily: type.sans,
    fontSize: 14,
    lineHeight: 20,
  },
  rowFooter: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  affordance: {
    fontFamily: type.sansSemiBold,
    fontSize: 13,
  },
  footerSpacer: {
    flex: 1,
  },
});
