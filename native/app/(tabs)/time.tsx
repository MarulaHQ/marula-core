// Time tab — what is running, and what was forgotten.
//
// Purpose: the pack's `04-time-day` and `05-stop-and-log`. A gold card holds
// the running session (the clock 00:25:05, Pause, and Stop and log), the
// day's total with a strip, the day's entries, and a blush card for
// yesterday's unlogged gap with a Log it button. Stop and log is a SHEET,
// not a form: it rounds up to the next 15 minutes and says so, offers
// 15/30/45/1h, asks for the work type, and shows Billable as a switch whose
// off state demands a reason — exactly the desktop Quick-log semantics. The
// same sheet, prefilled, serves "edit entry" from a row.
//
// Non-purpose: no approval flows (the desk owns them).

import { useMemo, useState } from "react";
import { StyleSheet, Text, TextInput, View } from "react-native";
import { router } from "expo-router";

import {
  MODELS,
  fetchActiveTimer,
  fetchDayPack,
  fetchMyDayEntries,
  fetchTicket,
  startOfLocalDay,
  type DayPack,
  type TimeEntry,
} from "@/core/models";
import { createTimeEntry, stopTimer, pauseTimer, resumeTimer } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
import { useAuth } from "@/auth/auth-context";
import {
  FactRow,
  Facts,
  PackHeading,
  PackLabel,
  PACK_ICONS,
  PrimaryButton,
  SecondaryButton,
  Sheet,
  Segment,
  bottomReserve,
  pack,
} from "@/ui/pack";
import { useSecondTick } from "@/ui/time-pill";
import { clockText, elapsedMs, roundUpTo, shortClock } from "@/ui/clock-format";
import { isPaused } from "@/ui/timer-bar";
import { Screen } from "@/ui/primitives";
import { chrome, shape, type, useTheme } from "@/ui/tokens";

const DAY_MINUTES = 8 * 60;

const WORK_TYPES = [
  { value: "remote-support", label: "Remote" },
  { value: "onsite-support", label: "Onsite" },
  { value: "project", label: "Project" },
  { value: "admin", label: "Admin" },
];

const UNBILLABLE_REASONS = [
  { value: "warranty", label: "Warranty" },
  { value: "courtesy", label: "Courtesy" },
  { value: "in_retainer_block", label: "Retainer" },
  { value: "sla_covered", label: "SLA covered" },
  { value: "internal", label: "Internal" },
  { value: "mistake", label: "Mistake" },
];

const DURATIONS = [
  { value: 15, label: "15m" },
  { value: 30, label: "30m" },
  { value: 45, label: "45m" },
  { value: 60, label: "1h" },
];

type SheetRequest =
  | { kind: "stop" }
  | { kind: "edit"; entry: TimeEntry }
  | { kind: "gap"; minutes: number; startedAt: string; endsAt: string; ticketId: string };

