// Today — the technician's home, drawn to the mobile pack
// (design/mockups/marula-app-mockup-images/mobile): a date-and-greeting
// header in the serif face, the next-visit card with its Due-today pill and
// the Start-travel → I've-arrived travel timer, the quick tiles, and the
// Later list of work grouped by due day. The schedule rail, on-call,
// travel drafts, and the day pack stay on the page (compose from the shared
// kit — the pack drew nothing for them), below the pack's named sections.
//
// Purpose: hydrate from the cached read models (dispatch day, my work,
// on-call, travel drafts) and live through the event stream: a rescheduled
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
import { Pressable, StyleSheet, Text, View } from "react-native";

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
  startTimer,
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
  Button,
  Card,
  EmptyState,
  ErrorNote,
  Loading,
  Pill,
  SectionHeader,
  Screen,
  SuccessNote,
} from "@/ui/primitives";
import { type, useTheme } from "@/ui/tokens";

/** Time-of-day greeting, the pack's "Afternoon, Thabo". */
function greetingFor(hour: number): string {
  if (hour < 12) return "Morning";
  if (hour < 17) return "Afternoon";
  return "Evening";
}

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
  const [bannerOk, setBannerOk] = useState(false);
  const [arriving, setArriving] = useState(false);

  const myAppointments: AppointmentRow[] = useMemo(() => {
    if (day.data === undefined || me === null) return [];
    const mine = day.data.technicians.find((tech) => tech.user_id === me.user_id);
    const rows = mine?.appointments ?? [];
    return [...rows]
      .filter((row) => row.status !== "cancelled")
      .sort((a, b) => a.starts_at.localeCompare(b.starts_at));
  }, [day.data, me]);

  const myDrafts: TravelDraft[] = useMemo(() => travel.data ?? [], [travel.data]);

  // The pack's next visit: the first appointment whose window has not
  // closed, regardless of whether it already started.
  const nextVisit: AppointmentRow | undefined = useMemo(
    () => myAppointments.find((appointment) => Date.parse(appointment.ends_at) >= now),
    [myAppointments, now],
  );
  const travelUnderway = useMemo(
    () => (nextVisit !== undefined ? myDrafts.some((d) => d.status === "draft" && d.ticket_id === (nextVisit.ticket_id ?? "")) : false),
    [myDrafts, nextVisit],
  );

  const myOnCall: OnCallBlock[] = useMemo(() => {
    if (onCall.data === undefined || me === null) return [];
    return onCall.data
      .filter((block) => block.user_id === me.user_id)
      .sort((a, b) => a.starts_at.localeCompare(b.starts_at));
  }, [onCall.data, me]);

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

  // The pack's Later list: work grouped by due day — today's rows carry a
  // clock time, later days carry the weekday label in the same column.
  const laterGroups = useMemo(() => {
    const items = (work.data ?? []).slice(0, 20);
    const today = new Date(now);
    const startOfToday = new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime();
    const groups: { key: string; label: string | null; items: MyWorkItem[] }[] = [];
    const bucketOf = (item: MyWorkItem): { key: string; label: string | null } => {
      const due = item.sla_resolution_due ?? item.due_at;
      if (due === null || due === undefined) return { key: "none", label: null };
      const time = Date.parse(due);
      if (Number.isNaN(time)) return { key: "none", label: null };
      if (time < startOfToday) return { key: "overdue", label: null };
      const dayDiff = Math.floor((time - startOfToday) / 86_400_000);
      if (dayDiff >= 1) {
        const d = new Date(time);
        return {
          key: `day-${dayDiff}`,
          label: d.toLocaleDateString(undefined, { weekday: "long" }),
        };
      }
      return { key: "today", label: null };
    };
    for (const item of items) {
      const bucket = bucketOf(item);
      if (bucket.key === "overdue") continue; // the attention strip owns overdue
      let group = groups.find((g) => g.key === bucket.key);
      if (group === undefined) {
        group = { key: bucket.key, label: bucket.label, items: [] };
        groups.push(group);
      }
      group.items.push(item);
    }
    return groups;
  }, [work.data, now]);

  const act = useCallback(
    async (appointment: AppointmentRow, status: "in_progress" | "done") => {
      setBusyAppointment(appointment.id);
      setBanner(null);
      setBannerOk(false);
      const result = await mutate(setAppointmentStatus, {
        appointmentId: appointment.id,
        appointmentTitle: appointment.client_name ?? appointment.title,
        date: today,
        status,
        previousStatus: appointment.status,
      });
      setBusyAppointment(null);
      if (result.ok && result.queued) {
        setBannerOk(true);
        setBanner("Saved on this device — the board updates when you're back online.");
        return;
      }
      if (!result.ok) {
        setBannerOk(false);
        setBanner(result.failure?.text ?? "The booking didn't change.");
      }
    },
    [mutate, today],
  );

  /** Pack travel flow: Start travel opens the pinned timer on the visit's
   * ticket (the work bar carries the elapsed time), the button flips to
   * "I've arrived" (the draft rides the Travel section for the receipt), a
   * quiet note confirms. Arriving opens the job. */
  const startTravel = useCallback(
    async (appointment: AppointmentRow) => {
      if (appointment.ticket_id === null || appointment.ticket_id === undefined) {
        setBannerOk(false);
        setBanner("The visit has no ticket to travel against.");
        return;
      }
      setArriving(true);
      setBanner(null);
      setBannerOk(false);
      const result = await mutate(startTimer, {
        ticketId: appointment.ticket_id,
        ticketNumber: appointment.ticket_number ?? undefined,
      });
      if (!result.ok) {
        setArriving(false);
        setBannerOk(false);
        setBanner(result.failure?.text ?? "Travel didn't start.");
        return;
      }
      const draft = await mutate(createTravelDraft, {
        ticketId: appointment.ticket_id,
        ticketNumber: appointment.ticket_number ?? undefined,
        startedAt: new Date().toISOString(),
        endedAt: new Date(Date.now() + appointment.travel_minutes * 60_000).toISOString(),
        distance: (appointment.travel_distance_meters ?? 0) / 1000,
        distanceUnit: "km",
        vehicle: "car",
        billable: true,
      });
      setArriving(false);
      if (draft.ok && draft.queued) {
        setBannerOk(true);
        setBanner("Travel timer started. The trip syncs when you're back online.");
      } else if (draft.ok) {
        setBannerOk(true);
        setBanner("Travel timer started.");
      } else {
        setBannerOk(false);
        setBanner("Travel timer started. The trip didn't save — log it from Travel.");
      }
    },
    [mutate],
  );

  const arriveVisit = useCallback(
    async (appointment: AppointmentRow) => {
      await act(appointment, "in_progress");
      if (appointment.ticket_id !== null && appointment.ticket_id !== undefined) {
        router.push(`/ticket/${appointment.ticket_id}`);
      }
    },
    [act],
  );

  const saveTravel = useCallback(
    async (values: TravelDraftFormValues) => {
      setTravelSaving(true);
      setBanner(null);
      setBannerOk(false);
      const option = ticketOptions.find((entry) => entry.id === values.ticketId);
      const result = await mutate(createTravelDraft, {
        ...values,
        ...(option !== undefined ? { ticketNumber: option.label.split(" · ")[0] } : {}),
      });
      setTravelSaving(false);
      if (!result.ok) {
        setBannerOk(false);
        setBanner(result.failure?.text ?? "The trip didn't save.");
        return;
      }
      setTravelOpen(false);
      if (result.queued) {
        setBannerOk(true);
        setBanner("Saved on this device — the trip syncs when you're back online.");
      }
    },
    [mutate, ticketOptions],
  );

  const submitDraft = useCallback(
    async (draft: TravelDraft) => {
      setBusyDraft(draft.id);
      setBanner(null);
      setBannerOk(false);
      const result = await mutate(submitTravelDraft, {
        draftId: draft.id,
        ticketId: draft.ticket_id,
      });
      setBusyDraft(null);
      if (!result.ok) setBannerOk(false);
        setBanner(result.failure?.text ?? "The trip didn't submit.");
    },
    [mutate],
  );

  const removeDraft = useCallback(
    async (draft: TravelDraft) => {
      setBusyDraft(draft.id);
      setBanner(null);
      setBannerOk(false);
      const result = await mutate(deleteTravelDraft, { draftId: draft.id, snapshot: draft });
      setBusyDraft(null);
      if (!result.ok) setBannerOk(false);
        setBanner(result.failure?.text ?? "The draft didn't delete.");
    },
    [mutate],
  );

  const firstName = (me?.display_name ?? "").split(/\s+/)[0] ?? "";
  const nowDate = new Date(now);

  const header = (
    <View style={styles.header}>
      <View style={styles.headerRow}>
        <View style={styles.headerText}>
          <Text style={[styles.date, { color: tokens.textMuted }]}>
            {nowDate.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" })}
          </Text>
          <Text style={[styles.greeting, { color: tokens.text }]}>
            {firstName !== "" ? `${greetingFor(nowDate.getHours())}, ${firstName}` : greetingFor(nowDate.getHours())}
          </Text>
        </View>
        <View
          style={[styles.avatar, { backgroundColor: tokens.success.background }]}
          accessibilityLabel={me?.display_name ?? "Your profile"}
        >
          <Text style={[styles.avatarText, { color: tokens.success.text }]}>
            {initials(me?.display_name ?? "")}
          </Text>
        </View>
      </View>
    </View>
  );

  const nextVisitCard =
    nextVisit !== undefined ? (
      <NextVisitCard
        appointment={nextVisit}
        now={now}
        travelUnderway={travelUnderway}
        busy={arriving || busyAppointment === nextVisit.id}
        onStartTravel={() => void startTravel(nextVisit)}
        onArrive={() => void arriveVisit(nextVisit)}
        onCall={() => { setBannerOk(false); setBanner("Calling runs through the device dialer — not wired up yet."); }}
        onDirections={() => { setBannerOk(false); setBanner("Directions hand off to the phone's maps — not wired up yet."); }}
      />
    ) : (
      <EmptyState
        message="No visits on today's board."
        actionLabel="Open the board"
        onAction={() => router.push("/board")}
      />
    );

  const quickTiles = (
    <View style={styles.actionGrid}>
      <Action label="Scan asset" icon="scan-outline" href="/scan" />
      <Action label="Add time" icon="time-outline" href="/(tabs)/time" />
      <Action
        label="Photo"
        icon="camera-outline"
        onPress={() => { setBannerOk(false); setBanner("Photo capture is not wired up yet — attach pictures from the ticket."); }}
      />
    </View>
  );

  const laterList = (
    <View>
      <SectionHeader title="Later" />
      {work.status === "error" && work.data === undefined ? (
        <ErrorNote text={work.error?.text ?? "Couldn't load your work."} onRetry={work.refresh} />
      ) : null}
      {work.status === "loading" && work.data === undefined ? <Loading label="Loading work" /> : null}
      {work.data !== undefined && work.data.length === 0 ? (
        <EmptyState message="Nothing assigned to you right now." />
      ) : null}
      {laterGroups.map((group) => (
        <View key={group.key} style={styles.laterGroup}>
          {group.label !== null ? (
            <Text style={[styles.laterDayLabel, { color: tokens.textMuted }]}>{group.label}</Text>
          ) : null}
          <Card style={styles.listCard}>
            {group.items.map((item: MyWorkItem) => {
              const due = item.sla_resolution_due ?? item.due_at;
              const dueDate = due !== null && due !== undefined ? new Date(due) : null;
              const startOfToday = new Date(now).setHours(0, 0, 0, 0);
              const dueTime =
                dueDate !== null && !Number.isNaN(dueDate.getTime())
                  ? dueDate.getTime() >= startOfToday
                    ? clockLabel(dueDate)
                    : null
                  : null;
              return (
                <LaterRow
                  key={item.id}
                  item={item}
                  dueTime={dueTime}
                  onPress={() => router.push(`/ticket/${item.id}`)}
                  trailing={slaPill(tokens, slaState(item.sla_resolution_due, now))}
                />
              );
            })}
          </Card>
        </View>
      ))}
      {work.offline && work.data !== undefined ? (
        <Text style={[styles.offlineNote, { color: tokens.textMuted }]}>
          Showing the last sync from this device.
        </Text>
      ) : null}
    </View>
  );

  const composeSections = (
    <>
      <DayPackCard />
      {myOnCall.length > 0 ? <OnCallStrip blocks={myOnCall} now={now} /> : null}

      {myAppointments.length > 0 ? (
        <DaySchedule
          appointments={myAppointments}
          now={now}
          busyId={busyAppointment}
          onAction={act}
        />
      ) : null}

      {banner !== null ? (
        bannerOk ? (
          <SuccessNote text={banner} />
        ) : (
          <ErrorNote text={banner} />
        )
      ) : null}

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

  return (
    <Screen testID="today-screen">
      {isRegular ? (
        <View style={styles.columns}>
          <View style={[styles.railColumn, { borderRightColor: tokens.border }]}>
            {header}
            {nextVisitCard}
            {quickTiles}
            {laterList}
          </View>
          <View style={styles.detailColumn}>{composeSections}</View>
        </View>
      ) : (
        <>
          {header}
          {nextVisitCard}
          {quickTiles}
          {laterList}
          {composeSections}
        </>
      )}
    </Screen>
  );
}

/** The pack's NEXT VISIT card: window label + Due-today pill, the client
 * over the ticket row, then Start travel primary beside square call and
 * directions buttons. Travel started flips the primary to "I've arrived",
 * which opens the job. */
function NextVisitCard({
  appointment,
  now,
  travelUnderway,
  busy,
  onStartTravel,
  onArrive,
  onCall,
  onDirections,
}: {
  appointment: AppointmentRow;
  now: number;
  travelUnderway: boolean;
  busy: boolean;
  onStartTravel: () => void;
  onArrive: () => void;
  onCall: () => void;
  onDirections: () => void;
}) {
  const { tokens } = useTheme();
  const start = new Date(appointment.starts_at);
  const end = new Date(appointment.ends_at);
  const window = `${clockLabel(start)}–${clockLabel(end)}`;
  const dueToday = sameDay(start.getTime(), now);
  const canStartTravel =
    appointment.ticket_id !== null &&
    appointment.ticket_id !== undefined &&
    appointment.status === "scheduled";
  const canArrive = appointment.status === "scheduled" && travelUnderway;
  return (
    <Card testID="next-visit-card" style={styles.visitCard}>
      <View style={styles.visitTop}>
        <Text style={[styles.visitWindow, { color: tokens.textMuted }]}>
          NEXT VISIT · {window}
        </Text>
        {dueToday ? <Pill pair={tokens.warning} label="Due today" /> : null}
      </View>
      <Text style={[styles.visitClient, { color: tokens.text }]} numberOfLines={1}>
        {appointment.client_name ?? appointment.title}
      </Text>
      <Text style={[styles.visitMeta, { color: tokens.textMuted }]} numberOfLines={1}>
        {appointment.kind === "remote"
          ? "Remote"
          : appointment.travel_minutes > 0
            ? `${appointment.travel_minutes} min travel`
            : (appointment.title !== appointment.client_name ? appointment.title : "On site")}
      </Text>
      {appointment.ticket_id !== null && appointment.ticket_id !== undefined ? (
        <Pressable
          style={[styles.visitTicket, { backgroundColor: tokens.surfaceHover }]}
          onPress={() => router.push(`/ticket/${appointment.ticket_id ?? ""}`)}
          accessibilityRole="button"
          accessibilityLabel={`Open ticket ${appointment.ticket_number ?? ""}`}
        >
          <Text style={[styles.visitTicketNumber, { color: tokens.textSecondary }]} numberOfLines={1}>
            {appointment.ticket_number ?? ""}
          </Text>
          <Text style={[styles.visitTicketTitle, { color: tokens.text }]} numberOfLines={1}>
            {" "}
            {appointment.title}
          </Text>
          <View style={styles.visitChevron}>
            <Ionicons name="chevron-forward" size={16} color={tokens.textMuted} />
          </View>
        </Pressable>
      ) : null}
      <View style={styles.visitActions}>
        {canArrive ? (
          <View style={styles.visitPrimary}>
            <Button
              testID="next-visit-arrive"
              label="I've arrived"
              variant="primary"
              loading={busy}
              onPress={onArrive}
            />
          </View>
        ) : canStartTravel ? (
          <View style={styles.visitPrimary}>
            <Button
              testID="next-visit-travel"
              label="Start travel"
              variant="primary"
              loading={busy}
              onPress={onStartTravel}
            />
          </View>
        ) : (
          <View style={styles.visitPrimary}>
            <Button
              testID="next-visit-open"
              label="Open job"
              variant="secondary"
              onPress={() => {
                if (appointment.ticket_id !== null && appointment.ticket_id !== undefined) {
                  router.push(`/ticket/${appointment.ticket_id}`);
                }
              }}
            />
          </View>
        )}
        <Pressable
          style={[styles.squareButton, { borderColor: tokens.border }]}
          accessibilityRole="button"
          accessibilityLabel={`Call about ${appointment.ticket_number ?? "the job"}`}
          onPress={onCall}
        >
          <Ionicons name="call-outline" size={20} color={tokens.textSecondary} />
        </Pressable>
        <Pressable
          style={[styles.squareButton, { borderColor: tokens.border }]}
          accessibilityRole="button"
          accessibilityLabel="Directions"
          onPress={onDirections}
        >
          <Ionicons name="navigate-outline" size={20} color={tokens.textSecondary} />
        </Pressable>
      </View>
    </Card>
  );
}

function clockLabel(date: Date): string {
  if (Number.isNaN(date.getTime())) return "--:--";
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

function sameDay(a: number, b: number): boolean {
  const da = new Date(a);
  const db = new Date(b);
  return (
    da.getFullYear() === db.getFullYear() &&
    da.getMonth() === db.getMonth() &&
    da.getDate() === db.getDate()
  );
}

function initials(name: string): string {
  return name
    .split(/\s+/)
    .slice(0, 2)
    .map((w) => w[0]?.toUpperCase() ?? "")
    .join("");
}

function Action({
  label,
  icon,
  href,
  onPress,
}: {
  label: string;
  icon: keyof typeof Ionicons.glyphMap;
  href?: string;
  onPress?: () => void;
}) {
  const { tokens } = useTheme();
  return (
    <Card
      style={styles.action}
      onPress={
        onPress ??
        (href !== undefined ? () => router.push(href as never) : undefined)
      }
      accessibilityLabel={label}
    >
      <Ionicons name={icon} size={22} color={tokens.textSecondary} />
      <Text style={[styles.actionLabel, { color: tokens.text }]}>{label}</Text>
    </Card>
  );
}

/** The pack's Later row: clock time in the left column, the client name
 * bold, "#num · title" under it, and the SLA pill at the right. */
function LaterRow({
  item,
  dueTime,
  onPress,
  trailing,
}: {
  item: MyWorkItem;
  dueTime: string | null;
  onPress: () => void;
  trailing?: React.ReactNode;
}) {
  const { tokens } = useTheme();
  return (
    <Pressable
      testID={`mywork-${item.id}`}
      style={styles.laterRow}
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={`${item.client_name ?? item.title}, ticket ${item.number}`}
    >
      <Text style={[styles.laterTime, { color: tokens.textSecondary }]} numberOfLines={1}>
        {dueTime ?? ""}
      </Text>
      <View style={styles.laterBody}>
        <Text style={[styles.laterClient, { color: tokens.text }]} numberOfLines={1}>
          {item.client_name ?? item.title}
        </Text>
        <Text style={[styles.laterTicket, { color: tokens.textMuted }]} numberOfLines={1}>
          #{item.number} · {item.title}
        </Text>
      </View>
      {trailing !== undefined ? trailing : null}
      <Ionicons name="chevron-forward" size={16} color={tokens.textMuted} />
    </Pressable>
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
  headerRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  headerText: {
    flex: 1,
    gap: 2,
  },
  date: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
  greeting: {
    fontFamily: type.serif,
    fontSize: 26,
    letterSpacing: -0.3,
  },
  avatar: {
    width: 44,
    height: 44,
    borderRadius: 22,
    alignItems: "center",
    justifyContent: "center",
  },
  avatarText: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  visitCard: {
    gap: 8,
  },
  visitTop: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
  },
  visitWindow: {
    fontFamily: type.sansMedium,
    fontSize: 12,
    letterSpacing: 0.6,
  },
  visitClient: {
    fontFamily: type.serif,
    fontSize: 22,
    letterSpacing: -0.2,
  },
  visitMeta: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  visitTicket: {
    flexDirection: "row",
    alignItems: "center",
    borderRadius: 10,
    paddingHorizontal: 12,
    paddingVertical: 10,
    gap: 4,
  },
  visitTicketNumber: {
    fontFamily: type.mono,
    fontSize: 13,
  },
  visitTicketTitle: {
    fontFamily: type.sansMedium,
    fontSize: 13,
    flexShrink: 1,
  },
  visitChevron: {
    marginLeft: "auto",
  },
  visitActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  visitPrimary: {
    flex: 1,
  },
  actionGrid: {
    flexDirection: "row",
    gap: 12,
  },
  action: {
    flex: 1,
    minHeight: 72,
    alignItems: "center",
    justifyContent: "center",
    gap: 6,
  },
  actionLabel: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
  laterGroup: {
    gap: 4,
    marginBottom: 8,
  },
  laterRow: {
    minHeight: 56,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingVertical: 8,
    paddingHorizontal: 12,
  },
  laterDayLabel: {
    fontFamily: type.sansMedium,
    fontSize: 12,
    textTransform: "capitalize",
  },
  laterTime: {
    fontFamily: type.mono,
    fontSize: 13,
    fontVariant: ["tabular-nums"],
    minWidth: 46,
  },
  laterBody: {
    flex: 1,
    gap: 1,
  },
  laterClient: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  laterTicket: {
    fontFamily: type.sans,
    fontSize: 12,
  },
  squareButton: {
    width: 52,
    height: 52,
    borderRadius: 12,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
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
