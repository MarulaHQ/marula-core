// CalendarSyncPanel — calendar-sync status/conflict surfaces (Wave 14
// lane 7 close-out; polished by Wave 17 lane 4).
//
// Purpose: the caller's external-calendar connections with live health —
// provider, direction, status dot (active / error / paused), relative
// last-successful-sync, the provider's fixed failure copy, and the last
// poll's apply counters. Inbound overlaps are the conflict signal:
// external busy time that landed on top of existing appointments is
// REPORTED, never silently dropped — the sync-now outcome flags overlaps,
// and (when the page hands us the rendered entries) each inbound
// connection offers an expandable mirror list whose events mark
// "overlaps your <appointment>" inline. Actions ride the typed client:
// connect (POST /calendar-sync), sync now (POST {id}/sync — counters
// rendered), rotate the outbound feed token (POST {id}/rotate-token —
// the raw URL is shown EXACTLY ONCE, copy-out-of-band per the
// invite-token doctrine), pause/resume (PATCH {id}), delete (DELETE {id},
// two-step confirm).
//
// Non-purpose: no worker scheduling UI (the worker polls on its own
// cadence), and no conflict resolution writes — external busy time is
// kept, never dropped, so there is nothing to resolve server-side.
//
// The Google + Microsoft providers (Wave 34 claims close-out, claim 5) are
// DELEGATED: connecting one is a consent round trip, not a URL paste, so
// the form below creates the connection first and then sends the operator
// to the vendor. A cancelled consent leaves a connection in place with no
// token, which the panel shows as "not connected" rather than pretending
// the row failed.
"use client";

import { Button, Card, EmptyState, FieldHint, Input, Label, Spinner, Tag } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useState } from "react";
import { API_BASE, api } from "../lib/api";
import { friendlyMutationText, resultErrorText } from "../lib/error-copy";
import { overlapsInterval, type CalendarEntry } from "../lib/calendar-view";

type Connection = components["schemas"]["CalendarSyncConnection"];
type Direction = components["schemas"]["CalendarSyncDirection"];
/** The three kinds the API's `provider` CHECK admits. */
type CalendarProvider = "ical" | "google" | "microsoft";

/** What a picker row shows, and whether it needs a URL or a consent trip. */
const PROVIDERS: {
  value: CalendarProvider;
  label: string;
  needsUrl: boolean;
  note: string;
}[] = [
  {
    value: "ical",
    label: "iCal feed",
    needsUrl: true,
    note: "Any provider that publishes a read-only .ics URL. Paste it below.",
  },
  {
    value: "google",
    label: "Google Calendar",
    needsUrl: false,
    note: "Read-only access to your primary calendar. You will be sent to Google's consent screen; Marula cannot read your calendar until you approve it.",
  },
  {
    value: "microsoft",
    label: "Microsoft 365",
    needsUrl: false,
    note: "Read-only access to your Outlook calendar, same consent step. Needs Calendars.Read, never write.",
  },
];
type SyncEvent = components["schemas"]["CalendarSyncEvent"];

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

function statusDotColor(status: string): string {
  if (status === "active") return "var(--m-status-success-fg)";
  if (status === "error") return "var(--m-status-danger-fg)";
  return "var(--m-text-muted)";
}

