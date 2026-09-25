// Deep-link receiver for meridian://scan-result?text=…
//
// Purpose: the DX-standard injector link. The dev menu (or a Maestro flow)
// opens this URL with any raw scan string; the screen resolves it through
// the same api the camera leg will use and renders the typed result with an
// explicit Open action.

import { router, useLocalSearchParams } from "expo-router";
import { useEffect } from "react";
import { StyleSheet, Text, View } from "react-native";

import { ScanResolutionCard, useScanResolver } from "@/scan/scan-flow";
import { RequireAuth } from "@/ui/require-auth";
import { Button, Screen, SectionHeader } from "@/ui/primitives";
import { type, useTheme } from "@/ui/tokens";

export default function ScanResultRoute() {
  const { text } = useLocalSearchParams<{ text?: string }>();
  const { tokens } = useTheme();
  const { resolution, error, busy, resolve, clear } = useScanResolver();

  useEffect(() => {
    if (text !== undefined && text !== "") void resolve(text);
  }, [text, resolve]);

  return (
    <RequireAuth>
      <Screen testID="scan-result-screen">
        <View style={styles.header}>
          <Text style={[styles.title, { color: tokens.text }]}>Scan result</Text>
          {busy ? (
            <Text style={[styles.subtitle, { color: tokens.textMuted }]}>Resolving…</Text>
          ) : null}
        </View>
        {error !== null ? (
          <Text style={[styles.subtitle, { color: tokens.danger.text }]}>{error}</Text>
        ) : null}
        {resolution !== null ? (
          <ScanResolutionCard resolution={resolution} onClear={clear} />
        ) : null}
        <SectionHeader title="" />
        <Button
          label="Open the Scan tab"
          variant="secondary"
          fullWidth
          testID="scan-result-open-tab"
          onPress={() => router.replace("/scan")}
        />
      </Screen>
    </RequireAuth>
  );
}

const styles = StyleSheet.create({
  header: {
    paddingTop: 16,
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
});
