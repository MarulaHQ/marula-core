// More — the pack's `06-more`: profile card, short lists, appearance.
//
// Purpose: the drawn surface: the avatar card (Tumi L., Service desk ·
// Online, the gold On call tonight pill), a rounded list group (Travel with
// its draft pill, Notifications with a new-count pill, On call rota), the
// Appearance Light/Dark segmented pair, Offline and sync (Synced time), a
// Sign out row, and the quiet footer (Marula 1.0 · device). Field/dev legs
// (push registration, biometrics, dev tools) stay — they are the foundation
// tests — but tuck under a second group so the drawn surface reads first.
//
// Non-purpose: no settings editing (delta!.

import { useEffect, useState } from "react";
import { router } from "expo-router";
import { StyleSheet, Text, View } from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";

import { useAuth } from "@/auth/auth-context";
import { APP_VERSION, DEV_TOOLS_ENABLED } from "@/config";
import { MODELS, fetchUnreadCount } from "@/core/models";
import { useConnectivity } from "@/data/connectivity";
import { useModel, useQueue } from "@/data/sync-context";
import {
  Card,
  ListRow,
  Screen,
  SectionHeader,
  Toggle,
} from "@/ui/primitives";
import { PackPill, PACK_ICONS, Segment } from "@/ui/pack";
import { chrome, type, useTheme, type ThemeMode } from "@/ui/tokens";

void PACK_ICONS;

export default function MoreScreen() {
  const { tokens, mode, setMode } = useTheme();
  const { me, signOut, biometricAvailable, biometricEnabled, setBiometricEnabled } = useAuth();
  const { online } = useConnectivity();
  const { pendingCount } = useQueue();
  const unread = useModel(MODELS.unreadCount, fetchUnreadCount, { maxAgeMs: 60_000 });
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    const interval = setInterval(() => setNow(new Date()), 60_000);
    return () => clearInterval(interval);
  }, []);

  const appearance = mode === "system" ? "system" : mode;

  return (
    <Screen testID="more-screen">
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>More</Text>
      </View>

      <Card testID="more-profile">
        <View style={styles.profileRow}>
          <View style={[styles.avatar, { backgroundColor: chrome.leaf }]}>
            <Text style={[styles.avatarText, { color: chrome.cream }]}>{initials(me?.display_name)}</Text>
          </View>
          <View style={styles.profileBody}>
            <Text style={[styles.name, { color: tokens.text }]}>{me?.display_name ?? "—"}</Text>
            <Text style={[styles.sub, { color: tokens.textSecondary }]}>
              Service desk · {online ? "Online" : "Offline"}
            </Text>
          </View>
          <PackPill pair={tokens.warning} label="On call tonight" />
        </View>
      </Card>

      <Card style={styles.listCard}>
        <ListRow
          testID="more-travel"
          title="Travel"
          subtitle={pendingCount > 0 ? `${pendingCount} draft${pendingCount === 1 ? "" : "s"}` : "No drafts"}
          onPress={() => router.push("/" as never)}
          trailing={<Ionicons name={PACK_ICONS.car} size={20} color={tokens.textSecondary} />}
        />
        <ListRow
          testID="more-notifications"
          title="Notifications"
          subtitle={
            unread.data !== undefined && unread.data.count > 0
              ? `${unread.data.count} new`
              : "You're caught up"
          }
          onPress={() => router.push("/notifications")}
          trailing={<Ionicons name={PACK_ICONS.bell} size={20} color={tokens.textSecondary} />}
        />
        <ListRow
          title="On call rota"
          onPress={() => router.push("/" as never)}
          trailing={<Ionicons name={PACK_ICONS.users} size={20} color={tokens.textSecondary} />}
        />
      </Card>

      <Card style={styles.listCard}>
        <View style={styles.appearanceRow}>
          <Text style={[styles.rowLabel, { color: tokens.text }]}>Appearance</Text>
          <View style={styles.appearanceSeg}>
            <Segment
              size="small"
              options={[
                { value: "light", label: "Light" },
                { value: "dark", label: "Dark" },
              ]}
              value={appearance === "dark" ? "dark" : "light"}
              onChange={(next) => setMode(next as ThemeMode)}
              testIDPrefix="more-theme"
            />
          </View>
        </View>
        <ListRow
          title="Offline and sync"
          subtitle={
            online && pendingCount === 0
              ? `Synced ${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`
              : pendingCount > 0
                ? `${pendingCount} change${pendingCount === 1 ? "" : "s"} waiting`
                : "Offline — changes sync when you're back"
          }
        />
      </Card>

      <Card style={styles.listCard}>
        <View style={styles.signOutRow}>
          <Ionicons name={PACK_ICONS.logout} size={20} color={tokens.text} />
          <Text
            testID="more-signout"
            style={[styles.signOutText, { color: tokens.text }]}
            onPress={() => void signOut()}
          >
            Sign out
          </Text>
        </View>
      </Card>

      <Text style={[styles.footer, { color: tokens.textMuted }]}>
        Marula {APP_VERSION} · Pixel 8
      </Text>

      {biometricAvailable ? (
        <>
          <SectionHeader title="Security" />
          <Card>
            <View style={styles.rowBetween}>
              <View style={styles.rowText}>
                <Text style={[styles.rowLabel, { color: tokens.text }]}>Biometric unlock</Text>
                <Text style={[styles.rowHint, { color: tokens.textMuted }]}>
                  Require Face ID (or Touch ID) to open your saved session.
                </Text>
              </View>
              <Toggle
                testID="more-biometric-toggle"
                value={biometricEnabled}
                onValueChange={(value) => {
                  void setBiometricEnabled(value);
                }}
                label="Biometric sign-in"
              />
            </View>
          </Card>
        </>
      ) : null}

      {DEV_TOOLS_ENABLED ? (
        <>
          <SectionHeader title="Dev tools" />
          <Card>
            <ListRow
              title="Offline queue inspector"
              subtitle="Per-op audit trail, retry, and discard"
              onPress={() => router.push("/dev/queue")}
            />
            <ListRow
              title="Send a scan deep link"
              subtitle="marula://scan-result?text=… with a sample payload"
              onPress={() =>
                router.push(`/scan-result?text=${encodeURIComponent("RSV-SRV-004")}` as never)
              }
            />
          </Card>
        </>
      ) : null}
    </Screen>
  );
}

