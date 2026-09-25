// Boot / brand screen — shown while the session restores.

import { ActivityIndicator, StyleSheet, Text, View } from "react-native";

import { type, useTheme } from "./tokens";

export function BootScreen() {
  const { tokens } = useTheme();
  return (
    <View style={[styles.screen, { backgroundColor: tokens.background }]}>
      <Text style={[styles.wordmark, { color: tokens.text }]}>Meridian</Text>
      <ActivityIndicator color={tokens.primary} />
    </View>
  );
}

const styles = StyleSheet.create({
  screen: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    gap: 20,
  },
  wordmark: {
    fontFamily: type.sansSemiBold,
    fontSize: 30,
    letterSpacing: -0.5,
  },
});
