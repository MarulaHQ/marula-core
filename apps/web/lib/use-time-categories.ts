// Time-category read model for capture surfaces (Wave 39 lane 1).
//
// Purpose: one hook owning the org's category taxonomy + the kept
// work-type mapping, fetched once per session and refetchable — the
// wrap-up sheet, quick-log, and approvals rows all read from it so the
// picker order and billing defaults agree everywhere. Also maps the
// ticket's routing context onto a suggested default slug (board-slug
// match first, travel affinity from the ticket's client/site name).
//
// Non-purpose: no writes — capture surfaces POST work_type /
// unbillable_reason through the envelope like any other field; no admin
// CRUD UI here (the settings surface reads the same endpoints).
"use client";

import { api } from "./api";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useState } from "react";

export type TimeCategory = components["schemas"]["TimeCategory"];
export type CaptureDefaults = components["schemas"]["CaptureDefaultsResponse"];

export interface CategoryPick {
  slug: string;
  label: string;
  description?: string | null;
  defaultBillable: boolean;
  requiresUnbillableReason: boolean;
}

/** The org's non-retired categories in picker order. */
/** The org's non-retired categories in picker order. */
export function useTimeCategories() {
  const [categories, setCategories] = useState<CategoryPick[] | null>(null);
  const [map, setMap] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const { data, error } = await api.api.GET("/api/v1/time-categories/capture-defaults");
      if (error !== undefined || data === undefined) throw new Error("could not load categories");
      setCategories(
        data.categories.map((c) => ({
          slug: c.slug,
          label: c.label,
          description: c.description ?? null,
          defaultBillable: c.default_billable,
          requiresUnbillableReason: c.requires_unbillable_reason,
        })),
      );
      const m: Record<string, string> = {};
      for (const row of data.work_type_map) {
        m[row.legacy_value.toLowerCase()] = row.category_slug;
      }
      setMap(m);
      setError(null);
    } catch {
      setError("couldn't load time categories");
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { categories, map, error, refresh };
}

/**
 * Suggests the default category slug for a ticket from its context:
 * board name match → mapping-table hit → client/site travel hint →
 * undefined (honest "pick one" default; never auto-applied server-side —
 * the field stays user-editable and submits verbatim).
 */
export function suggestCategorySlug(
  ticket: { boardName?: string | null; clientName?: string | null; siteName?: string | null },
  categories: CategoryPick[],
  map: Record<string, string>,
): string | undefined {
  const board = (ticket.boardName ?? "").toLowerCase();
  if (board !== "") {
    const boardHit = categories.find(
      (c) => board.includes(c.slug.replace(/_/g, " ")) || board.includes(c.slug),
    );
    if (boardHit !== undefined) return boardHit.slug;
  }
  const location = `${ticket.siteName ?? ""} ${ticket.clientName ?? ""}`.toLowerCase();
  if (/(on.?site|onlocation|field)/.test(location)) return "on_site";
  if (/remote/.test(board)) return "travel";
  return undefined;
}

/** The one action that clears a verdict, with the value it applies. The
 *  server sends it so the button can never apply a different fix than the one
 *  the verdict meant. */
export interface BillingVerdictFix {
  kind: "use_suggestion" | "trim_minutes" | "ask";
  label: string;
  suggested_description?: string | null;
  trim_minutes?: number | null;
}

export interface BillingVerdict {
  kind: "vague_description" | "overlaps" | "longer_than_usual";
  label: string;
  fix?: BillingVerdictFix | null;
}

export interface BillingSuggestion {
  chip: string;
  label: string;
  billableProbability?: number;
  state?: string | null;
  mode: string;
  recordedAt: string;
  /** The server's mechanical verdicts (Wave 41 lane 8). Empty when clean. */
  verdicts?: BillingVerdict[] | null;
}

/** The audit endpoint takes at most this many ids per request. */
const SUGGESTION_BATCH = 100;

/**
 * The decision_billing_audit shadow answers for one entry, as cohesive
 * chip data. Empty when the audit surface is unconfigured — the honest
 * visibly-absent state (chips never auto-apply anything).
 *
 * Ids go out in batches of {@link SUGGESTION_BATCH} and the answers merge, so a
 * surface that wants a verdict for every row it is showing (the approvals
 * queue's flagged strip) pays one request per hundred rows, not one per row.
 */
export function useBillingSuggestions(entryIds: string[]) {
  const [suggestions, setSuggestions] = useState<Record<string, BillingSuggestion[]>>({});
  const key = [...entryIds].sort().join(",");

  useEffect(() => {
    if (key === "") {
      setSuggestions({});
      return;
    }
    const ids = key.split(",");
    let cancelled = false;
    void (async () => {
      const merged: Record<string, BillingSuggestion[]> = {};
      try {
        for (let i = 0; i < ids.length; i += SUGGESTION_BATCH) {
          const batch = ids.slice(i, i + SUGGESTION_BATCH);
          const { data } = await api.api.GET("/api/v1/time-entries/billing-suggestions", {
            params: { query: { ids: batch.join(",") } },
          });
          if (cancelled) return;
          for (const item of data?.items ?? []) {
            merged[item.entry_id] = (item.suggestions ?? []).map((sg) => ({
              chip: sg.chip,
              label: sg.label,
              billableProbability: sg.billable_probability ?? undefined,
              state: sg.state ?? null,
              mode: sg.mode,
              recordedAt: sg.recorded_at,
              verdicts: (sg.verdicts ?? []).map((v) => ({
                kind: v.kind,
                label: v.label,
                fix:
                  v.fix === undefined || v.fix === null
                    ? null
                    : {
                        kind: v.fix.kind,
                        label: v.fix.label,
                        suggested_description: v.fix.suggested_description ?? null,
                        trim_minutes: v.fix.trim_minutes ?? null,
                      },
              })),
            }));
          }
        }
        setSuggestions(merged);
      } catch {
        if (!cancelled) setSuggestions({});
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [key]);

  return suggestions;
}

/**
 * One-ticket convenience for the quick-log dialog: the strongest audit
 * chip across the ticket's recent entries, as `"billable_false" |
 * "billable_true" | null` (null = no audit answers — render nothing).
 */
export function useBillingAuditForTicket(
  ticketId: string | null | undefined,
): "billable_false" | "billable_true" | null {
  const [entries, setEntries] = useState<string[]>([]);
  const [audit, setAudit] = useState<"billable_false" | "billable_true" | null>(null);

  useEffect(() => {
    if (ticketId == null || ticketId === "") {
      setEntries([]);
      setAudit(null);
      return;
    }
    let cancelled = false;
    void api.api
      .GET("/api/v1/tickets/{id}/time-entries", {
        params: { path: { id: ticketId } },
      })
      .then(({ data }) => {
        if (cancelled) return;
        setEntries((data ?? []).slice(0, 3).map((e) => e.id));
      })
      .catch(() => {
        if (!cancelled) setEntries([]);
      });
    return () => {
      cancelled = true;
    };
  }, [ticketId]);

  const byEntry = useBillingSuggestions(entries);

  useEffect(() => {
    let next: "billable_false" | "billable_true" | null = null;
    for (const id of entries) {
      for (const s of byEntry[id] ?? []) {
        if (s.state === "ignore" && s.chip !== "billable_false") continue;
        const chip = s.chip === "billable_false" ? "billable_false" : "billable_true";
        // A non-billable signal outranks a billable one for surfacing.
        if (chip === "billable_false") next = "billable_false";
        else if (next === null) next = "billable_true";
      }
    }
    setAudit(next);
  }, [entries, byEntry]);

  return audit;
}
