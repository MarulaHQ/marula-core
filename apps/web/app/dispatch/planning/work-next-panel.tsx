"use client";

// Curated per-technician work-next queue (Wave 34 claims close-out, claim 2).
//
// Purpose: the unassigned board answers "what is unassigned", not "what
// should THIS technician pick up next, in what order". This is the second
// question, and its ordering is deliberately NOT computed: a dispatcher
// dragging a ticket up has expressed an intent the product cannot infer
// from ticket fields, so the order is stored, not derived.
//
// Reordering is keyboard-accessible as well as draggable: the position
// buttons are the primary control and drag is the enhancement, because a
// queue a keyboard user cannot reorder is a queue half the technicians
// cannot use.
//
// Non-purpose: this is not a priority override. The order changes what a
// technician sees first and never what the board's P1–P4 filter or conflict
// detection believes.

import {
  Button,
  Card,
  CardHeader,
  EmptyState,
  Spinner,
  Tag,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useState } from "react";
import { api } from "../../../lib/api";

type QueueView = components["schemas"]["WorkNextQueueView"];
type QueueEntry = components["schemas"]["WorkNextEntryView"];

// Only the kit's four real tones (success | warning | danger | neutral) —
// p3 is neutral rather than a fourth colour, because the ramp has no
// "informational" step and inventing one would be an off-palette state.
const PRIORITY_TONE: Record<string, "danger" | "warning" | "neutral"> = {
  p1: "danger",
  p2: "warning",
  p3: "neutral",
  p4: "neutral",
};

export function WorkNextPanel({
  userId,
  onError,
}: {
  /** Whose queue. Omitted means the caller's own. */
  userId?: string;
  onError?: (message: string) => void;
}) {
  const [queue, setQueue] = useState<QueueView | null>(null);
  const [loading, setLoading] = useState(true);
  // Keyed by ticket_id, not index: two rapid moves on different rows must
  // not cancel each other's spinner.
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const res = await api.api.GET("/api/v1/dispatch/work-next", {
        params: { query: userId ? { user_id: userId } : {} },
      });
      if (res.data) setQueue(res.data as QueueView);
    } catch (e) {
      onError?.(e instanceof Error ? e.message : "could not load the queue");
    } finally {
      setLoading(false);
    }
  }, [userId, onError]);

  useEffect(() => {
    void load();
  }, [load]);

  /**
   * Move an entry and adopt the server's densified order wholesale.
   *
   * Adopting the response rather than patching one row is the point: the
   * move shifts everything between the old and new rank, so a client that
   * only updated the dragged row would leave every row it did not drag
   * showing a position the server has already renumbered.
   */
  const move = useCallback(
    async (entry: QueueEntry, to: number) => {
      if (!queue) return;
      if (to < 0 || to >= queue.entries.length || to === entry.position) return;
      setBusy(entry.ticket_id);
      try {
        const res = await api.api.POST("/api/v1/dispatch/work-next/reorder", {
          body: { user_id: queue.user_id, ticket_id: entry.ticket_id, position: to },
        });
        if (res.data) {
          const byTicket = new Map(
            (res.data as QueueEntry[]).map((r) => [r.ticket_id, r]),
          );
          setQueue({
            ...queue,
            entries: queue.entries
              .map((e) => {
                const updated = byTicket.get(e.ticket_id);
                return updated ? { ...e, position: updated.position } : e;
              })
              .sort((a, b) => a.position - b.position),
          });
        }
      } catch (e) {
        onError?.(e instanceof Error ? e.message : "could not reorder");
        await load();
      } finally {
        setBusy(null);
      }
    },
    [queue, onError, load],
  );

  const remove = useCallback(
    async (entry: QueueEntry) => {
      if (!queue) return;
      setBusy(entry.ticket_id);
      try {
        await api.api.DELETE("/api/v1/dispatch/work-next", {
          params: { query: { ticket_id: entry.ticket_id, user_id: queue.user_id } },
        });
        setQueue({
          ...queue,
          entries: queue.entries.filter((e) => e.ticket_id !== entry.ticket_id),
        });
      } catch (e) {
        onError?.(e instanceof Error ? e.message : "could not remove");
        await load();
      } finally {
        setBusy(null);
      }
    },
    [queue, onError, load],
  );

  if (loading) return <Spinner label="Loading the work queue" />;
  if (!queue) return null;

  return (
    <Card>
      <CardHeader>
        <h2 className="p-4 pb-0 text-sm font-semibold">
          Work next
          <span className="ml-3 font-normal" style={{ color: "var(--m-text-secondary)" }}>
            {queue.display_name
              ? `Curated order for ${queue.display_name}`
              : "Your curated order"}
          </span>
        </h2>
      </CardHeader>
      <p style={{ color: "var(--m-text-secondary)", fontSize: "12px" }}>
        The order a dispatcher set for this technician. It changes what is
        offered first and never a ticket&rsquo;s priority — the board&rsquo;s
        P1&ndash;P4 filter and conflict detection are unaffected.
      </p>

      {queue.entries.length === 0 ? (
        <EmptyState message="Nothing in this queue. Add tickets from the unassigned board to set the order a technician sees first." />
      ) : (
        <ol style={{ listStyle: "none", padding: 0, margin: "8px 0 0" }}>
          {queue.entries.map((entry, index) => (
            <li
              key={entry.ticket_id}
              style={{
                display: "flex",
                alignItems: "center",
                gap: "8px",
                padding: "6px 0",
                borderTop: index === 0 ? "none" : "1px solid var(--m-border)",
                opacity: busy === entry.ticket_id ? 0.5 : 1,
              }}
            >
              <span
                aria-hidden
                style={{
                  width: "20px",
                  textAlign: "right",
                  color: "var(--m-text-secondary)",
                  fontVariantNumeric: "tabular-nums",
                }}
              >
                {index + 1}
              </span>

              <span style={{ flex: 1, minWidth: 0 }}>
                <span
                  style={{
                    fontVariantNumeric: "tabular-nums",
                    color: "var(--m-text-secondary)",
                    marginRight: "6px",
                  }}
                >
                  {entry.ticket_number}
                </span>
                {entry.title}
                {entry.client_name ? (
                  <span style={{ color: "var(--m-text-secondary)" }}>
                    {" "}
                    &middot; {entry.client_name}
                  </span>
                ) : null}
              </span>

              <Tag tone={PRIORITY_TONE[entry.priority] ?? "neutral"}>
                {entry.priority.toUpperCase()}
              </Tag>

              <span style={{ display: "inline-flex", gap: "4px" }}>
                <Button
                  size="sm"
                  variant="ghost"
                  aria-label={`Move ${entry.ticket_number} up`}
                  disabled={index === 0 || busy === entry.ticket_id}
                  onClick={() => move(entry, index - 1)}
                >
                  ↑
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  aria-label={`Move ${entry.ticket_number} down`}
                  disabled={
                    index === queue.entries.length - 1 || busy === entry.ticket_id
                  }
                  onClick={() => move(entry, index + 1)}
                >
                  ↓
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  aria-label={`Remove ${entry.ticket_number} from the queue`}
                  disabled={busy === entry.ticket_id}
                  onClick={() => remove(entry)}
                >
                  Remove
                </Button>
              </span>
            </li>
          ))}
        </ol>
      )}
    </Card>
  );
}
