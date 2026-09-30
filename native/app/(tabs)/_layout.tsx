// Tab shell — Today, Board, Scan, Time, More; offline banner and timer pill.
//
// Purpose: the app's home frame, drawn to the pack's chrome: a 90px tab bar
// with a raised 60px scan button and a gold tick over the active tab, the
// Board tab carrying the queue badge, and the 56px ink timer pill above it
// with Pause and a Stop that opens the log sheet. The offline/sync banner is
// part of the shell so every tab shows queue state. Tab switches are instant
// (no slide) per the motion law.
//
// Non-purpose: no screen content and no navigation state.

import Ionicons from "@expo/vector-icons/Ionicons";
import { Redirect, Tabs } from "expo-router";
import { useMemo, useState } from "react";
import { StyleSheet, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { useAuth } from "@/auth/auth-context";
import { MODELS, fetchMyWork } from "@/core/models";

import { useConnectivity } from "@/data/connectivity";
import { useModel } from "@/data/sync-context";
import { BootScreen } from "@/ui/boot";
import { OfflineBanner } from "@/ui/offline-banner";
import { ShellChromeContext } from "@/ui/shell-chrome";
import { ShellTopContext } from "@/ui/primitives";
import { TimePill } from "@/ui/time-pill";
import { pack } from "@/ui/pack";
import { chrome, type, useTheme } from "@/ui/tokens";



export default function TabsLayout() {
  const { status } = useAuth();
  const { tokens, mode } = useTheme();
  const insets = useSafeAreaInsets();
  const { online } = useConnectivity();
  const work = useModel(MODELS.myWork, fetchMyWork, { maxAgeMs: 60_000 });
  // A pushed screen (a ticket) can take the pill and the tab bar away: the
  // pack replaces both with the ticket's own bottom action bar.
  const [chromeVisible, setChromeVisible] = useState(true);
  const shellChrome = useMemo(
    () => ({ visible: chromeVisible, setVisible: setChromeVisible }),
    [chromeVisible],
  );
  const chromeOn = chromeVisible;
  // The TOP band is the offline banner only. The timer pill used to live up
  // here and ride this inset; it is docked above the rail now, so a running
  // timer must NOT claim the top inset — the band would render nothing (a
  // gap) while the screens below still skipped their own inset, pushing
  // content under the Dynamic Island.
  const noticeBand = !online;


  if (status === "loading") return <BootScreen />;
  if (status === "locked") return <Redirect href="/locked" />;
  if (status === "mfa") return <Redirect href="/mfa" />;
  if (status === "signedOut") return <Redirect href="/login" />;

  const boardBadge = Array.isArray(work.data) && work.data.length > 0 ? work.data.length : null;

  return (
    <ShellTopContext.Provider value={noticeBand}>
      <ShellChromeContext.Provider value={shellChrome}>
      <View style={[styles.shell, { backgroundColor: tokens.background }]}>
        {chromeOn && noticeBand ? (
          <View style={{ paddingTop: insets.top }}>
            <OfflineBanner />
          </View>
        ) : null}
        <Tabs
          screenOptions={{
            headerShown: false,
            animation: "none",
            // The gold tick: active tint is gold in both modes (the pack's
            // tabact), labels and icons otherwise sit in the muted ink.
            tabBarActiveTintColor: mode === "dark" ? chrome.gold : chrome.goldText,
            tabBarInactiveTintColor: tokens.textSecondary,
            tabBarStyle: [
              styles.tabBar,
              {
                backgroundColor: mode === "dark" ? chrome.navDark : tokens.surface,
                borderTopColor: tokens.border,
                height: pack.tabBarHeight + insets.bottom,
                paddingBottom: insets.bottom,
                // The ticket's action bar replaces the rail while it is open.
                display: chromeOn ? "flex" : "none",
              },
            ],
            tabBarLabelStyle: {
              fontFamily: type.sansSemiBold,
              fontSize: 12,
            },
            sceneStyle: { backgroundColor: tokens.background },
          }}
        >
          <Tabs.Screen
            name="index"
            options={{
              title: "Today",
              tabBarIcon: ({ color }) => <Ionicons name="today-outline" color={color} size={24} />,
            }}
          />
          <Tabs.Screen
            name="board"
            options={{
              title: "Board",
              tabBarBadge: boardBadge ?? undefined,
              tabBarBadgeStyle: [
                styles.badge,
                {
                  backgroundColor: mode === "dark" ? chrome.gold : chrome.ink,
                  color: mode === "dark" ? chrome.ink : chrome.gold,
                },
              ],
              tabBarIcon: ({ color }) => <Ionicons name="grid-outline" color={color} size={24} />,
            }}
          />
          <Tabs.Screen
            name="scan"
            options={{
              title: "",
              tabBarIcon: ({ focused }) => (
                <View
                  style={[
                    styles.scanButton,
                    {
                      backgroundColor: mode === "dark" ? chrome.gold : chrome.ink,
                      borderColor: mode === "dark" ? chrome.navDark : tokens.surface,
                    },
                    focused ? null : null,
                  ]}
                >
                  <Ionicons
                    name="scan-outline"
                    size={26}
                    color={mode === "dark" ? chrome.ink : chrome.gold}
                  />
                </View>
              ),
            }}
          />
          <Tabs.Screen
            name="time"
            options={{
              title: "Time",
              tabBarIcon: ({ color }) => <Ionicons name="time-outline" color={color} size={24} />,
            }}
          />
          <Tabs.Screen
            name="more"
            options={{
              title: "More",
              tabBarIcon: ({ color }) => (
                <Ionicons name="ellipsis-horizontal" color={color} size={24} />
              ),
            }}
          />
        </Tabs>
        {/* The pack pins the timer pill to the BOTTOM, floating just above the
            90px rail — a ticket takes it away with the action bar. */}
        {chromeOn ? (
          <View style={[styles.pillDock, { bottom: pack.tabBarHeight + insets.bottom }]}>
            <TimePill />
          </View>
        ) : null}
      </View>
    </ShellChromeContext.Provider>
    </ShellTopContext.Provider>
  );
}

const styles = StyleSheet.create({
  shell: {
    flex: 1,
  },
  tabBar: {
    height: pack.tabBarHeight + 34,
    paddingTop: 8,
  },
  pillDock: {
    position: "absolute",
    left: 0,
    right: 0,
  },
  badge: {
    fontFamily: type.sansBold,
    fontSize: 12,
    minWidth: 18,
    height: 18,
  },
  scanButton: {
    width: pack.scanButtonSize,
    height: pack.scanButtonSize,
    borderRadius: 999,
    borderWidth: 4,
    alignItems: "center",
    justifyContent: "center",
    transform: [{ translateY: -18 }],
  },
});

