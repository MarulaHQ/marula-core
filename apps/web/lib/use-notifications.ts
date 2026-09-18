// Notification center state for the web app.
//
// Purpose: one hook owning unread badge count, the inbox page (keyset
// cursor), mark-read actions, and the live `user:<me>` socket subscription
// so badges and the dock update without polling. Reconnects with backoff;
// missed events self-heal via REST refresh on reconnect.
//
// Non-purpose: no preferences editing (settings screen calls the SDK
// directly) and no toast/toast-queue UX — the dock is the only surface.
"use client";

import { api, loadTokens, resolveApiBaseUrl, useAuth } from "./api";
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

function wsUrl(): string {
  if (process.env.NEXT_PUBLIC_WS_URL) return process.env.NEXT_PUBLIC_WS_URL;
  const base = resolveApiBaseUrl();
  const u = new URL(base);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  u.port = process.env.NEXT_PUBLIC_WS_PORT ?? "8081";
  u.pathname = "/ws";
  return u.toString();
}

const KIND_LABELS: Record<string, string> = {
  mention: "Mention",
  assignment: "Assignment",
  sla_risk: "SLA at risk",
  sla_breach: "SLA breach",
  escalation: "Escalation",
  missing_time: "Missing time",
  ticket_update: "Ticket update",
};

export function kindLabel(kind: string): string {
  return KIND_LABELS[kind] ?? kind;
}

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

  // Live leg: subscribe to this user's channel; created → bump + prepend,
  // read → resync counts. Reconnects with capped backoff.
  const socketRef = useRef<WebSocket | null>(null);
  useEffect(() => {
    if (auth.status !== "authed" || !auth.me) return;
    let closed = false;
    let attempt = 0;
    let retry: ReturnType<typeof setTimeout> | undefined;

    const connect = () => {
      const tokens = loadTokens();
      if (!tokens || closed) return;
      const sock = new WebSocket(`${wsUrl()}?token=${tokens.access_token}`);
      socketRef.current = sock;
      sock.onopen = () => {
        attempt = 0;
        sock.send(JSON.stringify({ type: "subscribe", channel: `user:${auth.me!.user_id}` }));
        void refresh();
      };
      sock.onmessage = (m) => {
        try {
          const frame = JSON.parse(String(m.data)) as {
            type: string;
            event?: { type?: string };
          };
          if (frame.type === "event" && frame.event?.type === "notification.created") {
            setUnread((n) => n + 1);
            void refresh();
          } else if (frame.type === "event" && frame.event?.type === "notification.read") {
            void refresh();
          }
        } catch {
          // ignore malformed frames; REST remains source of truth
        }
      };
      sock.onclose = () => {
        socketRef.current = null;
        if (closed) return;
        attempt += 1;
        retry = setTimeout(connect, Math.min(1000 * 2 ** attempt, 15000));
      };
    };
    connect();
    return () => {
      closed = true;
      if (retry) clearTimeout(retry);
      socketRef.current?.close();
      socketRef.current = null;
    };
  }, [auth.status, auth.me, refresh]);

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
