// Root ticket route — the deep-link and push-notification destination.

import { router, useLocalSearchParams } from "expo-router";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import { TicketDetail } from "@/tickets/ticket-detail";
import { useTopInset } from "@/ui/primitives";
import { RequireAuth } from "@/ui/require-auth";
import { type, useTheme } from "@/ui/tokens";

export default function TicketRoute() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { tokens } = useTheme();
  const topInset = useTopInset();
  return (
    <RequireAuth>
      <View style={[styles.shell, { backgroundColor: tokens.background, paddingTop: topInset }]}>
        <View style={[styles.navBar, { borderBottomColor: tokens.border }]}>
          <Text
            testID="ticket-back"
            onPress={() => {
              if (router.canGoBack()) router.back();
              else router.replace("/board");
            }}
            style={[styles.back, { color: tokens.primary }]}
          >
            ‹ Back
          </Text>
          <Text style={[styles.navTitle, { color: tokens.textSecondary }]}>Ticket</Text>
          <View style={styles.spacer} />
        </View>
        <ScrollView keyboardShouldPersistTaps="handled">
          {id !== undefined ? <TicketDetail id={id} /> : null}
        </ScrollView>
      </View>
    </RequireAuth>
  );
}

const styles = StyleSheet.create({
  shell: {
    flex: 1,
  },
  navBar: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    minHeight: 48,
    paddingHorizontal: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  back: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
    minWidth: 64,
  },
  navTitle: {
    fontFamily: type.sansMedium,
    fontSize: 13,
    letterSpacing: 0.8,
    textTransform: "uppercase",
  },
  spacer: {
    minWidth: 64,
  },
});
