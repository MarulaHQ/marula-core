// Counts and totals for the workbench tab row.
//
// Purpose: the pack draws `Activity 7 · Details · Time 0:45 · Files 1 ·
// Related 3`, where Time carries a DURATION rather than a count, because
// "0:45" answers a different question from "3 entries" — a technician wants to
// know what the ticket has cost so far, not how many times someone pressed
// start. The live workbench showed six bare words, which is the deviation this
// exists to close.
//
// Cost, stated plainly: Activity and Files are already on the detail read model
// and cost nothing. Time and Related each need one small request, both issued
// once on mount rather than per tab open, so opening every tab in sequence still
// costs two calls and not six. A missing figure renders as nothing rather than a
// zero, because "0" on a tab that has not loaded is a claim, and a false one.
"use client";

import type { components } from "@marula-core/sdk";
import { useEffect, useState } from "react";
import { api } from "../../lib/api";

/**
 * Every tab id is a key, including the two that never carry a figure. Details
 * describes the ticket rather than listing things, and Notes is a composer
 * rather than a collection, so both stay bare in the row. Declaring them keeps
 * the page's `counts[tab.id]` lookup honest instead of forcing a cast.
 */
export interface TabCounts {
  /** Comment count on the Activity tab. */
  activity?: number;
  /** Always absent: Details describes the ticket, it does not list things. */
  details?: undefined;
  /** Linked-ticket count on the Related tab. */
  related?: number;
  /** Total logged time on the Time tab, as `h:mm`. */
  time?: string;
  /** Attachment count on the Files tab. */
  files?: number;
  /** Always absent: Notes is a composer, not a collection. */
  notes?: undefined;
}

/** `0:45`, `1:30`, `12:05`. Zero minutes is `0:00` rather than an empty
 *  string, because a ticket with time logged and no minutes is still a real
 *  answer to "what has this cost". */
export function formatDuration(totalMinutes: number): string {
  const safe = Number.isFinite(totalMinutes) && totalMinutes > 0 ? Math.floor(totalMinutes) : 0;
  const h = Math.floor(safe / 60);
  const m = safe % 60;
  return `${h}:${String(m).padStart(2, "0")}`;
}

export function useTabCounts(
  ticketId: string | null,
  free: { comments: number; attachments: number } | null,
): TabCounts {
  const [fetched, setFetched] = useState<{ time?: string; related?: number }>({});

  useEffect(() => {
    if (ticketId === null) {
      setFetched({});
      return;
    }
    let cancelled = false;
    void (async () => {
      const [time, links] = await Promise.all([
        // A failed read leaves the tab bare rather than showing a wrong total.
        api.api
          .GET("/api/v1/tickets/{id}/time-entries", { params: { path: { id: ticketId } } })
          .then((res) => {
            const entries = res.data as components["schemas"]["TimeEntry"][] | undefined;
            if (entries === undefined) return undefined;
            return formatDuration(entries.reduce((sum, e) => sum + (e.minutes ?? 0), 0));
          })
          .catch(() => undefined),
        api.api
          .GET("/api/v1/tickets/{id}/links", { params: { path: { id: ticketId } } })
          .then((res) => {
            const view = res.data as components["schemas"]["LinksView"] | undefined;
            if (view === undefined) return undefined;
            return (view.from?.length ?? 0) + (view.to?.length ?? 0);
          })
          .catch(() => undefined),
      ]);
      if (cancelled) return;
      const next: { time?: string; related?: number } = {};
      if (time !== undefined) next.time = time;
      if (links !== undefined) next.related = links;
      setFetched(next);
    })();
    return () => {
      cancelled = true;
    };
  }, [ticketId]);

  return {
    activity: free?.comments,
    files: free?.attachments,
    time: fetched.time,
    related: fetched.related,
  };
}
