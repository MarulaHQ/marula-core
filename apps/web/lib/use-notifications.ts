// Notification center state for the web app.
//
// Purpose: one hook owning unread badge count, the inbox page (keyset
// cursor), mark-read actions, and the live notification leg off the
// org-wide SSE stream so badges and the dock update without polling. The
// stream reconnects itself and replays the bounded gap; missed events
// self-heal via REST refresh when the stream reopens.
//
// Non-purpose: no preferences editing (settings screen calls the SDK
// directly) and no toast/toast-queue UX — the dock is the only surface.
"use client";

import { api, useAuth } from "./api";
import { instantRuntime } from "./instant/runtime";
import { useCallback, useEffect, useRef, useState } from "react";

export interface NotificationItem {
  id: string;
  kind: string;
  title: string;
  body: string | null;
  ticket_id: string | null;
  read_at: string | null;
  created_at: string;
}

export interface NotificationPage {
  items: NotificationItem[];
  next_cursor: string | null;
}

const KIND_LABELS: Record<string, string> = {
  mention: "Mention",
  assignment: "Assignment",
  sla_risk: "SLA at risk",
  sla_breach: "SLA breach",
  escalation: "Escalation",
  missing_time: "Missing time",
  ticket_update: "Ticket update",
  report_delivery: "Report delivery",
  ai_cap_warn: "AI usage warning",
  secret_rotation: "Secret rotation",
  dedupe_report: "Duplicate report",
  stock_low: "Low stock",
  scan_detection: "Malware quarantine",
};

export function kindLabel(kind: string): string {
  return KIND_LABELS[kind] ?? kind;
}

/** How many created-notification ids the live leg remembers to ignore a
 * re-delivered frame. Frames are at-least-once; the window only has to cover
 * a reconnect replay, not a session. */
const SEEN_CREATED_LIMIT = 200;

export function useNotifications() {
  const auth = useAuth();
  const [unread, setUnread] = useState(0);
  const [items, setItems] = useState<NotificationItem[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [count, page] = await Promise.all([
        api.api.GET("/api/v1/notifications/unread-count"),
        api.api.GET("/api/v1/notifications", { params: { query: { limit: 50 } } }),
      ]);
      if (count.data) setUnread(count.data.count);
      if (page.data) {
        // The inbox read model resolves asynchronously; a malformed/null
        // payload never becomes a non-array here — the dock guards before
        // map/filter, this hook guarantees the invariant at the source.
        setItems(Array.isArray(page.data.items) ? (page.data.items as NotificationItem[]) : []);
        setNextCursor(page.data.next_cursor ?? null);
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  // Live leg: the SSE stream delivers this caller's own notification frames,
  // because the server filters a per-user frame to its recipient. The check
  // against `me.user_id` is the assertion that the contract held — never the
  // routing itself.
  //
  // `notification.created` bumps the badge optimistically: the frame reached
  // this connection, so the row is this user's. `notification.read` does not,
  // because it is the caller marking something read and raising the count
  // there would show an unread badge after a read. Both then refetch, and the
  // refetch stays authoritative: it runs after the row committed, so it
  // settles on the same number the bump predicted. A re-delivered frame for a
  // notification already counted is ignored by its `entity_id`, and the seen
  // set is bounded so a long-lived tab does not hoard ids.
  const runtime = instantRuntime();
  const seenCreated = useRef<Set<string>>(new Set());
  useEffect(() => {
    if (auth.status !== "authed" || !auth.me) return;
    const meId = auth.me.user_id;
    const offEvents = runtime.bus.subscribe((event) => {
      const created = event.type === "notification.created";
      if (!created && event.type !== "notification.read") return;
      if (event.recipient_user_id !== meId) return;
      if (!created) {
        void refresh();
        return;
      }
      const id = event.entity_id;
      if (id !== null) {
        if (seenCreated.current.has(id)) return;
        seenCreated.current.add(id);
        if (seenCreated.current.size > SEEN_CREATED_LIMIT) {
          const oldest = seenCreated.current.values().next().value;
          if (oldest !== undefined) seenCreated.current.delete(oldest);
        }
      }
      setUnread((n) => n + 1);
      void refresh();
    });
    const offStatus = runtime.stream.onStatus((status) => {
      if (status === "open") void refresh();
    });
    const offScope = runtime.stream.subscribeOrg();
    return () => {
      offEvents();
      offStatus();
      offScope();
    };
  }, [auth.status, auth.me, refresh, runtime]);

  const markRead = useCallback(async (ids: string[]) => {
    if (ids.length === 0) return;
    await api.api.POST("/api/v1/notifications/mark-read", { body: { ids } });
    setItems((prev) =>
      (Array.isArray(prev) ? prev : []).map((n) =>
        ids.includes(n.id) ? { ...n, read_at: n.read_at ?? new Date().toISOString() } : n,
      ),
    );
    setUnread((n) => Math.max(0, n - ids.length));
    void refresh();
  }, [refresh]);

  const markAllRead = useCallback(async () => {
    await api.api.POST("/api/v1/notifications/read-all");
    setItems((prev) =>
      (Array.isArray(prev) ? prev : []).map((n) => ({ ...n, read_at: n.read_at ?? new Date().toISOString() })),
    );
    setUnread(0);
  }, []);

  const loadMore = useCallback(async () => {
    if (!nextCursor || loading) return;
    setLoading(true);
    try {
      const { data } = await api.api.GET("/api/v1/notifications", {
        params: { query: { limit: 50, cursor: nextCursor } },
      });
      if (data) {
        setItems((prev) => [
          ...(Array.isArray(prev) ? prev : []),
          ...(Array.isArray(data.items) ? (data.items as NotificationItem[]) : []),
        ]);
        setNextCursor(data.next_cursor ?? null);
      }
    } finally {
      setLoading(false);
    }
  }, [nextCursor, loading]);

  return { unread, items, nextCursor, loading, refresh, markRead, markAllRead, loadMore };
}
