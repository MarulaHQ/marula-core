// Board index — the pack's `01-board-list`: the header with a filter disc
// and the three queue chips (Mine 12 / Unassigned 6 / All 47), then the
// rows. Chips carry the REAL counts — Mine from my-work, Unassigned from
// the dispatch queue, All from the tickets list — and the scope switch
// filters the list for real (a scope that only relabels the same rows is a
// lie). The filter disc is wired to the same scope chips: the phone board
// has exactly these three scopes, so the disc opens the scope row.
//
// Non-purpose: no drag physics — the phone board is read-browse; moves
// happen on the dispatch board.

import { useMemo, useState } from "react";
import { Pressable, StyleSheet, ScrollView, Text, View } from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";
import { router } from "expo-router";

import {
  MODELS,
  fetchDispatchQueue,
  fetchMyWork,
  fetchTicketsList,
  type MyWorkItem,
  type TicketsListRow,
} from "@/core/models";
import { TicketList } from "@/tickets/ticket-list";
import { useAdaptiveLayout } from "@/ui/adaptive";
import { useModel } from "@/data/sync-context";
import { PACK_ICONS, PackHeading, Segment, bottomReserve } from "@/ui/pack";
import { type, useTheme } from "@/ui/tokens";

import { BoardPlaceholder } from "./_layout";

type Queue = "mine" | "unassigned" | "all";

/** A list row in the shape TicketList already renders. */
function queueItemOfTicket(row: TicketsListRow): MyWorkItem {
  return {
    board_id: row.board_id,
    board_name: "",
    category: row.category,
    client_name: null,
    due_at: row.due_at ?? null,
    id: row.id,
    number: row.number,
    priority: row.priority,
    sla_resolution_due: null,
    status_name: row.category,
    title: row.title,
    updated_at: row.updated_at,
  };
}

export default function BoardIndex() {
  const { isRegular } = useAdaptiveLayout();
  const { tokens } = useTheme();
  const [queue, setQueue] = useState<Queue>("mine");
  const [scopeOpen, setScopeOpen] = useState(false);

  const mine = useModel(MODELS.myWork, fetchMyWork, { maxAgeMs: 30_000 });
  const unassigned = useModel(MODELS.dispatchQueue, fetchDispatchQueue, { maxAgeMs: 60_000 });
  const all = useModel(MODELS.ticketsList, fetchTicketsList, { maxAgeMs: 60_000 });

  const counts = useMemo(
    () => ({
      mine: (mine.data ?? []).length,
      unassigned: (unassigned.data?.queue ?? []).length,
      all: (all.data ?? []).length,
    }),
    [mine.data, unassigned.data, all.data],
  );

  const rows: MyWorkItem[] = useMemo(() => {
    if (queue === "mine") return mine.data ?? [];
    if (queue === "unassigned") return unassigned.data?.queue ?? [];
    return (all.data ?? []).map(queueItemOfTicket);
  }, [queue, mine.data, unassigned.data, all.data]);

  const options = useMemo(
    () => [
      { value: "mine" as const, label: `Mine ${counts.mine}` },
      { value: "unassigned" as const, label: `Unassigned ${counts.unassigned}` },
      { value: "all" as const, label: `All ${counts.all}` },
    ],
    [counts],
  );

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
        <Pressable
          style={[styles.filterDisc, { borderColor: tokens.border }]}
          accessibilityRole="button"
          accessibilityLabel="Filter the queue"
          accessibilityState={{ selected: scopeOpen }}
          onPress={() => setScopeOpen((open) => !open)}
        >
          <Ionicons name={PACK_ICONS.sliders} size={20} color={tokens.text} />
        </Pressable>
      </View>
      {scopeOpen || queue !== "mine" ? (
        <View style={styles.queueRow} testID="board-scopes">
          <Segment options={options} value={queue} onChange={setQueue} testIDPrefix="board-queue" />
        </View>
      ) : null}
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
        {rows.length > 0 ? (
          <TicketList rows={rows} onSelect={(id) => router.push(`/board/ticket/${id}` as never)} />
        ) : (
          <Text style={[styles.scopeNote, { color: tokens.textMuted }]}>
            {queue === "mine"
              ? "Nothing assigned to you right now."
              : queue === "unassigned"
                ? "The dispatch queue is empty — every open ticket has an owner."
                : "No open tickets."}
          </Text>
        )}
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
    fontSize: 13,
    textAlign: "center",
    paddingTop: 12,
  },
});
