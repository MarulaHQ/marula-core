// Notes & Tasks workbar panel — the Nilear-pattern notepad (Wave 17 lane
// 11, docs/notes-tasks-workbar.md). Slides up from the WorkBar's top edge:
// a slightly skeuomorphic small paper pad (ruled ledger lines aligned to
// the note row rhythm, faint rotate on the paper layer, overlay shadow)
// carrying the notes ledger and the tasks strip over the ticket checklist.
//
// One-pattern rule: the note entry inside is the SHARED `NoteComposer`;
// every submitted line is auto-bulleted by the shared formatNoteBody
// (server-verified rules), and machine-origin notes render the ⚙ machine
// avatar so humans vs workflow provenance is scannable (§2.2/§6).
//
// Motion law: transform/opacity only, ≤120ms, decel curve, stagger ≤40ms/
// item on first open; opacity-only 1ms under prefers-reduced-motion.
// Panel state is view state — the composer never unmounts while ticket
// context persists (display swap), drafts ride IF2 autosave.

"use client";

import type { components } from "@marula-core/sdk";
import { Icon } from "@marula-core/ui/kit";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { api, uploadAttachment } from "../../lib/api";
import { rejectionText } from "../../lib/error-copy";
import { formatNoteBody, initialsOf } from "../../lib/note-format";
import type { TimerSession } from "../../lib/use-timer";
import { ModelPicker, useLazySurfacePick } from "../ai/model-picker";
import { NoteComposer, type NoteComposerHandle } from "./note-composer";
import { LocalTime } from "../local-time";

type TicketComment = components["schemas"]["TicketComment"];
type ChecklistItem = components["schemas"]["ChecklistItem"];
type TicketChecklistView = components["schemas"]["TicketChecklistView"];
type OrgUser = components["schemas"]["OrgUser"];

export interface WorkbarPanelProps {
  open: boolean;
  /** The CURRENT tracked session — ticket context + session rings (§2.2). */
  session: TimerSession;
  /** Focus-restore target: the up-arrow trigger element. */
  triggerRef: { current: HTMLElement | null };
  onClose(): void;
  /** Option-B deep-link: pre-scroll the tasks strip to this row. */
  focusItemId?: string | null;
}

type UserMap = Map<string, string>;

/** §3.4 chart-ramp hash for avatar fills: stable per author id. */
const RAMP = ["--chart-1", "--chart-2", "--chart-3", "--chart-4", "--chart-5"];
function rampFor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) {
    h = (h * 31 + id.charCodeAt(i)) >>> 0;
  }
  return RAMP[h % RAMP.length] ?? RAMP[0]!;
}

