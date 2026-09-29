// Shared note composer — the ONE note-entry pattern (docs/notes-tasks-
// workbar.md rule §0.1): adopted by the workbar notepad, the workbench-v2
// floating composer, and checklist reason inputs. Plain textarea (ADR 0007
// scope boundary — never an editor), with the binding keyboard contract:
// Enter = new line, Shift+Enter = soft break (autosave-flush no-op,
// keying-identical by design), ⌘/Ctrl+Enter = submit, ⌘/Ctrl+Shift+F =
// Tidy. Submit path applies the shared formatNoteBody (§2.4) to the draft
// and posts through the caller's envelope write — never this component.
//
// Autosave rides the IF2 note-draft store (§5.2): 800ms debounce, 3-dot
// pulse while saving, "Draft saved · HH:MM" on settle, restore on mount.
//
// Non-purpose: no WYSIWYG, no note editing, no rich text.

"use client";

import type { components } from "@marula-core/sdk";
import { forwardRef, useEffect, useImperativeHandle, useRef, useState, type ReactNode } from "react";

import { api } from "../../lib/api";
import { formatNoteBody } from "../../lib/note-format";

type NoteDraftResponse = components["schemas"]["NoteDraftResponse"];

const MAX_CHARS = 20_000;
const COUNTER_AT = 18_000;
const AUTOSAVE_MS = 800;

export interface NoteComposerHandle {
  focus(): void;
  /** The autosave claim key the submited comment must echo so the server
   * deletes it in the same tx (delete-on-submit, §5.2). */
  draftKey(): string | null;
}

export interface NoteComposerProps {
  value: string;
  onChange(value: string): void;
  /** ⌘/Ctrl+Enter: receives the FORMATTED body (auto-bullet on submit). */
  onSubmit(body: string): void | Promise<void>;
  /** Called with the formatted body when Tidy (Tier-1) runs in place. */
  onFormatInPlace?(formatted: string): void;
  /** Tier-2 grammar pass — proposal flow, NEVER a direct write. */
  /** Per-surface model picker slot (this lane): the assist entry points
   * render their sticky pick inline; omitted = no picker. */
  pickerSlot?: ReactNode;
  refineEnabled?: boolean;
  busy?: boolean;
  placeholder?: string;
  ariaLabel?: string;
  /** Escape bubbles to the panel (scrim guard semantics live there). */
  onEscape?(): void;
  /** Autosave target ticket; omit to disable the draft store. */
  autosaveTicketId?: string | null;
  /** Compact variant for reason inputs (tasks strip row prompt). */
  compact?: boolean;
  /** Char counter + hint footer visibility. */
  showHints?: boolean;
  /** Submit-button label (default none — callers render their own send). */
  testPrefix?: string;
}

