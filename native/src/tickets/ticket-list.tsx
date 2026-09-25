// Ticket list — the Board tab's master pane and the compact list screen.

import { StyleSheet, Text, View } from "react-native";

import { MODELS, fetchMyWork, type MyWorkItem } from "@/core/models";
import { useModel } from "@/data/sync-context";
import { slaState, type SlaState } from "@/ui/sla";
import { useNow } from "@/ui/use-now";
import { Card, EmptyState, ErrorNote, ListRow, Loading, Pill } from "@/ui/primitives";
import { type, useTheme, type ThemeTokens } from "@/ui/tokens";

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
    <Card style={styles.card} testID="ticket-list">
      {rows.map((item: MyWorkItem) => {
        const selected = item.id === selectedId;
        return (
          <View
            key={item.id}
            style={selected ? { backgroundColor: tokens.surfaceHover, borderRadius: 8 } : undefined}
          >
            <ListRow
              testID={`ticket-row-${item.id}`}
              title={`${item.number} · ${item.title}`}
              subtitle={[item.client_name, item.status_name].filter(Boolean).join(" · ")}
              onPress={() => onSelect(item.id)}
              trailing={<SlaPill tokens={tokens} iso={item.sla_resolution_due ?? null} now={now} />}
            />
          </View>
        );
      })}
      {work.offline ? (
        <Text style={[styles.offlineNote, { color: tokens.textMuted }]}>
          Offline — showing the last sync from this device.
        </Text>
      ) : null}
    </Card>
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
    <Pill
      pair={state.severity === "overdue" ? tokens.danger : tokens.warning}
      label={state.label}
    />
  );
}

const styles = StyleSheet.create({
  card: {
    padding: 0,
    paddingHorizontal: 16,
  },
  offlineNote: {
    fontFamily: type.sans,
    fontSize: 12,
    textAlign: "center",
    paddingVertical: 8,
  },
});