export function WorkbarNotesPanel({
  open,
  session,
  triggerRef,
  onClose,
  focusItemId,
}: WorkbarPanelProps): ReactNode {
  const [comments, setComments] = useState<TicketComment[] | null>(null);
  const [checklist, setChecklist] = useState<TicketChecklistView | null>(null);
  const [users, setUsers] = useState<UserMap>(() => new Map());
  const [ticketNumber, setTicketNumber] = useState<string>(session.ticket_id.slice(0, 8));
  const [ticketTitle, setTicketTitle] = useState("");
  const [draft, setDraft] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const [staged, setStaged] = useState<File[]>([]);
  const [rowError, setRowError] = useState<string | null>(null);
  const [tasksCollapsed, setTasksCollapsed] = useState(false);
  const [reasonPrompt, setReasonPrompt] = useState<{ itemId: string; kind: "na" | "reopen"; text: string } | null>(null);
  const [revealed, setRevealed] = useState<string | null>(null);
  const composerRef = useRef<NoteComposerHandle | null>(null);
  const firstOpenRef = useRef(true);
  const unsentRef = useRef("");
  const hintUntilRef = useRef(0);
  const scrimRef = useRef<HTMLDivElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);
  const focusRowRef = useRef<HTMLLIElement | null>(null);

  const ticketId = session.ticket_id;
  const dirty = draft.trim() !== "";
  unsentRef.current = draft;

  // Ticket context.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void api.api.GET("/api/v1/tickets/{id}", { params: { path: { id: ticketId } } }).then(({ data }) => {
      if (cancelled || data === undefined) return;
      setTicketNumber(data.number);
      setTicketTitle(data.title);
    });
    return () => {
      cancelled = true;
    };
  }, [open, ticketId]);

  // Restore the autosaved draft once per ticket context.
  useEffect(() => {
    if (!open) {
      setStatus(null);
      return;
    }
    void api.api.GET("/api/v1/tickets/{id}/note-draft", { params: { path: { id: ticketId } } }).then(({ data }) => {
      if (data?.draft) {
        setDraft((prev) => (prev === "" ? data.draft!.body : prev));
      }
    });
  }, [open, ticketId]);

  const hydrate = useCallback(async () => {
    const [c, k, u] = await Promise.all([
      api.api.GET("/api/v1/tickets/{id}/comments", { params: { path: { id: ticketId } } }),
      api.api.GET("/api/v1/tickets/{id}/checklist", { params: { path: { id: ticketId } } }),
      api.api.GET("/api/v1/org/users", { params: { query: { limit: 200 } } }),
    ]);
    setComments(c.data ?? []);
    setChecklist(k.data ?? null);
    setUsers(
      new Map(
        ((u.data as { users?: OrgUser[] } | undefined)?.users ?? [])
          .filter((x) => x.id !== undefined && x.display_name !== undefined)
          .map((x) => [x.id as string, x.display_name as string]),
      ),
    );
  }, [ticketId]);

  useEffect(() => {
    if (!open) return;
    void hydrate();
  }, [open, hydrate]);

  // Session re-targets: switching tickets with the panel open re-hydrates.
  useEffect(() => {
    void hydrate();
  }, [hydrate]);

  // Keyboard toggle is TimerDock's job; here: focus + dismissal semantics.
  useEffect(() => {
    if (open) {
      const id = window.setTimeout(() => composerRef.current?.focus(), 130);
      return () => window.clearTimeout(id);
    }
    return undefined;
  }, [open]);

  // Focus returns to the trigger on close (§1.2).
  useEffect(() => {
    if (!open) {
      triggerRef.current?.focus();
    }
    return undefined;
  }, [open, triggerRef]);

  useEffect(() => {
    if (open && focusItemId) {
      setRevealed(focusItemId);
      setTasksCollapsed(false);
      const t = window.setTimeout(() => {
        focusRowRef.current?.scrollIntoView({ block: "center" });
      }, 200);
      return () => window.clearTimeout(t);
    }
    return undefined;
  }, [open, focusItemId]);

  // Section-order + composer-bottom scroll choreography: newest-last read.
  useEffect(() => {
    if (open && listRef.current !== null && firstOpenRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
      firstOpenRef.current = false;
    }
  }, [comments, open]);

  // Global shortcut ⌘/Ctrl+⇧N toggles (§1.1) while mounted.
  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && (e.key === "n" || e.key === "N")) {
        e.preventDefault();
        if (open) onClose();
        else composerRef.current?.focus();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  // Escape: reason prompt first, then the panel.
  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if (e.key !== "Escape" || !open) return;
      if (reasonPrompt !== null) {
        setReasonPrompt(null);
        return;
      }
      if (getComputedStyle(scrimRef.current ?? document.body).getPropertyValue("m-skip") === "1") return;
      onClose();
    }
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open, reasonPrompt, onClose]);

  const sessionStart = useMemo(() => {
    const segs = session.segments ?? [];
    return segs[0]?.started_at ?? null;
  }, [session]);

  const sessionSpanFor = useCallback(
    (createdAt: string): boolean => {
      return sessionStart !== null && createdAt >= sessionStart && createdAt <= (session.stopped_at ?? new Date().toISOString());
    },
    [sessionStart, session.stopped_at],
  );

  /** Submit: the ledger line — internal note with the formatted body and
   * the autosave claim key so the committed comment clears the draft. */
  async function submitNote(body: string): Promise<void> {
    if (dirty || body !== "") {
      setStatus("Saving…");
    }
    try {
      const { data, error } = await api.api.POST("/api/v1/tickets/{id}/comments", {
        params: { path: { id: ticketId } },
        body: {
          body,
          kind: "internal" as const,
          draft_key: composerRef.current?.draftKey() ?? null,
          attachment_ids: await uploadStaged(),
          instant: { idempotency_key: `note:${ticketId}:${Date.now()}:${Math.random().toString(36).slice(2, 8)}` },
        },
      });
      if (error) throw Object.assign(new Error(data === undefined ? "rejected" : "rejected"), { reason: (error as { reason?: string }).reason });
      if (data) {
        setComments((prev) => [...(prev ?? []), data]);
      }
      setStatus("Note saved");
      window.setTimeout(() => setStatus(null), 3000);
    } catch (e) {
      setStatus(rejectionText({ message: e instanceof Error ? e.message : "failed" }, "That note did not save — try again."));
    }
  }

  async function uploadStaged(): Promise<string[]> {
    const ids: string[] = [];
    for (const file of staged) {
      // Upload-first staged flow, exactly the workbench composer pattern
      // (§2.6): upload now, ids ride the comment create on submit.
      ids.push((await uploadAttachment(ticketId, file)).id);
    }
    setStaged([]);
    return ids;
  }

  /** Tasks strip mutations: three actions (§3.2) — state flips ride the
   * checklist toggle endpoint; N-A + reopen REQUIRE a reason (§3.4), which
   * the server composes into the ledger in the SAME tx. */
  async function setItemState(item: ChecklistItem, checked: boolean, na: boolean, reason?: string): Promise<void> {
    setRowError(null);
    try {
      const { data, error } = await api.api.POST("/api/v1/tickets/{id}/checklist/{item_id}", {
        params: { path: { id: ticketId, item_id: item.id } },
        body: {
          checked,
          na,
          note_reason: reason ?? null,
          instant: { idempotency_key: `task:${item.id}:${checked ? "c" : na ? "n" : "o"}:${Date.now()}` },
        },
      });
      if (error) {
        const r =
          (error as { reason?: string }).reason ??
          (error as { error?: { reason?: string } }).error?.reason;
        setRowError(
          rejectionText(
            r === undefined
              ? { message: "Could not update that item" }
              : { reason: r },
            "Could not update that item",
          ),
        );
        return;
      }
      // The toggle response FLATTENS the checklist view (additive
      // envelope): items/progress land top-level next to `mutation`.
      if (data) {
        setChecklist((prev) =>
          prev === null
            ? {
                items: data.items,
                progress: data.progress,
                enforcement_active: data.enforcement_active,
                blocking: data.blocking,
              }
            : {
                ...prev,
                items: data.items,
                progress: data.progress,
                enforcement_active: data.enforcement_active,
                blocking: data.blocking,
              },
        );
      }
    } catch {
      setRowError("Could not update that item — try again.");
    }
  }

  function askReason(item: ChecklistItem, kind: "na" | "reopen"): void {
    setRevealed(null);
    setReasonPrompt({ itemId: item.id, kind, text: "" });
  }

  const items = checklist?.items ?? [];
  const openCount = items.filter((i) => !i.checked && i.na !== true).length;
  const naCount = items.filter((i) => i.na === true).length;
  const doneCount = items.filter((i) => i.checked && i.na !== true).length;

  return (
    <div className="workbar-panel-root fixed inset-x-0 bottom-0 z-30 flex justify-center px-2" hidden={!open} data-testid="workbar-panel-root">
      {open && (
        <div
          ref={scrimRef}
          className="pointer-events-auto fixed inset-0"
          style={{ background: "color-mix(in srgb, var(--m-canvas) 94%, transparent)" }}
          onClick={(e) => {
            if (e.target !== e.currentTarget) return;
            // Dismiss guard (§1.2): unsent text → one-time hint.
            if (unsentRef.current.trim() !== "" && Date.now() >= hintUntilRef.current) {
              hintUntilRef.current = Date.now() + 5000;
              setStatus("⌘↵ to send · click again to dismiss");
              return;
            }
            onClose();
          }}
        />
      )}
      {open && (
        <section
          id="notes-task-panel"
          role="dialog"
          aria-modal="false"
          aria-label="Notes and tasks"
          data-testid="notes-task-panel"
          className="workbar-panel pointer-events-auto relative mb-0 flex w-full max-w-[720px] flex-col rounded-t-[0.75rem] border"
          style={{
            background: "var(--m-surface)",
            borderColor: "var(--m-border-dark)",
            boxShadow: "var(--m-shadow-overlay)",
          }}
        >
          {/* Header: pad margin rule (vertical accent at 60% alpha). */}
          <header className="workbar-panel-header relative flex items-center gap-2 border-b px-3 py-2" style={{ borderColor: "var(--m-border)" }}>
            <span aria-hidden className="workbar-panel-margin" />
            <h2 className="m-0 text-[13px] font-bold tracking-[-0.01em]" style={{ color: "var(--m-text)" }}>
              Notes &amp; tasks
            </h2>
            <span
              className="rounded-md px-1.5 py-0.5 text-[10px]"
              style={{ background: "var(--m-bg-subtle)", color: "var(--m-text-muted)" }}
            >
              {ticketNumber}
            </span>
            <button
              type="button"
              aria-label="Close notes and tasks panel"
              data-testid="workbar-panel-close"
              className="ml-auto flex h-7 w-7 cursor-pointer items-center justify-center rounded-md border-0 bg-transparent text-[14px]"
              style={{ color: "var(--m-text-muted)" }}
              onClick={onClose}
            >
              ×
            </button>
          </header>

          {/* The paper layer carries the tiny rotate; the scroll containers
              stay unrotated (hit-testing rule). */}
          <div className="workbar-paper flex min-h-0 flex-1 flex-col">
          {/* TASKS strip (§3) — work first, ledger second. */}
          <details open={!tasksCollapsed} className="workbar-panel-tasks border-b" style={{ borderColor: "var(--m-border)" }} onToggle={(e) => setTasksCollapsed(!(e.target as HTMLDetailsElement).open)}>
            <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-1.5 text-[11px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Tasks
              <span className="workbar-count-chip" style={{ background: "var(--m-bg-subtle)", color: "var(--m-text-muted)" }} data-testid="workbar-tasks-count">
                {openCount} open · {naCount} N/A · {doneCount} done
              </span>
              <span className="ml-auto" style={{ color: "var(--m-text-muted)" }}>▾</span>
            </summary>
            <ul className="m-0 max-h-[26vh] list-none overflow-y-auto px-3 pb-2 pt-1">
              {(items ?? []).map((item) => (
                <TaskRow
                  key={item.id}
                  item={item}
                  highlighted={focusItemId === item.id}
                  revealed={revealed === item.id}
                  onReveal={() => setRevealed(item.id)}
                  busy={false}
                  onComplete={() => void setItemState(item, true, false)}
                  onNa={() => askReason(item, "na")}
                onReopen={() => askReason(item, "reopen")}
                rowRef={focusRowRef as unknown as React.RefObject<HTMLLIElement | null>}
              />
              ))}
            </ul>
          </details>

          {reasonPrompt !== null && (
            <ReasonPrompt
              reasonPrompt={reasonPrompt}
              onChange={(text) => setReasonPrompt({ ...reasonPrompt, text })}
              onCancel={() => setReasonPrompt(null)}
              onSubmit={(reason) => {
                const item = items.find((i) => i.id === reasonPrompt.itemId);
                if (item === undefined) return;
                const kind = reasonPrompt.kind;
                void setItemState(
                  item,
                  false,
                  kind === "na",
                  reason,
                ).then(() => setReasonPrompt(null));
              }}
            />
          )}

          {/* NOTES ledger (§2.1): divider-separated, ruled backdrop, newest-last. */}
          <div ref={listRef} className="workbar-ledger min-h-0 flex-1 overflow-y-auto" data-testid="workbar-notes-ledger">
            {(comments ?? []).filter((c) => c.kind === "internal" || true).length === 0 && (
              <p className="px-4 py-6 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                No notes yet — type the first one below.
              </p>
            )}
            {(comments ?? []).map((c, i) => (
              <NoteRow
                key={c.id}
                comment={c}
                authorName={users.get(c.author_id ?? "") ?? "?"}
                inSession={sessionSpanFor(c.created_at)}
                staggerIndex={Math.min(i, 12)}
              />
            ))}
          </div>

          {/* Composer pinned at the pad's bottom edge — visually "on the pad". */}
          <div className="border-t px-3 pb-1.5 pt-1" style={{ borderColor: "var(--m-border)" }}>
            <p aria-live="polite" className="m-0 px-1 pb-0.5 text-[10.5px]" style={{ color: "var(--m-text-muted)" }} data-testid="workbar-panel-status">
              {status ?? rowError ?? ""}
            </p>
            <NoteComposer
              ref={composerRef as React.Ref<NoteComposerHandle>}
              value={draft}
              onChange={(v) => { setDraft(v); setRowError(null); }}
              onSubmit={(body) => submitNote(body)}
              onEscape={() => { if (Date.now() >= hintUntilRef.current) setTimeout(() => setStatus(null), 3000); }}
              autosaveTicketId={ticketId}
              ariaLabel="Compose this ticket's note"
              placeholder="Add note — Enter for a new line, ⌘↵ to save"
              testPrefix="workbar"
              showHints
              // Tier-2 live BYOK grammar pass (this lane): the refine
              // proposal rides the real CompletionProvider seam; when
              // the org has no credential the composer renders the
              // typed "unavailable" state instead of hiding the control.
              refineEnabled
              pickerSlot={<WorkbarNotePicker />}
              busy={false}
            />
            <div className="flex items-center gap-3 pb-1 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
              <label className="flex cursor-pointer items-center gap-1" title="Attach files">
                <Icon name="clip" size={11} />
                Attach
                <input
                  type="file"
                  multiple
                  className="hidden"
                  onChange={(e) => {
                    setStaged((prev) => [...prev, ...Array.from(e.target.files ?? [])]);
                    e.target.value = "";
                  }}
                />
              </label>
              {staged.length > 0 && <span data-testid="workbar-staged">{staged.length} file(s) staged</span>}
              <span className="flex-1" />
              <span>{ticketTitle.slice(0, 40)}</span>
            </div>
          </div>
          </div>
        </section>
      )}
    </div>
  );
}

