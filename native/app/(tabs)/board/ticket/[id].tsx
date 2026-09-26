// Board ticket route — renders inside the board stack (two-pane detail).

import { useLocalSearchParams } from "expo-router";
import { ScrollView, StyleSheet } from "react-native";

import { useTopInset } from "@/ui/primitives";

import { TicketDetail } from "@/tickets/ticket-detail";
import { useTheme } from "@/ui/tokens";

export default function BoardTicket() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { tokens } = useTheme();
  const topInset = useTopInset();
  if (id === undefined) return null;
  return (
    <ScrollView
      style={{ backgroundColor: tokens.background }}
      contentContainerStyle={[styles.content, { paddingTop: topInset }]}
      keyboardShouldPersistTaps="handled"
    >
      <TicketDetail id={id} />
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  content: {
    flexGrow: 1,
  },
});