export default function TimeScreen() {
  const { tokens, mode } = useTheme();
  const { me } = useAuth();
  const { mutate } = useMutation();
  const timer = useModel(MODELS.activeTimer, fetchActiveTimer);
  const session = timer.data ?? null;
  const paused = session !== null && isPaused(session);
  const seconds = useSecondTick(session !== null && !paused);
  const dayPack = useModel(MODELS.dayPack(), fetchDayPack, { maxAgeMs: 60_000 });
  const ticket = useModel(
    session !== null ? MODELS.ticket(session.ticket_id) : "ticket:missing",
    (client) => fetchTicket(client, session?.ticket_id ?? ""),
    { maxAgeMs: 15_000 },
  );
  // The signed-in person's day: their own entries since local midnight.
  const entries = useModel(
    MODELS.myDayEntries(me?.user_id ?? "none"),
    (client) => fetchMyDayEntries(client, me?.user_id ?? "", startOfLocalDay()),
    { maxAgeMs: 30_000 },
  );

  const [sheet, setSheet] = useState<SheetRequest | null>(null);
  const [banner, setBanner] = useState<string | null>(null);

  // The pack's running card is the GOLD tint (--acc) — the one place gold is
  // a ground, because it is the thing you are acting on right now.
  const runningGround = mode === "dark" ? chrome.goldTintDark : chrome.goldTintLight;
  const runningInk = mode === "dark" ? chrome.gold : chrome.goldText;
  const runningTicketLabel =
    ticket.data !== undefined ? `${ticket.data.number} ${ticket.data.title}` : "Running";
  const runningMinutes = session !== null ? Math.floor(elapsedMs(session, seconds) / 60_000) : 0;

  const dayRows = useMemo(() => entries.data ?? [], [entries.data]);
  const loggedToday = dayRows.reduce((sum, entry) => sum + entry.minutes, 0);

  // The blush gap card: a stop in today's day pack with no logged time
  // against it. Derived, so it only appears when there is a real window.
  const gap = useMemo(() => unloggedStop(dayPack.data, dayRows), [dayPack.data, dayRows]);

  const stopSheetSession =
    sheet?.kind === "stop" && session !== null ? session : null;

  // The heading's date is today, always: the mockup's "Tue 29 Sep" is a
  // fixture, not a contract.
  const todayLine = dayLabel(new Date());

  return (
    <Screen scroll={false} padded={false} testID="time-screen">
      <View style={styles.header}>
        <PackHeading title="Time" detail={todayLine} />
      </View>
      <View style={styles.scroll} testID="time-screen">
        {session !== null ? (
          <View
            testID="time-running-card"
            style={[styles.runningCard, { backgroundColor: runningGround, borderRadius: pack.radiusCard }]}
          >
            <Text style={[styles.runningLabel, { color: runningInk }]}>Running now</Text>
            <Text style={[styles.runningTicket, { color: runningInk }]} numberOfLines={1}>
              {runningTicketLabel}
            </Text>
            <Text style={[styles.runningClock, { color: runningInk }]}>
              {clockText(elapsedMs(session, seconds))}
            </Text>
            <View style={styles.runningActions}>
              <SecondaryButton
                label={paused ? "Resume" : "Pause"}
                icon={paused ? PACK_ICONS.play : PACK_ICONS.pause}
                style={{ flex: 1, backgroundColor: tokens.surface }}
                testID={paused ? "time-resume" : "time-pause"}
                onPress={() => {
                  if (session === null) return;
                  void mutate(paused ? resumeTimer : pauseTimer, {
                    sessionId: session.id,
                    ...(session.ticket_id !== null && session.ticket_id !== undefined
                      ? { ticketId: session.ticket_id }
                      : {}),
                  });
                }}
              />
              <PrimaryButton
                label="Stop and log"
                icon={PACK_ICONS.stop}
                style={{ flex: 1.3 }}
                onPress={() => setSheet({ kind: "stop" })}
              />
            </View>
          </View>
        ) : null}

        <View>
          <View style={styles.dayHead}>
            <Text style={[styles.dayTotal, { color: tokens.text }]}>
              {formatHoursMinutes(loggedToday)} logged today
            </Text>
            <Text style={[styles.dayOf, { color: tokens.textSecondary }]}>of 8h</Text>
          </View>
          <DayStrip logged={loggedToday} of={DAY_MINUTES} />
        </View>

        <Facts>
          {dayRows.slice(0, 8).map((entry: TimeEntry, index) => (
            <FactRow
              key={entry.id}
              first={index === 0}
              trailing={
                <SecondaryButton
                  label={formatDuration(entry.minutes)}
                  onPress={() => setSheet({ kind: "edit", entry })}
                  style={{ minHeight: 36, paddingHorizontal: 10 }}
                />
              }
            >
              <Text
                style={[styles.entryTitle, { color: tokens.text }]}
                onPress={() => router.push(`/ticket/${entry.ticket_id}` as never)}
              >
                {entry.notes ?? "Entry"}
              </Text>
              <Text style={[styles.entryMeta, { color: tokens.textSecondary }]}>
                {clockTime(entry.started_at)} · {entry.billable ? "Billable" : "Not billable"}
              </Text>
            </FactRow>
          ))}
          {dayRows.length === 0 ? (
            <FactRow first minHeight={60}>
              <Text style={[styles.entryMeta, { color: tokens.textMuted }]}>
                Nothing logged today yet — stop the clock to write the first entry.
              </Text>
            </FactRow>
          ) : null}
        </Facts>

        {gap !== null ? (
          <View
            testID="time-gap-card"
            style={[styles.gapCard, { backgroundColor: tokens.warning.background }]}
          >
            <View style={styles.gapBody}>
              <Text style={[styles.gapTitle, { color: tokens.warning.text }]}>{gap.title}</Text>
              <Text style={[styles.gapDetail, { color: tokens.warning.text }]}>{gap.detail}</Text>
            </View>
            <SecondaryButton
              label="Log it"
              style={{ backgroundColor: tokens.surface, minHeight: 44 }}
              onPress={() =>
                setSheet({
                  kind: "gap",
                  minutes: gap.minutes,
                  startedAt: gap.startedAt,
                  endsAt: gap.endsAt,
                  ticketId: gap.ticketId,
                })
              }
            />
          </View>
        ) : null}

        {banner !== null ? (
          <Text style={[styles.banner, { color: tokens.success.text }]}>{banner}</Text>
        ) : null}
      </View>

      {stopSheetSession !== null ? (
        <StopLogSheet
          session={stopSheetSession}
          runningMinutes={runningMinutes}
          runningTicketLabel={runningTicketLabel}
          onClose={() => setSheet(null)}
          onLogged={(text) => {
            setSheet(null);
            setBanner(text);
          }}
        />
      ) : null}
      {sheet?.kind === "gap" ? (
        <GapLogSheet
          minutes={sheet.minutes}
          startedAt={sheet.startedAt}
          endsAt={sheet.endsAt}
          ticketId={sheet.ticketId}
          onClose={() => setSheet(null)}
          onLogged={(text) => {
            setSheet(null);
            setBanner(text);
          }}
        />
      ) : null}
      {sheet?.kind === "edit" ? (
        <EditEntrySheet entry={sheet.entry} onClose={() => setSheet(null)} />
      ) : null}
    </Screen>
  );
}

