// Ticket list — the Board tab's master pane and the compact list screen.
//
// Purpose: the pack's `01-board-list` rows drawn on live data: a priority
// chip, a two-line title, ticket number and client, a status pill and, where
// there is one, an SLA pill — red only for a genuine breach, blush for due
// soon, and a gold row with a Timer running pill for the ticket the clock is
// on. Rows are at least 76px with 44px touch height.
//
// Non-purpose: no board mechanics (drag/assign) — the queue is read-only
// browse on the phone.

import { Pressable, StyleSheet, Text, View } from "react-native";

import { MODELS, fetchActiveTimer, fetchMyWork, type MyWorkItem } from "@/core/models";
import { useModel } from "@/data/sync-context";
import { slaState, type SlaState } from "@/ui/sla";
import { useNow } from "@/ui/use-now";
import { EmptyState, ErrorNote, Loading } from "@/ui/primitives";
import {
  PackPill,
  PriorityChip,
  bottomReserve,
  pack,
  slaPair,
  statusPairFor,
} from "@/ui/pack";
import { chrome } from "@/ui/tokens";
import { type, useTheme, type ThemeTokens } from "@/ui/tokens";
import Ionicons from "@expo/vector-icons/Ionicons";

export function TicketList({
  selectedId,
  onSelect,
}: {
  selectedId?: string;
  onSelect: (id: string) => void;
}) {
  const { tokens } = useTheme();
  const now = useNow(30_000);
  const work = useModel(MODELS.myWork, fetchMyWork, { maxAgeMs: 30_000 });
  // The pack marks the ticket the clock is ON: gold tint, a gold edge, and a
  // Timer running pill. Real state, not a fixture.
  const timer = useModel(MODELS.activeTimer, fetchActiveTimer);
  const runningId = timer.data?.ticket_id ?? null;

  if (work.status === "error" && work.data === undefined) {
    return <ErrorNote text={work.error?.text ?? "Couldn't load your work."} onRetry={work.refresh} />;
  }
  if (work.status === "loading" && work.data === undefined) {
    return <Loading label="Loading work" />;
  }
  const rows = work.data ?? [];
  if (rows.length === 0) {
    return <EmptyState message="Nothing assigned to you right now." />;
  }
  return (
    <View testID="ticket-list" style={styles.list}>
      {rows.map((item: MyWorkItem) => {
        const selected = item.id === selectedId;
        return (
          <Pressable
            key={item.id}
            testID={`ticket-row-${item.id}`}
            onPress={() => onSelect(item.id)}
            accessibilityRole="button"
            accessibilityState={{ selected }}
            accessibilityLabel={`${item.number} ${item.title}`}
            style={({ pressed }) => [
              styles.row,
              { borderBottomColor: tokens.border },
              item.id === runningId ? styles.rowRunning : null,
              item.id === runningId
                ? { backgroundColor: tokens.surfaceHover }
                : pressed && !selected
                  ? { backgroundColor: tokens.surfaceHover }
                  : null,
            ]}
          >
            {item.priority !== undefined && item.priority !== null ? (
              <PriorityChip label={String(item.priority)} />
            ) : null}
            <View style={styles.rowBody}>
              <Text style={[styles.rowTitle, { color: tokens.text }]} numberOfLines={2}>
                {item.title}
              </Text>
              <View style={styles.rowMeta}>
                <Text style={[styles.rowNumber, { color: tokens.textSecondary }]}>
                  {item.number}
                </Text>
                {item.client_name !== null && item.client_name !== undefined ? (
                  <Text style={[styles.rowClient, { color: tokens.textSecondary }]} numberOfLines={1}>
                    {item.client_name}
                  </Text>
                ) : null}
              </View>
              <View style={styles.rowPills}>
                {item.id === runningId ? (
                  <PackPill pair={tokens.neutral} label="Timer running" />
                ) : (
                  <PackPill pair={statusPairFor(tokens, item.category)} label={item.status_name} />
                )}
                <SlaPill tokens={tokens} iso={item.sla_resolution_due ?? null} now={now} />
              </View>
            </View>
            <Ionicons name="chevron-forward" size={18} color={tokens.textSecondary} />
          </Pressable>
        );
      })}
      {work.offline ? (
        <Text style={[styles.offlineNote, { color: tokens.textMuted }]}>
          Offline — showing the last sync from this device.
        </Text>
      ) : null}
    </View>
  );
}

function SlaPill({
  tokens,
  iso,
  now,
}: {
  tokens: ThemeTokens;
  iso: string | null;
  now: number;
}) {
  const state: SlaState | undefined = slaState(iso, now);
  if (state === undefined) return undefined;
  return (
    <PackPill
      pair={slaPair(tokens, state.severity === "overdue" ? "breach" : "due")}
      label={state.label}
      icon={state.severity === "overdue" ? "alert-circle" : "time-outline"}
    />
  );
}

const styles = StyleSheet.create({
  /** The pack's running row: a gold inset edge down the leading side. */
  rowRunning: {
    borderLeftWidth: 4,
    borderLeftColor: chrome.gold,
    paddingLeft: 8,
  },
  list: {
    gap: 0,
  },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
    minHeight: 76,
    paddingVertical: 12,
    paddingHorizontal: 4,
    borderBottomWidth: 1,
  },
  rowBody: {
    flex: 1,
    minWidth: 0,
    gap: 4,
  },
  rowTitle: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
    lineHeight: 20,
  },
  rowMeta: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  rowNumber: {
    fontFamily: type.mono,
    fontSize: 12,
    fontVariant: ["tabular-nums"],
  },
  rowClient: {
    fontFamily: type.sans,
    fontSize: 13,
    flexShrink: 1,
  },
  rowPills: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 6,
  },
  offlineNote: {
    fontFamily: type.sans,
    fontSize: 12,
    textAlign: "center",
    paddingVertical: 8,
  },
});

void pack;
void bottomReserve;
