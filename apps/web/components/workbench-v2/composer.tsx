// Workbench v2 — the floating composer (Wave 15 lane 7).
//
// Purpose: the mockup's floating note bar riding the REAL note/reply
// endpoints: an intent selector (Internal note ▾ / Client-visible /
// In-house update) mapping onto the note taxonomy, a textarea, and the
// tool row — Attach (real upload), AI Assist (inserts the agent panel's
// real draft), Snippets (the canned-response library, Wave 34 lane 3 —
// inserts the body rendered against this ticket), Dictate (real Web
// Speech recognition when the browser provides it, typed gap otherwise). Send
// posts through the instant-first envelope with optimistic render.
// Non-purpose: no note editing/deleting (the API creates notes once).

"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { Icon } from "@marula-core/ui/kit";

import { uploadAttachment } from "../../lib/api";
import { SnippetPicker } from "../ticket-workspace/snippet-picker";
import { NoteComposer } from "../notes/note-composer";
import { ModelPicker, useLazySurfacePick } from "../ai/model-picker";
import type { NoteKindWire } from "../ticket-workspace/notes-composer";

const INTENTS: { value: NoteKindWire; label: string }[] = [
  { value: "internal", label: "Internal note" },
  { value: "client_visible", label: "Client-visible reply" },
  { value: "in_house_update", label: "In-house update" },
];

interface SpeechRecognitionLike {
  lang: string;
  interimResults: boolean;
  continuous: boolean;
  onresult: ((event: { results: ArrayLike<ArrayLike<{ transcript: string }>> }) => void) | null;
  onerror: (() => void) | null;
  onend: (() => void) | null;
  start(): void;
  stop(): void;
}

/** The ticket_note_assist sticky pick rides the shared composer slot. */
export function WorkbenchNotePicker(): ReactNode {
  const pick = useLazySurfacePick("ticket_note_assist");
  return <ModelPicker pickState={pick} compact />;
}

export interface WbComposerProps {
  ticketId: string;
  disabled: boolean;
  busy: boolean;
  /** Agent-panel draft insertion target. */
  draftRef: { current: string | null };
  onPost(body: string, kind: NoteKindWire, attachmentIds: string[], draftKey?: string | null, setWaitingAfterSend?: boolean): Promise<void>;
}