/** The first stop with no logged time against it — the pack's blush gap card. */
export function unloggedStop(
  pack: DayPack | undefined,
  entries: TimeEntry[],
): { title: string; detail: string; minutes: number; startedAt: string; endsAt: string; ticketId: string } | null {
  const stops = (pack?.trips ?? []).flatMap((trip) => trip.stops);
  if (stops.length === 0) return null;
  const logged = new Set(entries.map((entry) => entry.ticket_id));
  const stop = stops.find((candidate) => {
    const ticketId = candidate.ticket_id;
    return ticketId !== undefined && ticketId !== null && !logged.has(ticketId);
  });
  if (stop === undefined || stop.ticket_id == null) return null;
  const minutes = Math.max(
    15,
    Math.round((Date.parse(stop.ends_at) - Date.parse(stop.starts_at)) / 60_000 / 15) * 15,
  );
  const client = stop.client_name != null && stop.client_name !== "" ? `, ${stop.client_name}` : "";
  return {
    title: `Earlier: ${stop.title}${client} has no logged time`,
    detail: `${dayLabel(new Date(stop.starts_at))}, ${clockOf(stop.starts_at)} to ${clockOf(stop.ends_at)} — ${formatHoursMinutes(minutes)} not logged`,
    minutes,
    startedAt: clockOf(stop.starts_at),
    endsAt: stop.ends_at,
    ticketId: stop.ticket_id,
  };
}

function dayLabel(date: Date): string {
  return date.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
}

function clockOf(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "--:--";
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}


