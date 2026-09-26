// Tab shell — Today, Board, Scan, More; offline banner and timer strip.
//
// Purpose: the app's home frame. The offline/sync banner is part of the
// shell (not a screen), so every tab shows queue state; the running-timer
// strip sits directly under it so a technician always sees the clock they
// are billing against. Tab switches are instant (no slide) per the motion
// law — tabs are peers, not a hierarchy.
//
// Non-purpose: no screen content and no navigation state.

import Ionicons from "@expo/vector-icons/Ionicons";
import { Redirect, Tabs } from "expo-router";
import { StyleSheet, View } from "react-native";

import { useAuth } from "@/auth/auth-context";
import { MODELS, fetchActiveTimer } from "@/core/models";
import { useConnectivity } from "@/data/connectivity";
import { useModel } from "@/data/sync-context";
import { BootScreen } from "@/ui/boot";
import { OfflineBanner } from "@/ui/offline-banner";
import { ShellTopContext } from "@/ui/primitives";
import { TimerBar } from "@/ui/timer-bar";
import { useTheme } from "@/ui/tokens";
import { useSafeAreaInsets } from "react-native-safe-area-context";

export default function TabsLayout() {
  const { status } = useAuth();
  const { tokens } = useTheme();
  const insets = useSafeAreaInsets();
  const { online } = useConnectivity();
  const timer = useModel(MODELS.activeTimer, fetchActiveTimer);
  // The notice chrome (offline banner, timer bar) is what can collide with
  // the status bar; when either is visible this shell clears the band and
  // the screens below skip their own top inset.
  const chrome = !online || (timer.data !== undefined && timer.data !== null);

  if (status === "loading") return <BootScreen />;
  if (status === "locked") return <Redirect href="/locked" />;
  if (status === "mfa") return <Redirect href="/mfa" />;
  if (status === "signedOut") return <Redirect href="/login" />;

  return (
    <ShellTopContext.Provider value={chrome}>
      <View style={[styles.shell, { backgroundColor: tokens.background }]}>
        <View style={chrome ? { paddingTop: insets.top } : undefined}>
          <OfflineBanner />
          <TimerBar />
        </View>
        <Tabs
        screenOptions={{
          headerShown: false,
          animation: "none",
          tabBarActiveTintColor: tokens.primary,
          tabBarInactiveTintColor: tokens.textMuted,
          tabBarStyle: {
            backgroundColor: tokens.surface,
            borderTopColor: tokens.border,
          },
          tabBarLabelStyle: {
            fontSize: 11,
            fontWeight: "600",
          },
        }}
      >
        <Tabs.Screen
          name="index"
          options={{
            title: "Today",
            tabBarIcon: ({ color, size }) => (
              <Ionicons name="calendar-outline" color={color} size={size} />
            ),
          }}
        />
        <Tabs.Screen
          name="board"
          options={{
            title: "Board",
            tabBarIcon: ({ color, size }) => (
              <Ionicons name="grid-outline" color={color} size={size} />
            ),
          }}
        />
        <Tabs.Screen
          name="scan"
          options={{
            title: "Scan",
            tabBarIcon: ({ color, size }) => (
              <Ionicons name="scan-outline" color={color} size={size} />
            ),
          }}
        />
        <Tabs.Screen
          name="more"
          options={{
            title: "More",
            tabBarIcon: ({ color, size }) => (
              <Ionicons name="ellipsis-horizontal" color={color} size={size} />
            ),
          }}
        />
      </Tabs>
      </View>
    </ShellTopContext.Provider>
  );
}

const styles = StyleSheet.create({
  shell: {
    flex: 1,
  },
});