interface TaskRowProps {
  item: ChecklistItem;
  highlighted: boolean;
  revealed: boolean;
  busy: boolean;
  onComplete(): void;
  onNa(): void;
  onReopen(): void;
  onReveal(): void;
  rowRef?: React.RefObject<HTMLLIElement | null>;
}

/** One task row (§3.1/§3.2): tri-state glyph + text + hover-revealed
 * action cluster; passive by design — never pulses, never nags. */
function TaskRow({ item, highlighted, revealed, busy, onComplete, onNa, onReopen, rowRef }: TaskRowProps): ReactNode {
  const isNa = item.na === true;
  const isDone = item.checked && !isNa;
  const isOpen = !isNa && !item.checked;
  const required = item.template_item_id !== undefined && item.template_item_id !== null;
  return (
    <li
      ref={rowRef}
      data-testid={`workbar-task-row-${item.id}`}
      className={`workbar-task-row group flex items-center gap-2 rounded-lg px-2 py-1 text-[12px] ${highlighted ? "workbar-task-target" : ""} ${isNa ? "line-through" : ""}`}
      style={{ color: isNa ? "var(--m-text-muted)" : "var(--m-text)", background: highlighted ? "var(--m-accent-muted)" : undefined }}
    >
      <span
        aria-hidden
        className="flex h-4 w-4 shrink-0 items-center justify-center rounded-[3px] border text-[9px]"
        style={{ borderColor: isDone ? "var(--m-status-success-fg)" : isNa ? "var(--m-warning)" : "var(--m-border)", color: isDone ? "var(--m-status-success-fg)" : isNa ? "var(--m-warning)" : "var(--m-text-muted)" }}
      >
        {isDone ? "✓" : isNa ? "∅" : ""}
      </span>
      <span className="min-w-0 flex-1 truncate">{item.text}</span>
      {required && (
        <span
          className="rounded px-1 text-[9px] uppercase"
          style={{ background: "var(--m-bg-subtle)", color: "var(--m-text-muted)" }}
        >
          Required
        </span>
      )}
      <span className={`flex items-center gap-1 ${revealed ? "" : "workbar-task-hover"}`}>
        <button
          type="button"
          aria-label={`Mark ${item.text} completed`}
          data-testid={`workbar-task-complete-${item.id}`}
          disabled={busy}
          className="cursor-pointer rounded-md border border-0 bg-transparent px-1.5 py-0.5 text-[10px]"
          style={{ color: "var(--m-status-success-fg)" }}
          onClick={() => !isDone && onComplete()}
        >
          Mark Completed
        </button>
        <button
          type="button"
          aria-label={`Mark ${item.text} not applicable`}
          data-testid={`workbar-task-na-${item.id}`}
          disabled={busy}
          className="cursor-pointer rounded-md border border-0 bg-transparent px-1.5 py-0.5 text-[10px]"
          style={{ color: "var(--m-warning)" }}
          onClick={() => !isNa && onNa()}
        >
          Mark Not Applicable
        </button>
        <button
          type="button"
          aria-label={`Mark ${item.text} incomplete (re-open)`}
          data-testid={`workbar-task-reopen-${item.id}`}
          disabled={isOpen || busy}
          title={isOpen ? "Already open — nothing to undo" : "Re-open with a mandatory reason"}
          className="cursor-pointer rounded-md border border-0 bg-transparent px-1.5 py-0.5 text-[10px]"
          style={{ color: isOpen ? "var(--m-text-disabled)" : "var(--m-text-muted)", cursor: isOpen ? "default" : "pointer" }}
          onClick={() => !isOpen && onReopen()}
        >
          Mark Incomplete
        </button>
      </span>
    </li>
  );
}

