// Offline / sync state banner and the synced chip.
//
// Purpose: offline is a first-class state, not a dead end (mockup:
// "Offline — 3 changes queued for sync"). One banner tells the operator
// what is queued, what is replaying, and what was rejected — and routes to
// the queue inspector when a human has to act.

import { Pressable, StyleSheet, Text, View } from "react-native";
import { router } from "expo-router";

import { useConnectivity } from "@/data/connectivity";
import { useQueue } from "@/data/sync-context";
import { shape, type, useTheme } from "./tokens";

export function OfflineBanner() {
  const { tokens } = useTheme();
  const { online } = useConnectivity();
  const { pendingCount, rejected } = useQueue();

  if (rejected.length > 0) {
    return (
      <Banner
        color={tokens.danger.text}
        background={tokens.danger.background}
        text={`${rejected.length === 1 ? "1 change" : `${rejected.length} changes`} couldn't sync — review them`}
        badge={String(rejected.length)}
        onPress={() => router.push("/dev/queue")}
        testID="banner-rejected"
      />
    );
  }
  if (!online) {
    return (
      <Banner
        color={tokens.warning.text}
        background={tokens.warning.background}
        text={
          pendingCount > 0
            ? `Offline — ${pendingCount === 1 ? "1 change" : `${pendingCount} changes`} queued for sync`
            : "Offline — reading from this device"
        }
        badge={pendingCount > 0 ? String(pendingCount) : undefined}
        onPress={() => router.push("/dev/queue")}
        testID="banner-offline"
      />
    );
  }
  if (pendingCount > 0) {
    return (
      <Banner
        color={tokens.warning.text}
        background={tokens.warning.background}
        text={`Syncing ${pendingCount === 1 ? "1 change" : `${pendingCount} changes`}…`}
        badge={String(pendingCount)}
        onPress={() => router.push("/dev/queue")}
        testID="banner-syncing"
      />
    );
  }
  return null;
}

function Banner({
  text,
  badge,
  color,
  background,
  onPress,
  testID,
}: {
  text: string;
  badge?: string;
  color: string;
  background: string;
  onPress: () => void;
  testID: string;
}) {
  return (
    <Pressable
      testID={testID}
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={text}
      style={[styles.banner, { backgroundColor: background }]}
    >
      <View style={[styles.stripe, { backgroundColor: color }]} />
      <Text style={[styles.text, { color }]} numberOfLines={2}>
        {text}
      </Text>
      {badge !== undefined ? (
        <View style={[styles.badge, { borderColor: color }]}>
          <Text style={[styles.badgeText, { color }]}>{badge}</Text>
        </View>
      ) : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  banner: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    borderBottomWidth: 1,
    borderBottomColor: "transparent",
    paddingRight: 12,
    minHeight: 44,
  },
  stripe: {
    width: 4,
    alignSelf: "stretch",
  },
  text: {
    flex: 1,
    fontFamily: type.sansMedium,
    fontSize: 13,
    paddingVertical: 8,
  },
  badge: {
    borderWidth: 1,
    borderRadius: shape.radiusControl,
    minWidth: 28,
    minHeight: 28,
    alignItems: "center",
    justifyContent: "center",
  },
  badgeText: {
    fontFamily: type.sansSemiBold,
    fontSize: 13,
  },
});
