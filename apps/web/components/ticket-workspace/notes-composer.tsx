// Notes composer — line-by-line note authoring on the 2.8 taxonomy.
//
// Purpose: the tech types; Enter moves to the next line; structure persists
// verbatim into the stored note body (line breaks preserved, rendered with
// the same line structure — no markdown wrangling). A visibility selector
// (internal / client-visible / in-house update) rides the shipped note
// taxonomy; attachments stage before posting per the 2.14 composer flow.
// The page's instant-first mutate handles optimistic + authoritative
// renders; this component owns the draft and the POST contract only.
// Non-purpose: no note editing/deleting (the API creates notes once).

"use client";

import {
  Badge,
  Button,
  Input,
  Label,
  Textarea,
} from "@marula-core/ui";
import { useState, type ReactNode } from "react";

import { PaperclipIcon } from "./icons";
import { SnippetPicker } from "./snippet-picker";

export type NoteKindWire = "internal" | "client_visible" | "in_house_update";

const KINDS: { value: NoteKindWire; label: string; tone: "neutral" | "success" | "warning"; hint: string }[] = [
  { value: "internal", label: "Internal", tone: "neutral", hint: "Only staff can see this note" },
  { value: "client_visible", label: "Client-visible", tone: "success", hint: "Visible to the client; may email the requester" },
  { value: "in_house_update", label: "In-house update", tone: "warning", hint: "Staff status update, not client-facing" },
];

export interface NotesComposerProps {
  busy: boolean;
  /** The ticket snippets render against (enables the snippet picker). */
  ticketId?: string;
  onPost(body: string, kind: NoteKindWire, attachmentIds: string[]): Promise<void>;
  onUpload(file: File): Promise<string>;
}

export function NotesComposer({ busy, ticketId, onPost, onUpload }: NotesComposerProps): ReactNode {
  const [draft, setDraft] = useState("");
  const [kind, setKind] = useState<NoteKindWire>("internal");
  const [staged, setStaged] = useState<File[]>([]);
  const [error, setError] = useState<string | null>(null);

  /**
   * Line-by-line editing: Enter inserts a newline (default textarea
   * behavior) — the stored body keeps every line break, and the note list
   * renders line structure, so what the tech typed is what everyone sees.
   */
  async function post(): Promise<void> {
    const body = draft.replace(/[ \t]+$/gm, "").trim();
    if (body === "") return;
    try {
      const uploaded: string[] = [];
      for (const file of staged) {
        uploaded.push(await onUpload(file));
      }
      await onPost(body, kind, uploaded);
      setDraft("");
      setStaged([]);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not post note");
    }
  }

  return (
    <div className="grid gap-2" data-testid="notes-composer">
      <div className="grid gap-1">
        <Label htmlFor="notes-composer-body">Add a note</Label>
        <Textarea
          id="notes-composer-body"
          rows={4}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            // Enter = new line (default). Ctrl/Cmd+Enter posts.
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              void post();
            }
          }}
          placeholder="Type line by line — Enter starts the next line, ⌘/Ctrl+Enter posts"
          data-testid="notes-composer-body"
        />
        <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          Line breaks are kept exactly as typed.
        </p>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <div role="radiogroup" aria-label="Note visibility" className="flex gap-1">
          {KINDS.map((k) => (
            <button
              key={k.value}
              type="button"
              role="radio"
              aria-checked={kind === k.value}
              title={k.hint}
              className="rounded-[var(--m-radius-container)] border px-2 py-0.5 text-[11px] transition-opacity duration-100"
              style={{
                borderColor: kind === k.value ? "var(--m-accent)" : "var(--m-border)",
                color: kind === k.value ? "var(--m-accent)" : "var(--m-text-muted)",
              }}
              data-testid={`note-kind-${k.value}`}
              onClick={() => setKind(k.value)}
            >
              {k.label}
            </button>
          ))}
        </div>
        {ticketId !== undefined && (
          <SnippetPicker
            ticketId={ticketId}
            testPrefix="notes-snippet-picker"
            onInsert={(body) => setDraft((prev) => (prev.trim() === "" ? body : `${prev}\n${body}`))}
          />
        )}
        <label
          className="flex cursor-pointer items-center gap-1 rounded-[var(--m-radius-container)] border px-2 py-1 text-xs"
          style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}
          title="Attach files"
        >
          <PaperclipIcon />
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
        {staged.map((f, i) => (
          <button
            key={`${f.name}-${i}`}
            type="button"
            className="flex items-center gap-1 rounded-[var(--m-radius-container)] px-1 text-xs"
            style={{ color: "var(--m-text-muted)" }}
            title="Remove"
            onClick={() => setStaged((prev) => prev.filter((_, idx) => idx !== i))}
          >
            <PaperclipIcon /> {f.name.length > 24 ? `${f.name.slice(0, 21)}…` : f.name} ✕
          </button>
        ))}
        <div className="ml-auto">
          <Button size="sm" variant="primary" disabled={busy || draft.trim() === ""} onClick={() => void post()} data-testid="note-post">
            Post {kind === "client_visible" ? "client-visible" : kind === "in_house_update" ? "in-house" : "internal"} note
          </Button>
        </div>
      </div>
      {error !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">{error}</p>
      )}
      {kind !== "internal" && (
        <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          {KINDS.find((k) => k.value === kind)?.hint}
        </p>
      )}
    </div>
  );
}
