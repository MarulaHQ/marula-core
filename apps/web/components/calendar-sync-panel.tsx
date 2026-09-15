// CalendarSyncPanel — calendar-sync status/conflict surfaces (Wave 14
// lane 7 close-out, wiring the merged lane-2 contract through the typed
// SDK).
//
// Purpose: the caller's external-calendar connections with live health —
// provider, direction, status (active / error / disabled), last successful
// sync, the provider's fixed failure copy, and the last poll's apply
// counters. Inbound overlaps are the conflict signal: external busy time
// that landed on top of existing appointments is REPORTED
// (`calendar.sync_completed.overlaps`), never silently dropped.
// Actions ride the typed client: connect (POST /calendar-sync), sync now
// (POST {id}/sync — outcome counters rendered), rotate the outbound feed
// token (POST {id}/rotate-token — the raw URL is shown EXACTLY ONCE,
// copy-out-of-band per the invite-token doctrine), pause/resume
// (PATCH {id}), delete (DELETE {id}, two-step confirm).
//
// Non-purpose: no worker scheduling UI (the worker polls on its own
// cadence), no OAuth provider flows (the ical provider is the only one),
// no per-event mirror browser (GET {id}/events stays an API surface).
"use client";

import { Button, Card, EmptyState, FieldHint, Input, Label, Spinner, Tag } from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useCallback, useEffect, useState } from "react";
import { API_BASE, api } from "../lib/api";
import { friendlyMutationText, resultErrorText } from "../lib/error-copy";

type Connection = components["schemas"]["CalendarSyncConnection"];
type Direction = components["schemas"]["CalendarSyncDirection"];

const DIRECTION_COPY: Readonly<Record<Direction, string>> = {
  outbound: "Outbound — publishes your bookings to an external calendar feed",
  inbound: "Inbound — external events block your booking time",
  two_way: "Two-way — publish + import",
};

function statusTone(status: string): "success" | "danger" | "neutral" {
  if (status === "active") return "success";
  if (status === "error") return "danger";
  return "neutral";
}

function statusLabel(status: string): string {
  if (status === "active") return "Active";
  if (status === "error") return "Error";
  return "Paused";
}

