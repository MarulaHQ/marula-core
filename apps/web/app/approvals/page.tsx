// /approvals — time-entry approval queues + lock states (Wave 14 lane 7
// close-out, wiring the merged lane-1 contract through the typed SDK).
//
// Purpose: approvers (`special:approve_time`) work the org queue — bulk
// approve/reject with the typed reason vocabulary, per-item bulk outcomes
// (applied / locked / not_in_queue / not_found) flagged on the row so
// refused entries are named exactly why, the append-only decision trail
// per entry, and lock states visible (entries consumed by an invoice are
// frozen forever). Non-approvers see their own submissions; rejected
// entries reopen back to draft when not locked. Read-your-write: applied
// decisions remove rows immediately using the returned entry snapshots.
//
// Non-purpose: no time-entry editing (ticket timers own creation), no
// rates/costs (RB2 — the queue carries no financial data; billing reads
// keep their own gate).
"use client";

import { Button, Card, EmptyState, FieldHint, Input, Label, Spinner, Tag } from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useCallback, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { api, useRequireAuth } from "../../lib/api";
import { friendlyMutationText, resultErrorText } from "../../lib/error-copy";
import { useBillingSuggestions, useTimeCategories } from "../../lib/use-time-categories";

type Entry = components["schemas"]["TimeEntry"];
type ApprovalRecord = components["schemas"]["TimeEntryApprovalRecord"];

/** Wire shape of a queue/history projection. NOTE: the OpenAPI pin types
 * `entry` as a bare uuid (core-domain's `schema(value_type)`), but the
 * wire carries the FULL TimeEntry — mirror of
 * `core_domain::entities::TimeEntryQueueItem` (time_tracking.rs). The cast
 * below is the documented reconciliation until the pin is tightened. */
type QueueItem = {
  entry: Entry;
  submitted_by: string | null;
  submitted_at: string | null;
  decided_by: string | null;
  decided_at: string | null;
  locked: boolean;
};

const REJECTION_REASONS = [
  { code: "missing_notes", label: "Missing notes" },
  { code: "wrong_ticket", label: "Wrong ticket" },
  { code: "incorrect_duration", label: "Incorrect duration" },
  { code: "duplicate_entry", label: "Duplicate entry" },
  { code: "non_billable_work", label: "Non-billable work" },
  { code: "policy_violation", label: "Policy violation" },
  { code: "other", label: "Other (explain in the note)" },
] as const;

function reasonLabel(code: string | null | undefined): string {
  return REJECTION_REASONS.find((r) => r.code === code)?.label ?? (code ?? "Rejected");
}

const OUTCOME_COPY: Readonly<Record<string, string>> = {
  locked: "Already invoiced — locked, nothing to decide",
  not_in_queue: "No longer in the queue — refresh to see its current state",
  not_found: "No longer visible in your scope",
};

/** Typed unbillable-reason vocabulary (Wave 39 V3) — label map for the
 * row badge; unknown codes render verbatim so future values stay honest. */
const UNBILLABLE_REASON_LABELS: Readonly<Record<string, string>> = {
  warranty: "covered by warranty",
  courtesy: "courtesy / goodwill",
  in_retainer_block: "inside a retainer block",
  sla_covered: "covered by the SLA",
  internal: "internal work",
  mistake: "mistake — do not bill",
};

/** Category label resolver over the org taxonomy (wave 39): the row
 * shows the human label; unknown/free-text legacy slugs render as-is. */
function useCategoryLabels(): (slug: string) => string {
  const { categories } = useTimeCategories();
  return useCallback(
    (slug: string) => categories?.find((c) => c.slug === slug)?.label ?? slug,
    [categories],
  );
}

/** The decision_billing_audit chip for one row (Wave 39 V6):
 * suggestion-grade only — reading never mutates, and the chip renders
 * NOTHING when the audit surface is unconfigured (visibly-absent). */