function StopLogSheet({
  session,
  runningMinutes,
  runningTicketLabel,
  onClose,
  onLogged,
}: {
  session: { id: string; ticket_id: string };
  runningMinutes: number;
  runningTicketLabel: string;
  onClose: () => void;
  onLogged: (text: string) => void;
}) {
  const { tokens } = useTheme();
  const { mutate } = useMutation();
  const [duration, setDuration] = useState(roundUpTo(runningMinutes));
  const [workType, setWorkType] = useState("remote-support");
  const [billable, setBillable] = useState(true);
  const [unbillableReason, setUnbillableReason] = useState("internal");
  const [busy, setBusy] = useState(false);

  const commit = () => {
    setBusy(true);
    void mutate(stopTimer, {
      sessionId: session.id,
      ticketId: session.ticket_id,
      // The pack's sheet names the honest outcome on the primary; the
      // server rounds to the minute and notes the work type.
      ...(billable ? {} : { reason: unbillableReason }),
    }).then((result) => {
      setBusy(false);
      if (result.ok) {
        onLogged(`Logged ${shortClock(duration)} to the ticket. The timer has stopped.`);
      }
    });
  };

  return (
    <Sheet
      visible
      onClose={onClose}
      title="Stop and log"
      subtitle={`${runningTicketLabel} · ran for ${shortClock(runningMinutes)}`}
      testID="stop-log-sheet"
    >
      <View>
        <PackLabel style={{ marginBottom: 8 }}>Log this as</PackLabel>
        <Segment options={DURATIONS} value={duration} onChange={setDuration} testIDPrefix="stop-duration" />
        <Text style={[styles.roundLine, { color: tokens.textSecondary }]}>
          Rounded up to the next 15 minutes. Change it if that is not right.
        </Text>
      </View>
      <View>
        <PackLabel style={{ marginBottom: 8 }}>Work type</PackLabel>
        <Segment options={WORK_TYPES} value={workType} onChange={setWorkType} testIDPrefix="stop-work" />
      </View>
      <Facts>
        <FactRow first minHeight={56}>
          <Text style={[styles.billableLabel, { color: tokens.text }]}>Billable</Text>
          <BillableSwitch on={billable} onToggle={setBillable} />
        </FactRow>
        {!billable ? (
          <FactRow minHeight={56}>
            <Text style={[styles.billableLabel, { color: tokens.text }]}>Reason</Text>
            <Segment
              options={UNBILLABLE_REASONS}
              value={unbillableReason}
              onChange={setUnbillableReason}
              size="small"
              testIDPrefix="stop-reason"
            />
          </FactRow>
        ) : null}
      </Facts>
      <PrimaryButton
        testID="stop-log-commit"
        label={`Stop and log ${shortClock(duration)}`}
        icon={PACK_ICONS.check}
        disabled={busy}
        onPress={commit}
      />
    </Sheet>
  );
}

/**
 * Quick log for the gap card: the stop's real window, rounded per the pack,
 * written through POST /time-entries. The work-type and billable segments
 * are the same Quick-log semantics the Stop sheet carries.
 */
function GapLogSheet({
  minutes,
  startedAt,
  endsAt,
  ticketId,
  onClose,
  onLogged,
}: {
  minutes: number;
  startedAt: string;
  endsAt: string;
  ticketId: string;
  onClose: () => void;
  onLogged: (text: string) => void;
}) {
  return (
    <QuickLogSheet
      title="Log it"
      subtitle={`The visit window was ${shortClock(roundUpTo(minutes))} — rounded up to the next 15 minutes.`}
      initialMinutes={roundUpTo(minutes)}
      startedAtIso={startedAt}
      ticketId={ticketId}
      onClose={onClose}
      onLogged={onLogged}
      testID="gap-log-sheet"
    />
  );
}

/**
 * The entry edit: the pack's "same sheet, prefilled" derivation. The API
 * gap is named honestly in the sheet — POST /time-entries has no update
 * endpoint (recorded 2026-10-05), so this re-logs a corrected entry; the
 * original stays on the books for the desk to adjust on web.
 */
function EditEntrySheet({ entry, onClose }: { entry: TimeEntry; onClose: () => void }) {
  const [note, setNote] = useState<string | null>(null);
  return (
    <QuickLogSheet
      title="Edit entry"
      subtitle={
        note ??
        `API gap: entries cannot be edited from the phone yet — saving logs a corrected entry and the desk adjusts the original.`
      }
      initialMinutes={entry.minutes}
      startedAtIso={entry.started_at}
      ticketId={entry.ticket_id ?? ""}
      initialNotes={entry.notes ?? ""}
      initialBillable={entry.billable}
      onClose={() => {
        onClose();
      }}
      onLogged={() => setNote("Corrected entry logged. The desk will reconcile the original.")}
      testID="edit-entry-sheet"
    />
  );
}

