// Snippet picker — the composer's canned-response popover (Wave 34 lane
// 3, `p34-canned-templates`).
//
// Purpose: one shared picker for every staff composition surface (the
// workbench floating composer and the classic notes composer). A toolbar
// button opens an anchored popover: a search input filtering the org's
// snippet library with the server's search-in-composer predicate, and
// rows that insert the snippet's body RENDERED against this ticket (the
// portal reply renders the result — tokens never leak to clients).
//
// Non-purpose: no library management here (that lives in
// /settings/reply-snippets); no client-side interpolation (render is the
// server's typed contract; a failed render inserts nothing).

"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";

import {
  listReplySnippets,
  renderReplySnippet,
  type ReplySnippet,
} from "../../lib/reply-snippets";

export interface SnippetPickerProps {
  /** The ticket variables resolve from (render-on-insert). */
  ticketId: string;
  /** Receives the rendered body to append at the caret/draft end. */
  onInsert(body: string): void;
  testPrefix?: string;
}

export function SnippetPicker({ ticketId, onInsert, testPrefix = "snippet-picker" }: SnippetPickerProps): ReactNode {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [snippets, setSnippets] = useState<ReplySnippet[] | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    searchRef.current?.focus();
    const list = listReplySnippets({});
    list
      .then((r) => setSnippets(r.snippets))
      .catch(() => {
        setSnippets([]);
        setNote("The snippet library could not be loaded.");
      });
    function onDocClick(e: MouseEvent) {
      if (rootRef.current !== null && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  async function insert(snippet: ReplySnippet): Promise<void> {
    if (busyId !== null) return;
    setBusyId(snippet.id);
    setNote(null);
    try {
      const body = await renderReplySnippet(snippet.id, ticketId);
      onInsert(body);
      setOpen(false);
      setQuery("");
    } catch (e) {
      setNote(e instanceof Error ? e.message : "The snippet could not be inserted.");
    } finally {
      setBusyId(null);
    }
  }

  const q = query.trim().toLowerCase();
  const visible = (snippets ?? []).filter(
    (s) => q === "" || s.name.toLowerCase().includes(q) || s.body.toLowerCase().includes(q),
  );

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        data-testid={`${testPrefix}-trigger`}
        className="m-wb-tool flex cursor-pointer items-center gap-1 border-none bg-transparent text-[10.5px]"
        style={{ color: "var(--m-text-muted)" }}
        title="Insert a canned reply snippet"
        onClick={() => setOpen((v) => !v)}
      >
        ▤ Snippets
      </button>
      {open && (
        <div
          role="dialog"
          aria-label="Reply snippets"
          data-testid={`${testPrefix}-popover`}
          className="absolute bottom-full left-0 z-30 mb-1.5 w-72 overflow-hidden rounded-lg border transition-opacity duration-100 motion-reduce:transition-none"
          style={{
            borderColor: "var(--m-border)",
            background: "var(--m-overlay-surface)",
            boxShadow: "var(--m-shadow-overlay)",
          }}
        >
          <div className="border-b p-1.5" style={{ borderColor: "var(--m-border)" }}>
            <input
              ref={searchRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search snippets…"
              data-testid={`${testPrefix}-search`}
              className="w-full rounded-md border bg-transparent px-2 py-1 text-[11px] outline-none"
              style={{ borderColor: "var(--m-border)", color: "var(--m-text)" }}
            />
          </div>
          <div className="max-h-56 overflow-y-auto py-0.5">
            {snippets === null && (
              <p className="px-3 py-2 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                Loading…
              </p>
            )}
            {snippets !== null && visible.length === 0 && (
              <p className="px-3 py-2 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                No snippets match. Manage the library in Settings → Reply snippets.
              </p>
            )}
            {visible.map((s) => (
              <button
                key={s.id}
                type="button"
                data-testid={`${testPrefix}-row`}
                disabled={busyId !== null}
                className="block w-full cursor-pointer border-0 bg-transparent px-3 py-1.5 text-left transition-opacity duration-100 hover:opacity-80 motion-reduce:transition-none"
                onClick={() => void insert(s)}
              >
                <span className="block text-[11px] font-bold" style={{ color: "var(--m-text)" }}>
                  {s.name}
                </span>
                <span
                  className="mt-0.5 block truncate text-[10px]"
                  style={{ color: "var(--m-text-muted)" }}
                >
                  {s.body.replace(/\s+/g, " ")}
                </span>
              </button>
            ))}
          </div>
          {note !== null && (
            <p className="border-t px-3 py-1.5 text-[10.5px]" role="status" style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}>
              {note}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
