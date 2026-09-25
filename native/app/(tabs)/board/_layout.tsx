// Board tab layout — two-pane at regular width, push at compact.
//
// Purpose: the adaptive seam, live from day one. At regular width (tablet,
// Stage Manager, landscape) the master list and the detail render side by
// side and selecting a row swaps the detail without navigation; at compact
// width the same routes push normally. Both classes share the same route
// tree, so deep links and push routing behave identically.

import { Stack, router, useLocalSearchParams } from "expo-router";
import { StyleSheet, View } from "react-native";

import { TicketList } from "@/tickets/ticket-list";
import { useAdaptiveLayout } from "@/ui/adaptive";
import { EmptyState } from "@/ui/primitives";
import { useTheme } from "@/ui/tokens";

export default function BoardLayout() {
  const { tokens } = useTheme();
  const { isRegular } = useAdaptiveLayout();
  const params = useLocalSearchParams<{ id?: string }>();

  return (
    <View style={[styles.shell, { backgroundColor: tokens.background }]}>
      {isRegular ? (
        <View
          style={[
            styles.master,
            { borderRightColor: tokens.border, backgroundColor: tokens.background },
          ]}
        >
          <View style={styles.listPadding}>
            <TicketList
              selectedId={params.id}
              onSelect={(id) => router.push(`/(tabs)/board/ticket/${id}`)}
            />
          </View>
        </View>
      ) : null}
      <View style={styles.detail}>
        <Stack
          screenOptions={{
            headerShown: false,
            animation: isRegular ? "none" : "default",
            contentStyle: { backgroundColor: tokens.background },
          }}
        />
      </View>
    </View>
  );
}

/** Placeholder pane shown in the detail area before a row is selected. */
export function BoardPlaceholder() {
  return (
    <View style={styles.placeholder}>
      <EmptyState message="Select a ticket to open it here." />
    </View>
  );
}

const styles = StyleSheet.create({
  shell: {
    flex: 1,
    flexDirection: "row",
  },
  master: {
    width: 380,
    borderRightWidth: 1,
    padding: 16,
  },
  listPadding: {
    paddingTop: 8,
  },
  detail: {
    flex: 1,
  },
  placeholder: {
    flex: 1,
    padding: 16,
    paddingTop: 24,
  },
});
