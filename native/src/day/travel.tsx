// Travel drafts — the drive between jobs, logged in seconds.
//
// Purpose: the mileage draft flow the dispatcher's travel page already
// owns, sized for a phone at the kerb. A draft is a row against a ticket
// (start, end, distance, unit, vehicle, billable); submitting converts it
// through the time-entry path, so the draft becomes real, approvable time.
//
// Non-purpose: no expense conversion on the phone (the travel page owns
// it), no map or route editing.

import { useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { TravelDraft } from "@/core/models";
import { Button, Card, EmptyState, Field, Mono, Pill, SectionHeader, Toggle } from "@/ui/primitives";
import { type, useTheme } from "@/ui/tokens";

import { clockInput, parseClock } from "./clock";

export interface TravelTicketOption {
  id: string;
  label: string;
}

export interface TravelSectionProps {
  drafts: TravelDraft[];
  busyId: string | null;
  onSubmit: (draft: TravelDraft) => void;
  onDelete: (draft: TravelDraft) => void;
  onLog: () => void;
}

export function TravelSection({
  drafts,
  busyId,
  onSubmit,
  onDelete,
  onLog,
}: TravelSectionProps) {
  const { tokens } = useTheme();
  const recent = useMemo(() => drafts.slice(0, 5), [drafts]);

  return (
    <View testID="travel-section">
      <SectionHeader
        title="Travel"
        action={
          <Pressable
            testID="travel-log"
            onPress={onLog}
            hitSlop={8}
            accessibilityRole="button"
            accessibilityLabel="Log travel"
          >
            <Text style={[styles.action, { color: tokens.primary }]}>Log travel</Text>
          </Pressable>
        }
      />
      {recent.length === 0 ? (
        <EmptyState
          message="No travel logged today — log the drive when you park."
          actionLabel="Log travel"
          onAction={onLog}
        />
      ) : (
        <Card style={styles.list}>
          {recent.map((draft) => {
            const submitted = draft.status === "submitted";
            const busy = busyId === draft.id;
            return (
              <View
                key={draft.id}
                testID={`travel-draft-${draft.id}`}
                style={[styles.row, { borderBottomColor: tokens.border }]}
              >
                <View style={styles.rowCopy}>
                  <View style={styles.rowHeader}>
                    <Text style={[styles.distance, { color: tokens.text }]}>
                      {formatDistance(draft.distance)} {draft.distance_unit}
                    </Text>
                    <Pill
                      pair={submitted ? tokens.success : tokens.neutral}
                      label={submitted ? "SUBMITTED" : "DRAFT"}
                    />
                  </View>
                  <Text style={[styles.meta, { color: tokens.textMuted }]} numberOfLines={1}>
                    {clock(draft.started_at)}–{clock(draft.ended_at)}
                    {draft.vehicle.length > 0 ? ` · ${draft.vehicle}` : ""}
                    {draft.billable ? " · billable" : ""}
                  </Text>
                </View>
                {submitted ? (
                  <Mono muted>{draft.submitted_time_entry_id !== null ? "logged" : ""}</Mono>
                ) : (
                  <View style={styles.rowActions}>
                    <Button
                      label="Submit"
                      variant="secondary"
                      testID={`travel-submit-${draft.id}`}
                      loading={busy}
                      disabled={busyId !== null && !busy}
                      onPress={() => onSubmit(draft)}
                    />
                    <Button
                      label="Delete"
                      variant="ghost"
                      testID={`travel-delete-${draft.id}`}
                      disabled={busyId !== null && !busy}
                      onPress={() => onDelete(draft)}
                    />
                  </View>
                )}
              </View>
            );
          })}
        </Card>
      )}
    </View>
  );
}

export interface TravelDraftFormValues {
  ticketId: string;
  startedAt: string;
  endedAt: string;
  distance: number;
  distanceUnit: string;
  vehicle: string;
  billable: boolean;
  notes: string;
}

export interface TravelDraftFormProps {
  tickets: TravelTicketOption[];
  saving: boolean;
  error: string | null;
  onCancel: () => void;
  onSave: (values: TravelDraftFormValues) => void;
}

export function TravelDraftForm({
  tickets,
  saving,
  error,
  onCancel,
  onSave,
}: TravelDraftFormProps) {
  const { tokens } = useTheme();
  const [ticketId, setTicketId] = useState(tickets[0]?.id ?? "");
  const [started, setStarted] = useState(() => clockInput(Date.now() - 30 * 60 * 1000));
  const [ended, setEnded] = useState(() => clockInput(Date.now()));
  const [distance, setDistance] = useState("");
  const [unit, setUnit] = useState<"km" | "mi">("km");
  const [vehicle, setVehicle] = useState("");
  const [billable, setBillable] = useState(true);
  const [notes, setNotes] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);

  const submit = () => {
    const start = parseClock(started);
    const end = parseClock(ended);
    const miles = Number.parseFloat(distance.replace(",", "."));
    if (ticketId === "" || start === null || end === null) {
      setLocalError("Ticket, start, and end are required.");
      return;
    }
    const startedAt = new Date(start);
    const endedAt = new Date(end);
    if (endedAt.getTime() <= startedAt.getTime()) {
      setLocalError("The end time has to be after the start.");
      return;
    }
    if (!Number.isFinite(miles) || miles <= 0) {
      setLocalError("Enter the distance the trip covered.");
      return;
    }
    if (vehicle.trim() === "") {
      setLocalError("Enter the vehicle you drove.");
      return;
    }
    setLocalError(null);
    onSave({
      ticketId,
      startedAt: startedAt.toISOString(),
      endedAt: endedAt.toISOString(),
      distance: miles,
      distanceUnit: unit,
      vehicle: vehicle.trim(),
      billable,
      notes: notes.trim(),
    });
  };

  return (
    <Card style={styles.form} testID="travel-form">
      <Text style={[styles.formTitle, { color: tokens.text }]}>Log travel</Text>
      {tickets.length === 0 ? (
        <Text style={[styles.meta, { color: tokens.textMuted }]}>
          Book a ticket first — a trip needs a job to bill against.
        </Text>
      ) : (
        <View style={styles.ticketChips}>
          {tickets.map((option) => {
            const selected = option.id === ticketId;
            return (
              <Pressable
                key={option.id}
                testID={`travel-ticket-${option.id}`}
                onPress={() => setTicketId(option.id)}
                accessibilityRole="button"
                accessibilityState={{ selected }}
                style={[
                  styles.chip,
                  {
                    backgroundColor: selected ? tokens.primary : tokens.surfaceHover,
                    borderColor: selected ? tokens.primary : tokens.border,
                  },
                ]}
              >
                <Text
                  style={[
                    styles.chipText,
                    { color: selected ? tokens.primaryText : tokens.textSecondary },
                  ]}
                  numberOfLines={1}
                >
                  {option.label}
                </Text>
              </Pressable>
            );
          })}
        </View>
      )}
      <View style={styles.timeRow}>
        <View style={styles.timeField}>
          <Field
            label="Started"
            testID="travel-started"
            value={started}
            onChangeText={setStarted}
            placeholder="08:30"
            keyboardType="numbers-and-punctuation"
            autoCapitalize="none"
          />
        </View>
        <View style={styles.timeField}>
          <Field
            label="Ended"
            testID="travel-ended"
            value={ended}
            onChangeText={setEnded}
            placeholder="09:05"
            keyboardType="numbers-and-punctuation"
            autoCapitalize="none"
          />
        </View>
      </View>
      <View style={styles.timeRow}>
        <View style={styles.timeField}>
          <Field
            label="Distance"
            testID="travel-distance"
            value={distance}
            onChangeText={setDistance}
            placeholder="12.4"
            keyboardType="decimal-pad"
          />
        </View>
        <View style={styles.unitColumn}>
          <Text style={[styles.metaLabel, { color: tokens.textMuted }]}>UNIT</Text>
          <View style={styles.unitToggle}>
            {(["km", "mi"] as const).map((option) => {
              const selected = option === unit;
              return (
                <Pressable
                  key={option}
                  testID={`travel-unit-${option}`}
                  onPress={() => setUnit(option)}
                  accessibilityRole="button"
                  accessibilityState={{ selected }}
                  style={[
                    styles.chip,
                    {
                      backgroundColor: selected ? tokens.primary : tokens.surfaceHover,
                      borderColor: selected ? tokens.primary : tokens.border,
                    },
                  ]}
                >
                  <Text
                    style={[
                      styles.chipText,
                      { color: selected ? tokens.primaryText : tokens.textSecondary },
                    ]}
                  >
                    {option}
                  </Text>
                </Pressable>
              );
            })}
          </View>
        </View>
      </View>
      <Field
        label="Vehicle"
        testID="travel-vehicle"
        value={vehicle}
        onChangeText={setVehicle}
        placeholder="Van, plate, or pool car"
      />
      <Toggle
        label="Billable"
        value={billable}
        onValueChange={setBillable}
        testID="travel-billable"
      />
      <Field
        label="Notes"
        testID="travel-notes"
        value={notes}
        onChangeText={setNotes}
        placeholder="Parking, tolls, traffic"
      />
      {localError !== null || error !== null ? (
        <Text style={[styles.error, { color: tokens.danger.text }]}>
          {localError ?? error}
        </Text>
      ) : null}
      <View style={styles.formActions}>
        <Button
          label="Cancel"
          variant="ghost"
          testID="travel-cancel"
          onPress={onCancel}
          disabled={saving}
        />
        <Button
          label="Save draft"
          testID="travel-save"
          loading={saving}
          disabled={saving || tickets.length === 0}
          onPress={submit}
        />
      </View>
    </Card>
  );
}