export const NoteComposer = forwardRef<NoteComposerHandle, NoteComposerProps>(
  function NoteComposer(
    {
      value,
      onChange,
      onSubmit,
      onFormatInPlace,
      pickerSlot = null,
      refineEnabled = false,
      busy = false,
      placeholder = "Add note — Enter for a new line, ⌘↵ to save",
      ariaLabel,
      onEscape,
      autosaveTicketId = null,
      compact = false,
      showHints = true,
      testPrefix = "nc",
    },
    ref,
  ): ReactNode {
    const areaRef = useRef<HTMLTextAreaElement | null>(null);
    useImperativeHandle(
      ref,
      () => ({
        focus: () => areaRef.current?.focus(),
        draftKey: () => draftKeyRef.current,
      }),
      [],
    );
    const draftKeyRef = useRef<string | null>(null);
    const [saveState, setSaveState] = useState<"idle" | "saving" | "saved">("idle");
    const [savedAt, setSavedAt] = useState<string | null>(null);
    const [refine, setRefine] = useState<
      { status: string; proposal: string | null; reason?: string | null } | null
    >(null);

    const text = value;
    const dirty = text.trim() !== "";

    // IF2 autosave: debounced 800ms; immediate flush on blur; panel-hide
    // flush rides the blur of unmount path. Keyed entity: server answers
    // with the claim key the submit echo must use (delete-on-submit).
    useEffect(() => {
      if (autosaveTicketId === null) {
        setSaveState("idle");
        return;
      }
      if (!dirty) {
        return;
      }
      setSaveState("saving");
      const timer = window.setTimeout(async () => {
        try {
          const { data } = await api.api.PUT("/api/v1/tickets/{id}/note-draft", {
            params: { path: { id: autosaveTicketId } },
            body: { body: text, instant: { idempotency_key: `draft:${autosaveTicketId}:${Date.now()}` } },
          });
          if (data?.draft) {
            draftKeyRef.current = data.draft.idempotency_key;
            setSavedAt(
              new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }),
            );
          }
          setSaveState("saved");
        } catch {
          setSaveState("idle");
        }
      }, AUTOSAVE_MS);
      return () => window.clearTimeout(timer);
    }, [text, autosaveTicketId, dirty]);

    // Flush immediately when the composer unmounts (panel-hide/blur).
    async function submit(): Promise<void> {
      if (busy || !dirty) return;
      const body = formatNoteBody(text);
      await onSubmit(body);
      onChange("");
      if (autosaveTicketId !== null) {
        // The submit's envelope claims the draft (server deletes it in
        // the same tx when the caller attached draftKey); local echo
        // resets here.
        draftKeyRef.current = null;
        setSavedAt(null);
        setSaveState("idle");
      }
    }

    /** Tier-1 Tidy: deterministic, in-place, offline-safe, 120ms swap. */
    function tidy(): void {
      if (!dirty) return;
      const formatted = formatNoteBody(text);
      onChange(formatted);
      onFormatInPlace?.(formatted);
      // The pre-tidy draft persists in the autosave ledger within
      // retention (§6); nothing else is needed for the trail.
    }

    /** Tier-2 refine: PROPOSAL only. The ledger is untouched here. */
    async function refineWording(): Promise<void> {
      if (autosaveTicketId === null || !dirty) return;
      try {
        const { data } = await api.api.POST("/api/v1/tickets/{id}/notes/refine", {
          params: { path: { id: autosaveTicketId } },
          body: { body: text },
        });
        if (data === undefined) {
          setRefine({ status: "unavailable", proposal: null, reason: "not available yet" });
          return;
        }
        setRefine({
          status: data.status,
          proposal: data.proposal ?? null,
          reason: data.reason ?? undefined,
        });
      } catch {
        setRefine({ status: "unavailable", proposal: null, reason: "not available yet" });
      }
    }

    return (
      <div data-testid={`${testPrefix}-composer`}>
        {refine?.status === "proposal" && refine.proposal !== null && (
          <div
            className="mb-1.5 rounded-lg border p-2"
            style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)" }}
            data-testid={`${testPrefix}-refine-proposal`}
          >
            <div className="mb-1 flex items-center gap-2 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
              <span>Proposed wording — nothing is written until you apply.</span>
              <span
                className="text-[9px] uppercase"
                style={{ color: "var(--m-text-muted)" }}
              >
                {refine.status}
              </span>
            </div>
            <pre
              className="m-0 max-h-24 overflow-auto whitespace-pre-wrap text-[11px]"
              style={{ color: "var(--m-text)" }}
            >
              {refine.proposal}
            </pre>
            <div className="mt-1.5 flex gap-2 text-[10px]">
              <button
                type="button"
                className="cursor-pointer rounded-md border px-2 py-0.5"
                style={{ borderColor: "var(--m-accent)", color: "var(--m-accent)", background: "var(--m-accent-muted)" }}
                data-testid={`${testPrefix}-refine-apply`}
                onClick={() => {
                  onChange(refine.proposal ?? "");
                  setRefine(null);
                  areaRef.current?.focus();
                }}
              >
                Apply
              </button>
              <button
                type="button"
                className="cursor-pointer rounded-md border px-2 py-0.5"
                style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}
                data-testid={`${testPrefix}-refine-discard`}
                onClick={() => setRefine(null)}
              >
                Discard
              </button>
            </div>
          </div>
        )}
        {refine?.status === "unavailable" && (
          <p className="mb-1 text-[10px]" style={{ color: "var(--m-text-muted)" }} data-testid={`${testPrefix}-refine-unavailable`}>
            Refinement is not available yet in this workspace — the plain Tidy still works.
          </p>
        )}
        <textarea
          ref={areaRef}
          value={value}
          data-testid={`${testPrefix}-body`}
          aria-label={ariaLabel}
          placeholder={placeholder}
          disabled={busy}
          maxLength={MAX_CHARS}
          className={`block w-full resize-none rounded-lg border bg-transparent px-2.5 outline-none ${
            compact ? "min-h-[44px] py-1.5 text-[12px]" : "min-h-[72px] py-2 text-[12.5px]"
          }`}
          style={{ color: "var(--m-text)", borderColor: "var(--m-border)" }}
          onChange={(e) => onChange(e.target.value)}
          onPaste={(e) => {
            // Plain-text paste: rich markup is stripped client-side (§2.3).
            const pasted = e.clipboardData.getData("text/plain");
            if (pasted !== "" && e.clipboardData.types.includes("text/html")) {
              e.preventDefault();
              const el = e.currentTarget;
              const start = el.selectionStart ?? value.length;
              const end = el.selectionEnd ?? value.length;
              onChange(value.slice(0, start) + pasted + value.slice(end));
            }
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape" && onEscape) {
              e.stopPropagation();
              onEscape();
              return;
            }
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && !e.shiftKey) {
              e.preventDefault();
              void submit();
              return;
            }
            if (e.key === "Enter" && e.shiftKey) {
              // Soft break: paragraph semantics — same as Enter (new line),
              // but the autosave-debounce flush is intentionally a no-op:
              // the pair exists for muscle-memory parity, never submission.
              return;
            }
            if ((e.metaKey || e.ctrlKey) && e.shiftKey && (e.key === "f" || e.key === "F")) {
              e.preventDefault();
              tidy();
            }
          }}
        />
        <div className="flex items-center gap-3 px-1 pt-1 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
          <button
            type="button"
            className="m-wb-tool cursor-pointer border-none bg-transparent text-[10.5px]"
            onClick={() => tidy()}
            disabled={!dirty}
            title="Tidy note — capitalization, bullets (⌘⇧F)"
            data-testid={`${testPrefix}-tidy`}
          >
            ✦ Tidy
          </button>
          {refineEnabled && (
            <button
              type="button"
              className="m-wb-tool cursor-pointer border-none bg-transparent text-[10.5px]"
              onClick={() => void refineWording()}
              disabled={!dirty}
              title="Refine wording — proposal to apply (Tier-2, BYOK)"
              data-testid={`${testPrefix}-refine`}
            >
              ⌇ Refine wording
            </button>
          )}
          {pickerSlot != null && <span className="inline-flex items-center">{pickerSlot}</span>}
          {showHints && value.length > COUNTER_AT && (
            <span aria-live="polite">
              {value.length}/{MAX_CHARS}
            </span>
          )}
          <span className="flex-1" />
          {showHints && (
            <span aria-live="polite" data-testid={`${testPrefix}-save-state`}>
              {saveState === "saving"
                ? "···"
                : saveState === "saved" && savedAt !== null
                  ? `Draft saved · ${savedAt}`
                  : ""}
            </span>
          )}
        </div>
      </div>
    );
  },
);

/** The aria-live submit-status line above a composer (§2.3). */
export function ComposerStatus({ children }: { children: ReactNode }): ReactNode {
  return (
    <p aria-live="polite" className="m-0 px-1 pb-1 text-[10.5px]" style={{ color: "var(--m-text-muted)" }}>
      {children}
    </p>
  );
}
