// Today — the technician's home: today's schedule, travel, on-call, work.
//
// Purpose: hydrate from the cached read models (dispatch day, my work,
// on-call, travel drafts) and live through WebSocket events: a rescheduled
// booking, a new unassigned ticket, or a rotation change lands here without
// a pull-to-refresh. Accepting a booking writes through the offline queue
// and renders before the server echo — the dispatcher's board sees the same
// state when the write lands.
//
// Non-purpose: no dispatch-board mechanics (the board tab owns browse), no
// rotation editing, no expense conversion.

import Ionicons from "@expo/vector-icons/Ionicons";
import { router } from "expo-router";
import { useCallback, useMemo, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { useAuth } from "@/auth/auth-context";
import { DayPackCard } from "@/fieldops/day-pack-card";
import {
  MODELS,
  ON_CALL_WINDOW_DAYS,
  fetchDispatchDay,
  fetchMyWork,
  fetchOnCallBlocks,
  fetchTravelDrafts,
  type AppointmentRow,
  type MyWorkItem,
  type OnCallBlock,
  type TravelDraft,
} from "@/core/models";
import {
  createTravelDraft,
  deleteTravelDraft,
  setAppointmentStatus,
  submitTravelDraft,
} from "@/core/writes";
import { OnCallStrip } from "@/day/on-call";
import { DaySchedule } from "@/day/schedule";
import {
  TravelDraftForm,
  TravelSection,
  type TravelDraftFormValues,
  type TravelTicketOption,
} from "@/day/travel";
import { useAdaptiveLayout } from "@/ui/adaptive";
import { useModel, useMutation } from "@/data/sync-context";
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
  const { mutate } = useMutation();
  const { isRegular } = useAdaptiveLayout();
  const now = useNow(30_000);
  const today = new Date(now).toISOString().slice(0, 10);
  const work = useModel(MODELS.myWork, fetchMyWork, { maxAgeMs: 30_000 });
  const day = useModel(MODELS.dispatchDay(today), (client) => fetchDispatchDay(client, today), {
    maxAgeMs: 60_000,
  });
  const onCall = useModel(
    MODELS.onCall(today),
    useCallback((client) => {
      const from = new Date(`${today}T00:00:00`);
      const to = new Date(from.getTime() + ON_CALL_WINDOW_DAYS * 24 * 60 * 60 * 1000);
      return fetchOnCallBlocks(client, from, to);
    }, [today]),
    { maxAgeMs: 5 * 60_000 },
  );
  const travel = useModel(MODELS.travelDrafts, fetchTravelDrafts, { maxAgeMs: 60_000 });

  const [busyAppointment, setBusyAppointment] = useState<string | null>(null);
  const [busyDraft, setBusyDraft] = useState<string | null>(null);
  const [travelOpen, setTravelOpen] = useState(false);
  const [travelSaving, setTravelSaving] = useState(false);
  const [banner, setBanner] = useState<string | null>(null);

  const myAppointments: AppointmentRow[] = useMemo(() => {
    if (day.data === undefined || me === null) return [];
    const mine = day.data.technicians.find((tech) => tech.user_id === me.user_id);
    const rows = mine?.appointments ?? [];
    return [...rows].sort((a, b) => a.starts_at.localeCompare(b.starts_at));
  }, [day.data, me]);

  const myOnCall: OnCallBlock[] = useMemo(() => {
    if (onCall.data === undefined || me === null) return [];
    return onCall.data
      .filter((block) => block.user_id === me.user_id)
      .sort((a, b) => a.starts_at.localeCompare(b.starts_at));
  }, [onCall.data, me]);

  const myDrafts: TravelDraft[] = travel.data ?? [];

  const ticketOptions: TravelTicketOption[] = useMemo(() => {
    const options = new Map<string, TravelTicketOption>();
    for (const appointment of myAppointments) {
      if (appointment.ticket_id != null && appointment.ticket_number != null) {
        options.set(appointment.ticket_id, {
          id: appointment.ticket_id,
          label: `${appointment.ticket_number} · ${appointment.client_name ?? appointment.title}`,
        });
      }
    }
    for (const item of work.data ?? []) {
      if (item.id !== undefined && !options.has(item.id)) {
        options.set(item.id, { id: item.id, label: `${item.number} · ${item.title}` });
      }
    }
    return [...options.values()].slice(0, 6);
  }, [myAppointments, work.data]);

  const act = useCallback(
    async (appointment: AppointmentRow, status: "in_progress" | "done") => {
      setBusyAppointment(appointment.id);
      setBanner(null);
      const result = await mutate(setAppointmentStatus, {
        appointmentId: appointment.id,
        appointmentTitle: appointment.client_name ?? appointment.title,
        date: today,
        status,
        previousStatus: appointment.status,
      });
      setBusyAppointment(null);
      if (result.ok && result.queued) {
        setBanner("Saved on this device — the board updates when you're back online.");
        return;
      }
      if (!result.ok) {
        setBanner(result.failure?.text ?? "The booking didn't change.");
      }
    },
    [mutate, today],
  );

  const saveTravel = useCallback(
    async (values: TravelDraftFormValues) => {
      setTravelSaving(true);
      setBanner(null);
      const option = ticketOptions.find((entry) => entry.id === values.ticketId);
      const result = await mutate(createTravelDraft, {
        ...values,
        ...(option !== undefined ? { ticketNumber: option.label.split(" · ")[0] } : {}),
      });
      setTravelSaving(false);
      if (!result.ok) {
        setBanner(result.failure?.text ?? "The trip didn't save.");
        return;
      }
      setTravelOpen(false);
      if (result.queued) {
        setBanner("Saved on this device — the trip syncs when you're back online.");
      }
    },
    [mutate, ticketOptions],
  );

  const submitDraft = useCallback(
    async (draft: TravelDraft) => {
      setBusyDraft(draft.id);
      setBanner(null);
      const result = await mutate(submitTravelDraft, {
        draftId: draft.id,
        ticketId: draft.ticket_id,
      });
      setBusyDraft(null);
      if (!result.ok) setBanner(result.failure?.text ?? "The trip didn't submit.");
    },
    [mutate],
  );

  const removeDraft = useCallback(
    async (draft: TravelDraft) => {
      setBusyDraft(draft.id);
      setBanner(null);
      const result = await mutate(deleteTravelDraft, { draftId: draft.id, snapshot: draft });
      setBusyDraft(null);
      if (!result.ok) setBanner(result.failure?.text ?? "The draft didn't delete.");
    },
    [mutate],
  );

  const schedule = (
    <>
      <View style={styles.header}>
        <Text style={[styles.title, { color: tokens.text }]}>Today</Text>
        <Text style={[styles.subtitle, { color: tokens.textMuted }]}>
          {me?.display_name ?? ""}
          {day.data !== undefined && Array.isArray(day.data.unassigned) && day.data.unassigned.length > 0
            ? ` · ${day.data.unassigned.length} unassigned`
            : ""}
        </Text>
      </View>

      {myOnCall.length > 0 ? <OnCallStrip blocks={myOnCall} now={now} /> : null}

      {myAppointments.length > 0 ? (
        <DaySchedule
          appointments={myAppointments}
          now={now}
          busyId={busyAppointment}
          onAction={act}
        />
      ) : (
        <EmptyState
          message="No bookings on today's board."
          actionLabel="Open the board"
          onAction={() => router.push("/board")}
        />
      )}

      {banner !== null ? <ErrorNote text={banner} /> : null}

      {travelOpen ? (
        <TravelDraftForm
          tickets={ticketOptions}
          saving={travelSaving}
          error={null}
          onCancel={() => setTravelOpen(false)}
          onSave={(values) => void saveTravel(values)}
        />
      ) : (
        <TravelSection
          drafts={myDrafts}
          busyId={busyDraft}
          onSubmit={(draft) => void submitDraft(draft)}
          onDelete={(draft) => void removeDraft(draft)}
          onLog={() => setTravelOpen(true)}
        />
      )}
    </>
  );

  const rest = (
    <>
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
    </>
  );

  return (
    <Screen testID="today-screen">
      {isRegular ? (
        <View style={styles.columns}>
          <View style={[styles.railColumn, { borderRightColor: tokens.border }]}>{schedule}</View>
          <View style={styles.detailColumn}>{rest}</View>
        </View>
      ) : (
        <>
          {schedule}
          {rest}
        </>
      )}
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
  columns: {
    flexDirection: "row",
    gap: 20,
  },
  railColumn: {
    width: 430,
    borderRightWidth: StyleSheet.hairlineWidth,
    paddingRight: 20,
    gap: 12,
  },
  detailColumn: {
    flex: 1,
    gap: 12,
  },
});