interface ReasonPromptProps {
  reasonPrompt: { itemId: string; kind: "na" | "reopen"; text: string };
  onChange(text: string): void;
  onCancel(): void;
  onSubmit(reason: string): void;
}

/** Inline reason prompt under the task row (§3.4) — NOT a modal; the
 * shared composer carries the entry semantics (⌘↵ submits). */
/** The ticket_note_assist sticky pick rides the shared composer slot. */
function WorkbarNotePicker(): ReactNode {
  const pick = useLazySurfacePick("ticket_note_assist");
  return <ModelPicker pickState={pick} compact />;
}

function ReasonPrompt({ reasonPrompt, onChange, onCancel, onSubmit }: ReasonPromptProps): ReactNode {
  const ok = reasonPrompt.text.trim() !== "";
  return (
    <div className="mx-3 mb-2 rounded-lg border px-2 py-2" style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)" }} data-testid="workbar-reason-prompt">
      <label className="text-[10.5px]" style={{ color: "var(--m-text-muted)" }}>
        Reason (required) — e.g. "Customer already resolved via vendor"
      </label>
      <NoteComposer
        value={reasonPrompt.text}
        onChange={onChange}
        onSubmit={(body) => onSubmit(reasonPrompt.text.trim())}
        onEscape={onCancel}
        compact
        showHints={false}
        placeholder="Type the reason, then ⌘↵ to save"
        ariaLabel="Reason for task state change"
        testPrefix="workbar-reason"
      />
      <div className="flex gap-2 pt-1">
        <button
          type="button"
          data-testid="workbar-reason-save"
          disabled={reasonPrompt.text.trim() === ""}
          className="cursor-pointer rounded-md border px-2 py-1 text-[10px] font-semibold"
          style={{ borderColor: "var(--m-accent)", color: "var(--m-on-accent)", background: reasonPrompt.text.trim() === "" ? "var(--m-accent-muted)" : "var(--m-accent)" }}
          onClick={() => onSubmit(reasonPrompt.text.trim())}
        >
          Save &amp; mark {reasonPrompt.kind === "na" ? "N/A" : "incomplete"}
        </button>
        <button
          type="button"
          className="cursor-pointer border-0 bg-transparent px-2 text-[10px]"
          style={{ color: "var(--m-text-muted)" }}
          onClick={onCancel}
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

/** Renders `**bold**` spans (the standardized workflow leads) without
 * pulling in a markdown engine — the notepad stays plain-text otherwise. */
function renderBoldLeads(body: string): ReactNode {
  const parts = body.split(/(\*\*[^*]+\*\*)/g);
  return parts.map((part, i) =>
    part.startsWith("**") && part.endsWith("**") && part.length > 4 ? (
      <strong key={i}>{part.slice(2, -2)}</strong>
    ) : (
      part
    ),
  );
}

interface NoteRowProps {
  comment: TicketComment;
  authorName: string;
  inSession: boolean;
  staggerIndex: number;
}

/** One ledger row: 24px initials avatar (ramp-picked), session ring when
 * the note falls in the tracked session span, machine ⚙ avatar for
 * workflow notes (workflow_meta at the row level), divider-separated. */
function NoteRow({ comment, authorName, inSession, staggerIndex }: NoteRowProps): ReactNode {
  const meta = (comment.workflow_meta ?? null) as { kind?: string; auto?: boolean } | null;
  const isMachine = meta?.kind !== undefined && meta.auto === true;
  const isClientVisible = comment.kind === "client_visible";
  const ramp = rampFor(comment.author_id ?? comment.id);
  return (
    <article
      className="workbar-note-row flex gap-2 border-b px-3 py-3 first-of-type:border-t-0"
      style={{ borderColor: "color-mix(in srgb, var(--m-border) 100%, transparent)", ["--nc-stagger" as string]: `${Math.min(staggerIndex * 40, 480)}ms` }}
      data-workflow-kind={meta?.kind ?? undefined}
      data-client-visible={isClientVisible || undefined}
      data-testid={`workbar-note-${comment.id}`}
    >
      <span
        aria-hidden
        className="relative flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-[9px] font-semibold"
        style={
          isMachine
            ? { background: "var(--m-muted)", color: "var(--m-text-muted)" }
            : { background: `var(${ramp})`, color: "var(--m-onsurface)" }
        }
        title={isMachine ? "System · workflow note" : `${authorName}${inSession ? " · this session" : ""}`}
        data-testid={isMachine ? "workbar-note-machine-avatar" : "workbar-note-avatar"}
      >
        {isMachine ? "⚙" : initialsOf(authorName)}
        {inSession && !isMachine && <span aria-hidden className="workbar-session-ring" />}
      </span>
      <div className="min-w-0 flex-1">
        <p className="m-0 whitespace-pre-wrap text-[12px]" style={{ color: "var(--m-text)" }}>
          {renderBoldLeads(comment.body)}
        </p>
        <p className="m-0 flex items-center gap-2 pt-0.5 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
          <span>●</span>
          <span className={isMachine ? "workbar-machine-pill" : ""} style={isMachine ? { color: "var(--m-accent)" } : undefined}>
            {isMachine ? "auto · task" : authorName}
          </span>
          <LocalTime
            title={comment.created_at}
            value={comment.created_at}
            kind="time"
          />
          {comment.updated_at != null && <span>edited</span>}
        </p>
      </div>
    </article>
  );
}
