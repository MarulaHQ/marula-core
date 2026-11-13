// /settings/dead-work — the dead-work admin surface (Wave 67 lane
// `w67-read-surfaces`).
//
// Purpose: `GET|POST /api/v1/admin/dead-work*` has answered since Wave 52 lane
// R3, and nothing showed it. This page is the list plus the three decisions an
// operator can make about abandoned work: redrive one row, cancel one row, and
// resume a webhook destination that repeated failures paused.
//
// # Why the copy is explicit about what a redrive does
//
// A redrive re-sends. For a webhook row that means the same signed body leaves
// this instance towards a third party again, and the receiver may act on it
// twice. The confirmation says so in those words rather than "retry", because
// an operator reading it during an incident is deciding for someone else's
// system. A cancel is terminal and the API refuses to revive it, which the
// dialog also says — a button that looks reversible when it is not is the worse
// lie.
//
// # What this page does not do
//
// No bulk actions (each row is one decision, audited separately), no redrive of
// a dead stream event (`replayable: false` — that payload needs the relay's
// stream writer), and no editing of the error a ledger recorded.
//
// The payload is fetched with an authenticated request instead of the typed
// SDK, the same posture /settings/quarantine takes: these routes' OpenAPI pins
// are regenerated in the lane's gated verification sequence
// (scripts/regen-openapi-pins.sh). The interfaces below mirror the server DTOs
// in crates/core-api/src/routes/dead_work.rs.
"use client";

import {
  Badge,
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  FieldError,
  FieldHint,
  Label,
  Spinner,
  Table,
  Td,
  Textarea,
  Th,
  Tr,
  formatInTimeZone,
} from "@marula-core/ui";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  NotAvailable,
  SettingsHeader,
} from "../../../components/settings/settings-shell";
import { apiBaseUrl, authHeaders, useRequireAuth } from "../../../lib/api";
import { useTimeFormat } from "../../../lib/use-time-format";

type DeadWorkSource = "job" | "email" | "webhook" | "event";

interface DeadWorkItem {
  source: DeadWorkSource;
  id: string;
  subject: string;
  attempts: number;
  last_error: string | null;
  dead_since: string;
  replayable: boolean;
  cancelled_at: string | null;
}

interface DeadWorkPageView {
  items: DeadWorkItem[];
  next_offset?: number;
}

interface Destination {
  id: string;
  url: string;
  enabled: boolean;
  consecutive_failures: number;
  paused_at: string | null;
  paused_reason: string | null;
}

const BASE = "/api/v1/admin/dead-work";

/** The ledgers, in the order the API's own vocabulary lists them. */
const SOURCES: { value: DeadWorkSource; label: string; blurb: string }[] = [
  { value: "job", label: "Jobs", blurb: "the job queue" },
  { value: "email", label: "Email", blurb: "the outbound mail ledger" },
  { value: "webhook", label: "Webhooks", blurb: "signed deliveries to an endpoint" },
  { value: "event", label: "Stream events", blurb: "the domain-event stream" },
];

function sourceLabel(source: DeadWorkSource): string {
  return SOURCES.find((s) => s.value === source)?.label ?? source;
}

