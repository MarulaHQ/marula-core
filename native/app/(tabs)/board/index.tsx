// Board index — the list at compact width, a placeholder at regular width
// (where the master pane already shows the list).

import { router } from "expo-router";
import { StyleSheet, Text, View } from "react-native";

import { TicketList } from "@/tickets/ticket-list";
import { useTopInset } from "@/ui/primitives";
import { useAdaptiveLayout } from "@/ui/adaptive";
import { type, useTheme } from "@/ui/tokens";

import { BoardPlaceholder } from "./_layout";

export default function BoardIndex() {
  const { isRegular } = useAdaptiveLayout();
  const { tokens } = useTheme();
  const topInset = useTopInset();

  if (isRegular) return <BoardPlaceholder />;

  return (
    <View style={[styles.screen, { backgroundColor: tokens.background, paddingTop: topInset }]}>
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>Board</Text>
        <Text style={[styles.subtitle, { color: tokens.textMuted }]}>Your assigned work</Text>
      </View>
      <TicketList onSelect={(id) => router.push(`/board/ticket/${id}`)} />
    </View>
  );
}

const styles = StyleSheet.create({
  screen: {
    flex: 1,
    padding: 16,
    gap: 12,
  },
  header: {
    paddingTop: 8,
    gap: 2,
  },
  title: {
    fontFamily: type.sansBold,
    fontSize: 28,
    letterSpacing: -0.5,
  },
  subtitle: {
    fontFamily: type.sans,
    fontSize: 14,
  },
});