export function WbComposer({ ticketId, disabled, busy, draftRef, onPost }: WbComposerProps): ReactNode {
  const [draft, setDraft] = useState("");
  const [intent, setIntent] = useState<NoteKindWire>("internal");
  const [intentOpen, setIntentOpen] = useState(false);
  // The spec's "Set to Waiting after send", offered only on a client-visible
  // note. It rides the post, so the caller's onPost needs the intent and the
  // move together — see the prop note on `onPost`.
  const [setWaitingAfterSend, setSetWaitingAfterSend] = useState(false);
  const [staged, setStaged] = useState<File[]>([]);
  const [toolNote, setToolNote] = useState<string | null>(null);
  const [dictating, setDictating] = useState(false);
  const recognitionRef = useRef<SpeechRecognitionLike | null>(null);
  const composerRef = useRef<{ focus(): void; draftKey(): string | null } | null>(null);

  // AI Assist: the agent panel drops its draft here through the shared ref.
  useEffect(() => {
    const tick = window.setInterval(() => {
      if (draftRef.current !== null) {
        setDraft((prev) => (prev === "" ? (draftRef.current ?? "") : `${prev}\n\n${draftRef.current ?? ""}`));
        draftRef.current = null;
        composerRef.current?.focus();
      }
    }, 200);
    return () => window.clearInterval(tick);
  }, [draftRef]);

  async function post(formattedBody: string): Promise<void> {
    const body = formattedBody.trim();
    if (body === "" || busy) return;
    try {
      const uploaded: string[] = [];
      for (const file of staged) {
        uploaded.push((await uploadAttachment(ticketId, file)).id);
      }
      // The autosave claim key rides the envelope so the committed
      // comment clears this surface's draft row (delete-on-submit, §5.2).
      await onPost(
        body,
        intent,
        uploaded,
        composerRef.current?.draftKey() ?? null,
        intent === "internal" ? false : setWaitingAfterSend,
      );
      setDraft("");
      setStaged([]);
      setToolNote(null);
    } catch (e) {
      setToolNote(e instanceof Error ? e.message : "Could not post note");
    }
  }

  function toggleDictate() {
    if (dictating) {
      recognitionRef.current?.stop();
      return;
    }
    const w = window as unknown as {
      SpeechRecognition?: new () => SpeechRecognitionLike;
      webkitSpeechRecognition?: new () => SpeechRecognitionLike;
    };
    const Ctor = w.SpeechRecognition ?? w.webkitSpeechRecognition;
    if (Ctor === undefined) {
      setToolNote("Dictation is not available in this browser yet — Chrome or Safari provide it.");
      return;
    }
    const rec = new Ctor();
    rec.lang = navigator.language || "en-US";
    rec.interimResults = false;
    rec.continuous = true;
    rec.onresult = (event) => {
      let text = "";
      for (let i = 0; i < event.results.length; i++) {
        const alt = event.results[i]?.[0];
        text += alt?.transcript ?? "";
      }
      setDraft(text);
    };
    rec.onerror = () => setDictating(false);
    rec.onend = () => setDictating(false);
    recognitionRef.current = rec;
    setDictating(true);
    setToolNote(null);
    rec.start();
  }

  return (
    <div className="pointer-events-none relative z-20 px-4 pb-3">
      {/* Boundary fade: rows crossing the scroll edge melt into the composer
          band (mockup's floating-composer grammar; transform/opacity-only
          static gradient — no layout animation). */}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-x-0 -top-5 h-5"
        style={{ background: "linear-gradient(to bottom, color-mix(in srgb, var(--m-canvas) 0%, transparent), var(--m-canvas))" }}
      />
      <div
        className="m-wb-composer-inner pointer-events-auto overflow-hidden rounded-[10px] border"
        style={{
          // The composer takes the internal-note tint when it is writing an
          // internal note, so the mode is visible on the whole surface rather
          // than on a label inside it (DESIGN-NOTES §4). The tint is the same
          // one internal timeline entries use, which is the point: a note and
          // the thing being written are unmistakably the same kind of thing.
          background: intent === "internal" ? "var(--m-note)" : "var(--m-surface)",
          borderColor: intent === "internal" ? "transparent" : "var(--m-border-strong)",
          boxShadow: "var(--m-shadow-overlay)",
        }}
        data-testid="notes-composer"
        data-composer-mode={intent === "internal" ? "internal" : "reply"}
      >
        <div className="relative flex items-center gap-2 border-b px-2.5 py-1.5" style={{ borderColor: "var(--m-border)" }}>
          <button
            type="button"
            aria-haspopup="menu"
            aria-expanded={intentOpen}
            data-testid="note-kind-current"
            className="cursor-pointer border-none bg-transparent px-1.5 py-0.5 text-[11px] font-bold"
            style={{ color: "var(--m-text)" }}
            onClick={() => setIntentOpen((v) => !v)}
          >
            {INTENTS.find((i) => i.value === intent)?.label} ▾
          </button>
          {intentOpen && (
            <div
              role="menu"
              aria-label="Composer intent"
              className="absolute left-2 top-full z-30 mt-1 grid overflow-hidden rounded-lg border"
              style={{ borderColor: "var(--m-border)", background: "var(--m-overlay-surface)", boxShadow: "var(--m-shadow-overlay)" }}
            >
              {INTENTS.map((i) => (
                <button
                  key={i.value}
                  type="button"
                  role="menuitem"
                  data-testid={`note-kind-${i.value}`}
                  className="cursor-pointer border-0 bg-transparent px-3 py-1.5 text-left text-[11px]"
                  style={{ color: i.value === intent ? "var(--m-accent)" : "var(--m-text)" }}
                  onClick={() => {
                    setIntent(i.value);
                    setIntentOpen(false);
                  }}
                >
                  {i.label}
                </button>
              ))}
            </div>
          )}
          {intent !== "internal" ? (
            <>
              <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                Visible to the client · may email the requester
              </span>
              {/* Only a client-visible note can move the ticket, and the spec
                  offers the move only where it means something. An internal
                  note changes nothing, so offering the control would imply it
                  might. */}
              <label
                className="ml-auto flex items-center gap-1.5 text-[10px]"
                style={{ color: "var(--m-text-secondary)" }}
              >
                <input
                  type="checkbox"
                  checked={setWaitingAfterSend}
                  onChange={(e) => setSetWaitingAfterSend(e.target.checked)}
                  data-testid="composer-set-waiting"
                />
                <span>Set to Waiting after send</span>
              </label>
            </>
          ) : (
            <span className="text-[10px]" style={{ color: "var(--m-status-neutral-fg)" }}>
              Only your team can see this
            </span>
          )}
        </div>
        {/* The SHARED note composer entry (docs/notes-tasks-workbar.md
            rule §0.1): one keyboard contract (Enter=new line, ⌘↵=submit),
            format-on-submit, IF2 draft autosave, paste-plain, Tidy. */}
        <NoteComposer
          ref={composerRef as React.Ref<{ focus(): void; draftKey(): string | null } | null>}
          value={draft}
          onChange={(v) => setDraft(v)}
          onSubmit={(body) => post(body)}
          autosaveTicketId={disabled ? null : ticketId}
          placeholder="Add a note, update, or ask a question…"
          ariaLabel="Compose a ticket note"
          testPrefix="notes-composer"
          busy={busy}
          showHints={false}
          refineEnabled
          pickerSlot={<WorkbenchNotePicker />}
        />
        <div className="flex items-center gap-3.5 px-2.5 py-1.5">
          <label className="m-wb-tool flex cursor-pointer items-center gap-1 text-[10.5px]" style={{ color: "var(--m-text-muted)" }} title="Attach files">
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
          <button
            type="button"
            className="m-wb-tool flex cursor-pointer items-center gap-1 border-none bg-transparent text-[10.5px]"
            style={{ color: "var(--m-text-muted)" }}
            title="Insert the agent's drafted customer update"
            onClick={() => setToolNote("Open the Agent panel and pick “Draft customer update” — the draft lands here.")}
          >
            ✦ AI Assist
          </button>
          <SnippetPicker
            ticketId={ticketId}
            onInsert={(body) => setDraft((prev) => (prev.trim() === "" ? body : `${prev}\n\n${body}`))}
          />
          <button
            type="button"
            aria-pressed={dictating}
            className="m-wb-tool flex cursor-pointer items-center gap-1 border-none bg-transparent text-[10.5px]"
            style={{ color: dictating ? "var(--m-accent)" : "var(--m-text-muted)" }}
            onClick={toggleDictate}
          >
            ◉ {dictating ? "Listening…" : "Dictate"}
          </button>
          <span className="flex-1" />
          {staged.length > 0 && (
            <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
              {staged.length} file(s) staged
            </span>
          )}
          <button
            type="button"
            data-testid="note-post"
            disabled={busy || disabled || draft.trim() === ""}
            onClick={() => void post(draft)}
            className="flex h-[27px] cursor-pointer items-center gap-1.5 rounded-md border px-3 text-[11px] font-bold"
            style={{
              borderColor: "var(--m-accent)",
              background: busy || disabled || draft.trim() === "" ? "var(--m-accent-muted)" : "var(--m-accent)",
              color: "var(--m-on-accent)",
            }}
          >
            {intent === "internal" ? "➤ Add note" : "➤ Send reply"}
          </button>
        </div>
        {toolNote !== null && (
          <p className="border-t px-3 py-1.5 text-[10.5px]" role="status" style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}>
            {toolNote}
          </p>
        )}
        {staged.length > 0 && (
          <div className="flex flex-wrap gap-1.5 border-t px-3 py-1.5" style={{ borderColor: "var(--m-border)" }}>
            {staged.map((f, i) => (
              <button
                key={`${f.name}-${i}`}
                type="button"
                className="flex cursor-pointer items-center gap-1 rounded-md border px-1.5 py-0.5 text-[10px]"
                style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}
                title="Remove"
                onClick={() => setStaged((prev) => prev.filter((_, idx) => idx !== i))}
              >
                {f.name.length > 24 ? `${f.name.slice(0, 21)}…` : f.name} ✕
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
