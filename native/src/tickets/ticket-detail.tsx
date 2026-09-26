// Ticket detail — the workbar IA on a phone: clock, notes, photos, time.
//
// Purpose: the live-sync proof surface. Notes are comments (queued offline
// with a stable idempotency key, optimistic append, authoritative reconcile
// on the server snapshot), the timer is the org's timer session (start /
// pause / stop through the same queue), photos stage through the upload
// endpoint before the note links them, and the time entries section shows
// what the day already booked. Every inbound ticket/comment/timer event
// marks these models stale so the pane re-renders without a pull-to-refresh.
// 404 and 409 render the same honest copy the web app renders.
//
// Non-purpose: no board mechanics or field workflows (lanes 2/3).

import { useCallback, useEffect, useMemo, useState } from "react";
import { router } from "expo-router";
import { Image, Pressable, StyleSheet, Text, View } from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";

import { useAuth } from "@/auth/auth-context";
import { API_URL } from "@/config";
import {
  MODELS,
  fetchActiveTimer,
  fetchStockItems,
  fetchTicket,
  fetchTicketAttachments,
  fetchTicketComments,
  fetchTimeEntries,
  fetchTicketStockMovements,
  type StockItemView,
  type TicketAttachment,
  type TicketComment,
  type TicketStockMovement,
  type TimeEntry,
} from "@/core/models";
import { REASON_COPY } from "@/core/errors";
import {
  addTicketNote,
  pauseTimer,
  resumeTimer,
  startTimer,
  stopTimer,
} from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
import {
  ATTACH_INJECTOR_ENABLED,
  AttachmentUploadError,
  attachmentUrl,
  formatBytes,
  pickPhoto,
  samplePhoto,
  uploadTicketAttachment,
  type PickedPhoto,
  type StagedAttachment,
} from "@/tickets/attachments";
import {
  Button,
  Card,
  EmptyState,
  ErrorNote,
  Field,
  Loading,
  Mono,
  Pill,
  SectionHeader,
} from "@/ui/primitives";
import { elapsedMs, clock, isPaused } from "@/ui/timer-bar";
import { infoPair, shape, type, useTheme, type StatusPair } from "@/ui/tokens";

type NoteKind = "internal" | "client_visible" | "in_house_update";

const NOTE_KINDS: { kind: NoteKind; label: string }[] = [
  { kind: "internal", label: "Internal" },
  { kind: "client_visible", label: "Client" },
  { kind: "in_house_update", label: "Update" },
];