function ApprovalAuditChip({ entryId, entryBillable }: { entryId: string; entryBillable: boolean }) {
  const suggestions = useBillingSuggestions([entryId]);
  const chip = suggestions[entryId]?.[0];
  if (chip === undefined) return null;
  const disagreeing =
    (chip.chip === "billable_false" && entryBillable) ||
    (chip.chip === "billable_true" && !entryBillable);
  const tone = disagreeing ? "var(--m-status-warning-fg, var(--m-text-secondary))" : "var(--m-text-muted)";
  return (
    <div className="mt-1" data-testid={`approval-audit-${entryId}`}>
      <span
        className="rounded-full border px-2 py-0.5 text-[10px]"
        style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)", color: tone }}
        title={`decision_billing_audit · ${chip.mode} · p=${chip.billableProbability ?? "?"}`}
      >
        ✦ Audit: {chip.label}
      </span>
    </div>
  );
}

function fmtMins(mins: number): string {
  const m = Math.max(0, Math.round(mins));
  const h = Math.floor(m / 60);
  return h > 0 ? `${h}h ${m % 60}m` : `${m}m`;
}

function fmtWhen(iso: string | null | undefined): string {
  if (!iso) return "—";
  return new Date(iso).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

type Tab = "queue" | "approved" | "rejected" | "decisions";

export default function ApprovalsPage() {
  const auth = useRequireAuth();
  const isApprover = (auth.me?.permissions ?? []).includes("special:approve_time");
  const [tab, setTab] = useState<Tab>("queue");
  const [items, setItems] = useState<QueueItem[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [rowFlags, setRowFlags] = useState<Readonly<Record<string, string>>>({});
  const [rejectOpen, setRejectOpen] = useState(false);
  const [rejectReason, setRejectReason] = useState<string>("");
  const [rejectNote, setRejectNote] = useState<string>("");
  const [rejectError, setRejectError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [history, setHistory] = useState<Readonly<Record<string, ApprovalRecord[]>>>({});
  const [historyOpenId, setHistoryOpenId] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const query: Record<string, string> = {};
      if (tab === "queue") query.status = "submitted";
      else if (tab === "approved") query.status = "approved";
      else if (tab === "rejected") query.status = "rejected";
      else {
        query.decided_by = auth.me?.user_id ?? "";
      }
      const res = await api.api.GET("/api/v1/time-entries/approvals/queue", {
        params: { query },
      });
      const data = res.data;
      if (data === undefined) {
        setError(resultErrorText(res, "Couldn't load the approval queue."));
        setItems([]);
      } else {
        setError(null);
        // Pin wart: `entry` is the full TimeEntry on the wire (see QueueItem).
        setItems(data as unknown as QueueItem[]);
      }
    } catch (e) {
      setError(friendlyMutationText(e, "Couldn't load the approval queue."));
      setItems([]);
    } finally {
      setLoading(false);
    }
  }, [tab, auth.me?.user_id]);

  useEffect(() => {
    setSelected(new Set());
    setRowFlags({});
    setRejectOpen(false);
    setRejectError(null);
    void reload();
  }, [reload]);

  function toggleSelect(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  /** Bulk decide. Applied entries leave the list immediately
   * (read-your-write from the returned snapshots); refused entries get a
   * row flag naming exactly why — never a silent guess. */
  async function decide(decision: "approve" | "reject") {
    if (selected.size === 0) return;
    if (decision === "reject" && rejectReason === "") {
      setRejectError("Pick a rejection reason — approvers reject with a typed code, not prose alone.");
      return;
    }
    setBusy(true);
    setRejectError(null);
    try {
      const res = await api.api.POST("/api/v1/time-entries/approvals/decide", {
        body: {
          entry_ids: [...selected],
          decision,
          ...(decision === "reject" && rejectReason ? { reason_code: rejectReason } : {}),
          ...(decision === "reject" && rejectNote.trim() ? { note: rejectNote.trim() } : {}),
        },
      });
      const data = res.data;
      if (data === undefined) {
        setError(resultErrorText(res));
      } else {
        const applied = new Set<string>();
        const flags: Record<string, string> = {};
        for (const item of data.results) {
          if (item.outcome === "applied") applied.add(item.entry_id);
          else flags[item.entry_id] = OUTCOME_COPY[item.outcome] ?? "Refused — refresh and retry";
        }
        setRowFlags((prev) => ({ ...prev, ...flags }));
        setItems((prev) => prev?.filter((it) => !applied.has(it.entry.id)) ?? prev);
        setSelected(new Set());
        setRejectOpen(false);
        setRejectReason("");
        setRejectNote("");
      }
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setBusy(false);
    }
  }

  async function decideIds(ids: readonly string[], decision: "approve" | "reject") {
    setBusy(true);
    setError(null);
    try {
      const res = await api.api.POST("/api/v1/time-entries/approvals/decide", {
        body: { entry_ids: [...ids], decision },
      });
      const data = res.data;
      if (data === undefined) {
        setError(resultErrorText(res));
      } else {
        const applied = new Set<string>();
        const flags: Record<string, string> = {};
        for (const item of data.results) {
          if (item.outcome === "applied") applied.add(item.entry_id);
          else flags[item.entry_id] = OUTCOME_COPY[item.outcome] ?? "Refused — refresh and retry";
        }
        setRowFlags((prev) => ({ ...prev, ...flags }));
        setItems((prev) => prev?.filter((it) => !applied.has(it.entry.id)) ?? prev);
      }
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setBusy(false);
    }
  }

  /** Rejected → draft so it can be corrected and resubmitted. */
  async function reopen(id: string) {
    setBusy(true);
    try {
      const res = await api.api.POST("/api/v1/time-entries/{id}/reopen", {
        params: { path: { id } },
      });
      if (res.data === undefined) {
        setError(resultErrorText(res));
      } else {
        setItems((prev) => prev?.filter((it) => it.entry.id !== id) ?? prev);
      }
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setBusy(false);
    }
  }

  async function loadHistory(id: string) {
    if (historyOpenId === id) {
      setHistoryOpenId(null);
      return;
    }
    setHistoryOpenId(id);
    if (history[id] !== undefined) return;
    try {
      const res = await api.api.GET("/api/v1/time-entries/{id}/approvals", {
        params: { path: { id } },
      });
      const records = res.data;
      if (records !== undefined) {
        setHistory((prev) => ({ ...prev, [id]: records }));
      }
    } catch {
      // History stays collapsed on failure; the row remains usable.
    }
  }

  const tabs: { id: Tab; label: string }[] = useMemo(() => {
    const base: { id: Tab; label: string }[] = [{ id: "queue", label: "Queue" }];
    if (isApprover) {
      base.push({ id: "approved", label: "Approved" });
      base.push({ id: "decisions", label: "My decisions" });
    }
    base.push({ id: "rejected", label: "Rejected" });
    return base;
  }, [isApprover]);

  const allSelected = items !== null && items.length > 0 && selected.size === items.length;

  return (
    <div className="mx-auto grid w-full max-w-[1100px] gap-4 p-4 md:p-6">
      <header>
        <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
          Time approvals
        </h1>
        <FieldHint>
          {isApprover
            ? "Submitted work waits here until you approve or reject it. Approved entries are the only ones billing may consume; invoiced entries lock permanently."
            : "Your submitted time entries. Approved entries go to billing; rejected entries come back here with a reason until reopened."}
        </FieldHint>
      </header>

      <div className="flex flex-wrap items-center gap-2">
        {tabs.map((t) => (
          <Button key={t.id} variant={tab === t.id ? "primary" : "secondary"} onClick={() => setTab(t.id)}>
            {t.label}
          </Button>
        ))}
      </div>

      {error && (
        <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      )}

      {isApprover && tab === "queue" && items !== null && items.length > 0 && (
        <div className="flex flex-wrap items-center gap-2" data-testid="bulk-bar">
          <label className="flex items-center gap-1.5 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            <input
              type="checkbox"
              checked={allSelected}
              onChange={() => setSelected(allSelected ? new Set() : new Set(items.map((it) => it.entry.id)))}
              aria-label="Select all"
            />
            {selected.size} selected
          </label>
          <Button disabled={selected.size === 0 || busy} onClick={() => void decideIds([...selected], "approve")}>
            Approve selected
          </Button>
          <Button
            variant="secondary"
            disabled={selected.size === 0 || busy}
            onClick={() => {
              setRejectOpen((v) => !v);
              setRejectError(null);
            }}
          >
            Reject…
          </Button>
        </div>
      )}

      {rejectOpen && selected.size > 0 && (
        <div
          className="m-rise grid gap-2 rounded-[var(--m-radius-control)] border p-3"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
          data-testid="reject-panel"
        >
          <Label>Rejection reason</Label>
          <select
            value={rejectReason}
            onChange={(e) => setRejectReason(e.target.value)}
            className="h-[var(--m-control-height)] rounded-[var(--m-radius-control)] border px-2 text-[13px]"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
            data-testid="reject-reason"
          >
            <option value="">Pick a reason…</option>
            {REJECTION_REASONS.map((r) => (
              <option key={r.code} value={r.code}>
                {r.label}
              </option>
            ))}
          </select>
          <Label>Note (optional)</Label>
          <Input
            value={rejectNote}
            onChange={(e) => setRejectNote(e.target.value)}
            placeholder="Context the submitter will see on the decision trail"
          />
          {rejectError && (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {rejectError}
            </p>
          )}
          <div>
            <Button disabled={busy} onClick={() => void decide("reject")} data-testid="reject-send">
              Reject {selected.size} entr{selected.size === 1 ? "y" : "ies"}
            </Button>
          </div>
        </div>
      )}

      {loading ? (
        <div className="p-6 flex justify-center">
          <Spinner />
        </div>
      ) : items !== null && items.length === 0 ? (
        <Card className="p-6">
          <EmptyState
            message={
              tab === "queue"
                ? "Nothing waiting for approval — submitted work lands here."
                : tab === "rejected"
                  ? "No rejected entries."
                  : tab === "approved"
                    ? "No approved entries yet."
                    : "You haven't decided any entries yet."
            }
          />
        </Card>
      ) : (
        <div className="grid gap-2">
          {items?.map((item) => (
            <QueueRow
              key={item.entry.id}
              item={item}
              tab={tab}
              isApprover={isApprover}
              busy={busy}
              meId={auth.me?.user_id ?? ""}
              flag={rowFlags[item.entry.id]}
              selected={selected.has(item.entry.id)}
              records={history[item.entry.id]}
              historyOpen={historyOpenId === item.entry.id}
              onToggle={() => toggleSelect(item.entry.id)}
              onApprove={() => void decideIds([item.entry.id], "approve")}
              onReject={() => {
                setSelected(new Set([item.entry.id]));
                setRejectOpen(true);
                setRejectError(null);
              }}
              onReopen={() => void reopen(item.entry.id)}
              onHistory={() => void loadHistory(item.entry.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function statusTone(status: string | null | undefined): "success" | "danger" | "warning" | "neutral" {
  if (status === "approved") return "success";
  if (status === "rejected") return "danger";
  if (status === "submitted") return "warning";
  return "neutral";
}

const ACTION_LABEL: Readonly<Record<string, string>> = {
  submitted: "Submitted",
  approved: "Approved",
  rejected: "Rejected",
  reopened: "Reopened",
};

function QueueRow({
  item,
  tab,
  isApprover,
  busy,
  meId,
  flag,
  selected,
  records,
  historyOpen,
  onToggle,
  onApprove,
  onReject,
  onReopen,
  onHistory,
}: {
  item: QueueItem;
  tab: Tab;
  isApprover: boolean;
  busy: boolean;
  meId: string;
  flag?: string;
  selected: boolean;
  records?: ApprovalRecord[];
  historyOpen: boolean;
  onToggle: () => void;
  onApprove: () => void;
  onReject: () => void;
  onReopen: () => void;
  onHistory: () => void;
}) {
  const e = item.entry;
  const categoryLabel = useCategoryLabels();
  const ownEntry = e.user_id === meId;
  const inQueueTab = tab === "queue" && isApprover;
  return (
    <div
      className="m-rise rounded-[var(--m-radius-control)] border p-3 transition-shadow duration-100"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      data-testid={`approval-row-${e.id}`}
    >
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="flex min-w-0 items-start gap-2">
          {inQueueTab && (
            <input
              type="checkbox"
              checked={selected}
              onChange={onToggle}
              aria-label={`Select entry on ticket ${e.ticket_id}`}
              className="mt-1"
            />
          )}
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-[13px] font-medium tabular-nums" style={{ color: "var(--m-text)" }}>
                {fmtMins(e.minutes)}
              </span>
              <Link
                href={`/tickets/${e.ticket_id}`}
                className="text-[13px] no-underline transition-opacity duration-100 hover:opacity-80"
                style={{ color: "var(--m-accent)" }}
              >
                ticket {e.ticket_id.slice(0, 8)}…
              </Link>
              {/* Time-entry context entry point (Wave 32 lane 7): log the
                  trip's actual mileage against this entry's ticket. */}
              {ownEntry && (
                <Link
                  href={`/dispatch/travel?ticket_id=${e.ticket_id}`}
                  className="text-[12px] no-underline transition-opacity duration-100 hover:opacity-80"
                  style={{ color: "var(--m-text-secondary)" }}
                  data-testid={`approval-log-travel-${e.id}`}
                >
                  log travel
                </Link>
              )}
              <Tag tone={statusTone(e.approval_status)}>{e.approval_status ?? "draft"}</Tag>
              {item.locked && <Tag tone="neutral">Locked — invoiced</Tag>}
            </div>
            <div className="mt-0.5 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              {fmtWhen(e.started_at)} → {fmtWhen(e.ended_at)}
              {item.submitted_at ? ` · submitted ${fmtWhen(item.submitted_at)}` : ""}
              {item.decided_at ? ` · decided ${fmtWhen(item.decided_at)}` : ""}
            </div>
            {e.notes && (
              <p className="mt-1 truncate text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                {e.notes}
              </p>
            )}
            <div className="mt-1 flex flex-wrap items-center gap-2 text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
              {e.work_type ? (
                <span data-testid={`approval-category-${e.id}`}>{categoryLabel(e.work_type)}</span>
              ) : (
                <span style={{ color: "var(--m-status-warning-fg, var(--m-text-muted))" }}>Uncategorized</span>
              )}
              {!e.billable && (
                <span>
                  Non-billable —{" "}
                  {UNBILLABLE_REASON_LABELS[e.unbillable_reason ?? ""] ??
                    e.unbillable_reason ??
                    "reason pending"}
                </span>
              )}
            </div>
            <ApprovalAuditChip entryId={e.id} entryBillable={e.billable} />
            {e.approval_reason_code && (
              <p role="alert" className="mt-1 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                Rejected — {reasonLabel(e.approval_reason_code)}
              </p>
            )}
          </div>
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-1.5">
          {inQueueTab && (
            <>
              <Button variant="secondary" disabled={busy} onClick={onApprove}>
                Approve
              </Button>
              <Button variant="secondary" disabled={busy} onClick={onReject}>
                Reject…
              </Button>
            </>
          )}
          {tab === "rejected" && ownEntry && !item.locked && (
            <Button variant="secondary" disabled={busy} onClick={onReopen} data-testid={`reopen-${e.id}`}>
              Reopen to draft
            </Button>
          )}
          <Button variant="ghost" onClick={onHistory} data-testid={`history-${e.id}`}>
            {historyOpen ? "Hide trail" : "Trail"}
          </Button>
        </div>
      </div>
      {flag && (
        <p role="status" className="mt-2 text-[12px]" style={{ color: "var(--m-status-warning-fg)" }}>
          {flag}
        </p>
      )}
      {historyOpen && (
        <div className="mt-2 grid gap-1 text-[12px]" style={{ color: "var(--m-text-secondary)" }} data-testid={`trail-${e.id}`}>
          {(records ?? []).length === 0 ? (
            <span style={{ color: "var(--m-text-muted)" }}>No recorded decisions yet.</span>
          ) : (
            (records ?? []).map((r) => (
              <span key={r.id}>
                <strong style={{ color: "var(--m-text)" }}>{ACTION_LABEL[r.action] ?? r.action}</strong>
                {" · "}
                {fmtWhen(r.created_at)}
                {r.reason_code ? ` · ${reasonLabel(r.reason_code)}` : ""}
                {r.note ? ` · “${r.note}”` : ""}
              </span>
            ))
          )}
        </div>
      )}
    </div>
  );
}
