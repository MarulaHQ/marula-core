// More — account, appearance, security, sync, notifications, dev tools.
//
// Purpose: the settings surface the foundation needs: theme mode (light and
// dark are both first-class), the biometric-unlock preference, the offline
// queue's state with a manual sync, push registration state (with its
// honest server gap), and the dev tools that make scan/offline behaviour
// testable on a simulator.

import { router } from "expo-router";
import { useEffect, useState } from "react";
import { Linking, StyleSheet, Text, View } from "react-native";

import { useAuth } from "@/auth/auth-context";
import {
  API_URL,
  APP_VERSION,
  DEV_TOOLS_ENABLED,
  EAS_PROJECT_ID,
  WS_URL,
} from "@/config";
import { MODELS, fetchUnreadCount } from "@/core/models";
import { useConnectivity } from "@/data/connectivity";
import { useModel, useQueue, useEngine } from "@/data/sync-context";
import { liveTimerActivity } from "@/live/activity";
import { useLiveTimerEnabled } from "@/live/live-pref";
import { registerForPush, type PushState } from "@/push/push";
import {
  Button,
  Card,
  ListRow,
  Mono,
  Pill,
  Screen,
  SectionHeader,
  Toggle,
} from "@/ui/primitives";
import { shape, type, useTheme, type ThemeMode } from "@/ui/tokens";

const MODES: ThemeMode[] = ["system", "light", "dark"];

