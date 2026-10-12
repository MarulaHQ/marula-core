// EntityPicker — the cross-entity target picker (Wave 17 lane 2).
//
// Purpose: replaces paste-a-UUID in every shared link panel (asset
// related-items, document related-items, secret-record links). One
// dialog: kind select, debounced search over the real list endpoints
// (lib/entity-links.ts), keyboard-navigable result rows, and a genuine
// empty state — the backend already scopes what each principal may see,
// so empty means invisible-or-nonexistent, never a fabricated row.
// Non-purpose: no link mutation here — the panel POSTs the seam with the
// returned id; the picker only resolves `LinkTarget`s.

"use client";

import {
  Badge,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useEffect, useRef, useState } from "react";
import { LINK_KIND_OPTIONS, searchLinkTargets, type LinkTarget } from "../../lib/entity-links";

export interface EntityPickerProps {
  open: boolean;
  /** Optional preselection (e.g. opened from a client context). */
  defaultKind?: components["schemas"]["LinkKind"];
  /** Narrows client-scoped kinds when the panel knows a client scope. */
  clientId?: string;
  onPick: (target: LinkTarget) => void;
  onClose: () => void;
}

export function EntityPicker({ open, defaultKind, clientId, onPick, onClose }: EntityPickerProps) {
  const [kind, setKind] = useState<components["schemas"]["LinkKind"]>(defaultKind ?? "document");
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<LinkTarget[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [active, setActive] = useState(0);
  const latestRef = useRef("");
  latestRef.current = query;

  useEffect(() => {
    if (!open) return;
    setKind(defaultKind ?? "document");
    setQuery("");
    setResults(null);
    setActive(0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  useEffect(() => {
    if (!open) return;
    setResults(null);
    setActive(0);
    const t = setTimeout(() => {
      void (async () => {
        setBusy(true);
        const found = await searchLinkTargets(kind, query, clientId);
        if (latestRef.current === query) setResults(found);
        setBusy(false);
      })();
    }, 200);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, kind, query]);

  const pick = (index: number) => {
    const target = results?.[index];
    if (target === undefined) return;
    onPick(target);
    onClose();
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent title="Pick a target" description="Search real records — the picker lists only what your role can see.">
        <div className="grid gap-3 p-4">
          <div className="flex gap-2">
            <Select
              value={kind}
              onValueChange={(v) => {
                setResults(null);
                setKind(v as components["schemas"]["LinkKind"]);
              }}
            >
              <SelectTrigger aria-label="Link target kind" className="w-52 whitespace-nowrap">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {LINK_KIND_OPTIONS.map((k) => (
                  <SelectItem key={k.value} value={k.value}>
                    {k.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Input
              aria-label="Search targets"
              placeholder={`Search ${LINK_KIND_OPTIONS.find((k) => k.value === kind)?.label.toLowerCase() ?? "targets"}…`}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="flex-1"
              autoFocus
            />
          </div>
          <div className="max-h-[40vh] min-h-24 overflow-auto rounded-[var(--m-radius-container)] border" style={{ borderColor: "var(--m-border)" }}>
            {results === null ? (
              <div className="p-4 text-center">
                <Spinner />
              </div>
            ) : results.length === 0 ? (
              <p className="p-4 text-center text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Nothing matches. Results are limited to what your role can see.
              </p>
            ) : (
              <ul role="listbox" aria-label="Link targets">
                {results.map((r, i) => (
                  <li key={r.id}>
                    <button
                      type="button"
                      role="option"
                      aria-selected={i === active}
                      className="flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] transition-colors duration-[100ms] hover:bg-[var(--m-canvas)]"
                      style={{ background: i === active ? "var(--m-accent-soft)" : undefined }}
                      onMouseEnter={() => setActive(i)}
                      onClick={() => pick(i)}
                      onKeyDown={(e) => {
                        if (e.key === "ArrowDown") {
                          e.preventDefault();
                          setActive((prev) => Math.min(prev + 1, (results?.length ?? 1) - 1));
                        } else if (e.key === "ArrowUp") {
                          e.preventDefault();
                          setActive((prev) => Math.max(prev - 1, 0));
                        } else if (e.key === "Enter") {
                          e.preventDefault();
                          pick(i);
                        }
                      }}
                    >
                      <Badge tone="neutral">{LINK_KIND_OPTIONS.find((k) => k.value === r.kind)?.label ?? r.kind}</Badge>
                      <span className="min-w-0 flex-1 truncate font-medium">{r.title}</span>
                      <span className="shrink-0 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                        {r.meta}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
          {busy && results !== null ? <span className="sr-only" role="status">Searching…</span> : null}
          <DialogActions>
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button variant="secondary" onClick={() => pick(active)} disabled={results === null || results.length === 0}>
              Choose selected
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}