export function TicketDetail({ id }: { id: string }) {
  const { tokens } = useTheme();
  const { mutate } = useMutation();
  const { accessToken } = useAuth();
  const ticket = useModel(MODELS.ticket(id), useCallback((client) => fetchTicket(client, id), [id]), {
    maxAgeMs: 15_000,
  });
  const comments = useModel(
    MODELS.ticketComments(id),
    useCallback((client) => fetchTicketComments(client, id), [id]),
    { maxAgeMs: 15_000 },
  );
  const time = useModel(
    MODELS.timeEntries(id),
    useCallback((client) => fetchTimeEntries(client, id), [id]),
    { maxAgeMs: 30_000 },
  );
  const attachments = useModel(
    MODELS.ticketAttachments(id),
    useCallback((client) => fetchTicketAttachments(client, id), [id]),
    { maxAgeMs: 60_000 },
  );
  const timer = useModel(MODELS.activeTimer, fetchActiveTimer);
  const movements = useModel(
    MODELS.ticketStockMovements(id),
    useCallback((client) => fetchTicketStockMovements(client, id), [id]),
    { maxAgeMs: 30_000 },
  );
  const catalog = useModel(MODELS.stockItems, fetchStockItems, { maxAgeMs: 5 * 60_000 });

  const [body, setBody] = useState("");
  const [kind, setKind] = useState<NoteKind>("internal");
  const [banner, setBanner] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [staged, setStaged] = useState<StagedAttachment[]>([]);
  const [uploading, setUploading] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  const token = accessToken();
  const runningHere =
    timer.data != null && timer.data.ticket_id === id ? timer.data : null;
  const paused = runningHere !== null && isPaused(runningHere);

  const notFound =
    ticket.status === "error" &&
    ticket.data === undefined &&
    ticket.error?.reason === "not_found";

  useEffect(() => {
    if (banner === null) return;
    const timeout = setTimeout(() => setBanner(null), 6_000);
    return () => clearTimeout(timeout);
  }, [banner]);

  useEffect(() => {
    if (runningHere === null || paused) return;
    const interval = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(interval);
  }, [runningHere, paused]);

  const sortedComments = useMemo(() => {
    const rows = comments.data ?? [];
    return [...rows].sort((a, b) => a.created_at.localeCompare(b.created_at));
  }, [comments.data]);

  const entries = useMemo(() => {
    const rows = time.data ?? [];
    return [...rows].sort((a, b) => b.started_at.localeCompare(a.started_at));
  }, [time.data]);

  const totalMinutes = useMemo(
    () => entries.reduce((sum, entry) => sum + entry.minutes, 0),
    [entries],
  );

  const attachmentsByComment = useMemo(() => {
    const map = new Map<string, TicketAttachment[]>();
    for (const attachment of attachments.data ?? []) {
      const commentId = attachment.ticket_comment_id;
      if (commentId === null || commentId === undefined) continue;
      const rows = map.get(commentId) ?? [];
      rows.push(attachment);
      map.set(commentId, rows);
    }
    return map;
  }, [attachments.data]);

  const stage = useCallback(
    async (file: PickedPhoto) => {
      const access = accessToken();
      if (access === null) {
        setBanner("Sign in again to attach photos.");
        return;
      }
      setUploading(true);
      setBanner(null);
      try {
        const uploaded = await uploadTicketAttachment({
          apiUrl: API_URL,
          accessToken: access,
          ticketId: id,
          file,
        });
        setStaged((current) => [...current, uploaded]);
      } catch (error) {
        setBanner(
          error instanceof AttachmentUploadError
            ? error.message
            : "The photo upload didn't complete.",
        );
      } finally {
        setUploading(false);
      }
    },
    [accessToken, id],
  );

  const attachPhoto = useCallback(async () => {
    try {
      const file = await pickPhoto();
      if (file !== null) await stage(file);
    } catch (error) {
      setBanner(error instanceof Error ? error.message : "The photo picker didn't open.");
    }
  }, [stage]);

  const send = useCallback(async () => {
    const text = body.trim();
    if (text.length === 0) return;
    setSending(true);
    setBanner(null);
    // The operation itself renders the note optimistically (pending id) and
    // reconciles from the server snapshot; a rejection reverts it.
    const result = await mutate(addTicketNote, {
      ticketId: id,
      ...(ticket.data !== undefined ? { ticketNumber: ticket.data.number } : {}),
      body: text,
      kind,
      ...(staged.length > 0 ? { attachmentIds: staged.map((item) => item.id) } : {}),
    });
    setSending(false);
    if (result.ok && result.queued) {
      setBody("");
      setStaged([]);
      setBanner("Saved on this device — will sync when you're back online.");
      return;
    }
    if (result.ok) {
      setBody("");
      setStaged([]);
      return;
    }
    setBanner(result.failure?.text ?? "The note couldn't be saved.");
  }, [body, id, kind, mutate, staged, ticket.data]);

  if (notFound) {
    return (
      <View style={[styles.screen, { backgroundColor: tokens.background }]}>
        <EmptyState message={REASON_COPY.not_found ?? "That ticket no longer exists."} />
      </View>
    );
  }

  if (ticket.status === "loading" && ticket.data === undefined) {
    return (
      <View style={[styles.screen, { backgroundColor: tokens.background }]}>
        <Loading label="Loading ticket" />
      </View>
    );
  }

  return (
    <View style={[styles.screen, { backgroundColor: tokens.background }]}>
      <View style={styles.header}>
        {ticket.data !== undefined ? (
          <>
            <View style={styles.headerRow}>
              <Text style={[styles.number, { color: tokens.textSecondary }]}>
                {ticket.data.number}
              </Text>
              {ticket.data.priority !== undefined && ticket.data.priority !== null ? (
                <Pill
                  pair={priorityPair(tokens, String(ticket.data.priority))}
                  label={String(ticket.data.priority).toUpperCase()}
                />
              ) : null}
            </View>
            <Text style={[styles.title, { color: tokens.text }]}>{ticket.data.title}</Text>
          </>
        ) : (
          <Text style={[styles.title, { color: tokens.text }]}>Ticket</Text>
        )}
      </View>

      {ticket.error !== undefined && ticket.data !== undefined ? (
        <ErrorNote text={ticket.error.text} onRetry={ticket.refresh} />
      ) : null}
      {banner !== null ? <ErrorNote text={banner} /> : null}

      <Card testID="ticket-timer">
        <View style={styles.timerRow}>
          <View style={styles.timerInfo}>
            <Text style={[styles.metaLabel, { color: tokens.textMuted }]}>CLOCK</Text>
            <Text
              style={[styles.clock, { color: runningHere !== null ? tokens.text : tokens.textMuted }]}
            >
              {runningHere !== null ? clock(elapsedMs(runningHere, now)) : "00:00:00"}
            </Text>
            <Text style={[styles.timerState, { color: tokens.textSecondary }]}>
              {runningHere !== null
                ? paused
                  ? "Paused on this ticket"
                  : "Running on this ticket"
                : timer.data != null
                  ? "Running on another ticket"
                  : "Not running"}
            </Text>
          </View>
          <View style={styles.timerActions}>
            {runningHere !== null ? (
              <>
                <Button
                  label={paused ? "Resume" : "Pause"}
                  variant="ghost"
                  testID={paused ? "ticket-timer-resume" : "ticket-timer-pause"}
                  onPress={() => {
                    void mutate(paused ? resumeTimer : pauseTimer, {
                      sessionId: runningHere.id,
                      ticketId: id,
                    }).then((result) =>
                      setBanner(
                        result.ok ? null : (result.failure?.text ?? "Couldn't change the timer."),
                      ),
                    );
                  }}
                />
                <Button
                  label="Stop"
                  variant="secondary"
                  testID="ticket-timer-stop"
                  onPress={() => {
                    void mutate(stopTimer, {
                      sessionId: runningHere.id,
                      ticketId: id,
                    }).then((result) =>
                      setBanner(result.ok ? null : (result.failure?.text ?? "Couldn't stop the timer.")),
                    );
                  }}
                />
              </>
            ) : (
              <Button
                label="Clock in"
                testID="ticket-timer-start"
                disabled={timer.data != null}
                onPress={() => {
                  void mutate(startTimer, {
                    ticketId: id,
                    ...(ticket.data !== undefined ? { ticketNumber: ticket.data.number } : {}),
                  }).then((result) =>
                    setBanner(result.ok ? null : (result.failure?.text ?? "Couldn't start the clock.")),
                  );
                }}
              />
            )}
          </View>
        </View>
      </Card>

      <SectionHeader
        title={`Time${totalMinutes > 0 ? ` · ${formatMinutes(totalMinutes)}` : ""}`}
      />
      {time.status === "error" && time.data === undefined ? (
        <ErrorNote text={time.error?.text ?? "Couldn't load time entries."} onRetry={time.refresh} />
      ) : null}
      {entries.length === 0 && time.data !== undefined ? (
        <EmptyState message="No time booked on this ticket yet — the clock writes the first entry." />
      ) : null}
      {entries.slice(0, 6).map((entry: TimeEntry) => (
        <Card key={entry.id} style={styles.entry} testID={`time-entry-${entry.id}`}>
          <View style={styles.entryRow}>
            <Text style={[styles.entryMinutes, { color: tokens.text }]}>
              {formatMinutes(entry.minutes)}
            </Text>
            <Mono muted>
              {shortDate(entry.started_at)} · {clockTime(entry.started_at)}–
              {clockTime(entry.ended_at)}
            </Mono>
          </View>
          <View style={styles.entryMeta}>
            {entry.billable ? <Pill pair={tokens.success} label="BILLABLE" /> : null}
            {entry.approval_status !== null && entry.approval_status !== undefined ? (
              <Pill pair={approvalPair(tokens, entry.approval_status)} label={entry.approval_status.toUpperCase()} />
            ) : null}
            {entry.notes !== null && entry.notes !== undefined && entry.notes !== "" ? (
              <Text style={[styles.entryNote, { color: tokens.textMuted }]} numberOfLines={1}>
                {entry.notes}
              </Text>
            ) : null}
          </View>
        </Card>
      ))}

      <SectionHeader title="Parts & shipping" />
      <Card testID="ticket-parts">
        <View style={styles.partsActions}>
          <Button
            label="Use a part"
            testID="ticket-use-part"
            onPress={() => router.push(`/move?mode=consume&ticket=${id}` as never)}
          />
          <Button
            label="Receive"
            variant="secondary"
            testID="ticket-receive-part"
            onPress={() => router.push(`/move?mode=receive&ticket=${id}` as never)}
          />
        </View>
        {(movements.data ?? []).length === 0 ? (
          <Text style={[styles.partsEmpty, { color: tokens.textMuted }]}>
            No parts movements on this job yet. A scan stages the part; you confirm the
            movement.
          </Text>
        ) : (
          (movements.data ?? []).slice(0, 6).map((movement: TicketStockMovement) => {
            const itemName =
              (catalog.data ?? []).find(
                (candidate: StockItemView) => candidate.id === movement.stock_item_id,
              )?.name ?? "Item";
            const shipped = movement.kind === "ship_out";
            return (
              <View key={movement.id} style={styles.markerRow} testID={`marker-${movement.id}`}>
                <View style={styles.markerBody}>
                  <View style={styles.markerHeader}>
                    <Pill
                      pair={markerPair(tokens, movement.kind)}
                      label={movement.kind.replace(/_/g, " ").toUpperCase()}
                    />
                    <Mono muted>{formatTime(movement.created_at)}</Mono>
                  </View>
                  <View style={styles.markerLine}>
                    <Text style={[styles.markerName, { color: tokens.text }]} numberOfLines={1}>
                      {itemName}
                    </Text>
                    <Mono>×{formatQty(Math.abs(movement.qty_milli))}</Mono>
                  </View>
                  {shipped ? (
                    <Mono muted>
                      {[movement.carrier, movement.tracking_ref]
                        .filter((value): value is string => value != null && value !== "")
                        .join(" · ") || "No carrier on file"}
                    </Mono>
                  ) : null}
                </View>
                {shipped ? (
                  <Button
                    label="Confirm"
                    variant="secondary"
                    testID={`confirm-shipment-${movement.id}`}
                    onPress={() =>
                      router.push(
                        `/move?mode=receive&ticket=${id}&item=${movement.stock_item_id}` as never,
                      )
                    }
                  />
                ) : null}
              </View>
            );
          })
        )}
        {movements.offline && movements.data !== undefined ? (
          <Text style={[styles.partsEmpty, { color: tokens.textMuted }]}>
            Showing the last sync from this device.
          </Text>
        ) : null}
      </Card>

      <SectionHeader title={`Notes${sortedComments.length > 0 ? ` (${sortedComments.length})` : ""}`} />
      {comments.status === "error" && comments.data === undefined ? (
        <ErrorNote text={comments.error?.text ?? "Couldn't load notes."} onRetry={comments.refresh} />
      ) : null}
      {comments.data !== undefined && sortedComments.length === 0 ? (
        <EmptyState message="No notes yet — the first one starts the record." />
      ) : null}
      {sortedComments.map((comment: TicketComment) => {
        const files = attachmentsByComment.get(comment.id) ?? [];
        return (
          <Card key={comment.id} style={styles.comment} testID={`comment-${comment.id}`}>
            <View style={styles.commentHeader}>
              <View style={styles.commentPills}>
                <Pill pair={noteKindPair(tokens, comment.kind)} label={noteKindLabel(comment.kind)} />
                {comment.id.startsWith("pending-") ? (
                  <Pill pair={tokens.warning} label="QUEUED" />
                ) : null}
              </View>
              <Mono muted>{formatTime(comment.created_at)}</Mono>
            </View>
            <Text style={[styles.commentBody, { color: tokens.text }]}>{comment.body}</Text>
            {files.length > 0 ? (
              <View style={styles.photoRow}>
                {files.map((attachment) =>
                  token !== null && isImage(attachment.content_type) ? (
                    <Image
                      key={attachment.id}
                      testID={`comment-photo-${attachment.id}`}
                      source={{
                        uri: attachmentUrl(API_URL, attachment.id),
                        headers: { Authorization: `Bearer ${token}` },
                      }}
                      style={[styles.photo, { borderColor: tokens.border }]}
                      accessibilityLabel={attachment.filename}
                    />
                  ) : (
                    <View
                      key={attachment.id}
                      style={[styles.fileChip, { borderColor: tokens.border, backgroundColor: tokens.surfaceHover }]}
                    >
                      <Ionicons name="document-outline" size={14} color={tokens.textSecondary} />
                      <Text style={[styles.fileName, { color: tokens.textSecondary }]} numberOfLines={1}>
                        {attachment.filename}
                      </Text>
                      <Text style={[styles.fileName, { color: tokens.textMuted }]}>
                        {formatBytes(attachment.size_bytes)}
                      </Text>
                    </View>
                  ),
                )}
              </View>
            ) : null}
          </Card>
        );
      })}

      <Card style={styles.composer}>
        <Text style={[styles.metaLabel, { color: tokens.textMuted }]}>ADD A NOTE</Text>
        <View style={styles.kindRow}>
          {NOTE_KINDS.map((option) => {
            const selected = option.kind === kind;
            return (
              <Pressable
                key={option.kind}
                testID={`note-kind-${option.kind}`}
                onPress={() => setKind(option.kind)}
                accessibilityRole="button"
                accessibilityState={{ selected }}
                style={[
                  styles.kindChip,
                  {
                    backgroundColor: tokens.surfaceHover,
                    borderColor: selected ? tokens.primary : tokens.border,
                  },
                ]}
              >
                <Text
                  style={[
                    styles.kindChipText,
                    { color: selected ? tokens.text : tokens.textSecondary },
                  ]}
                >
                  {option.label}
                </Text>
              </Pressable>
            );
          })}
        </View>
        <Field
          testID="note-input"
          value={body}
          onChangeText={setBody}
          placeholder="What happened on site?"
          multiline
          numberOfLines={3}
          style={styles.noteInput}
          submitBehavior="submit"
          onSubmitEditing={() => {
            void send();
          }}
        />
        {staged.length > 0 ? (
          <View style={styles.photoRow}>
            {staged.map((attachment) => (
              <View
                key={attachment.id}
                testID={`staged-photo-${attachment.id}`}
                style={[styles.fileChip, { borderColor: tokens.primary, backgroundColor: tokens.primary }]}
              >
                <Ionicons name="image-outline" size={14} color={tokens.primaryText} />
                <Text style={[styles.fileName, { color: tokens.primaryText }]} numberOfLines={1}>
                  {attachment.filename}
                </Text>
                <Pressable
                  testID={`staged-remove-${attachment.id}`}
                  hitSlop={10}
                  onPress={() =>
                    setStaged((current) => current.filter((item) => item.id !== attachment.id))
                  }
                  accessibilityRole="button"
                  accessibilityLabel={`Remove ${attachment.filename}`}
                >
                  <Ionicons name="close" size={14} color={tokens.primaryText} />
                </Pressable>
              </View>
            ))}
          </View>
        ) : null}
        <View style={styles.composerActions}>
          <Button
            label={uploading ? "Uploading…" : "Photo"}
            variant="ghost"
            testID="note-attach-photo"
            disabled={uploading || sending}
            onPress={() => {
              void attachPhoto();
            }}
          />
          {ATTACH_INJECTOR_ENABLED ? (
            <Button
              label="Sample photo"
              variant="ghost"
              testID="note-attach-sample"
              disabled={uploading || sending}
              onPress={() => {
                void stage(samplePhoto());
              }}
            />
          ) : null}
          <View style={styles.composerSpacer} />
          <Button
            label="Add note"
            testID="note-submit"
            loading={sending}
            disabled={body.trim().length === 0 || sending || uploading}
            onPress={() => {
              void send();
            }}
          />
        </View>
      </Card>
    </View>
  );
}

