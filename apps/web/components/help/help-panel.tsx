// HelpPanel — the route-keyed contextual help affordance (Wave 26 lane 2).
//
// Purpose: a "?" button in the top bar on EVERY staff surface opening a
// right-hand panel that lists the docs-site guides for the CURRENT
// surface (route key → docs pipeline manifest), ranked newest-first,
// audience-filtered (admin-only guides hidden from members), with
// panel-wide search and a per-guide "Was this helpful?" pair riding the
// typed settings seam. Deep links out to the published docs bundle for
// the full page — markdown stays the single source of truth.
//
// Non-purpose: no markdown rendering in-app (the docs site is the page),
// no tour logic here (TourScheduler owns that), no settings editing
// beyond the tours toggle deep-link.

"use client";

import { Button, Tag } from "@meridian/ui";
import { usePathname } from "next/navigation";
import { useEffect, useMemo, useRef, useState } from "react";
import { useAuth } from "../../lib/api";
import {
  DEFAULT_ROUTE_KEY,
  docsForRouteKey,
  docsUrl,
  filterByAudience,
  rankSurfaceFirst,
  routeKeyForPath,
  searchAllDocs,
  type HelpDoc,
} from "../../lib/help/manifest";
import { recordDocFeedback } from "../../lib/help/help-settings";

const AUDIENCE_TONES: Record<string, "neutral" | "warning"> = {
  admin: "warning",
  staff: "neutral",
  client: "neutral",
  both: "neutral",
  all: "neutral",
};

function HelpDocRow({ doc }: { doc: HelpDoc }) {
  const [answered, setAnswered] = useState<"up" | "down" | null>(null);
  const answer = (up: boolean) => {
    setAnswered(up ? "up" : "down");
    void recordDocFeedback(doc.path, up);
  };
  return (
    <div
      data-testid="help-doc-entry"
      className="border-b px-3 py-2.5 last:border-b-0"
      style={{ borderColor: "var(--m-border)" }}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
            {doc.title}
          </p>
          <p className="mt-0.5 flex items-center gap-1.5 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            <Tag tone={AUDIENCE_TONES[doc.audience] ?? "neutral"}>
              {doc.audience}
            </Tag>
            <span>updated {doc.updated}</span>
          </p>
        </div>
        <a
          href={docsUrl(doc)}
          target="_blank"
          rel="noreferrer"
          data-testid="help-doc-open"
          className="shrink-0 text-[12px] underline-offset-2 hover:underline"
          style={{ color: "var(--m-accent)" }}
        >
          Open in docs ↗
        </a>
      </div>
      <div className="mt-1.5 flex items-center gap-2" data-testid={`help-feedback-${doc.path.replaceAll("/", "-")}`}>
        {answered === null ? (
          <>
            <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              Was this helpful?
            </span>
            <button
              type="button"
              data-testid="help-feedback-up"
              aria-label={`Yes, ${doc.title} was helpful`}
              onClick={() => answer(true)}
              className="cursor-pointer rounded px-1.5 py-0.5 text-[12px] transition-opacity hover:opacity-80"
              style={{ border: "1px solid var(--m-border)", color: "var(--m-text-secondary)" }}
            >
              Yes
            </button>
            <button
              type="button"
              data-testid="help-feedback-down"
              aria-label={`No, ${doc.title} was not helpful`}
              onClick={() => answer(false)}
              className="cursor-pointer rounded px-1.5 py-0.5 text-[12px] transition-opacity hover:opacity-80"
              style={{ border: "1px solid var(--m-border)", color: "var(--m-text-secondary)" }}
            >
              No
            </button>
          </>
        ) : (
          <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            {answered === "up" ? "Thanks for the yes." : "Thanks — that helps us fix the guide."}
          </span>
        )}
      </div>
    </div>
  );
}

function QuestionMarkIcon() {
  return (
    <svg aria-hidden="true" width="15" height="15" viewBox="0 0 16 16" fill="none">
      <path
        d="M6 5.6A2.1 2.1 0 0 1 10.1 6c0 1.4-2.1 1.6-2.1 3"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
      <circle cx="8" cy="12" r="0.9" fill="currentColor" />
    </svg>
  );
}

export function HelpPanel() {
  const pathname = usePathname();
  const { me } = useAuth();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const closeRef = useRef<HTMLButtonElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  const routeKey = routeKeyForPath(pathname);

  const surfaceDocs = useMemo(() => {
    const permissions = me?.permissions ?? [];
    const fallback = filterByAudience(
      docsForRouteKey(DEFAULT_ROUTE_KEY),
      permissions,
    );
    if (routeKey === DEFAULT_ROUTE_KEY) return fallback;
    return rankSurfaceFirst(
      filterByAudience(docsForRouteKey(routeKey), permissions),
      fallback,
    );
  }, [routeKey, me?.permissions]);

  const searchResults = useMemo(
    () => searchAllDocs(query, me?.permissions ?? []),
    [query, me?.permissions],
  );

  const rows = query.trim().length >= 2 ? searchResults : surfaceDocs;

  useEffect(() => {
    if (!open) return;
    closeRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);

  // Close on surface change so the panel never shows stale-surface copy.
  useEffect(() => {
    setOpen(false);
    setQuery("");
  }, [pathname]);

  return (
    <div ref={rootRef} className="relative">
      <Button
        variant="ghost"
        size="sm"
        data-testid="help-button"
        aria-label="Help"
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => setOpen((v) => !v)}
        className="px-2"
      >
        <QuestionMarkIcon />
      </Button>
      {open && (
        <div
          data-testid="help-panel"
          role="dialog"
          aria-label="Help"
          className="fixed right-2 top-14 z-40 flex max-h-[calc(100dvh-4rem)] w-[min(380px,calc(100vw-1rem))] flex-col rounded-xl border shadow-lg"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
        >
          <div
            className="flex shrink-0 items-center justify-between gap-2 border-b px-3 py-2"
            style={{ borderColor: "var(--m-border)" }}
          >
            <div className="min-w-0">
              <p className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                Help
              </p>
              <p className="truncate text-[11px]" style={{ color: "var(--m-text-muted)" }} data-testid="help-surface-label">
                guides for {routeKey.replaceAll("-", " ")}
              </p>
            </div>
            <Button
              ref={closeRef}
              variant="ghost"
              size="sm"
              data-testid="help-close"
              aria-label="Close help"
              onClick={() => setOpen(false)}
              className="px-2"
            >
              ✕
            </Button>
          </div>
          <div className="shrink-0 border-b px-3 py-2" style={{ borderColor: "var(--m-border)" }}>
            <input
              type="search"
              data-testid="help-search"
              placeholder="Search all guides…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              aria-label="Search all guides"
              className="h-8 w-full rounded-lg border px-2.5 text-[12.5px] outline-none"
              style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)", color: "var(--m-text)" }}
            />
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto">
            {rows.length === 0 ? (
              <p className="px-3 py-6 text-center text-[12.5px]" style={{ color: "var(--m-text-muted)" }}>
                {query.trim().length >= 2
                  ? "No guide matches that search."
                  : "No guides for this surface yet."}
              </p>
            ) : (
              rows.map((doc) => <HelpDocRow key={`${doc.path}`} doc={doc} />)
            )}
          </div>
          <div
            className="shrink-0 border-t px-3 py-2 text-[11px]"
            style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}
          >
            Guides come straight from the public docs site.
          </div>
        </div>
      )}
    </div>
  );
}
