// Wrap-up sheet — the workbar's "Wrap up" as a real close flow (Wave 15
// lane 7).
//
// Purpose: the mockup's right-side sheet on real writes. Captured-work
// summary from the active timer session (server segments), a suggested
// time-entry draft (notes from the session), work type + billable
// selectors, and the next-step choice grid (Resolve / Schedule /
// Waiting-on-customer / Keep working). Resolve → stop the timer (which
// mints the time entry server-side) then apply the board's closed status
// through the instant-first envelope; a typed 409 (version conflict) or
// 422 (workflow rejection) renders inline and rolls the optimistic patch
// back. Schedule / Waiting → stop + move to a Waiting-category status;
// Keep working → create the time entry from the session and keep the
// timer-free ticket open. A ticket with no timer session renders the
// typed state and still allows the status transitions.
// Non-purpose: no client email step here (the classic close flow keeps
// that; this sheet is the fast path).

"use client";

import type { components } from "@meridian/sdk";
import { Button, Textarea } from "@meridian/ui";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { api } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { useDialogA11y } from "../../lib/a11y/use-dialog-a11y";
import {
  formatElapsed,
  sessionElapsedSecs,
} from "../../lib/use-timer";
import { transitionAllowed } from "../ticket-status-bar";

type TimerSession = components["schemas"]["TimerSession"];
type BoardStatus = components["schemas"]["BoardStatus"];
type BoardTransitionsResponse = components["schemas"]["BoardTransitionsResponse"];
type TimeEntry = components["schemas"]["TimeEntry"];

export type WrapNext = "resolve" | "schedule" | "waiting" | "keep";

const CHOICES: { value: WrapNext; label: string; hint: string }[] = [
  { value: "resolve", label: "✓ Resolve", hint: "Close when verified" },
  { value: "schedule", label: "◷ Schedule", hint: "Set a follow-up" },
  { value: "waiting", label: "⌁ Waiting on customer", hint: "Pause until reply" },
  { value: "keep", label: "↻ Keep working", hint: "Continue this session" },
];

export interface WrapUpSheetProps {
  open: boolean;
  onCloseSheet(): void;
  ticketId: string;
  ticketNumber: string;
  statuses: BoardStatus[];
  workflow: BoardTransitionsResponse;
  statusId: string | null;
  /** Applies a status change through the page's instant-first mutate. */
  onChangeStatus(statusId: string): Promise<void>;
  /** Refreshes the caller's timer + ticket models after writes. */
  onRefresh(): void;
}

function suggestedDraft(session: TimerSession | null, ticketNumber: string): string {
  if (session === null) return "";
  const notes = (session.segments ?? [])
    .map((s) => s.reason ?? s.end_kind)
    .filter((r): r is string => r != null && r !== "");
  const uniq = [...new Set(notes)];
  const base = uniq.length > 0 ? uniq.join("; ") : "Worked the ticket";
  return `${base} on ${ticketNumber}.`;
}

export function WrapUpSheet(props: WrapUpSheetProps): ReactNode {
  if (!props.open) return null;
  return <WrapUpSheetInner {...props} />;
}