function noteKindLabel(kind: string): string {
  if (kind === "client_visible") return "CLIENT";
  if (kind === "in_house_update") return "UPDATE";
  return "INTERNAL";
}

function priorityPair(
  tokens: ReturnType<typeof useTheme>["tokens"],
  priority: string,
): StatusPair {
  if (priority === "p1" || priority === "1" || priority.toLowerCase() === "urgent") {
    return tokens.danger;
  }
  if (priority === "p2" || priority === "2" || priority.toLowerCase() === "high") {
    return tokens.warning;
  }
  return tokens.neutral;
}

/** Movement quantities are milli-units; the field reads whole/loose units. */
function formatQty(qtyMilli: number): string {
  return (qtyMilli / 1000).toLocaleString(undefined, { maximumFractionDigits: 3 });
}

function noteKindPair(tokens: ReturnType<typeof useTheme>["tokens"], kind: string): StatusPair {
  if (kind === "client_visible") return infoPair(tokens);
  if (kind === "in_house_update") return tokens.warning;
  return tokens.neutral;
}

function approvalPair(
  tokens: ReturnType<typeof useTheme>["tokens"],
  status: string,
): StatusPair {
  if (status === "approved") return tokens.success;
  if (status === "rejected") return tokens.danger;
  if (status === "submitted") return tokens.warning;
  return tokens.neutral;
}

