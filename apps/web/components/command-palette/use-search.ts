// Palette search reads.
//
// Purpose: the palette's one search leg — GET /api/v1/search through the typed
// client, debounced 150 ms, minimum two characters, with a monotonic sequence
// guard so a slow answer cannot overwrite a newer one. Mirrors the /search
// page's query shape without touching that page.
//
// Non-purpose: no pins, recents, paging or re-ranking. Those stay on /search.
// In Ask mode the hook is disabled (see `enabled`): the field's text is a
// question, not a query, and the answer nobody reads is a wasted search.

"use client";

import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../../lib/api";

type SearchResponse = components["schemas"]["SearchResponse"];

export type PaletteSearchState = {
  /** The query the server answered, or would answer when it is long enough. */
  query: string;
  response: SearchResponse | null;
  loading: boolean;
  error: string | null;
  retry: () => void;
};

export function useDebounced(value: string, ms: number): string {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(value), ms);
    return () => window.clearTimeout(timer);
  }, [value, ms]);
  return debounced;
}

export function usePaletteSearch(rawQuery: string, enabled = true): PaletteSearchState {
  // Ask mode types a question into the same field; a search for that text
  // would be a server round trip nobody reads, so the hook is switched off.
  const query = useDebounced(enabled ? rawQuery.trim() : "", 150);
  const [response, setResponse] = useState<SearchResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const seqRef = useRef(0);

  const run = useCallback(async (q: string, seq: number) => {
    if (q.length < 2) {
      setResponse(null);
      setLoading(false);
      setError(null);
      return;
    }
    try {
      const { data, error: apiError } = await api.api.GET("/api/v1/search", {
        params: { query: { q, limit: 30 } },
      });
      if (seq !== seqRef.current) return;
      if (apiError !== undefined || data === undefined) throw new Error("search failed");
      setResponse(data);
      setError(null);
    } catch {
      if (seq !== seqRef.current) return;
      setResponse(null);
      setError("The search did not run. Check the connection and try again.");
    } finally {
      if (seq === seqRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    seqRef.current += 1;
    setLoading(query.length >= 2);
    // A new query never keeps the previous answer on screen as if it answered
    // this one; the brief 150 ms loading line takes its place.
    setResponse((current) => (current !== null && current.query !== query ? null : current));
    void run(query, seqRef.current);
  }, [query, run]);

  const retry = useCallback(() => {
    seqRef.current += 1;
    setLoading(query.length >= 2);
    void run(query, seqRef.current);
  }, [query, run]);

  // `retry` re-runs the same query from the error row.
  return { query, response, loading, error, retry };
}