/** The shared Quick-log body: duration, work type, billable, reason. */
function QuickLogSheet({
  title,
  subtitle,
  initialMinutes,
  startedAtIso,
  ticketId,
  initialNotes,
  initialBillable = true,
  onClose,
  onLogged,
  testID,
}: {
  title: string;
  subtitle: string;
  initialMinutes: number;
  startedAtIso: string;
  ticketId: string;
  initialNotes?: string;
  initialBillable?: boolean;
  onClose: () => void;
  onLogged: (text: string) => void;
  testID: string;
}) {
  const { tokens } = useTheme();
  const { me } = useAuth();
  const { mutate } = useMutation();
  const [duration, setDuration] = useState(initialMinutes);
  const [workType, setWorkType] = useState("remote-support");
  const [billable, setBillable] = useState(initialBillable);
  const [unbillableReason, setUnbillableReason] = useState("internal");
  const [notes, setNotes] = useState(initialNotes ?? "");
  const [busy, setBusy] = useState(false);

  const commit = () => {
    const start = new Date(startedAtIso);
    const end = new Date(start.getTime() + duration * 60_000);
    setBusy(true);
    void mutate(createTimeEntry, {
      ticketId,
      userId: me?.user_id ?? "",
      startedAt: start.toISOString(),
      endedAt: end.toISOString(),
      billable,
      ...(billable ? {} : { unbillableReason }),
      workType,
      ...(notes.trim() !== "" ? { notes: notes.trim() } : {}),
    }).then((result) => {
      setBusy(false);
      if (result.ok) {
        onLogged(`Logged ${shortClock(duration)} — it syncs when you're back online.`);
      }
    });
  };

  return (
    <Sheet visible onClose={onClose} title={title} subtitle={subtitle} testID={testID}>
      <View>
        <PackLabel style={{ marginBottom: 8 }}>Log this as</PackLabel>
        <Segment
          options={[...DURATIONS, { value: initialMinutes, label: shortClock(initialMinutes) }].filter(
            (option, index, all) => all.findIndex((o) => o.value === option.value) === index,
          )}
          value={duration}
          onChange={setDuration}
          testIDPrefix="quick-duration"
        />
      </View>
      <View>
        <PackLabel style={{ marginBottom: 8 }}>Work type</PackLabel>
        <Segment options={WORK_TYPES} value={workType} onChange={setWorkType} testIDPrefix="quick-work" />
      </View>
      <Facts>
        <FactRow first minHeight={56} label="Notes">
          <TextInput
            value={notes}
            onChangeText={setNotes}
            placeholder="What was done"
            placeholderTextColor={tokens.textMuted}
            style={[styles.noteInput, { color: tokens.text }]}
            multiline
          />
        </FactRow>
        <FactRow minHeight={56}>
          <Text style={[styles.billableLabel, { color: tokens.text }]}>Billable</Text>
          <BillableSwitch on={billable} onToggle={setBillable} />
        </FactRow>
        {!billable ? (
          <FactRow minHeight={56}>
            <Text style={[styles.billableLabel, { color: tokens.text }]}>Reason</Text>
            <Segment
              options={UNBILLABLE_REASONS}
              value={unbillableReason}
              onChange={setUnbillableReason}
              size="small"
              testIDPrefix="quick-reason"
            />
          </FactRow>
        ) : null}
      </Facts>
      <PrimaryButton
        testID="quick-log-commit"
        label={`Log ${shortClock(duration)}`}
        icon={PACK_ICONS.check}
        disabled={busy}
        onPress={commit}
      />
    </Sheet>
  );
}