function isImage(contentType: string): boolean {
  return contentType.startsWith("image/");
}

function formatMinutes(minutes: number): string {
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`;
}

function shortDate(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function clockTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "--:--";
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

/** Ledger marker colors: outbound shipments warn, receipts succeed. */
function markerPair(
  tokens: ReturnType<typeof useTheme>["tokens"],
  kind: string,
): StatusPair {
  if (kind === "ship_out") return tokens.warning;
  if (kind === "receive" || kind === "checkin" || kind === "loan_return") {
    return tokens.success;
  }
  return tokens.neutral;
}

function formatTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

const styles = StyleSheet.create({
  screen: {
    flex: 1,
    width: "100%",
    // Home-indicator-era tablets stretch a detail pane past comfortable
    // measure; cap it and centre the column (S13).
    maxWidth: 760,
    alignSelf: "center",
    padding: shape.screenPadding,
    gap: 12,
  },
  header: {
    gap: 4,
  },
  headerRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  number: {
    fontFamily: type.mono,
    fontSize: 13,
    fontVariant: ["tabular-nums"],
  },
  title: {
    fontFamily: type.sansSemiBold,
    fontSize: 20,
  },
  timerRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  timerInfo: {
    gap: 2,
    flexShrink: 1,
  },
  timerActions: {
    flexDirection: "row",
    gap: 6,
  },
  metaLabel: {
    fontFamily: type.sansMedium,
    fontSize: 11,
    letterSpacing: 0.8,
  },
  clock: {
    fontFamily: type.monoMedium,
    fontSize: 26,
    fontVariant: ["tabular-nums"],
  },
  timerState: {
    fontFamily: type.sansMedium,
    fontSize: 13,
  },
  entry: {
    gap: 6,
  },
  entryRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
  },
  entryMinutes: {
    fontFamily: type.sansSemiBold,
    fontSize: 15,
    fontVariant: ["tabular-nums"],
  },
  entryMeta: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  entryNote: {
    fontFamily: type.sans,
    fontSize: 12,
    flex: 1,
  },
  partsActions: {
    flexDirection: "row",
    gap: 8,
  },
  partsEmpty: {
    fontFamily: type.sans,
    fontSize: 13,
    lineHeight: 19,
  },
  markerRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 44,
    paddingVertical: 4,
  },
  markerBody: {
    flex: 1,
    gap: 2,
  },
  markerHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
  },
  markerLine: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
  },
  markerName: {
    flex: 1,
    fontFamily: type.sansMedium,
    fontSize: 14,
  },
  comment: {
    gap: 6,
  },
  commentHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
  },
  commentPills: {
    flexDirection: "row",
    gap: 6,
  },
  commentBody: {
    fontFamily: type.sans,
    fontSize: 15,
    lineHeight: 21,
  },
  photoRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
  },
  photo: {
    width: 96,
    height: 96,
    borderRadius: 10,
    borderWidth: 1,
  },
  fileChip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 10,
    minHeight: 28,
    maxWidth: 240,
  },
  fileName: {
    fontFamily: type.sansMedium,
    fontSize: 12,
    flexShrink: 1,
  },
  composer: {
    gap: 10,
  },
  kindRow: {
    flexDirection: "row",
    gap: 8,
  },
  kindChip: {
    minHeight: 36,
    paddingHorizontal: 12,
    borderRadius: shape.radiusPill,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  kindChipText: {
    fontFamily: type.sansSemiBold,
    fontSize: 12,
  },
  noteInput: {
    minHeight: 88,
    textAlignVertical: "top",
    paddingTop: 10,
  },
  composerActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  composerSpacer: {
    flex: 1,
  },
});
