// /search — unified search across tickets, comments, clients/sites/contacts,
// KB articles and documents (2.16 + the 0062 vector leg).
//
// Purpose: one query box over GET /api/v1/search; the core fuses the FTS,
// trigram and pgvector legs by reciprocal rank fusion and returns one
// blended ranking — this surface renders whatever the core ranks honestly
// (server rank order, grouped by kind, semantic hits indistinguishable from
// lexical ones because they ARE one ranking). Kinds without a detail page
// (site, contact, KB article) render as informational rows, never links to
// nowhere. Tokens only — renders correctly in both themes.
// Non-purpose: no filters/facets UI, no portal visibility logic.
"use client";

import { Button, EmptyState, Input, Spinner, StatusTag } from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useCallback, useEffect, useRef, useState } from "react";

import { api, useRequireAuth } from "../../lib/api";

type SearchResult = components["schemas"]["SearchResult"];
type SearchResponse = components["schemas"]["SearchResponse"];

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

export default function SearchPage() {
  const auth = useRequireAuth();
  const [rawQuery, setRawQuery] = useState("");
  const [response, setResponse] = useState<SearchResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Monotonic sequence guard against out-of-order debounced responses.
  const seqRef = useRef(0);
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
      setError(err instanceof Error ? err.message : "Search failed");
    } finally {
      if (seq === seqRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    seqRef.current += 1;
    setLoading(true);
    void runSearch(query, seqRef.current);
  }, [query, runSearch]);

  if (auth.status !== "authed") {
    return (
      <main className="flex min-h-full items-center justify-center">
        <Spinner />
      </main>
    );
  }

  const groups = new Map<string, SearchResult[]>();
  for (const hit of response?.results ?? []) {
    const key = groupOf(hit.entity_kind);
    const list = groups.get(key) ?? [];
    list.push(hit);
    groups.set(key, list);
  }

  return (
    <div className="flex h-full flex-col">
      <main className="flex min-h-0 flex-1 flex-col gap-4 overflow-auto p-4">
        <Input
          autoFocus
          type="search"
          placeholder="Search tickets, clients, documents, KB…"
          value={rawQuery}
          onChange={(event) => setRawQuery(event.target.value)}
          aria-label="Search"
          style={{ maxWidth: 480 }}
        />
        {error !== null && (
          <p className="text-sm" style={{ color: "var(--m-status-danger-fg)" }}>
            {error}
          </p>
        )}
        {loading && query.length >= 2 && <Spinner />}
        {!loading && error === null && query.length < 2 && (
          <EmptyState message='Type at least two characters. Use "quotes" for exact phrases and -word to exclude.' />
        )}
        {!loading && error === null && query.length >= 2 && response !== null && response.total === 0 && (
          <EmptyState message={`No results for “${response.query}”.`} />
        )}
        {GROUP_ORDER.map((key) => {
          const hits = groups.get(key);
          if (hits === undefined || hits.length === 0) return null;
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
                  const body = (
                    <>
                      <span className="text-sm">{hit.title}</span>
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
                      className="flex items-center gap-3 border-b px-1 py-2 last:border-b-0"
                      style={{ borderColor: "var(--m-border)" }}
                    >
                      <StatusTag tone="neutral">{hit.entity_kind}</StatusTag>
                      {href !== null ? (
                        <Link
                          href={href}
                          className="flex min-w-0 flex-col hover:underline"
                        >
                          {body}
                        </Link>
                      ) : (
                        <div className="flex min-w-0 flex-col">{body}</div>
                      )}
                    </li>
                  );
                })}
              </ul>
            </section>
          );
        })}
        {response !== null && response.total > (response.results?.length ?? 0) && (
          <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
            Showing {response.results?.length ?? 0} of {response.total} matches.
          </p>
        )}
      </main>
    </div>
  );
}
