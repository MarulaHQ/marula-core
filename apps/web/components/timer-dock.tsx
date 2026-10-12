// WorkBar — the dark bottom work bar (Wave 15 lane 6, mockup paradigm).
//
// Purpose: the restyled docked ACTIVE TIMER bar per the canonical mockup
// (design/mockups/marula-ticket-workbench-html-mockup-v2.html): live
// work-dot + mono elapsed digits, work meta (ticket number · title ·
// preserved pause-reason chip), a quick note input (posts an internal
// ticket comment through the instant-first envelope), and the primary
// "Wrap up" action (stops the session, carrying the note). Pause/Resume
// and Switch stay as quiet secondary actions.
//
// In-flow shell chrome per the p11-ui-quality occlusion fix: this bar is a
// flex child of the shell column — NEVER a fixed overlay occluding content.
//
// Non-purpose: no timer math (server owns segments — useTimer reconciles
// via timer.* WS events); no ticket queue — Switch deep-links to /today
// where per-ticket start lives.
"use client";

import {
  Badge,
} from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { api, useAuth } from "../lib/api";
import { runEnvelopeMutation } from "../lib/envelope-write";
import {
  formatElapsed,
  lastSegmentReason,
  sessionElapsedSecs,
  useTimer,
} from "../lib/use-timer";
import { selectWorkSession } from "../lib/work-session";
import {
  WorkbarNotesPanel,
} from "./notes/workbar-notes-panel";
import { registerWorkbarTarget } from "./notes/workbar-bridge";
import { requestWrapUp } from "./workbench-v2/wrap-up-bridge";

type Ticket = components["schemas"]["Ticket"];




/** Up-arrow trigger icon (the notepad opener), house stroke weight. */

