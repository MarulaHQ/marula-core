// Today — the technician's home: today's bookings, my work, quick actions.
//
// Purpose: hydrate from two read models (dispatch day + my work) and live
// through WebSocket events: a rescheduled booking or an updated ticket
// lands here without a pull-to-refresh. Foundation scope — lane 3 grows the
// full day surface on top of this shell.

import Ionicons from "@expo/vector-icons/Ionicons";
import { Link, router } from "expo-router";
import { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";

import { useAuth } from "@/auth/auth-context";
import { DayPackCard } from "@/fieldops/day-pack-card";
import {
  MODELS,
  fetchDispatchDay,
  fetchMyWork,
  type AppointmentRow,
  type MyWorkItem,
} from "@/core/models";
import { useModel } from "@/data/sync-context";
import { slaState, type SlaState } from "@/ui/sla";
import { useNow } from "@/ui/use-now";
import {
  Card,
  EmptyState,
  ErrorNote,
  ListRow,
  Loading,
  Pill,
  SectionHeader,
  Screen,
} from "@/ui/primitives";
import { type, useTheme } from "@/ui/tokens";

export default function TodayScreen() {
  const { tokens } = useTheme();
  const { me } = useAuth();
  const now = useNow(30_000);
  const today = new Date(now).toISOString().slice(0, 10);
  const work = useModel(MODELS.myWork, fetchMyWork, { maxAgeMs: 30_000 });
  const day = useModel(MODELS.dispatchDay(today), (client) => fetchDispatchDay(client, today), {
    maxAgeMs: 60_000,
  });

  const myAppointments: AppointmentRow[] = useMemo(() => {
    if (day.data === undefined || me === null) return [];
    const mine = day.data.technicians.find((tech) => tech.user_id === me.user_id);
    const rows = mine?.appointments ?? [];
    return [...rows].sort((a, b) => a.starts_at.localeCompare(b.starts_at));
  }, [day.data, me]);

  const next = myAppointments.find(
    (appointment) => Date.parse(appointment.ends_at) >= now,
  );

  return (
    <Screen testID="today-screen">
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>Today</Text>
        <Text style={[styles.subtitle, { color: tokens.textMuted }]}>
          {me?.display_name ?? ""}
          {day.data !== undefined && Array.isArray(day.data.unassigned) && day.data.unassigned.length > 0
            ? ` · ${day.data.unassigned.length} unassigned`
            : ""}
        </Text>
      </View>

      {next !== undefined ? (
        <Card testID={`appointment-${next.id}`}>
          <View style={styles.cardHeader}>
            <Text style={[styles.client, { color: tokens.text }]} numberOfLines={1}>
              {next.client_name ?? "Booking"}
            </Text>
            <Pill
              pair={
                slaState(next.ends_at, now)?.severity === "overdue"
                  ? tokens.danger
                  : tokens.warning
              }
              label="DUE TODAY"
            />
          </View>
          <Text style={[styles.body, { color: tokens.textSecondary }]} numberOfLines={2}>
            {next.title}
          </Text>
          <View style={styles.metaRow}>
            <Text style={[styles.metaLabel, { color: tokens.textMuted }]}>ARRIVAL WINDOW</Text>
            <Text style={[styles.window, { color: tokens.text }]}>
              {formatWindow(next.starts_at, next.ends_at)}
            </Text>
          </View>
          {next.ticket_id !== null ? (
            <Link href={`/ticket/${next.ticket_id}`} asChild>
              <Text style={[styles.ticketLink, { color: tokens.primary }]}>
                {next.ticket_number ?? "Open ticket"} →
              </Text>
            </Link>
          ) : null}
        </Card>
      ) : (
        <EmptyState
          message="No bookings on today's board."
          actionLabel="Open the board"
          onAction={() => router.push("/board")}
        />
      )}

      <View style={styles.actionGrid}>
        <Action label="Scan" icon="scan-outline" href="/scan" />
        <Action label="Move stock" icon="swap-horizontal-outline" href="/move" />
        <Action label="Tools" icon="construct-outline" href="/tools" />
        <Action label="Stock" icon="cube-outline" href="/stock" />
        <Action label="Board" icon="grid-outline" href="/board" />
        <Action label="Inbox" icon="notifications-outline" href="/notifications" />
      </View>

      <DayPackCard />

      <SectionHeader title="My work" />
      {work.status === "error" && work.data === undefined ? (
        <ErrorNote text={work.error?.text ?? "Couldn't load your work."} onRetry={work.refresh} />
      ) : null}
      {work.status === "loading" && work.data === undefined ? <Loading label="Loading work" /> : null}
      {work.data !== undefined && work.data.length === 0 ? (
        <EmptyState message="Nothing assigned to you right now." />
      ) : null}
      <Card style={work.data !== undefined ? styles.listCard : undefined}>
        {(work.data ?? []).slice(0, 20).map((item: MyWorkItem) => (
          <ListRow
            key={item.id}
            testID={`mywork-${item.id}`}
            title={`${item.number} · ${item.title}`}
            subtitle={[item.client_name, item.status_name].filter(Boolean).join(" · ")}
            onPress={() => router.push(`/ticket/${item.id}`)}
            trailing={slaPill(tokens, slaState(item.sla_resolution_due, now))}
          />
        ))}
      </Card>
      {work.offline && work.data !== undefined ? (
        <Text style={[styles.offlineNote, { color: tokens.textMuted }]}>
          Showing the last sync from this device.
        </Text>
      ) : null}
    </Screen>
  );
}

function Action({
  label,
  icon,
  href,
}: {
  label: string;
  icon: keyof typeof Ionicons.glyphMap;
  href: string;
}) {
  const { tokens } = useTheme();
  return (
    <Card style={styles.action} onPress={() => router.push(href as never)} accessibilityLabel={label}>
      <Ionicons name={icon} size={22} color={tokens.textSecondary} />
      <Text style={[styles.actionLabel, { color: tokens.text }]}>{label}</Text>
    </Card>
  );
}

function slaPill(
  tokens: ReturnType<typeof useTheme>["tokens"],
  state: SlaState | undefined,
) {
  if (state === undefined) return undefined;
  return (
    <Pill
      pair={state.severity === "overdue" ? tokens.danger : tokens.warning}
      label={state.label}
    />
  );
}

function formatWindow(startsAt: string, endsAt: string): string {
  const start = new Date(startsAt);
  const end = new Date(endsAt);
  const format = (date: Date) =>
    `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
  return `${format(start)}–${format(end)}`;
}

const styles = StyleSheet.create({
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
  cardHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
  },
  client: {
    fontFamily: type.sansSemiBold,
    fontSize: 18,
    flex: 1,
  },
  body: {
    fontFamily: type.sans,
    fontSize: 14,
  },
  metaRow: {
    gap: 2,
  },
  metaLabel: {
    fontFamily: type.sansMedium,
    fontSize: 11,
    letterSpacing: 0.8,
  },
  window: {
    fontFamily: type.mono,
    fontSize: 18,
    fontVariant: ["tabular-nums"],
  },
  ticketLink: {
    fontFamily: type.sansSemiBold,
    fontSize: 14,
  },
  actionGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 12,
  },
  action: {
    flexBasis: "47%",
    flexGrow: 1,
    minHeight: 72,
    alignItems: "flex-start",
    justifyContent: "center",
    gap: 6,
  },
  actionLabel: {
    fontFamily: type.sansMedium,
    fontSize: 14,
  },
  listCard: {
    padding: 0,
    paddingHorizontal: 16,
  },
  offlineNote: {
    fontFamily: type.sans,
    fontSize: 12,
    textAlign: "center",
  },
});