function initials(displayName: string | null | undefined): string {
  if (displayName === null || displayName === undefined || displayName.trim() === "") return "·";
  const parts = displayName.trim().split(/\s+/);
  return parts
    .slice(0, 2)
    .map((part) => part.slice(0, 1).toUpperCase())
    .join("");
}



const styles = StyleSheet.create({
  header: {
    paddingTop: 8,
  },
  title: {
    fontFamily: type.sansBold,
    fontSize: 26,
    letterSpacing: -0.4,
  },
  profileRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  avatar: {
    width: 48,
    height: 48,
    borderRadius: 999,
    alignItems: "center",
    justifyContent: "center",
  },
  avatarText: {
    fontFamily: type.sansBold,
    fontSize: 16,
  },
  profileBody: {
    flex: 1,
    minWidth: 0,
    gap: 2,
  },
  name: {
    fontFamily: type.sansSemiBold,
    fontSize: 17,
  },
  sub: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  listCard: {
    paddingVertical: 0,
    paddingHorizontal: 0,
    overflow: "hidden",
  },
  appearanceRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    paddingHorizontal: 16,
    minHeight: 64,
    paddingVertical: 10,
  },
  appearanceSeg: {
    width: 170,
  },
  rowLabel: {
    fontFamily: type.sansMedium,
    fontSize: 15,
  },
  rowHint: {
    fontFamily: type.sans,
    fontSize: 12,
    lineHeight: 17,
  },
  rowText: {
    flex: 1,
    gap: 2,
  },
  rowBetween: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    minHeight: 56,
  },
  signOutRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 16,
    minHeight: 56,
  },
  signOutText: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
    minHeight: 44,
    lineHeight: 44 - 8,
  },
  footer: {
    fontFamily: type.sans,
    fontSize: 12,
    textAlign: "center",
    paddingBottom: 4,
  },
});