export default function DeadWorkSettingsPage() {
  const auth = useRequireAuth();
  const { timeZone } = useTimeFormat();
  const canRead = (auth.me?.permissions ?? []).includes("admin:view");
  const canAct = (auth.me?.permissions ?? []).includes("admin:edit");

  const [source, setSource] = useState<"" | DeadWorkSource>("");
  const [includeCancelled, setIncludeCancelled] = useState(false);
  const [page, setPage] = useState<DeadWorkPageView | null>(null);
  const [destinations, setDestinations] = useState<Destination[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);

  const [redriving, setRedriving] = useState<DeadWorkItem | null>(null);
  const [cancelling, setCancelling] = useState<DeadWorkItem | null>(null);
  const [reason, setReason] = useState("");
  const [reasonTouched, setReasonTouched] = useState(false);
  const [resuming, setResuming] = useState<Destination | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const query = new URLSearchParams({ limit: "200" });
      if (source !== "") query.set("source", source);
      if (includeCancelled) query.set("include_cancelled", "true");
      const [listRes, destRes] = await Promise.all([
        fetch(`${apiBaseUrl()}${BASE}?${query.toString()}`, {
          headers: authHeaders(),
          cache: "no-store",
        }),
        fetch(`${apiBaseUrl()}${BASE}/destinations`, {
          headers: authHeaders(),
          cache: "no-store",
        }),
      ]);
      if (listRes.status === 403) {
        setLoadError("forbidden");
        return;
      }
      if (!listRes.ok) {
        setLoadError(`The dead-work list could not be loaded (${listRes.status}).`);
        return;
      }
      const body = (await listRes.json()) as DeadWorkPageView;
      setPage({
        items: Array.isArray(body.items) ? body.items : [],
        next_offset: body.next_offset,
      });
      // The destinations list is a second read of the same admin gate; a
      // failure there must not hide the abandoned rows, so it degrades to an
      // empty section rather than an error page.
      if (destRes.ok) {
        const rows = (await destRes.json()) as Destination[];
        setDestinations(Array.isArray(rows) ? rows : []);
      } else {
        setDestinations([]);
      }
    } catch {
      setLoadError("The dead-work list could not be loaded — check the connection and try again.");
    } finally {
      setLoading(false);
    }
  }, [source, includeCancelled]);

  useEffect(() => {
    if (auth.status !== "authed" || !canRead) return;
    void load();
  }, [auth.status, canRead, load]);

  const items = useMemo(() => page?.items ?? [], [page]);
  const paused = useMemo(() => destinations.filter((d) => d.paused_at !== null), [destinations]);
  const reasonOk = reason.trim().length > 0;
  const showReasonError = reasonTouched && !reasonOk;

  const closeCancel = () => {
    if (busyKey !== null) return;
    setCancelling(null);
    setReason("");
    setReasonTouched(false);
    setActionError(null);
  };

  const markKey = (item: { source: string; id: string }) => `${item.source}:${item.id}`;

  const redrive = async () => {
    if (redriving === null) return;
    const key = markKey(redriving);
    setBusyKey(key);
    setActionError(null);
    setNotice(null);
    try {
      const res = await fetch(`${apiBaseUrl()}${BASE}/${redriving.source}/${redriving.id}/redrive`, {
        method: "POST",
        headers: authHeaders(),
      });
      if (res.status === 409) {
        const body = (await res.json().catch(() => null)) as { error?: { message?: string } } | null;
        setActionError(
          body?.error?.message ??
            "The server refused this redrive. Nothing was re-sent; the row's reason is in the audit trail.",
        );
        return;
      }
      if (!res.ok) {
        setActionError(`The redrive was not recorded (${res.status}). Nothing was re-sent.`);
        return;
      }
      const body = (await res.json()) as { outcome: string; changed: boolean };
      setNotice(
        body.changed
          ? `Requeued ${sourceLabel(redriving.source)} work — it is due now with a fresh attempt budget.`
          : "That row had already moved; nothing changed and no second audit row was written.",
      );
      setRedriving(null);
      await load();
    } catch {
      setActionError("The redrive could not be sent — check the connection and try again.");
    } finally {
      setBusyKey(null);
    }
  };

  const cancel = async () => {
    if (cancelling === null || !reasonOk) {
      setReasonTouched(true);
      return;
    }
    const key = markKey(cancelling);
    setBusyKey(key);
    setActionError(null);
    setNotice(null);
    try {
      const res = await fetch(`${apiBaseUrl()}${BASE}/${cancelling.source}/${cancelling.id}/cancel`, {
        method: "POST",
        headers: { ...authHeaders(), "content-type": "application/json" },
        body: JSON.stringify({ reason: reason.trim() }),
      });
      if (!res.ok) {
        setActionError(`The cancel was not recorded (${res.status}). Nothing changed.`);
        return;
      }
      setNotice("Cancelled. The decision is on record, and the row will not be revived by a redrive.");
      setCancelling(null);
      setReason("");
      setReasonTouched(false);
      await load();
    } catch {
      setActionError("The cancel could not be sent — check the connection and try again.");
    } finally {
      setBusyKey(null);
    }
  };

  const resume = async () => {
    if (resuming === null) return;
    setBusyKey(resuming.id);
    setActionError(null);
    setNotice(null);
    try {
      const res = await fetch(`${apiBaseUrl()}${BASE}/destinations/${resuming.id}/resume`, {
        method: "POST",
        headers: authHeaders(),
      });
      if (!res.ok && res.status !== 204) {
        setActionError(`The destination was not resumed (${res.status}). It is still paused.`);
        return;
      }
      setNotice(
        "Resumed with a full failure budget: the destination gets the whole allowance again before it pauses.",
      );
      setResuming(null);
      await load();
    } catch {
      setActionError("The resume could not be sent — check the connection and try again.");
    } finally {
      setBusyKey(null);
    }
  };

  if (auth.status !== "authed") return null;

  if (!canRead) {
    return (
      <div className="grid max-w-5xl gap-6">
        <SettingsHeader
          title="Dead work"
          description="Work every ledger gave up on, and the two decisions an operator can make about it."
        />
        <NotAvailable
          what="Dead work"
          hint="this page is limited to roles that can read the admin module"
        />
      </div>
    );
  }

  return (
    <div className="grid max-w-5xl gap-6">
      <SettingsHeader
        title="Dead work"
        description="Work a ledger abandoned after its last attempt: jobs, outbound email, signed webhook deliveries and stream events. Newest abandonment first."
      />

      {/*
        The redrive statement is the page's most consequential sentence, so it
        is a note above the list rather than a line in the dialog: an operator
        scanning for the button should meet the consequence first.
      */}
      <div
        role="note"
        data-testid="redrive-note"
        className="rounded-[var(--m-radius-container)] border px-4 py-3 text-[13px] leading-[1.5]"
        style={{
          borderColor: "var(--m-border)",
          background: "var(--m-surface-sunken)",
          color: "var(--m-text-secondary)",
        }}
      >
        <strong className="font-semibold" style={{ color: "var(--m-text)" }}>
          A redrive re-sends.
        </strong>{" "}
        Requeueing an item does not ask the other side whether it already acted on the
        first attempt — a webhook receiver may see the same body twice. A cancel is
        terminal: the row will not be requeued by anything on this page. Both decisions
        are written to the audit trail with your name.
      </div>

      {notice !== null && (
        <div
          role="status"
          data-testid="dead-work-notice"
          className="rounded-[var(--m-radius-control)] px-3 py-2 text-[13px] font-medium"
          style={{ background: "var(--m-status-success-bg)", color: "var(--m-status-success-fg)" }}
        >
          {notice}
        </div>
      )}

      {loadError === "forbidden" ? (
        <NotAvailable what="Dead work" hint="your role cannot read the admin module" />
      ) : loadError !== null ? (
        <div
          role="alert"
          data-testid="dead-work-load-error"
          className="flex flex-wrap items-center gap-3 rounded-[var(--m-radius-control)] px-3 py-2 text-[13px] font-medium"
          style={{ background: "var(--m-status-due-bg)", color: "var(--m-status-due-fg)" }}
        >
          <span className="flex-1">{loadError}</span>
          <Button variant="secondary" size="sm" onClick={() => void load()}>
            Try again
          </Button>
        </div>
      ) : null}

      {actionError !== null && (
        <div
          role="alert"
          data-testid="dead-work-action-error"
          className="rounded-[var(--m-radius-control)] px-3 py-2 text-[13px] font-medium"
          style={{ background: "var(--m-status-due-bg)", color: "var(--m-status-due-fg)" }}
        >
          {actionError}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <select
          className="m-input"
          aria-label="Restrict to one ledger"
          value={source}
          onChange={(e) => setSource(e.target.value as "" | DeadWorkSource)}
        >
          <option value="">Every ledger</option>
          {SOURCES.map((s) => (
            <option key={s.value} value={s.value}>
              {s.label}
            </option>
          ))}
        </select>
        <label className="inline-flex items-center gap-1.5 text-[12px]">
          <input
            type="checkbox"
            checked={includeCancelled}
            onChange={(e) => setIncludeCancelled(e.target.checked)}
          />
          Include rows an operator already cancelled
        </label>
        <span className="flex-1" />
        <Button variant="ghost" size="sm" onClick={() => void load()} disabled={loading}>
          Refresh
        </Button>
      </div>

      <Card className="overflow-x-auto">
        {loading && page === null ? (
          <div className="flex justify-center p-6">
            <Spinner />
          </div>
        ) : items.length === 0 ? (
          <div className="p-4">
            <EmptyState
              message={
                source === ""
                  ? "Nothing is dead. Every ledger drained its last attempt."
                  : `Nothing in ${sourceLabel(source).toLowerCase()} is dead.`
              }
            />
          </div>
        ) : (
          <Table data-testid="dead-work-table">
            <thead>
              <tr>
                <Th>Ledger</Th>
                <Th>What it is</Th>
                <Th className="tabular-nums">Attempts</Th>
                <Th className="tabular-nums">Dead since</Th>
                <Th>Last error</Th>
                <Th>
                  <span className="sr-only">Actions</span>
                </Th>
              </tr>
            </thead>
            <tbody>
              {items.map((item) => {
                const key = markKey(item);
                const busy = busyKey === key;
                return (
                  <Tr key={key} data-testid={`dead-work-row-${key}`}>
                    <Td>
                      <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                        {sourceLabel(item.source)}
                      </span>
                    </Td>
                    <Td>
                      <div className="grid gap-0.5">
                        <span
                          className="break-all text-[13px] font-medium"
                          style={{ color: "var(--m-text)" }}
                        >
                          {item.subject}
                        </span>
                        <span
                          className="font-mono text-[11px]"
                          style={{ color: "var(--m-text-muted)" }}
                        >
                          {item.id.slice(0, 8)}
                        </span>
                      </div>
                    </Td>
                    <Td className="tabular-nums">
                      <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                        {item.attempts}
                      </span>
                    </Td>
                    <Td className="tabular-nums">
                      <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                        {formatInTimeZone(item.dead_since, timeZone, "datetime")}
                      </span>
                    </Td>
                    <Td>
                      <span
                        className="line-clamp-2 text-[11px]"
                        style={{ color: "var(--m-text-muted)" }}
                        title={item.last_error ?? undefined}
                      >
                        {item.last_error ?? "—"}
                      </span>
                    </Td>
                    <Td>
                      <div className="flex flex-wrap items-center justify-end gap-1">
                        {item.cancelled_at !== null ? (
                          <Badge tone="neutral">cancelled</Badge>
                        ) : item.replayable ? (
                          <Button
                            variant="secondary"
                            size="sm"
                            disabled={busy || !canAct}
                            onClick={() => {
                              setRedriving(item);
                              setActionError(null);
                              setNotice(null);
                            }}
                            data-testid={`redrive-${key}`}
                          >
                            Redrive
                          </Button>
                        ) : (
                          <Badge tone="neutral">not replayable here</Badge>
                        )}
                        {item.cancelled_at === null && (
                          <Button
                            variant="ghost"
                            size="sm"
                            disabled={busy || !canAct}
                            onClick={() => {
                              setCancelling(item);
                              setReason("");
                              setReasonTouched(false);
                              setActionError(null);
                              setNotice(null);
                            }}
                            data-testid={`cancel-${key}`}
                          >
                            Cancel
                          </Button>
                        )}
                      </div>
                    </Td>
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}
        <div
          className="flex min-h-13 flex-wrap items-center gap-3 px-3 pb-2 text-[13px]"
          style={{ color: "var(--m-text-secondary)" }}
        >
          <span data-testid="dead-work-counts">
            <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
              {items.length}
            </b>{" "}
            shown
            {source === "" ? " across every ledger" : ` in ${sourceLabel(source).toLowerCase()}`}
          </span>
          {page?.next_offset !== undefined ? (
            <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              Showing the newest 200; older rows are not on this page yet.
            </span>
          ) : null}
          <span className="flex-1" />
          <Link
            href="/audit"
            className="text-xs underline underline-offset-2"
            style={{ color: "var(--m-text-muted)" }}
          >
            Every redrive and cancel is in the audit trail
          </Link>
        </div>
      </Card>

      {/* --- paused destinations ------------------------------------------ */}
      <Card>
        <div className="grid gap-2 p-4">
          <div className="flex flex-wrap items-baseline justify-between gap-2">
            <h2 className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
              Webhook destinations
            </h2>
            <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              paused first, then closest to pausing
            </span>
          </div>
          {destinations.length === 0 ? (
            <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              No webhook destinations are configured, so nothing can pause.
            </p>
          ) : (
            destinations.map((d) => (
              <div
                key={d.id}
                data-testid={`destination-${d.id}`}
                className="flex flex-wrap items-center gap-2 py-1.5"
                style={{ borderTop: "1px solid var(--m-border)" }}
              >
                <code className="min-w-0 flex-1 break-all text-[12px]" style={{ color: "var(--m-text)" }}>
                  {d.url}
                </code>
                <span className="text-[11px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                  {d.consecutive_failures} consecutive failures
                </span>
                {d.paused_at !== null ? (
                  <>
                    <Badge tone="warning">paused</Badge>
                    <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                      since {formatInTimeZone(d.paused_at, timeZone, "date")}
                      {d.paused_reason !== null ? ` — ${d.paused_reason}` : ""}
                    </span>
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={busyKey === d.id || !canAct}
                      onClick={() => {
                        setResuming(d);
                        setActionError(null);
                        setNotice(null);
                      }}
                      data-testid={`resume-${d.id}`}
                    >
                      Resume
                    </Button>
                  </>
                ) : (
                  <Badge tone={d.enabled ? "neutral" : "warning"}>
                    {d.enabled ? "delivering" : "disabled"}
                  </Badge>
                )}
              </div>
            ))
          )}
          {paused.length === 0 && destinations.length > 0 ? (
            <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              Nothing is paused. A destination pauses after its failure allowance is spent;
              resuming it resets the counter.
            </p>
          ) : null}
        </div>
      </Card>

      {/* --- redrive ------------------------------------------------------ */}
      <Dialog open={redriving !== null} onOpenChange={(open) => !open && setRedriving(null)}>
        <DialogContent
          title="Redrive this item?"
          description="The row is requeued in place, due now, with a fresh attempt budget. A webhook keeps its de-duplication key, so the replay cannot deliver the same event twice — but the receiver may still see a body it already acted on."
        >
          {redriving !== null ? (
            <dl className="grid gap-x-4 gap-y-2 text-xs sm:grid-cols-[8rem_1fr]">
              <dt style={{ color: "var(--m-text-muted)" }}>Ledger</dt>
              <dd style={{ color: "var(--m-text)" }}>{sourceLabel(redriving.source)}</dd>
              <dt style={{ color: "var(--m-text-muted)" }}>Item</dt>
              <dd className="break-all" style={{ color: "var(--m-text)" }}>
                {redriving.subject}
              </dd>
              <dt style={{ color: "var(--m-text-muted)" }}>Attempts</dt>
              <dd style={{ color: "var(--m-text)" }}>{redriving.attempts}</dd>
              <dt style={{ color: "var(--m-text-muted)" }}>Last error</dt>
              <dd style={{ color: "var(--m-text)" }}>{redriving.last_error ?? "—"}</dd>
            </dl>
          ) : null}
          <DialogActions>
            <Button variant="ghost" onClick={() => setRedriving(null)} disabled={busyKey !== null}>
              Leave it dead
            </Button>
            <Button variant="primary" onClick={() => void redrive()} disabled={busyKey !== null}>
              Re-send it now
            </Button>
          </DialogActions>
        </DialogContent>
      </Dialog>

      {/* --- cancel ------------------------------------------------------- */}
      <Dialog open={cancelling !== null} onOpenChange={(open) => !open && closeCancel()}>
        <DialogContent
          title="Cancel this work?"
          description="Cancelling says this work is not wanted. It is terminal: a redrive will be refused afterwards, so a later operator cannot quietly overturn it. The reason is required and is recorded with your name."
        >
          {cancelling !== null ? (
            <div className="grid gap-2">
              <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                {sourceLabel(cancelling.source)} · {cancelling.subject}
              </span>
              <Label htmlFor="cancel-reason">Why is it not wanted?</Label>
              <Textarea
                id="cancel-reason"
                value={reason}
                onChange={(e) => setReason(e.target.value)}
                onBlur={() => setReasonTouched(true)}
                aria-invalid={showReasonError}
                maxLength={500}
                rows={3}
                placeholder="Receiver retired; the customer moved off this integration."
              />
              <FieldHint>A month from now this line is the whole explanation.</FieldHint>
              {showReasonError ? <FieldError>A reason is required to cancel.</FieldError> : null}
            </div>
          ) : null}
          <DialogActions>
            <Button variant="ghost" onClick={closeCancel} disabled={busyKey !== null}>
              Keep it
            </Button>
            <Button
              variant="primary"
              onClick={() => void cancel()}
              disabled={busyKey !== null || !reasonOk}
            >
              Cancel this work
            </Button>
          </DialogActions>
        </DialogContent>
      </Dialog>

      {/* --- resume ------------------------------------------------------- */}
      <Dialog open={resuming !== null} onOpenChange={(open) => !open && setResuming(null)}>
        <DialogContent
          title="Resume this destination?"
          description="New events are matched for it again, and the failure counter resets — it gets the full allowance before it pauses again. Resume only once the receiver is fixed: the next failures are real."
        >
          {resuming !== null ? (
            <p className="break-all text-xs" style={{ color: "var(--m-text)" }}>
              {resuming.url}
            </p>
          ) : null}
          <DialogActions>
            <Button variant="ghost" onClick={() => setResuming(null)} disabled={busyKey !== null}>
              Leave it paused
            </Button>
            <Button variant="primary" onClick={() => void resume()} disabled={busyKey !== null}>
              Resume deliveries
            </Button>
          </DialogActions>
        </DialogContent>
      </Dialog>
    </div>
  );
}