export function TimerDock() {
  const auth = useAuth();
  const router = useRouter();
  const { active, sessions, nowMs, refresh } = useTimer();
  // Notes & Tasks panel (Wave 17 lane 11): view state only — the panel
  // never unmounts while the bar has a tracked session (display swap), so
  // composer text and tasks scroll persist across open/close.
  const [panelOpen, setPanelOpen] = useState(false);
  const [panelFocusItem, setPanelFocusItem] = useState<string | null>(null);
  const panelTriggerRef = useRef<HTMLButtonElement | null>(null);
  // The bar follows the ticket being worked: the running session, or, when
  // everything is paused, the most recently touched open session. See
  // lib/work-session.ts for the rule and its test.
  const current = selectWorkSession(active, sessions);
  const [ticket, setTicket] = useState<Ticket | null>(null);
  const [note, setNote] = useState("");
  const [noteState, setNoteState] = useState<"idle" | "saving" | "saved">("idle");
  const [noteError, setNoteError] = useState<string | null>(null);
  const [stopping, setStopping] = useState(false);
  const [stopError, setStopError] = useState<string | null>(null);
  const flashTimer = useRef<number | null>(null);

  useEffect(() => {
    if (auth.status !== "authed" || current === null) {
      setTicket(null);
      return;
    }
    let cancelled = false;
    void api.api
      .GET("/api/v1/tickets/{id}", { params: { path: { id: current.ticket_id } } })
      .then(({ data }) => {
        if (!cancelled) setTicket(data ?? null);
      });
    return () => {
      cancelled = true;
    };
  }, [auth.status, current?.ticket_id]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(
    () => () => {
      if (flashTimer.current !== null) window.clearTimeout(flashTimer.current);
    },
    [],
  );

  // Option-B bridge registration: any closing surface (gate dialog,
  // wrap-up sheet) can ask the shell to open the panel pre-scrolled to a
  // given unassigned checklist row. One slot, replaced on session change.
  useEffect(() => {
    registerWorkbarTarget((req) => {
      if (req.ticketId !== current?.ticket_id) return;
      setPanelFocusItem(req.focusItemId ?? null);
      setPanelOpen(true);
    });
    return () => registerWorkbarTarget(null);
  }, [current?.ticket_id]);

  if (auth.status !== "authed" || current === null) return null;

  const live = current.stopped_at === null && current.segments?.slice(-1)[0]?.ended_at === null;
  const elapsed = sessionElapsedSecs(current, nowMs);
  const reason = lastSegmentReason(current);
  const ticketNumber = ticket?.number ?? current.ticket_id.slice(0, 8);
  const ticketTitle = ticket?.title ?? "…";

  async function pauseOrResume() {
    if (current === null) return;
    if (live) {
      await api.api.POST("/api/v1/timers/pause", { body: { session_id: current.id } });
    } else {
      await api.api.POST("/api/v1/timers/resume", { body: { session_id: current.id } });
    }
    void refresh();
  }

  /** Quick note: posts an internal comment on the tracked ticket. */
  async function saveNote() {
    const body = note.trim();
    if (body.length === 0 || current === null) return;
    setNoteState("saving");
    setNoteError(null);
    const result = await runEnvelopeMutation({
      describe: `Add work note to ${ticketNumber}`,
      entityId: current.ticket_id,
      buildBody: (base) => ({ body, kind: "internal" as const, ...base }),
      optimistic: () => undefined,
      revert: () => undefined,
      request: (client, typed) =>
        client.POST("/api/v1/tickets/{id}/comments", {
          params: { path: { id: current.ticket_id } },
          body: typed,
        }),
    });
    if (!result.ok) {
      setNoteState("idle");
      setNoteError(result.friendly ?? "That note did not save — try again.");
      return;
    }
    setNote("");
    setNoteState("saved");
    if (flashTimer.current !== null) window.clearTimeout(flashTimer.current);
    flashTimer.current = window.setTimeout(() => setNoteState("idle"), 4000);
  }

  /** Primary action: wrap up. When the ticket page is mounted, hand the
   *  request to the workbench's wrap-up sheet (stop → mint the time entry
   *  → apply the next status); elsewhere, stop directly and carry the
   *  note. */
  async function wrapUp() {
    if (current === null) return;
    if (requestWrapUp({ ticketId: current.ticket_id })) return;
    setStopping(true);
    setStopError(null);
    const body = note.trim();
    const result = await runEnvelopeMutation({
      describe: `Stop timer on ${ticketNumber}`,
      entityId: current.ticket_id,
      buildBody: (base) => ({ session_id: current.id, notes: body || null, ...base }),
      optimistic: () => undefined,
      revert: () => undefined,
      request: (client, typed) => client.POST("/api/v1/timers/stop", { body: typed }),
    });
    setStopping(false);
    if (!result.ok) {
      setStopError(result.friendly ?? "The timer did not stop — try again.");
      return;
    }
    setNote("");
    void refresh();
  }

  return (
    <div
      role="region"
      aria-label="Work bar"
      data-testid="work-bar"
      className="m-workbar flex shrink-0 flex-wrap items-center gap-x-4 gap-y-2 border-t px-3 py-2 md:px-4"
    >
      <div className="flex items-center gap-2.5">
        <span
          aria-hidden="true"
          className={live ? "m-work-dot m-work-dot-live" : "m-work-dot m-work-dot-paused"}
        />
        <span
          data-testid="timer-elapsed"
 className="m-workbar-timer text-2xl font-semibold tabular-nums leading-none"
        >
          {formatElapsed(elapsed)}
        </span>
      </div>

      <div
        aria-hidden="true"
        className="hidden h-5 w-px md:block"
        style={{ background: "var(--m-border-dark)" }}
      />

      <button
        type="button"
        onClick={() => router.push(`/tickets/${current.ticket_id}`)}
        className="m-workbar-meta hidden min-w-0 max-w-[380px] flex-shrink items-baseline gap-2.5 overflow-hidden text-left md:flex"
      >
        <span className="m-workbar-number truncate text-[13px] font-semibold">
          {ticketNumber}
        </span>
        <span className="m-workbar-title min-w-0 truncate text-[13px]">{ticketTitle}</span>
      </button>
      <span className="m-workbar-number shrink-0 text-[13px] font-semibold md:hidden">
        {ticketNumber}
      </span>

      <span className="hidden shrink-0 sm:contents">
        <Badge tone={live ? "success" : "warning"}>{live ? "tracking" : (reason ?? "paused")}</Badge>
      </span>

      <div className="ml-auto flex shrink-0 items-center gap-2">
        <span aria-live="polite" className="m-workbar-note-feedback hidden text-[11px] lg:inline">
          {noteError ?? stopError ?? (noteState === "saved" ? "Note saved" : noteState === "saving" ? "Saving…" : "")}
        </span>
        <input
          data-testid="workbar-note-input"
          value={note}
          onChange={(event) => {
            setNote(event.target.value);
            setNoteError(null);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") void saveNote();
          }}
          placeholder="Add note…"
          aria-label="Add a note to this ticket"
          className="m-workbar-note h-[var(--m-density-control)] min-w-16 max-w-[260px] flex-1 rounded-lg border px-2.5 text-[12.5px] outline-none md:flex-none md:w-56"
        />
        <button
          type="button"
          ref={panelTriggerRef}
          data-testid="workbar-panel-trigger"
          aria-expanded={panelOpen}
          aria-controls="notes-task-panel"
          title="Notes & tasks (⌘⇧N)"
          className="m-workbar-ghost m-workbar-panel-trigger flex h-[var(--m-density-control)] w-8 shrink-0 items-center justify-center"
          onClick={() => {
            setPanelFocusItem(null);
            setPanelOpen(true);
          }}
        >
          <Icon name="up" size={12} />
        </button>
        <button
          type="button"
          onClick={() => void pauseOrResume()}
          aria-label={live ? "Pause timer" : "Resume timer"}
          className="m-workbar-ghost flex h-[var(--m-density-control)] w-8 shrink-0 cursor-pointer items-center justify-center rounded-lg border-0"
        >
          {live ? <Icon name="pause" size={12} /> : <Icon name="play" size={12} />}
        </button>
        <button
          type="button"
          onClick={() => router.push("/today")}
          aria-label="Switch ticket (opens Today)"
          className="m-workbar-ghost flex h-[var(--m-density-control)] w-8 shrink-0 cursor-pointer items-center justify-center rounded-lg border-0"
        >
          <Icon name="refresh" size={12} />
        </button>
        <button
          type="button"
          data-testid="workbar-wrap-up"
          onClick={() => void wrapUp()}
          disabled={stopping}
          className="m-workbar-wrapup flex h-[var(--m-density-control)] shrink-0 cursor-pointer items-center gap-1.5 rounded-lg border-0 px-3 text-[12.5px] font-semibold"
          style={{ background: "var(--m-accent)", color: "var(--m-on-accent)" }}
        >
          <Icon name="check" size={11} />
          Wrap up
        </button>
      </div>

      {/* The Notes & Tasks notepad: an anchored layer above page content,
          below the wrap-up sheet and native dialogs (§1.2 stacking). */}
      <WorkbarNotesPanel
        open={panelOpen}
        session={current}
        triggerRef={panelTriggerRef}
        focusItemId={panelFocusItem}
        onClose={() => setPanelOpen(false)}
      />
    </div>
  );
}