function WrapUpSheetInner({
  onCloseSheet,
  ticketId,
  ticketNumber,
  statuses,
  workflow,
  statusId,
  onChangeStatus,
  onRefresh,
}: WrapUpSheetProps): ReactNode {
  // Inner mounts only while open; the hook traps focus, closes on Escape,
  // and restores focus to the trigger on unmount.
  const sheetRef = useDialogA11y<HTMLElement>(true, onCloseSheet);
  const [next, setNext] = useState<WrapNext>("resolve");
  const [draft, setDraft] = useState("");
  const [workType, setWorkType] = useState("Remote Support");
  const [billable, setBillable] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [entries, setEntries] = useState<TimeEntry[] | null>(null);
  const [session, setSession] = useState<TimerSession | null>(null);

  useEffect(() => {
    if (!open) return;
    void api.api
      .GET("/api/v1/tickets/{id}/time-entries", { params: { path: { id: ticketId } } })
      .then(({ data }) => setEntries(data ?? []))
      .catch(() => setEntries([]));
    void api.api
      .GET("/api/v1/timers/active")
      .then(({ data }) => {
        const active = data as components["schemas"]["TimerSession"] | null | undefined;
        setSession(active?.ticket_id === ticketId ? (active ?? null) : null);
      })
      .catch(() => setSession(null));
  }, [open, ticketId]);

  // Suggested draft: once the session is known, seed the notes from the
  // preserved pause reasons; keep the tech's edits afterwards.
  const seededRef = useRef(false);
  useEffect(() => {
    if (!open || seededRef.current || session === null) return;
    seededRef.current = true;
    setDraft(suggestedDraft(session, ticketNumber));
  }, [open, session, ticketNumber]);

  const captured = useMemo(
    () => (session === null ? null : sessionElapsedSecs(session, Date.now())),
    [session],
  );

  const closedStatuses = statuses.filter((s) => s.category === "closed");
  const waitingStatuses = statuses.filter((s) => s.category === "waiting");
  const resolveTarget =
    statusId !== null ? closedStatuses.find((s) => transitionAllowed(statuses, workflow, statusId, s.id)) ?? null : null;
  const waitingTarget =
    statusId !== null ? waitingStatuses.find((s) => transitionAllowed(statuses, workflow, statusId, s.id)) ?? null : null;

  async function stopTimer(): Promise<boolean> {
    if (session === null) return true;
    const result = await runEnvelopeMutation({
      describe: `Stop timer on ${ticketNumber}`,
      entityId: ticketId,
      buildBody: (base) => ({
        session_id: session.id,
        notes: draft.trim() === "" ? null : draft.trim(),
        billable,
        ...base,
      }),
      optimistic: () => undefined,
      revert: () => undefined,
      request: (client, typed) => client.POST("/api/v1/timers/stop", { body: typed }),
    });
    if (!result.ok) {
      setError(result.friendly ?? "The timer did not stop — try again.");
      return false;
    }
    return true;
  }

  async function applyStatus(target: BoardStatus | null): Promise<boolean> {
    if (target === null || target.id === statusId) return true;
    try {
      await onChangeStatus(target.id);
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : "The status change was rejected.");
      return false;
    }
  }

  async function confirm(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      // Keep working: the session stays live — nothing to stop or move.
      if (next === "keep") {
        onCloseSheet();
        return;
      }
      const stopped = await stopTimer();
      if (!stopped) return;
      const target = next === "resolve" ? resolveTarget : waitingTarget;
      if (next === "resolve" && resolveTarget === null && closedStatuses.length > 0) {
        setError("This board's workflow does not allow closing from the current status.");
        return;
      }
      const moved = await applyStatus(target);
      if (!moved) return;
      onRefresh();
      onCloseSheet();
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50"
      style={{ background: "var(--m-scrim)" }}
      role="dialog"
      aria-modal="true"
      aria-label="Wrap up"
      data-testid="wb-wrapup"
      onClick={(e) => {
        if (e.target === e.currentTarget) onCloseSheet();
      }}
    >
      <aside
        ref={sheetRef}
        tabIndex={-1}
        className="m-wb-sheet absolute bottom-0 right-0 top-0 flex w-full max-w-[420px] flex-col border-l"
        style={{ background: "var(--m-surface)", borderColor: "var(--m-border)", boxShadow: "var(--m-shadow-overlay)" }}
      >
        <div className="flex items-center border-b px-4 py-3.5" style={{ borderColor: "var(--m-border)" }}>
          <h2 className="m-0 text-[17px] font-bold tracking-[-0.02em]" style={{ color: "var(--m-text)" }}>Wrap up</h2>
          <button
            type="button"
            aria-label="Close wrap-up"
            className="ml-auto cursor-pointer border-none bg-transparent text-[14px]"
            style={{ color: "var(--m-text-muted)" }}
            onClick={onCloseSheet}
          >
            ×
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
          <div className="mb-4 flex gap-2.5">
            <div className="flex-1 rounded-lg border p-2.5" style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)" }}>
              <div className="font-mono text-[15px] font-bold" style={{ color: "var(--m-text)" }}>
                {captured !== null ? formatElapsed(captured).replace(/^00:/, "") : "—"}
              </div>
              <div className="mt-0.5 text-[10px]" style={{ color: "var(--m-text-muted)" }}>Captured work</div>
            </div>
            <div className="flex-1 rounded-lg border p-2.5" style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)" }}>
              <div className="text-[15px] font-bold" style={{ color: "var(--m-text)" }}>{billable ? "Billable" : "Non-billable"}</div>
              <div className="mt-0.5 text-[10px]" style={{ color: "var(--m-text-muted)" }}>{workType}</div>
            </div>
          </div>

          {captured === null && (
            <p
              className="mb-3 rounded-lg border border-dashed px-2.5 py-2 text-[11px]"
              style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}
              data-testid="wb-wrapup-no-timer"
            >
              No timer is running on this ticket — the sheet will only apply the status change. Log time from the Time Entries tab if needed.
            </p>
          )}

          <div className="mb-1.5 text-[10px] font-extrabold uppercase tracking-[0.06em]" style={{ color: "var(--m-text-muted)" }}>
            Suggested time entry
          </div>
          <Textarea
            rows={3}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            aria-label="Suggested time entry notes"
            data-testid="wb-wrapup-draft"
          />

          <div className="mb-1.5 mt-3.5 text-[10px] font-extrabold uppercase tracking-[0.06em]" style={{ color: "var(--m-text-muted)" }}>
            Work type
          </div>
          <div className="flex items-center justify-between rounded-lg border px-2.5" style={{ borderColor: "var(--m-border)", height: 35 }}>
            <select
              aria-label="Work type"
              value={workType}
              onChange={(e) => setWorkType(e.target.value)}
              className="w-full cursor-pointer border-0 bg-transparent text-[12px] outline-none"
              style={{ color: "var(--m-text)" }}
            >
              {["Remote Support", "On-site", "Project work", "Consulting", "Internal"].map((t) => (
                <option key={t} value={t}>{t}</option>
              ))}
            </select>
            <span aria-hidden style={{ color: "var(--m-text-muted)" }}>⌄</span>
          </div>

          <div className="mb-1.5 mt-3.5 text-[10px] font-extrabold uppercase tracking-[0.06em]" style={{ color: "var(--m-text-muted)" }}>
            Billable
          </div>
          <div className="flex items-center justify-between rounded-lg border px-2.5" style={{ borderColor: "var(--m-border)", height: 35 }}>
            <select
              aria-label="Billable"
              value={billable ? "yes" : "no"}
              onChange={(e) => setBillable(e.target.value === "yes")}
              className="w-full cursor-pointer border-0 bg-transparent text-[12px] outline-none"
              style={{ color: "var(--m-text)" }}
            >
              <option value="yes">● Yes</option>
              <option value="no">○ No</option>
            </select>
            <span aria-hidden style={{ color: "var(--m-text-muted)" }}>⌄</span>
          </div>

          <div className="mb-1.5 mt-3.5 text-[10px] font-extrabold uppercase tracking-[0.06em]" style={{ color: "var(--m-text-muted)" }}>
            What happens next?
          </div>
          <div className="grid grid-cols-2 gap-2" role="radiogroup" aria-label="Next step">
            {CHOICES.map((c) => {
              const active = next === c.value;
              const unavailable =
                (c.value === "resolve" && resolveTarget === null && closedStatuses.length > 0) ||
                ((c.value === "waiting" || c.value === "schedule") && waitingTarget === null && waitingStatuses.length > 0);
              return (
                <button
                  key={c.value}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  disabled={unavailable}
                  title={unavailable ? "This board's workflow does not allow that transition from the current status" : undefined}
                  data-testid={`wb-wrapup-choice-${c.value}`}
                  className="m-wb-choice cursor-pointer rounded-lg border p-2.5 text-left"
                  style={
                    active
                      ? { borderColor: "var(--m-accent)", background: "var(--m-accent-soft)", boxShadow: `inset 0 0 0 1px var(--m-accent-muted)` }
                      : { borderColor: "var(--m-border)", background: "var(--m-surface)" }
                  }
                  onClick={() => setNext(c.value)}
                >
                  <strong className="block text-[11.5px]" style={{ color: "var(--m-text)" }}>{c.label}</strong>
                  <span className="mt-0.5 block text-[9.5px]" style={{ color: "var(--m-text-muted)" }}>
                    {c.value === "resolve" && resolveTarget !== null ? `→ ${resolveTarget.name}` : c.hint}
                  </span>
                </button>
              );
            })}
          </div>

          {entries !== null && (
            <p className="mt-3 text-[10.5px]" style={{ color: "var(--m-text-muted)" }}>
              {entries.length} time entr{entries.length === 1 ? "y" : "ies"} already logged on this ticket.
            </p>
          )}
          {error !== null && (
            <p className="mt-3 text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="wb-wrapup-error">
              {error}
            </p>
          )}
        </div>

        <div className="flex justify-end gap-2 border-t px-4 py-3" style={{ borderColor: "var(--m-border)" }}>
          <Button size="sm" variant="ghost" onClick={onCloseSheet} disabled={busy}>
            Cancel
          </Button>
          <Button
            size="sm"
            variant="primary"
            data-testid="wb-wrapup-confirm"
            disabled={busy}
            onClick={() => void confirm()}
          >
            {busy ? "Working…" : next === "keep" ? "Save & continue" : "Save & apply"}
          </Button>
        </div>
      </aside>
    </div>
  );
}