function fmtWhen(iso: string | null | undefined): string {
  if (!iso) return "never";
  return new Date(iso).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export default function CalendarSyncPanel() {
  const [connections, setConnections] = useState<Connection[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<{ id: string; text: string; overlaps: number } | null>(null);
  const [oneTime, setOneTime] = useState<{ url: string; label: string } | null>(null);
  const [copied, setCopied] = useState(false);
  const [connectOpen, setConnectOpen] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [form, setForm] = useState<{ name: string; direction: Direction; feedUrl: string }>({
    name: "",
    direction: "inbound",
    feedUrl: "",
  });

  const load = useCallback(async () => {
    try {
      const res = await api.api.GET("/api/v1/calendar-sync");
      if (res.data === undefined) {
        setError(resultErrorText(res, "Couldn't load your calendar connections."));
        return;
      }
      setError(null);
      setConnections(res.data);
    } catch (e) {
      setError(friendlyMutationText(e, "Couldn't load your calendar connections."));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function run(conn: Connection) {
    setBusyId(conn.id);
    setOutcome(null);
    try {
      const res = await api.api.POST("/api/v1/calendar-sync/{id}/sync", {
        params: { path: { id: conn.id, force: false } },
      });
      const data = res.data;
      if (data === undefined) {
        setError(resultErrorText(res));
      } else {
        const o = data.outcome;
        setOutcome({
          id: conn.id,
          text: `Synced — imported ${o.imported} · updated ${o.updated} · released ${o.released} · duplicates ${o.duplicates}`,
          overlaps: o.overlaps,
        });
        await load();
      }
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setBusyId(null);
    }
  }

  async function rotate(conn: Connection) {
    setBusyId(conn.id);
    try {
      const res = await api.api.POST("/api/v1/calendar-sync/{id}/rotate-token", {
        params: { path: { id: conn.id } },
      });
      const data = res.data;
      if (data === undefined) {
        setError(resultErrorText(res));
      } else if (data.feed_token) {
        setOneTime({
          url: `${API_BASE}/api/v1/calendar-sync/feed/${data.feed_token}`,
          label: conn.name,
        });
        setCopied(false);
        await load();
      }
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setBusyId(null);
    }
  }

  async function setStatus(conn: Connection, status: "active" | "disabled") {
    setBusyId(conn.id);
    try {
      const res = await api.api.PATCH("/api/v1/calendar-sync/{id}", {
        params: { path: { id: conn.id } },
        body: { status },
      });
      const updated = res.data;
      if (updated === undefined) {
        setError(resultErrorText(res));
      } else {
        setConnections((prev) => prev?.map((c) => (c.id === updated.id ? updated : c)) ?? prev);
      }
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setBusyId(null);
    }
  }

  async function remove(conn: Connection) {
    setBusyId(conn.id);
    try {
      // DELETE is 204-only in the pin (no error schema): a typed envelope
      // can't be extracted here, so any thrown failure surfaces friendly
      // copy and completion removes the row (a 404 means already gone).
      await api.api.DELETE("/api/v1/calendar-sync/{id}", {
        params: { path: { id: conn.id } },
      });
      setConnections((prev) => prev?.filter((c) => c.id !== conn.id) ?? prev);
      setConfirmDeleteId(null);
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setBusyId(null);
    }
  }

  async function connect() {
    if (!form.name.trim()) return;
    setBusyId("new");
    try {
      const res = await api.api.POST("/api/v1/calendar-sync", {
        body: {
          name: form.name.trim(),
          direction: form.direction,
          ...(form.direction !== "outbound" && form.feedUrl.trim()
            ? { feed_url: form.feedUrl.trim() }
            : {}),
        },
      });
      const data = res.data;
      if (data === undefined) {
        setError(resultErrorText(res));
      } else {
        if (data.feed_token) {
          setOneTime({
            url: `${API_BASE}/api/v1/calendar-sync/feed/${data.feed_token}`,
            label: form.name.trim(),
          });
          setCopied(false);
        }
        setConnectOpen(false);
        setForm({ name: "", direction: "inbound", feedUrl: "" });
        await load();
      }
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setBusyId(null);
    }
  }

  function copyOneTime() {
    if (oneTime === null) return;
    void navigator.clipboard?.writeText(oneTime.url);
    setCopied(true);
  }

  return (
    <Card className="p-4">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <div>
          <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            Calendar sync
          </h2>
          <FieldHint>
            Two-way sync with external calendars. Inbound busy time feeds the same
            booking double-check as portal requests — overlaps are reported, never
            silently dropped.
          </FieldHint>
        </div>
        <Button
          variant={connectOpen ? "secondary" : "primary"}
          onClick={() => setConnectOpen((v) => !v)}
          data-testid="connect-calendar"
        >
          {connectOpen ? "Cancel" : "Connect calendar"}
        </Button>
      </div>

      {error && (
        <p role="alert" className="mb-2 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      )}

      {connectOpen && (
        <div className="mb-3 grid gap-2 p-3" style={{ border: "1px solid var(--m-border)" }} data-testid="connect-form">
          <Label>Name</Label>
          <Input
            value={form.name}
            onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
            placeholder="Work calendar"
          />
          <Label>Direction</Label>
          <select
            value={form.direction}
            onChange={(e) => setForm((f) => ({ ...f, direction: e.target.value as Direction }))}
            className="h-[var(--m-control-height)] rounded-[var(--m-radius-control)] border px-2 text-[13px]"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
            data-testid="connect-direction"
          >
            <option value="outbound">Outbound — publish my bookings</option>
            <option value="inbound">Inbound — import external busy time</option>
            <option value="two_way">Two-way</option>
          </select>
          {form.direction !== "outbound" && (
            <>
              <Label>External iCal URL</Label>
              <Input
                value={form.feedUrl}
                onChange={(e) => setForm((f) => ({ ...f, feedUrl: e.target.value }))}
                placeholder="https://calendar.example.com/feed.ics"
              />
            </>
          )}
          <div>
            <Button
              disabled={!form.name.trim() || busyId === "new"}
              onClick={() => void connect()}
              data-testid="connect-save"
            >
              {busyId === "new" ? "Connecting…" : "Connect"}
            </Button>
          </div>
        </div>
      )}

      {oneTime && (
        <div
          className="mb-3 p-3 text-[12px]"
          style={{ border: "1px solid var(--m-status-warning-fg)", color: "var(--m-text)" }}
          role="alert"
          data-testid="one-time-feed-url"
        >
          <p className="mb-1 font-semibold" style={{ color: "var(--m-status-warning-fg)" }}>
            Feed URL for “{oneTime.label}” — shown once. Copy it now; it can't be retrieved again
            (rotate to get a new one).
          </p>
          <code className="break-all" style={{ color: "var(--m-text-secondary)" }}>
            {oneTime.url}
          </code>
          <div className="mt-2">
            <Button variant="secondary" onClick={copyOneTime}>
              {copied ? "Copied" : "Copy URL"}
            </Button>
          </div>
        </div>
      )}

      {connections === null && !error ? (
        <div className="p-4 flex justify-center">
          <Spinner />
        </div>
      ) : connections !== null && connections.length === 0 ? (
        <EmptyState message="No external calendars connected yet — connect one to publish or import busy time." />
      ) : (
        <div className="grid gap-2">
          {connections?.map((conn) => (
            <ConnectionRow
              key={conn.id}
              conn={conn}
              busy={busyId === conn.id}
              outcome={outcome?.id === conn.id ? outcome : null}
              confirmDelete={confirmDeleteId === conn.id}
              onSync={() => void run(conn)}
              onRotate={() => void rotate(conn)}
              onToggle={(status) => void setStatus(conn, status)}
              onDelete={() => void remove(conn)}
              onConfirmDelete={() => setConfirmDeleteId(conn.id)}
            />
          ))}
        </div>
      )}
    </Card>
  );
}

function ConnectionRow({
  conn,
  busy,
  outcome,
  confirmDelete,
  onSync,
  onRotate,
  onToggle,
  onDelete,
  onConfirmDelete,
}: {
  conn: Connection;
  busy: boolean;
  outcome: { text: string; overlaps: number } | null;
  confirmDelete: boolean;
  onSync: () => void;
  onRotate: () => void;
  onToggle: (status: "active" | "disabled") => void;
  onDelete: () => void;
  onConfirmDelete: () => void;
}) {
  const hasFeed = conn.feed_path !== null && (conn.direction === "outbound" || conn.direction === "two_way");
  return (
    <div className="p-3" style={{ border: "1px solid var(--m-border)" }} data-testid={`sync-connection-${conn.id}`}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-[13px] font-medium truncate" style={{ color: "var(--m-text)" }}>
              {conn.name}
            </span>
            <Tag tone="neutral">{conn.provider}</Tag>
            <Tag tone={statusTone(conn.status)}>{statusLabel(conn.status)}</Tag>
          </div>
          <div className="mt-0.5 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            {DIRECTION_COPY[conn.direction] ?? conn.direction} · last sync {fmtWhen(conn.last_synced_at)}
          </div>
          {conn.status === "error" && conn.last_error && (
            <p role="alert" className="mt-1 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {conn.last_error}
            </p>
          )}
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-1.5">
          <Button variant="secondary" disabled={busy} onClick={onSync} data-testid={`sync-now-${conn.id}`}>
            {busy ? "Syncing…" : "Sync now"}
          </Button>
          {hasFeed && (
            <Button variant="secondary" disabled={busy} onClick={onRotate}>
              Rotate feed token
            </Button>
          )}
          {conn.status === "disabled" ? (
            <Button variant="secondary" disabled={busy} onClick={() => onToggle("active")}>
              Resume
            </Button>
          ) : (
            <Button variant="secondary" disabled={busy} onClick={() => onToggle("disabled")}>
              Pause
            </Button>
          )}
          {confirmDelete ? (
            <Button variant="secondary" disabled={busy} onClick={onDelete}>
              Confirm delete
            </Button>
          ) : (
            <Button variant="ghost" disabled={busy} onClick={onConfirmDelete}>
              Delete
            </Button>
          )}
        </div>
      </div>
      {hasFeed && (
        <p className="mt-1 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          Feed active — subscribe your external calendar to it (rotate to replace the token).
        </p>
      )}
      {outcome && (
        <p
          className="mt-2 text-[12px]"
          style={{ color: outcome.overlaps > 0 ? "var(--m-status-warning-fg)" : "var(--m-text-secondary)" }}
          role="status"
          data-testid={`sync-outcome-${conn.id}`}
        >
          {outcome.text}
          {outcome.overlaps > 0
            ? ` · ${outcome.overlaps} external event${outcome.overlaps === 1 ? "" : "s"} overlapped existing bookings — kept on your calendar, and new bookings check against them`
            : ""}
        </p>
      )}
    </div>
  );
}
