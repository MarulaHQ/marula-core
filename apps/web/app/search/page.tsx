// /search — unified search across tickets, comments, clients/sites/contacts,
// KB articles and documents (2.16 + the 0062 vector leg; Wave 17 lane 6 UX pass).
//
// Purpose: one query box over GET /api/v1/search; the core fuses the FTS,
// trigram and pgvector legs by reciprocal rank fusion and returns one
// blended ranking — this surface renders whatever the core ranks honestly
// (server rank order, grouped by kind, semantic hits indistinguishable from
// lexical ones because they ARE one ranking). Each group shows its count;
// ArrowUp/ArrowDown moves the keyboard cursor and Enter opens the focused
// result; recent queries persist locally and re-run with one click. Kinds
// without a detail page (site, contact, KB article) render as
// informational rows, never links to nowhere.
//
// Non-purpose: no server-side filters/facets, no portal visibility logic;
// local recents are unchanged. Pins are PERSISTENT and server-backed
// (GET/POST/DELETE /api/v1/search/pins): a pin stores the query + the
// snapshot title at pin time and re-runs its query with one click.
"use client";

import { friendlyMutationText } from "../../lib/error-copy";
import { Button, EmptyState, Input, Spinner, Tag } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api, useRequireAuth } from "../../lib/api";

type SearchResult = components["schemas"]["SearchResult"];
type SearchResponse = components["schemas"]["SearchResponse"];
type SearchPinView = components["schemas"]["SearchPinView"];

const GROUP_ORDER = ["ticket", "comment", "client", "document", "kb_article", "site", "contact"] as const;

const GROUP_LABEL: Record<string, string> = {
  ticket: "Tickets",
  comment: "Comments",
  client: "Clients",
  site: "Sites",
  contact: "Contacts",
  kb_article: "KB articles",
  document: "Documents",
};

const RECENT_KEY = "marula.search.recent";

const GROUP_KIND_LABEL: Record<string, string> = {
  ticket: "ticket",
  comment: "comment",
  client: "client",
  document: "doc",
  kb_article: "kb",
  site: "site",
  contact: "contact",
};

function groupOf(kind: string): (typeof GROUP_ORDER)[number] {
  if (kind === "comment") return "comment";
  if (kind === "ticket") return "ticket";
  if (kind === "document") return "document";
  if (kind === "kb_article") return "kb_article";
  if (kind === "site") return "site";
  if (kind === "contact") return "contact";
  return "client";
}

/** Debounces the raw input so typing stays snappy against the API. */
function useDebounced(value: string, ms: number): string {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return debounced;
}

function resultHref(hit: SearchResult): string | null {
  if (hit.entity_kind === "ticket") return `/tickets/${hit.entity_id}`;
  if (hit.entity_kind === "comment" && hit.parent_id) {
    return `/tickets/${hit.parent_id}`;
  }
  if (hit.entity_kind === "client") return `/clients/${hit.entity_id}`;
  if (hit.entity_kind === "document") return `/documents/${hit.entity_id}`;
  return null;
}

/** The flat keyboard-navigable result list, in render order. */
function flatResults(response: SearchResponse | null): Array<{ hit: SearchResult; href: string | null }> {
  const groups = groupMap(response);
  const flat: Array<{ hit: SearchResult; href: string | null }> = [];
  for (const key of GROUP_ORDER) {
    for (const hit of groups.get(key) ?? []) flat.push({ hit, href: resultHref(hit) });
  }
  return flat;
}

function groupMap(response: SearchResponse | null): Map<string, SearchResult[]> {
  const groups = new Map<string, SearchResult[]>();
  for (const hit of response?.results ?? []) {
    const key = groupOf(hit.entity_kind);
    const list = groups.get(key) ?? [];
    list.push(hit);
    groups.set(key, list);
  }
  return groups;
}

function loadRecent(): string[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(RECENT_KEY);
    const parsed: unknown = raw === null ? null : JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string").slice(0, 8) : [];
  } catch {
    return [];
  }
}