function fmtWhen(iso: string | null | undefined): string {
  if (!iso) return "never";
  const then = new Date(iso).getTime();
  const mins = Math.max(0, Math.round((Date.now() - then) / 60_000));
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return new Date(iso).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export default function CalendarSyncPanel({ entries }: { entries?: CalendarEntry[] } = {}) {
  const [connections, setConnections] = useState<Connection[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<{ id: string; text: string; overlaps: number } | null>(null);
  const [oneTime, setOneTime] = useState<{ url: string; label: string } | null>(null);
  const [copied, setCopied] = useState(false);
  const [connectOpen, setConnectOpen] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [form, setForm] = useState<{
    name: string;
    direction: Direction;
    feedUrl: string;
    provider: CalendarProvider;
  }>({
    name: "",
    direction: "inbound",
    feedUrl: "",
    // ical first: it is the only provider that needs no OAuth client
    // configured, so it is the one that works on a fresh install.
    provider: "ical",
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

  /**
   * Resume a consent flow that was started and abandoned, or that failed
   * because the OAuth client is not configured yet.
   *
   * Same flow as create, from a different starting point: ask the API for a
   * fresh authorize URL and navigate. A stale `state` is not a problem
   * because starting consent overwrites it.
   */
  async function reconnect(connectionId: string) {
    setBusyId(connectionId);
    try {
      const res = await api.api.POST(
        "/api/v1/calendar-sync/{connection_id}/authorize",
        { params: { path: { connection_id: connectionId } } },
      );
      const start = res.data;
      if (start === undefined) {
        setError(resultErrorText(res));
      } else {
        window.location.assign(start.authorize_url);
      }
    } finally {
      setBusyId(null);
    }
  }

  async function connect() {
    if (!form.name.trim()) return;
    setBusyId("new");
    try {
      const delegated = form.provider !== "ical";
      const res = await api.api.POST("/api/v1/calendar-sync", {
        body: {
          name: form.name.trim(),
          direction: form.direction,
          provider: form.provider,
          ...(form.direction !== "outbound" && form.feedUrl.trim()
            ? { feed_url: form.feedUrl.trim() }
            : {}),
        },
      });
      const data = res.data;
      if (data === undefined) {
        setError(resultErrorText(res));
      } else if (delegated) {
        // The connection exists but has no token until consent completes, so
        // the next step is the consent redirect rather than a feed token.
        // A failure here leaves an unconnected row the operator can retry
        // from, which is recoverable; nothing is lost.
        try {
          const auth = await api.api.POST(
            "/api/v1/calendar-sync/{connection_id}/authorize",
            { params: { path: { connection_id: data.connection } } },
          );
          const start = auth.data;
          if (start === undefined) {
            setError(resultErrorText(auth));
            await load();
          } else {
            await load();
            // A full navigation, not a router push: the vendor's consent
            // screen must not be able to reach back into this app's history.
            window.location.assign(start.authorize_url);
          }
        } catch {
          await load();
          setError(
            "the connection was created but the consent link could not be built — open it again to finish connecting",
          );
        }
      } else {
        if (data.feed_token) {
          setOneTime({
            url: `${API_BASE}/api/v1/calendar-sync/feed/${data.feed_token}`,
            label: form.name.trim(),
          });
          setCopied(false);
        }
        setConnectOpen(false);
        setForm({ name: "", direction: "inbound", feedUrl: "", provider: "ical" });
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
        <div className="mb-3 grid gap-2 rounded-[var(--m-radius-control)] border p-3" style={{ borderColor: "var(--m-border)" }} data-testid="connect-form">
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
            className="h-[var(--m-density-control)] rounded-[var(--m-radius-control)] border px-2 text-[13px]"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
            data-testid="connect-direction"
          >
            <option value="outbound">Outbound — publish my bookings</option>
            <option value="inbound">Inbound — import external busy time</option>
            <option value="two_way">Two-way</option>
          </select>
          {form.direction !== "outbound" && (
            <>
              <Label>Calendar source</Label>
              <select
                value={form.provider}
                onChange={(e) =>
                  setForm((f) => ({
                    ...f,
                    provider: e.target.value as CalendarProvider,
                  }))
                }
                style={{ width: "100%" }}
              >
                {PROVIDERS.map((p) => (
                  <option key={p.value} value={p.value}>
                    {p.label}
                  </option>
                ))}
              </select>
              <FieldHint>
                {PROVIDERS.find((p) => p.value === form.provider)?.note}
              </FieldHint>
            </>
          )}

          {/*
            The URL box is iCal-only, and it is HIDDEN rather than disabled
            for a delegated provider: a disabled field with a placeholder
            about .ics next to a Google consent flow reads as "this is
            broken" rather than "this is not how Google works".
          */}
          {form.direction !== "outbound" && form.provider === "ical" && (
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
          className="mb-3 rounded-[var(--m-radius-control)] p-3 text-[12px]"
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
              entries={entries}
              onSync={() => void run(conn)}
              onReconnect={() => void reconnect(conn.id)}
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
  entries,
  onSync,
  onReconnect,
  onRotate,
  onToggle,
  onDelete,
  onConfirmDelete,
}: {
  conn: Connection;
  busy: boolean;
  outcome: { text: string; overlaps: number } | null;
  confirmDelete: boolean;
  entries?: CalendarEntry[];
  onSync: () => void;
  /** Resume a delegated consent flow that was started and abandoned. */
  onReconnect: () => void;
  onRotate: () => void;
  onToggle: (status: "active" | "disabled") => void;
  onDelete: () => void;
  onConfirmDelete: () => void;
}) {
  const hasFeed = conn.feed_path !== null && (conn.direction === "outbound" || conn.direction === "two_way");
  const [mirrorOpen, setMirrorOpen] = useState(false);
  return (
    <div
      className="rounded-[var(--m-radius-control)] border p-3 transition-colors"
      style={{ borderColor: conn.status === "error" ? "var(--m-status-danger-fg)" : "var(--m-border)" }}
      data-testid={`sync-connection-${conn.id}`}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span
              aria-hidden="true"
              className="inline-block h-2 w-2 shrink-0 rounded-full"
              style={{ background: statusDotColor(conn.status) }}
            />
            <span className="text-[13px] font-medium truncate" style={{ color: "var(--m-text)" }}>
              {conn.name}
            </span>
            <Tag tone="neutral">{conn.provider}</Tag>
            {/* A delegated connection with no token is NOT connected. Saying
                so plainly is the difference between an operator who
                retries and one who assumes the sync is running. */}
            {conn.provider !== "ical" && !conn.access_token ? (
              <>
                <Tag tone="warning">not connected</Tag>
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={onReconnect}
                  data-testid={`reconnect-${conn.id}`}
                >
                  Finish connecting
                </Button>
              </>
            ) : null}
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
        <div className="flex flex-wrap items-center gap-1.5">
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
      {conn.direction !== "outbound" ? (
        <div className="mt-2">
          <button
            type="button"
            onClick={() => setMirrorOpen((v) => !v)}
            aria-expanded={mirrorOpen}
            className="text-[12px] font-medium underline decoration-[var(--m-border)] underline-offset-2"
            style={{ color: "var(--m-text-secondary)" }}
            data-testid={`mirror-toggle-${conn.id}`}
          >
            {mirrorOpen ? "Hide" : "Show"} imported busy time
          </button>
          {mirrorOpen ? (
            <InboundMirror connId={conn.id} entries={entries} />
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function InboundMirror({ connId, entries }: { connId: string; entries?: CalendarEntry[] }) {
  const [events, setEvents] = useState<SyncEvent[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void api.api
      .GET("/api/v1/calendar-sync/{id}/events", { params: { path: { id: connId } } })
      .then((res) => {
        if (cancelled) return;
        if (res.data === undefined) {
          setError("Couldn't load the imported events.");
          return;
        }
        setEvents(res.data.events);
      })
      .catch(() => {
        if (!cancelled) setError("Couldn't load the imported events.");
      });
    return () => {
      cancelled = true;
    };
  }, [connId]);

  if (error !== null) {
    return (
      <p role="alert" className="mt-1 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
        {error}
      </p>
    );
  }
  if (events === null) {
    return (
      <div className="mt-1 flex justify-center">
        <Spinner />
      </div>
    );
  }
  if (events.length === 0) {
    return (
      <p className="mt-1 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
        Nothing imported yet from this feed.
      </p>
    );
  }
  const sorted = [...events].sort((a, b) => a.starts_at.localeCompare(b.starts_at));
  return (
    <ul className="mt-1 grid gap-1" data-testid={`mirror-list-${connId}`}>
      {sorted.map((event) => {
        const hit = entries?.find(
          (entry) =>
            entry.kind === "appointment" &&
            overlapsInterval(
              new Date(event.starts_at),
              new Date(event.ends_at),
              new Date(entry.startsAt),
              new Date(entry.endsAt),
            ),
        );
        return (
          <li
            key={event.id}
            className="flex flex-wrap items-center gap-x-2 rounded px-2 py-1 text-[12px]"
            style={{ background: "var(--m-bg-subtle)" }}
          >
            <span className="truncate font-medium">{event.title ?? "External event"}</span>
            <span className="tabular-nums" style={{ color: "var(--m-text-muted)" }}>
              {new Date(event.starts_at).toLocaleString(undefined, {
                month: "short",
                day: "numeric",
                hour: "2-digit",
                minute: "2-digit",
              })}
            </span>
            {hit !== undefined ? (
              <span className="font-semibold" style={{ color: "var(--m-status-warning-fg)" }}>
                overlaps your {hit.title}
              </span>
            ) : null}
          </li>
        );
      })}
    </ul>
  );
}
