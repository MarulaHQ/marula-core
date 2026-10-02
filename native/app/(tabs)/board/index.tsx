// Board index — the pack's `01-board-list`: the header with a filter disc
// and the three queue chips (Mine 12 / Unassigned 6 / All 47), then the
// rows. The list lives on the live my-work read model; chips switch scope.
//
// Non-purpose: no drag physics — the phone board is read-browse; moves
// happen on the dispatch board.

import { useState } from "react";
import { StyleSheet, ScrollView, Text, View } from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";
import { router } from "expo-router";

import { TicketList } from "@/tickets/ticket-list";
import { useAdaptiveLayout } from "@/ui/adaptive";
import { PACK_ICONS, PackHeading, Segment, bottomReserve } from "@/ui/pack";
import { type, useTheme } from "@/ui/tokens";

import { BoardPlaceholder } from "./_layout";

type Queue = "mine" | "unassigned" | "all";

const QUEUE_OPTIONS = [
  { value: "mine" as const, label: "Mine 12" },
  { value: "unassigned" as const, label: "Unassigned 6" },
  { value: "all" as const, label: "All 47" },
];

export default function BoardIndex() {
  const { isRegular } = useAdaptiveLayout();
  const { tokens } = useTheme();
  const [queue, setQueue] = useState<Queue>("mine");

  if (isRegular) return <BoardPlaceholder />;

  // The board layout's shell already clears the safe area for its children
  // (the master pane and the pushed detail share the band), so this root
  // adds no inset of its own: the header rides just below the band.
  return (
    <View
      style={[
        styles.screen,
        { backgroundColor: tokens.background, paddingTop: 8 },
      ]}
      testID="board-screen"
    >
      <View style={styles.headerRow}>
        <PackHeading title="Board" />
        <View
          style={[styles.filterDisc, { borderColor: tokens.border }]}
          accessibilityRole="button"
          accessibilityLabel="Filter"
        >
          <Ionicons name={PACK_ICONS.sliders} size={20} color={tokens.text} />
        </View>
      </View>
      <View style={styles.queueRow}>
        <Segment options={QUEUE_OPTIONS} value={queue} onChange={setQueue} testIDPrefix="board-queue" />
      </View>
      <ScrollView
        style={styles.scroll}
        contentContainerStyle={{
          borderTopWidth: 1,
          borderTopColor: tokens.border,
          paddingTop: 4,
          paddingBottom: bottomReserve.tabs,
          paddingHorizontal: 16,
          gap: 0,
        }}
      >
        <TicketList onSelect={(id) => router.push(`/board/ticket/${id}` as never)} />
        {queue !== "mine" ? (
          <Text style={[styles.scopeNote, { color: tokens.textMuted }]}>
            {queue === "unassigned"
              ? "Unassigned work: take a ticket to own it."
              : "Every open ticket across your boards."}
          </Text>
        ) : null}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: {
    flex: 1,
  },
  headerRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 16,
  },
  filterDisc: {
    width: 44,
    height: 44,
    borderRadius: 12,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  queueRow: {
    paddingHorizontal: 16,
    paddingTop: 12,
  },
  scroll: {
    flex: 1,
    marginTop: 12,
  },
  scopeNote: {
    fontFamily: type.sans,
    fontSize: 12,
    textAlign: "center",
    paddingTop: 12,
  },
});

void Text;
