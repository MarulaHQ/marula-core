// Workbench v2 — the floating composer (Wave 15 lane 7).
//
// Purpose: the mockup's floating note bar riding the REAL note/reply
// endpoints: an intent selector (Internal note ▾ / Client-visible /
// In-house update) mapping onto the note taxonomy, a textarea, and the
// tool row — Attach (real upload), AI Assist (inserts the agent panel's
// real draft), Template (typed not-available: note templates don't exist
// yet — checklist templates live on boards), Dictate (real Web Speech
// recognition when the browser provides it, typed gap otherwise). Send
// posts through the instant-first envelope with optimistic render.
// Non-purpose: no note editing/deleting (the API creates notes once).

"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";

import { uploadAttachment } from "../../lib/api";
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

export interface WbComposerProps {
  ticketId: string;
  disabled: boolean;
  busy: boolean;
  /** Agent-panel draft insertion target. */
  draftRef: { current: string | null };
  onPost(body: string, kind: NoteKindWire, attachmentIds: string[]): Promise<void>;
}

export function WbComposer({ ticketId, disabled, busy, draftRef, onPost }: WbComposerProps): ReactNode {
  const [draft, setDraft] = useState("");
  const [intent, setIntent] = useState<NoteKindWire>("internal");
  const [intentOpen, setIntentOpen] = useState(false);
  const [staged, setStaged] = useState<File[]>([]);
  const [toolNote, setToolNote] = useState<string | null>(null);
  const [dictating, setDictating] = useState(false);
  const recognitionRef = useRef<SpeechRecognitionLike | null>(null);
  const areaRef = useRef<HTMLTextAreaElement | null>(null);

  // AI Assist: the agent panel drops its draft here through the shared ref.
  useEffect(() => {
    const tick = window.setInterval(() => {
      if (draftRef.current !== null) {
        setDraft((prev) => (prev === "" ? (draftRef.current ?? "") : `${prev}\n\n${draftRef.current ?? ""}`));
        draftRef.current = null;
        areaRef.current?.focus();
      }
    }, 200);
    return () => window.clearInterval(tick);
  }, [draftRef]);

  async function post(): Promise<void> {
    const body = draft.trim();
    if (body === "" || busy) return;
    try {
      const uploaded: string[] = [];
      for (const file of staged) {
        uploaded.push((await uploadAttachment(ticketId, file)).id);
      }
      await onPost(body, intent, uploaded);
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
    <div className="m-wb-composer pointer-events-none relative z-20 px-4 pb-3">
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
          background: "var(--m-surface)",
          borderColor: "var(--m-border-strong)",
          boxShadow: "var(--m-shadow-overlay)",
        }}
        data-testid="notes-composer"
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
          {intent !== "internal" && (
            <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
              {intent === "client_visible" ? "Visible to the client · may email the requester" : "Staff status update, not client-facing"}
            </span>
          )}
        </div>
        <textarea
          ref={areaRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              void post();
            }
          }}
          disabled={disabled}
          placeholder="Add a note, update, or ask a question…"
          aria-label="Compose a ticket note"
          data-testid="notes-composer-body"
          className="block min-h-[58px] w-full resize-none border-0 bg-transparent px-3 py-2.5 text-[12.5px] outline-none"
          style={{ color: "var(--m-text)" }}
        />
        <div className="flex items-center gap-3.5 px-2.5 py-1.5">
          <label className="m-wb-tool flex cursor-pointer items-center gap-1 text-[10.5px]" style={{ color: "var(--m-text-muted)" }} title="Attach files">
            <svg aria-hidden width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48" />
            </svg>
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
          <button
            type="button"
            className="m-wb-tool flex cursor-pointer items-center gap-1 border-none bg-transparent text-[10.5px]"
            style={{ color: "var(--m-text-muted)" }}
            onClick={() => setToolNote("Note templates are not available yet — checklist templates live on the board.")}
          >
            ▤ Template
          </button>
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
            onClick={() => void post()}
            className="flex h-[27px] cursor-pointer items-center gap-1.5 rounded-md border px-3 text-[11px] font-bold"
            style={{
              borderColor: "var(--m-accent)",
              background: busy || disabled || draft.trim() === "" ? "var(--m-accent-muted)" : "var(--m-accent)",
              color: "var(--m-on-accent)",
            }}
          >
            ➤ Send
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