function formatDistance(distance: number): string {
  return Number.isInteger(distance) ? String(distance) : distance.toFixed(1);
}

function clock(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "--:--";
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

const styles = StyleSheet.create({
  action: {
    fontFamily: type.sansSemiBold,
    fontSize: 14,
  },
  list: {
    padding: 0,
    paddingHorizontal: 16,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    paddingVertical: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  rowCopy: {
    flex: 1,
    gap: 3,
  },
  rowHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  distance: {
    fontFamily: type.monoMedium,
    fontSize: 15,
    fontVariant: ["tabular-nums"],
  },
  meta: {
    fontFamily: type.sans,
    fontSize: 12,
  },
  rowActions: {
    flexDirection: "row",
    gap: 6,
  },
  form: {
    gap: 12,
  },
  formTitle: {
    fontFamily: type.sansSemiBold,
    fontSize: 17,
  },
  ticketChips: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
  },
  chip: {
    minHeight: 40,
    paddingHorizontal: 12,
    borderRadius: 999,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
    maxWidth: 240,
  },
  chipText: {
    fontFamily: type.sansSemiBold,
    fontSize: 12,
  },
  timeRow: {
    flexDirection: "row",
    gap: 12,
  },
  timeField: {
    flex: 1,
  },
  unitColumn: {
    gap: 6,
    justifyContent: "flex-end",
  },
  metaLabel: {
    fontFamily: type.sansMedium,
    fontSize: 11,
    letterSpacing: 0.8,
  },
  unitToggle: {
    flexDirection: "row",
    gap: 6,
  },
  error: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
  formActions: {
    flexDirection: "row",
    justifyContent: "flex-end",
    gap: 8,
  },
});