export default function MoreScreen() {
  const { tokens, mode, setMode } = useTheme();
  const { me, signOut, biometricAvailable, biometricEnabled, setBiometricEnabled } = useAuth();
  const { online, forcedOffline, setForcedOffline } = useConnectivity();
  const { pendingCount, rejected } = useQueue();
  const engine = useEngine();
  const unread = useModel(MODELS.unreadCount, fetchUnreadCount, { maxAgeMs: 60_000 });
  const [push, setPush] = useState<PushState | null>(null);
  const [pushBusy, setPushBusy] = useState(false);
  const [liveEnabled, setLiveEnabled] = useLiveTimerEnabled();
  const [liveSupported, setLiveSupported] = useState<boolean | null>(null);

  useEffect(() => {
    void liveTimerActivity.isSupported().then(setLiveSupported);
  }, []);

  return (
    <Screen testID="more-screen">
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>More</Text>
      </View>

      <Card>
        <Text style={[styles.name, { color: tokens.text }]}>{me?.display_name ?? "—"}</Text>
        <Text style={[styles.email, { color: tokens.textMuted }]}>{me?.email ?? ""}</Text>
        <View style={styles.rowBetween}>
          <Pill
            pair={online ? tokens.success : tokens.warning}
            label={online ? "ONLINE" : "OFFLINE"}
          />
          <Mono muted>{me?.timezone !== undefined ? `Times in ${me.timezone}` : ""}</Mono>
        </View>
        <Button label="Sign out" variant="secondary" fullWidth testID="more-signout" onPress={() => void signOut()} />
      </Card>

      <SectionHeader title="Appearance" />
      <Card>
        <View style={styles.segmented}>
          {MODES.map((option) => {
            const selected = mode === option;
            return (
              <Button
                key={option}
                testID={`theme-${option}`}
                label={option === "system" ? "System" : option === "light" ? "Light" : "Dark"}
                variant={selected ? "primary" : "secondary"}
                onPress={() => setMode(option)}
              />
            );
          })}
        </View>
      </Card>

      <SectionHeader title="Sync" />
      <Card>
        <ListRow
          title="Pending changes"
          subtitle={
            pendingCount > 0
              ? `${pendingCount} queued on this device`
              : online
                ? "Everything is synced"
                : "None on this device — nothing is waiting to sync"
          }
          onPress={() => router.push("/dev/queue")}
        />
        {rejected.length > 0 ? (
          <ListRow
            title="Couldn't sync"
            subtitle={`${rejected.length} change${rejected.length === 1 ? "" : "s"} need review`}
            onPress={() => router.push("/dev/queue")}
          />
        ) : null}
        <View style={styles.rowBetween}>
          <Text style={[styles.rowLabel, { color: tokens.text }]}>Sync now</Text>
          <Button
            label="Sync"
            variant="secondary"
            testID="more-sync-now"
            disabled={pendingCount === 0}
            onPress={() => void engine?.catchUp()}
          />
        </View>
        {DEV_TOOLS_ENABLED ? (
          <View style={styles.rowBetween}>
            <View style={styles.rowText}>
              <Text style={[styles.rowLabel, { color: tokens.text }]}>Simulate offline (dev)</Text>
              <Text style={[styles.rowHint, { color: tokens.textMuted }]}>
                Forces the offline path so queued writes replay on toggle back.
              </Text>
            </View>
            <Toggle
              testID="more-airplane-toggle"
              value={forcedOffline}
              onValueChange={setForcedOffline}
              label="Simulate offline"
            />
          </View>
        ) : null}
      </Card>

      <SectionHeader title="Security" />
      <Card>
        <View style={styles.rowBetween}>
          <View style={styles.rowText}>
            <Text style={[styles.rowLabel, { color: tokens.text }]}>Biometric unlock</Text>
            <Text style={[styles.rowHint, { color: tokens.textMuted }]}>
              {biometricAvailable
                ? "Require Face ID (or Touch ID) to open your saved session."
                : "No enrolled biometrics on this device."}
            </Text>
          </View>
          <Toggle
            testID="more-biometric-toggle"
            value={biometricEnabled}
            disabled={!biometricAvailable}
            onValueChange={(value) => {
              void setBiometricEnabled(value);
            }}
            label="Biometric sign-in"
          />
        </View>
      </Card>

      <SectionHeader title="Notifications" />
      <Card>
        <ListRow
          title="Inbox"
          subtitle={
            unread.data !== undefined
              ? `${unread.data.count} unread`
              : "Open the notification center"
          }
          onPress={() => router.push("/notifications")}
        />
        <View style={styles.rowBetween}>
          <View style={styles.rowText}>
            <Text style={[styles.rowLabel, { color: tokens.text }]}>Push registration</Text>
            <Text style={[styles.rowHint, { color: tokens.textMuted }]}>
              {push?.detail ?? "Allow notifications on this device."}
            </Text>
          </View>
          <Button
            label={push?.status === "registered" ? "Registered" : "Enable"}
            variant="secondary"
            testID="more-push-register"
            loading={pushBusy}
            onPress={() => {
              setPushBusy(true);
              void registerForPush(EAS_PROJECT_ID)
                .then(setPush)
                .finally(() => setPushBusy(false));
            }}
          />
        </View>
      </Card>

      <SectionHeader title="Field" />
      <Card>
        <View style={styles.rowBetween}>
          <View style={styles.rowText}>
            <Text style={[styles.rowLabel, { color: tokens.text }]}>
              Live timer on the lock screen
            </Text>
            <Text style={[styles.rowHint, { color: tokens.textMuted }]}>
              {liveSupported === null
                ? "Checking this build…"
                : liveSupported
                  ? "Mirrors the running job timer so the phone, the lock screen, and the desk all show one clock."
                  : "This build has no live-timer module — the in-app timer bar still works."}
            </Text>
          </View>
          <Toggle
            value={liveEnabled}
            onValueChange={setLiveEnabled}
            disabled={liveSupported !== true}
            label="Live timer"
          />
        </View>
        <ListRow
          title="Live timer status"
          subtitle="Support, push-token leg, and a manual start/stop"
          onPress={() => router.push("/dev/live")}
        />
      </Card>

      {DEV_TOOLS_ENABLED ? <SectionHeader title="Dev tools" /> : null}
      {DEV_TOOLS_ENABLED ? (
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
        <ListRow
          title="Scan injector"
          subtitle="Paste any raw scan in the Scan tab"
          onPress={() => router.push("/scan")}
        />
      </Card>
      ) : null}

      <SectionHeader title="About" />
      <Card>
        <Text style={[styles.rowHint, { color: tokens.textMuted }]}>
          Marula {APP_VERSION} · Metro 8181 · web preview 8182
        </Text>
        {DEV_TOOLS_ENABLED ? <Mono muted>API {API_URL}</Mono> : null}
        {DEV_TOOLS_ENABLED ? <Mono muted>WS {WS_URL}</Mono> : null}
        <Text
          style={[styles.rowHint, { color: tokens.primary }]}
          onPress={() => void Linking.openURL("https://marula.app/docs")}
        >
          Documentation
        </Text>
      </Card>
    </Screen>
  );
}

const styles = StyleSheet.create({
  header: {
    paddingTop: 8,
  },
  title: {
    fontFamily: type.sansBold,
    fontSize: 28,
    letterSpacing: -0.5,
  },
  name: {
    fontFamily: type.sansSemiBold,
    fontSize: 18,
  },
  email: {
    fontFamily: type.sans,
    fontSize: 14,
  },
  segmented: {
    flexDirection: "row",
    gap: 8,
  },
  rowBetween: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    minHeight: shape.rowHeight,
  },
  rowText: {
    flex: 1,
    gap: 2,
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
});
