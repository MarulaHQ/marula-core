// Root layout — fonts, providers, and the one stack every route hangs off.
//
// Purpose: boot the real Inter / JetBrains Mono faces, then nest the
// providers in dependency order (theme → connectivity → auth → sync) so
// every screen below can read tokens, network state, the session, and the
// live-sync engine. Auth branching lives in the route groups (index, tabs,
// RequireAuth) rather than here, so deep links resolve before redirects.
//
// Non-purpose: no feature UI.

import {
  Inter_400Regular,
  Inter_500Medium,
  Inter_600SemiBold,
  Inter_700Bold,
} from "@expo-google-fonts/inter";
import {
  JetBrainsMono_400Regular,
  JetBrainsMono_500Medium,
} from "@expo-google-fonts/jetbrains-mono";
import { useFonts } from "expo-font";
import { Stack } from "expo-router";
import * as SplashScreen from "expo-splash-screen";
import { StatusBar } from "expo-status-bar";
import { useCallback, useEffect, useState } from "react";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { SafeAreaProvider } from "react-native-safe-area-context";

import { AuthProvider } from "@/auth/auth-context";
import { ConnectivityProvider } from "@/data/connectivity";
import { SyncProvider } from "@/data/sync-context";
import { asyncStore } from "@/data/storage";
import {
  configureForegroundPresentation,
  useNotificationObserver,
} from "@/push/push";
import { ThemeProvider, darkTokens, useTheme, type ThemeMode } from "@/ui/tokens";

void SplashScreen.preventAutoHideAsync().catch(() => undefined);
configureForegroundPresentation();

const prefs = asyncStore("prefs");

export default function RootLayout() {
  const [fontsLoaded, fontError] = useFonts({
    Inter_400Regular,
    Inter_500Medium,
    Inter_600SemiBold,
    Inter_700Bold,
    JetBrainsMono_400Regular,
    JetBrainsMono_500Medium,
  });
  const [themeMode, setThemeMode] = useState<ThemeMode>("system");

  useEffect(() => {
    void prefs.get("theme.mode").then((stored) => {
      if (stored === "light" || stored === "dark" || stored === "system") {
        setThemeMode(stored);
      }
    });
  }, []);

  const changeMode = useCallback((mode: ThemeMode) => {
    setThemeMode(mode);
    void prefs.set("theme.mode", mode);
  }, []);

  useEffect(() => {
    if (fontsLoaded || fontError !== null) {
      void SplashScreen.hideAsync().catch(() => undefined);
    }
  }, [fontsLoaded, fontError]);

  if (!fontsLoaded && fontError === null) return null;

  return (
    <GestureHandlerRootView style={{ flex: 1 }}>
      <SafeAreaProvider>
        <ThemeProvider mode={themeMode} onModeChange={changeMode}>
          <ConnectivityProvider>
            <AuthProvider>
              <SyncProvider>
                <ThemedStack />
              </SyncProvider>
            </AuthProvider>
          </ConnectivityProvider>
        </ThemeProvider>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}

function ThemedStack() {
  const { tokens } = useTheme();
  useNotificationObserver();
  return (
    <>
      <StatusBar style={tokens.mode === "dark" ? "light" : "dark"} />
      <Stack
        screenOptions={{
          headerShown: false,
          contentStyle: { backgroundColor: tokens.background },
          animation: "none",
        }}
      >
        <Stack.Screen name="(tabs)" />
        <Stack.Screen name="login" />
        <Stack.Screen name="mfa" />
        <Stack.Screen name="locked" options={{ animation: "fade" }} />
        <Stack.Screen name="notifications" />
        <Stack.Screen name="ticket/[id]" />
        <Stack.Screen name="stock-item/[id]" />
        <Stack.Screen name="stock-serial/[id]" />
        <Stack.Screen name="stock-location/[id]" />
        <Stack.Screen name="asset/[id]" />
        <Stack.Screen name="tools" />
        <Stack.Screen name="tool-kit/[id]" />
        <Stack.Screen name="stock" />
        <Stack.Screen name="move" />
        <Stack.Screen name="scan-result" options={{ animation: "fade" }} />
        <Stack.Screen name="dev/queue" />
        <Stack.Screen name="dev/live" />
      </Stack>
    </>
  );
}

/** Dark-mode reference for tests/tools that need the raw tokens. */
export { darkTokens };
