// Today's schedule rail — the technician's bookings with accept/finish.
//
// Purpose: the workbar IA on a phone grammar. One time-ordered rail, a
// state pill per booking, and exactly one primary action on the booking
// that needs it. Accept moves a scheduled booking to in progress; finish
// closes it. Both ride the appointment PATCH the server already exposes,
// so the dispatcher's board sees the same state.
//
// Non-purpose: no dispatch-board mechanics, no rescheduling (the board and
// calendar own moves).

import { StyleSheet, Text, View } from "react-native";

import type { AppointmentRow } from "@/core/models";
import { Button, Card, Pill } from "@/ui/primitives";
import { type, useTheme, type StatusPair } from "@/ui/tokens";

export interface DayScheduleProps {
  appointments: AppointmentRow[];
  now: number;
  busyId: string | null;
  onAction: (appointment: AppointmentRow, status: "in_progress" | "done") => void;
}

export function DaySchedule({ appointments, now, busyId, onAction }: DayScheduleProps) {
  const { tokens } = useTheme();

  if (appointments.length === 0) return null;

  const nextId = appointments.find(
    (appointment) => Date.parse(appointment.ends_at) >= now,
  )?.id;

  return (
    <View style={styles.rail} testID="day-schedule">
      {appointments.map((appointment) => {
        const state = appointmentState(tokens, appointment.status);
        const isNext = appointment.id === nextId;
        const canAccept = appointment.status === "scheduled";
        const canFinish = appointment.status === "in_progress";
        const busy = busyId === appointment.id;
        return (
          <View key={appointment.id} style={styles.row}>
            <View style={styles.gutter}>
              <Text style={[styles.start, { color: tokens.text }]}>
                {clock(appointment.starts_at)}
              </Text>
              <Text style={[styles.end, { color: tokens.textMuted }]}>
                {clock(appointment.ends_at)}
              </Text>
            </View>
            <Card
              testID={`appointment-${appointment.id}`}
              style={[
                styles.booking,
                isNext ? { borderColor: tokens.primary } : null,
              ]}
            >
              <View style={styles.bookingHeader}>
                <Text style={[styles.client, { color: tokens.text }]} numberOfLines={1}>
                  {appointment.client_name ?? appointment.title}
                </Text>
                <Pill pair={state.pair} label={state.label} />
              </View>
              <Text style={[styles.title, { color: tokens.textSecondary }]} numberOfLines={2}>
                {appointment.client_name !== null && appointment.client_name !== undefined
                  ? appointment.title
                  : ""}
              </Text>
              <View style={styles.metaRow}>
                {appointment.travel_minutes > 0 ? (
                  <Text style={[styles.meta, { color: tokens.textMuted }]}>
                    {appointment.travel_minutes} min travel
                  </Text>
                ) : null}
                {appointment.ticket_number !== null && appointment.ticket_number !== undefined ? (
                  <Text style={[styles.meta, { color: tokens.textMuted }]}>
                    {appointment.ticket_number}
                  </Text>
                ) : null}
                {appointment.dispatch_status !== null &&
                appointment.dispatch_status !== undefined ? (
                  <Text style={[styles.meta, { color: tokens.textMuted }]}>
                    dispatch: {appointment.dispatch_status.status.replace(/_/g, " ")}
                  </Text>
                ) : null}
              </View>
              {canAccept || canFinish ? (
                <Button
                  testID={`appointment-action-${appointment.id}`}
                  label={canAccept ? "Accept job" : "Finish"}
                  variant={canAccept ? "primary" : "secondary"}
                  loading={busy}
                  disabled={busyId !== null && !busy}
                  onPress={() => onAction(appointment, canAccept ? "in_progress" : "done")}
                />
              ) : null}
            </Card>
          </View>
        );
      })}
    </View>
  );
}

function appointmentState(
  tokens: ReturnType<typeof useTheme>["tokens"],
  status: string,
): { pair: StatusPair; label: string } {
  switch (status) {
    case "in_progress":
      return { pair: tokens.success, label: "IN PROGRESS" };
    case "done":
      return { pair: tokens.neutral, label: "DONE" };
    case "cancelled":
      return { pair: tokens.danger, label: "CANCELLED" };
    default:
      return { pair: tokens.warning, label: "SCHEDULED" };
  }
}

function clock(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "--:--";
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

const styles = StyleSheet.create({
  rail: {
    gap: 10,
  },
  row: {
    flexDirection: "row",
    gap: 10,
  },
  gutter: {
    width: 52,
    paddingTop: 14,
    alignItems: "flex-end",
    gap: 2,
  },
  start: {
    fontFamily: type.monoMedium,
    fontSize: 15,
    fontVariant: ["tabular-nums"],
  },
  end: {
    fontFamily: type.mono,
    fontSize: 12,
    fontVariant: ["tabular-nums"],
  },
  booking: {
    flex: 1,
    gap: 8,
  },
  bookingHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
  },
  client: {
    fontFamily: type.sansSemiBold,
    fontSize: 17,
    flex: 1,
  },
  title: {
    fontFamily: type.sans,
    fontSize: 14,
  },
  metaRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 10,
  },
  meta: {
    fontFamily: type.sansMedium,
    fontSize: 12,
  },
});