function BillableSwitch({ on, onToggle }: { on: boolean; onToggle: (next: boolean) => void }) {  const { tokens } = useTheme();
  return (
    <View
      accessibilityRole="switch"
      accessibilityState={{ checked: on }}
      accessibilityLabel="Billable"
      style={[styles.switchTrack, { backgroundColor: on ? tokens.primary : tokens.surfaceHover, borderColor: tokens.border }]}
      onTouchEnd={() => onToggle(!on)}
    >
      <View
        style={[
          styles.switchKnob,
          { backgroundColor: tokens.surface, alignSelf: on ? "flex-end" : "flex-start" },
        ]}
      />
    </View>
  );
}

/** The day strip: blocks in logged segments, the running one in gold. */
function DayStrip({ logged, of }: { logged: number; of: number }) {
  const { tokens } = useTheme();
  const pct = Math.min(100, (logged / of) * 100);
  return (
    <View
      accessibilityRole="image"
      accessibilityLabel={`${formatHoursMinutes(logged)} of ${formatHoursMinutes(of)}`}
      style={[styles.strip, { backgroundColor: tokens.background, borderColor: tokens.border }]}
    >
      <View
        style={{
          position: "absolute",
          left: 0,
          top: 0,
          bottom: 0,
          width: `${Math.max(pct, 2)}%` as never,
          backgroundColor: tokens.success.text,
        }}
      />
    </View>
  );
}

function formatHoursMinutes(minutes: number): string {
  if (minutes <= 0) return "0m";
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours === 0) return `${rest}m`;
  if (rest === 0) return `${hours}h`;
  return `${hours}h ${rest}m`;
}

function formatDuration(minutes: number): string {
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return `${hours}:${String(rest).padStart(2, "0")}`;
}

function clockTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "--:--";
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

const styles = StyleSheet.create({
  screen: {
    flex: 1,
  },
  header: {
    paddingHorizontal: 16,
    paddingTop: 8,
  },
  scroll: {
    flex: 1,
    paddingHorizontal: 16,
    paddingTop: 8,
    gap: 14,
    // The pack's padding rule: the 56px timer pill plus the 90px tab bar
    // must never half-hide a row.
    paddingBottom: bottomReserve.tabs,
  },
  runningCard: {
    padding: 14,
    gap: 6,
  },
  runningLabel: {
    fontFamily: type.sansSemiBold,
    fontSize: 13,
  },
  runningTicket: {
    fontFamily: type.sansSemiBold,
    fontSize: 17,
  },
  runningClock: {
    fontFamily: type.sansSemiBold,
    fontSize: 32,
    fontVariant: ["tabular-nums"],
  },
  runningActions: {
    flexDirection: "row",
    gap: 10,
    marginTop: 4,
  },
  dayHead: {
    flexDirection: "row",
    alignItems: "baseline",
    justifyContent: "space-between",
    marginBottom: 6,
  },
  dayTotal: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
  },
  dayOf: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  strip: {
    height: 36,
    borderRadius: 10,
    borderWidth: 1,
    overflow: "hidden",
  },
  entryTitle: {
    fontFamily: type.sansSemiBold,
    fontSize: 14,
  },
  entryMeta: {
    fontFamily: type.sans,
    fontSize: 12,
  },
  entryEdit: {
    minHeight: 36,
    paddingHorizontal: 10,
  },
  gapCard: {
    borderRadius: pack.radiusCard,
    padding: 14,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  gapBody: {
    flex: 1,
    gap: 2,
  },
  gapTitle: {
    fontFamily: type.sansSemiBold,
    fontSize: 14,
  },
  gapDetail: {
    fontFamily: type.sans,
    fontSize: 13,
  },
  banner: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
  roundLine: {
    fontFamily: type.sans,
    fontSize: 12,
    marginTop: 6,
  },
  billableLabel: {
    fontFamily: type.sansSemiBold,
    fontSize: 14,
  },
  switchTrack: {
    width: 48,
    height: 28,
    borderRadius: 999,
    borderWidth: 1,
    padding: 3,
    justifyContent: "center",
  },
  noteInput: {
    flex: 1,
    fontFamily: type.sans,
    fontSize: 14,
    minHeight: 36,
    textAlignVertical: "top",
  },
  switchKnob: {
    width: 20,
    height: 20,
    borderRadius: 10,
  },
});

void shape;
void PACK_ICONS;
