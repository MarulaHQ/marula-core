// Ticket detail — notes and the timer, both server-backed entities.
//
// Purpose: the live-sync proof surface. Notes are comments (queued offline
// with a stable idempotency key, optimistic append, authoritative reconcile
// on the server snapshot), the timer is the org's timer session (start /
// stop through the same queue), and every inbound ticket/comment/timer event
// marks these models stale so the pane re-renders without a pull-to-refresh.
// 404 and 409 render the same honest copy the web app renders.
//
// Non-purpose: no board mechanics or field workflows (lanes 2/3).

import { useCallback, useEffect, useMemo, useState } from "react";
import { router } from "expo-router";
import { Pressable, StyleSheet, Text, View } from "react-native";

import {
  MODELS,
  fetchActiveTimer,
  fetchStockItems,
  fetchTicket,
  fetchTicketComments,
  fetchTicketStockMovements,
  type StockItemView,
  type TicketComment,
  type TicketStockMovement,
} from "@/core/models";
import { REASON_COPY } from "@/core/errors";
import { addTicketNote, startTimer, stopTimer } from "@/core/writes";
import { useModel, useMutation } from "@/data/sync-context";
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
  const ticket = useModel(MODELS.ticket(id), useCallback((client) => fetchTicket(client, id), [id]), {
    maxAgeMs: 15_000,
  });
  const comments = useModel(
    MODELS.ticketComments(id),
    useCallback((client) => fetchTicketComments(client, id), [id]),
    { maxAgeMs: 15_000 },
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

  const runningHere =
    timer.data != null && timer.data.ticket_id === id ? timer.data : null;

  const notFound =
    ticket.status === "error" &&
    ticket.data === undefined &&
    ticket.error?.reason === "not_found";

  useEffect(() => {
    if (banner === null) return;
    const timeout = setTimeout(() => setBanner(null), 6_000);
    return () => clearTimeout(timeout);
  }, [banner]);

  const sortedComments = useMemo(() => {
    const rows = comments.data ?? [];
    return [...rows].sort((a, b) => a.created_at.localeCompare(b.created_at));
  }, [comments.data]);

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
    });
    setSending(false);
    if (result.ok && result.queued) {
      setBody("");
      setBanner("Saved on this device — will sync when you're back online.");
      return;
    }
    if (result.ok) {
      setBody("");
      return;
    }
    setBanner(result.failure?.text ?? "The note couldn't be saved.");
  }, [body, id, kind, mutate, ticket.data]);

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

      <Card>
        <View style={styles.timerRow}>
          <View style={styles.timerInfo}>
            <Text style={[styles.metaLabel, { color: tokens.textMuted }]}>TIMER</Text>
            <Text style={[styles.timerState, { color: tokens.text }]}>
              {runningHere !== null
                ? "Running on this ticket"
                : timer.data != null
                  ? "Running on another ticket"
                  : "Not running"}
            </Text>
          </View>
          {runningHere !== null ? (
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
          ) : (
            <Button
              label="Start timer"
              testID="ticket-timer-start"
              disabled={timer.data != null}
              onPress={() => {
                void mutate(startTimer, {
                  ticketId: id,
                  ...(ticket.data !== undefined ? { ticketNumber: ticket.data.number } : {}),
                }).then((result) =>
                  setBanner(result.ok ? null : (result.failure?.text ?? "Couldn't start the timer.")),
                );
              }}
            />
          )}
        </View>
      </Card>

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
                    <Mono>×{formatQty(movement.qty_milli)}</Mono>
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
      {sortedComments.map((comment: TicketComment) => (
        <Card key={comment.id} style={styles.comment}>
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
        </Card>
      ))}

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
                    backgroundColor: selected ? tokens.accent : tokens.surfaceHover,
                    borderColor: selected ? tokens.primary : tokens.border,
                  },
                ]}
              >
                <Text
                  style={[
                    styles.kindChipText,
                    { color: selected ? tokens.accentText : tokens.textSecondary },
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
        />
        <Button
          label="Add note"
          testID="note-submit"
          fullWidth
          loading={sending}
          disabled={body.trim().length === 0 || sending}
          onPress={() => {
            void send();
          }}
        />
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
  },
  metaLabel: {
    fontFamily: type.sansMedium,
    fontSize: 11,
    letterSpacing: 0.8,
  },
  timerState: {
    fontFamily: type.sansMedium,
    fontSize: 15,
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
});
