// Notification dock — the bell + grouped inbox panel.
//
// Purpose: unread badge on the app header, a grouped (Today / Yesterday /
// Earlier) inbox list, per-item mark-read with deep-link to the ticket,
// bulk "mark all read", and an entry point to the preferences screen.
//
// Non-purpose: no toasts, no notification creation, no preference editing
// here — settings live at /settings/notifications.
"use client";

import { Button } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { kindLabel, useNotifications, type NotificationItem } from "../lib/use-notifications";

function relativeTime(iso: string): string {
  const diffMs = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diffMs / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

function dayBucket(iso: string): string {
  const d = new Date(iso);
  const now = new Date();
  const startOf = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const deltaDays = Math.round((startOf(now) - startOf(d)) / 86400000);
  if (deltaDays <= 0) return "Today";
  if (deltaDays === 1) return "Yesterday";
  return "Earlier";
}

const BUCKET_ORDER = ["Today", "Yesterday", "Earlier"];

function BellIcon() {
  return (
    <svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16" fill="none">
      <path
        d="M8 1.5a4.2 4.2 0 0 0-4.2 4.2c0 3.1-1.05 4.35-1.55 4.85a.6.6 0 0 0 .43 1.02h10.64a.6.6 0 0 0 .43-1.02c-.5-.5-1.55-1.75-1.55-4.85A4.2 4.2 0 0 0 8 1.5Z"
        stroke="currentColor"
        strokeWidth="1.4"
      />
      <path d="M6.5 13.5a1.5 1.5 0 0 0 3 0" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}

function NotificationRow({
  item,
  onOpen,
}: {
  item: NotificationItem;
  onOpen: (item: NotificationItem) => void;
}) {
  const unread = item.read_at === null;
  return (
    <button
      type="button"
      onClick={() => onOpen(item)}
      className="w-full text-left px-3 py-2.5 flex gap-2.5 items-start border-b transition-colors last:border-b-0 hover:bg-[var(--m-canvas-light)]"
      style={{ borderColor: "var(--m-border)" }}
    >
      <span
        aria-hidden="true"
        className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full"
        style={{ background: unread ? "var(--m-accent)" : "transparent" }}
      />
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline justify-between gap-2">
          <span
            className="text-[13px] truncate"
            style={{ color: "var(--m-text)", fontWeight: unread ? 600 : 400 }}
          >
            {item.title}
          </span>
          <span className="text-[11px] shrink-0" style={{ color: "var(--m-text-muted)" }}>
            {relativeTime(item.created_at)}
          </span>
        </span>
        {item.body ? (
          <span
            className="block text-[12px] mt-0.5 line-clamp-2"
            style={{ color: "var(--m-text-secondary)" }}
          >
            {kindLabel(item.kind)} · {item.body}
          </span>
        ) : (
          <span className="block text-[12px] mt-0.5" style={{ color: "var(--m-text-secondary)" }}>
            {kindLabel(item.kind)}
          </span>
        )}
      </span>
    </button>
  );
}

export function NotificationDock() {
  const router = useRouter();
  const { unread, items, nextCursor, loading, markRead, markAllRead, loadMore } =
    useNotifications();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const openItem = (item: NotificationItem) => {
    void markRead([item.id]);
    if (item.ticket_id) {
      setOpen(false);
      router.push(`/tickets/${item.ticket_id}`);
    }
  };

  const groups = BUCKET_ORDER.map((label) => ({
    label,
    rows: items.filter((n) => dayBucket(n.created_at) === label),
  })).filter((g) => g.rows.length > 0);

  return (
    <div ref={rootRef} className="relative">
      <Button
        variant="ghost"
        size="sm"
        aria-label={`Notifications${unread > 0 ? ` (${unread} unread)` : ""}`}
        onClick={() => setOpen((v) => !v)}
      >
        <BellIcon />
        {unread > 0 && (
          <span
            data-testid="unread-badge"
            className="ml-1 min-w-[18px] h-[18px] px-1 inline-flex items-center justify-center rounded-full text-[11px] font-medium"
            style={{ background: "var(--m-accent)", color: "var(--m-on-accent)" }}
          >
            {unread > 9 ? "9+" : unread}
          </span>
        )}
      </Button>

      {open && (
        <div
          role="dialog"
          aria-label="Notifications"
          className="absolute right-0 top-full mt-1 w-[360px] max-w-[calc(100vw-24px)] rounded-md border shadow-lg z-50 overflow-hidden"
          style={{
            background: "var(--m-surface)",
            borderColor: "var(--m-border)",
            boxShadow: "var(--m-shadow-strong)",
          }}
        >
          <div
            className="flex items-center justify-between px-3 h-10 border-b"
            style={{ borderColor: "var(--m-border)" }}
          >
            <span className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
              Notifications
            </span>
            <div className="flex items-center gap-1">
              {unread > 0 && (
                <Button variant="ghost" size="sm" onClick={() => void markAllRead()}>
                  Mark all read
                </Button>
              )}
            </div>
          </div>

          <div className="max-h-[420px] overflow-y-auto">
            {groups.length === 0 ? (
              <p className="px-3 py-8 text-center text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                You&apos;re all caught up.
              </p>
            ) : (
              groups.map((group) => (
                <div key={group.label}>
                  <div
                    className="px-3 py-1.5 text-[11px] font-medium uppercase tracking-wide border-b"
                    style={{
                      color: "var(--m-text-muted)",
                      background: "var(--m-canvas)",
                      borderColor: "var(--m-border)",
                    }}
                  >
                    {group.label}
                  </div>
                  {group.rows.map((item) => (
                    <NotificationRow key={item.id} item={item} onOpen={openItem} />
                  ))}
                </div>
              ))
            )}
            {nextCursor && (
              <div className="p-2 text-center">
                <Button variant="secondary" size="sm" disabled={loading} onClick={() => void loadMore()}>
                  Load older
                </Button>
              </div>
            )}
          </div>

          <div
            className="border-t px-3 py-2 text-center"
            style={{ borderColor: "var(--m-border)" }}
          >
            <Button variant="ghost" size="sm" onClick={() => router.push("/settings/notifications")}>
              Notification settings
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