export default function SearchPage() {
  const auth = useRequireAuth();
  const [rawQuery, setRawQuery] = useState("");
  const [response, setResponse] = useState<SearchResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [recent, setRecent] = useState<string[]>([]);
  // Server-backed pins: the caller's own, newest-first.
  const [pins, setPins] = useState<SearchPinView[] | null>(null);
  const [pinError, setPinError] = useState<string | null>(null);
  const [rowPinError, setRowPinError] = useState<string | null>(null);
  const [cursorIndex, setCursorIndex] = useState<number | null>(null);
  // Monotonic sequence guard against out-of-order debounced responses.
  const seqRef = useRef(0);
  const cursorRef = useRef<HTMLDivElement>(null);
  const query = useDebounced(rawQuery.trim(), 250);

  const runSearch = useCallback(async (q: string, seq: number) => {
    if (q.length < 2) {
      setResponse(null);
      setLoading(false);
      setError(null);
      return;
    }
    try {
      const { data, error: apiError } = await api.api.GET("/api/v1/search", {
        params: { query: { q, limit: 50 } },
      });
      if (apiError !== undefined) throw new Error("search failed");
      if (seq !== seqRef.current) return;
      setResponse(data);
      setError(null);
    } catch (err) {
      if (seq !== seqRef.current) return;
      setError(friendlyMutationText(err, "Search failed"));
    } finally {
      if (seq === seqRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    seqRef.current += 1;
    setCursorIndex(null);
    setLoading(true);
    void runSearch(query, seqRef.current);
  }, [query, runSearch]);

  useEffect(() => {
    setRecent(loadRecent());
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void api.api
      .GET("/api/v1/search/pins")
      .then(({ data }) => {
        if (!cancelled) setPins(data?.pins ?? []);
      })
      .catch(() => {
        if (!cancelled) setPins([]);
      });
    return () => {
      cancelled = true;
    };
  }, [auth.status]);

  const refreshPins = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/search/pins");
    if (error !== undefined || data === undefined) {
      setPinError("Your pins could not be reloaded — the strip may be stale.");
      return;
    }
    setPins(data.pins);
    setPinError(null);
  }, []);

  const handlePin = useCallback(
    async (hit: SearchResult, querySnapshot: string) => {
      setRowPinError(null);
      const { error, data } = await api.api.POST("/api/v1/search/pins", {
        body: { query: querySnapshot, entity_kind: hit.entity_kind, entity_id: hit.entity_id },
      });
      if (error !== undefined || data === undefined) {
        // 404: the pinned row left the org's index — honest, no fake pin.
        setRowPinError(
          friendlyMutationText(
            error,
            "The result could not be pinned.",
          ),
        );
        return;
      }
      // Idempotent POST answers the stored pin — reflect server state.
      setPins((cur) => {
        const rest = (cur ?? []).filter((p) => p.id !== data.id);
        return [data, ...rest];
      });
    },
    [],
  );

  const handleUnpin = useCallback(
    async (pin: SearchPinView) => {
      const prev = pins;
      setPins((cur) => (cur ?? []).filter((p) => p.id !== pin.id));
      const { error } = await api.api.DELETE("/api/v1/search/pins/{id}", {
        params: { path: { id: pin.id } },
      });
      if (error !== undefined) {
        setPins(prev);
        setPinError(friendlyMutationText(error, "The pin could not be removed. Try again."));
        return;
      }
      setPinError(null);
    },
    [pins],
  );

  const flat = useMemo(() => flatResults(response), [response]);

  // Keyboard navigation over rendered results; Enter opens the link.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (flat.length === 0) return;
      if (e.key === "ArrowDown" || e.key === "j") {
        e.preventDefault();
        setCursorIndex((cur) => (cur === null ? 0 : Math.min(cur + 1, flat.length - 1)));
      } else if (e.key === "ArrowUp" || e.key === "k") {
        e.preventDefault();
        setCursorIndex((cur) => (cur === null ? flat.length - 1 : Math.max(cur - 1, 0)));
      } else if (e.key === "Enter" && cursorIndex !== null) {
        const target = flat[cursorIndex];
        if (target !== undefined && target.href !== null) window.location.assign(target.href);
      } else if (e.key === "Escape") {
        setCursorIndex(null);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [flat, cursorIndex]);

  // Scroll the keyboard cursor into view.
  useEffect(() => {
    if (cursorIndex === null) return;
    const el = cursorRef.current?.querySelector('[data-cursor="true"]');
    if (el !== null && el !== undefined) (el as HTMLElement).scrollIntoView({ block: "nearest" });
  }, [cursorIndex]);

  const rememberRecent = useCallback((q: string) => {
    if (q.length < 2) return;
    const next = [q, ...loadRecent().filter((x) => x !== q)].slice(0, 5);
    setRecent(next);
    try {
      window.localStorage.setItem(RECENT_KEY, JSON.stringify(next));
    } catch {
      // localStorage may be unavailable; recents are best-effort.
    }
  }, []);

  // Record a non-empty settled query as recent.
  useEffect(() => {
    rememberRecent(query);
  }, [query, rememberRecent]);

  if (auth.status !== "authed") {
    return (
      <main className="flex min-h-full items-center justify-center">
        <Spinner />
      </main>
    );
  }

  const groups = groupMap(response);

  return (
    <div className="flex h-full flex-col">
      <main className="flex min-h-0 flex-1 flex-col gap-4 overflow-auto p-4">
        <Input
          autoFocus
          type="search"
          placeholder="Search tickets, clients, documents, KB…   (↑↓ move · Enter open)"
          value={rawQuery}
          onChange={(event) => setRawQuery(event.target.value)}
          aria-label="Search"
          aria-describedby="search-kbd-hint"
          style={{ maxWidth: 520 }}
        />
        <p id="search-kbd-hint" className="sr-only">
          Use the up and down arrows to move between results and Enter to open the focused one.
        </p>

        {recent.length > 0 ? (
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-[11px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Recent
            </span>
            {recent.map((q) => (
              <button
                key={q}
                className="rounded-full border px-2 py-0.5 text-[11px] transition-opacity duration-100 hover:opacity-80"
                style={{ borderColor: "var(--m-border)", color: "var(--m-text-secondary)" }}
                onClick={() => setRawQuery(q)}
              >
                {q}
              </button>
            ))}
          </div>
        ) : null}

        {error !== null && (
          <p className="text-sm" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
            {error}
          </p>
        )}
        {loading && query.length >= 2 && <Spinner />}
        {!loading && error === null && query.length < 2 && (
          <>
            <EmptyState message='Type at least two characters. Use "quotes" for exact phrases and -word to exclude.' />
        {pinError !== null ? (
          <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {pinError}
          </p>
        ) : null}
        {/* Persistent pins strip — snapshot titles, click re-runs the query. */}
        {pins !== null ? (
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-[11px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Pinned
            </span>
            {(pins ?? []).length === 0 ? (
              <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                Pin a result to keep it here.
              </span>
            ) : (
              (pins ?? []).map((pin) => (
                <span
                  key={pin.id}
                  className="flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px]"
                  style={{ borderColor: "var(--m-border)", background: "color-mix(in srgb, var(--m-accent) 6%, transparent)" }}
                >
                  <button
                    className="flex items-center gap-1 transition-opacity duration-100 hover:opacity-80"
                    onClick={() => setRawQuery(pin.query)}
                    title={`Runs “${pin.query}”`}
                  >
                    <Tag>{GROUP_KIND_LABEL[pin.entity_kind] ?? pin.entity_kind}</Tag>
                    {pin.title}
                  </button>
                  <button
                    data-testid={`search-unpin-${pin.id}`}
                    aria-label={`Unpin ${pin.title}`}
                    className="underline-offset-2 hover:underline"
                    style={{ color: "var(--m-text-muted)" }}
                    onClick={() => void handleUnpin(pin)}
                  >
                    ×
                  </button>
                </span>
              ))
            )}
          </div>
        ) : null}

        {recent.length > 0 ? (
              <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                Recent searches re-run with one click above.
              </p>
            ) : null}
          </>
        )}
        {!loading && error === null && query.length >= 2 && response !== null && response.total === 0 && (
          <EmptyState message={`No results for “${response.query}”.`} />
        )}
        {!loading && response !== null && response.total > 0 ? (
          <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
            {response.total} match{response.total === 1 ? "" : "es"}
            {" · "}
            {GROUP_ORDER.filter((k) => (groups.get(k)?.length ?? 0) > 0)
              .map((k) => `${GROUP_LABEL[k]} ${groups.get(k)?.length}`)
              .join(" · ")}
          </p>
        ) : null}
        {GROUP_ORDER.map((key) => {
          const hits = groups.get(key);
          if (hits === undefined || hits.length === 0) return null;
          let indexInFlat = -1;
          return (
            <section key={key} className="flex flex-col gap-1">
              <h2
                className="text-xs font-semibold uppercase tracking-wide"
                style={{ color: "var(--m-text-muted)" }}
              >
                {GROUP_LABEL[key]} · {hits.length}
              </h2>
              <ul className="flex flex-col">
                {hits.map((hit) => {
                  const href = resultHref(hit);
                  // The flat cursor index counting only rendered rows so far.
                  indexInFlat += 1;
                  const flatIndex = flatIndexOffset(key, groups) + indexInFlat - 1;
                  const focused = cursorIndex === flatIndex;
                  const body = (
                    <>
                      <span className="text-sm" style={focused ? { fontWeight: 600 } : undefined}>
                        {hit.title}
                      </span>
                      {hit.snippet.length > 0 && (
                        <span
                          className="truncate text-xs"
                          style={{ color: "var(--m-text-secondary)" }}
                        >
                          {hit.snippet}
                        </span>
                      )}
                    </>
                  );
                  return (
                    <li
                      key={`${hit.entity_kind}-${hit.entity_id}`}
                      data-cursor={focused ? "true" : undefined}
                      className="flex items-center gap-3 rounded-md border-b px-1 py-2 transition-colors last:border-b-0"
                      style={{
                        borderColor: "var(--m-border)",
                        background: focused ? "color-mix(in srgb, var(--m-accent) 6%, transparent)" : "transparent",
                      }}
                    >
                      <Tag>{GROUP_KIND_LABEL[hit.entity_kind] ?? hit.entity_kind}</Tag>
                      {href !== null ? (
                        <Link
                          href={href}
                          className="flex min-w-0 flex-1 flex-col hover:underline"
                          onFocus={() => setCursorIndex(flatIndex)}
                        >
                          {body}
                        </Link>
                      ) : (
                        <div className="min-w-0 flex-1">{body}</div>
                      )}
                      <button
                        data-testid={`search-pin-${hit.entity_kind}-${hit.entity_id}`}
                        className="shrink-0 text-[11px] underline-offset-2 hover:underline"
                        style={{ color: "var(--m-text-muted)" }}
                        onClick={() => void handlePin(hit, query)}
                      >
                        Pin
                      </button>
                    </li>
                  );
                })}
              </ul>
            </section>
          );
        })}
        {rowPinError !== null ? (
          <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {rowPinError}
          </p>
        ) : null}
        {response !== null && response.total > (response.results?.length ?? 0) && (
          <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
            Showing {response.results?.length ?? 0} of {response.total} matches.
          </p>
        )}
        <div ref={cursorRef} className="hidden" />
      </main>
    </div>
  );
}

/** Rows rendered before the given group's rows, in GROUP_ORDER. */
function flatIndexOffset(
  key: (typeof GROUP_ORDER)[number],
  groups: Map<string, SearchResult[]>,
): number {
  let offset = 0;
  for (const prior of GROUP_ORDER) {
    if (prior === key) break;
    offset += groups.get(prior)?.length ?? 0;
  }
  return offset;
}
