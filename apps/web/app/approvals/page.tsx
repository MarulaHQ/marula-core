// /approvals — the time-entry approval queue (Wave 14 lane 7 close-out, wiring
// the merged lane-1 contract through the typed SDK; Wave 41 lane 8 rebuilt to
// the Marula surface pack's approvals reference).
//
// Purpose: approvers (`special:approve_time`) work the org queue — bulk
// approve/reject with the typed reason vocabulary, per-item bulk outcomes
// (applied / locked / not_in_queue / not_found) flagged on the row so refused
// entries are named exactly why, the append-only decision trail per entry, and
// lock states visible (entries consumed by an invoice are frozen forever).
// Non-approvers see their own submissions; rejected entries reopen back to
// draft when not locked. Read-your-write: applied decisions remove rows
// immediately using the returned entry snapshots.
//
// The pack's frame (DESIGN-NOTES-approvals.md, source/approvals.html): the
// money sub-navigation, the shortcut pills, the queue as a table beside the
// entry in context at 420px, the "N waiting · H:MM in all" bar with the
// Select-the-N-that-look-fine link, the bulk bar that replaces the column
// header in place, J/K/X/A/R, and AI pattern P3 — a mark in the row with a
// one-click fix and a strip above the table. No panel, no inverted surface, no
// sparkle.
//
// Non-purpose: no time-entry editing (ticket timers own creation — which is
// also why the P3 fix is the only action a queue offers: send it back with the
// reason, never edit a tech's entry), no rates/costs (RB2 — the queue carries
// no financial data), no invented person names (the staff directory is
// manager-gated; without it a row says "You" or "Another approver", never a
// guess).
"use client";

import {
  Badge,
  BulkHeaderRow,
  Button,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  ExportControl,
  FieldHint,
  FilterBar,
  Input,
  Kbd,
  Label,
  Spinner,
  SurfaceFooter,
  Table,
  Td,
  Th,
  Tr,
  ViewSwitch,
  applyDensity,
  dayKeyInTimeZone,
  formatInTimeZone,
  shiftDayKey,
  type FilterClause,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";

import { api, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { useDataSurfacePrefs } from "../../lib/data-surface-prefs";
import { useTimeFormat } from "../../lib/use-time-format";
import { downloadRowsCsv } from "../../lib/csv-download";
import { RecordWindow } from "../../components/record-window";
import { friendlyMutationText, resultErrorText } from "../../lib/error-copy";
import {
  hms,
  looksFine,
  notFineReason,
  REJECT_REASONS,
  rejectReasonLabel,
  UNBILLABLE_REASON_LABELS,
  type MoneyPill,
} from "../../lib/money-ui";
import {
  AgeingPill,
  Hms,
  MoneyAvatar,
  MoneyCard,
  MoneyEmpty,
  MoneyHeader,
  MoneyKv,
  MoneyPillTag,
  MoneySaid,
  MoneyStatPill,
  MoneyStrip,
  VerdictMark,
} from "../../components/money";
import { useBillingSuggestions, useTimeCategories } from "../../lib/use-time-categories";

type Entry = components["schemas"]["TimeEntry"];
type ApprovalRecord = components["schemas"]["TimeEntryApprovalRecord"];
type OrgUser = components["schemas"]["OrgUser"];

/** Wire shape of a queue/history projection. NOTE: the OpenAPI pin types
 * `entry` as a bare uuid (core-domain's `schema(value_type)`), but the
 * wire carries the FULL TimeEntry — mirror of
 * `core_domain::entities::TimeEntryQueueItem` (time_tracking.rs). The cast
 * below is the documented reconciliation until the pin is tightened. */
type QueueItem = {
  entry: Entry;
  submitted_by: string | null;
  submitted_at: string | null;
  decided_by: string | null;
  decided_at: string | null;
  locked: boolean;
  /** The ticket named for a human reader; absent for project-scoped work. */
  ticket?: {
    id: string;
    number: string;
    title: string;
    client_name?: string | null;
  } | null;
  /**
   * The member's own zone on the wire (Wave 50 T2). The queue is read in the
   * approver's frame, so this is what the pane draws beside it; absent when the
   * member row is gone.
   */
  user_timezone?: string | null;
};

/** The pack's exact refusal copy (pack §5): named, never a silent guess. */
const OUTCOME_COPY: Readonly<Record<string, string>> = {
  locked: "Already invoiced — locked, nothing to decide",
  not_in_queue: "No longer in the queue — refresh to see its current state",
  not_found: "No longer visible in your scope",
};

const REJECTION_REASON_CODES = new Set(REJECT_REASONS.map((r) => r.code));

/** Category label resolver over the org taxonomy (wave 39). */
function useCategoryLabels(): (slug: string) => string {
  const { categories } = useTimeCategories();
  return useCallback(
    (slug: string) => categories?.find((c) => c.slug === slug)?.label ?? slug,
    [categories],
  );
}

/**
 * "Today" / "Yesterday" / "Thu 18 Oct" — the pack's day label. A queue is read
 * as "today's work", so spelling the date out wastes the column.
 *
 * Today and yesterday are compared as day *keys* in the reader's zone, not as
 * midnight arithmetic on the device's calendar: an entry filed at 01:00 in
 * Tokyo is yesterday's work to a colleague in London, and each of them should
 * see their own answer.
 */
function dayLabel(iso: string | null | undefined, timeZone: string): string {
  if (!iso) return "—";
  const key = dayKeyInTimeZone(iso, timeZone);
  const today = dayKeyInTimeZone(Date.now(), timeZone);
  if (key === today) return "Today";
  if (key === shiftDayKey(today, -1)) return "Yesterday";
  return formatInTimeZone(iso, timeZone, "day");
}

/**
 * Approvals read the member's window beside the approver's; that aside lives
 * in `components/record-window.tsx` so it can be tested on its own.
 */

function initialsOf(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  const first = parts[0]?.[0] ?? "?";
  const last = parts.length > 1 ? (parts[parts.length - 1]?.[0] ?? "") : "";
  return `${first}${last}`.toUpperCase();
}

type Tab = "queue" | "approved" | "rejected" | "decisions";

/** The queue's columns, in the pack's order. Person first because the table is
 *  a list of PEOPLE's time, and the ticket is the evidence for one row of it. */
interface ColumnDef {
  key: string;
  label: string;
  /** A column a user cannot turn off: the row's identity. */
  required?: boolean;
  /** The pack's width for this column (`minmax(230px, 1.25fr)` and friends).
   *  Without them the browser distributes the table by content and the ticket
   *  and date cells wrap to three and five lines — which is what the first
   *  render showed. A data table's proportions are design, not arithmetic. */
  width: string;
}

const COLUMNS: readonly ColumnDef[] = [
  // Percentages, not the pack's px: the pane takes 420px of this canvas, so
  // the pack's px floors exceed what is left and the table ran 1167px inside a
  // 914px card, clipping its own last column. The ratios are the pack's; the
  // units have to survive a narrower card. Requires the fixed layout in
  // `globals.css` (`.m-money-scroll table`), since `fr` is auto-only.
  { key: "person", label: "Person", required: true, width: "15%" },
  { key: "ticket", label: "Ticket and client", width: "23%" },
  // 10%, not 7%: "Yesterday" is the longest day label and 7% clipped it to
  // "Yesterda", 8% still clipped it — measured with a real yesterday-dated
  // entry, not assumed. A day label with no ellipsis reads as a broken cell.
  { key: "when", label: "When", width: "10%" },
  { key: "time", label: "Time", width: "6%" },
  { key: "billable", label: "Billable", width: "9%" },
  { key: "note", label: "What they wrote", width: "33%" },
];

type ColumnKey = string;

/** What a row's mark says, and the one action that clears it. `fix` is the
 *  server's payload verbatim, so the button can only ever apply the remedy the
 *  verdict meant. */
interface RowVerdict {
  label: string;
  fix: {
    kind: "use_suggestion" | "trim_minutes" | "ask";
    label: string;
    suggested_description?: string | null;
    trim_minutes?: number | null;
  } | null;
  chip: string;
}

const FILTER_FIELDS = [
  { key: "person", label: "Person" },
  { key: "ticket", label: "Ticket" },
  { key: "note", label: "What they wrote" },
  { key: "time", label: "Time" },
];

export default function ApprovalsPage() {
  const auth = useRequireAuth();
  const meId = auth.me?.user_id ?? "";
  const isApprover = (auth.me?.permissions ?? []).includes("special:approve_time");
  const prefs = useDataSurfacePrefs();
  const categoryLabel = useCategoryLabels();
  const { timeZone } = useTimeFormat();

  const [tab, setTab] = useState<Tab>("queue");
  const [items, setItems] = useState<QueueItem[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [rowFlags, setRowFlags] = useState<Readonly<Record<string, string>>>({});
  const [rejectOpen, setRejectOpen] = useState(false);
  const [rejectReason, setRejectReason] = useState<string>("");
  const [rejectNote, setRejectNote] = useState<string>("");
  const [rejectError, setRejectError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [history, setHistory] = useState<Readonly<Record<string, ApprovalRecord[]>>>({});
  const [historyOpenId, setHistoryOpenId] = useState<string | null>(null);
  /** The row the keyboard is on. The pane shows it; J and K move it. */
  const [cursorId, setCursorId] = useState<string | null>(null);
  /** Review all: the flagged entries only (pack state 04). */
  const [flaggedOnly, setFlaggedOnly] = useState(false);
  const [clauses, setClauses] = useState<FilterClause[]>([]);
  const [people, setPeople] = useState<Readonly<Record<string, OrgUser>>>({});

  // `prefs` is memoized, so this reads the saved document once per document
  // rather than once per render — otherwise the effect below would rebuild the
  // column state on every paint.
  const savedColumns = useMemo(() => prefs.view("approvals").columns, [prefs]);
  const [columns, setColumns] = useState(() =>
    COLUMNS.map((c) => {
      const stored = savedColumns.find((s) => s.key === c.key);
      return {
        key: c.key as string,
        label: c.label,
        visible: stored?.visible ?? true,
        required: c.required,
        width: c.width,
      };
    }),
  );
  const [order, setOrder] = useState<ColumnKey[]>(() => {
    const storedOrder = savedColumns.map((c) => c.key);
    const known = storedOrder.filter((k): k is ColumnKey => COLUMNS.some((c) => c.key === k));
    return [...known, ...COLUMNS.map((c) => c.key).filter((k) => !known.includes(k))];
  });
  useEffect(() => {
    if (!prefs.ready) return;
    setColumns(
      COLUMNS.map((c) => {
        const stored = savedColumns.find((s) => s.key === c.key);
        return {
          key: c.key as string,
          label: c.label,
          visible: stored?.visible ?? true,
          required: c.required,
          width: c.width,
        };
      }),
    );
    const storedOrder = savedColumns.map((c) => c.key);
    const known = storedOrder.filter((k): k is ColumnKey => COLUMNS.some((c) => c.key === k));
    setOrder([...known, ...COLUMNS.map((c) => c.key).filter((k) => !known.includes(k))]);
  }, [prefs.ready, savedColumns]);

  const persistColumns = useCallback(
    (next: readonly { key: string; visible: boolean; width?: number }[]) => {
      // The chooser knows keys and visibility; the labels and the "required"
      // flag belong to the surface, so they are re-read from the definitions
      // rather than trusted from the popover.
      const merged = COLUMNS.map((def) => {
        const incoming = next.find((c) => c.key === def.key);
        return {
          key: def.key,
          label: def.label,
          visible: incoming?.visible ?? true,
          required: def.required,
          width: def.width,
        };
      }).filter((c) => next.some((n) => n.key === c.key));
      setColumns(merged);
      void prefs.save("approvals", {
        columns: merged.map((c) => ({ key: c.key, visible: c.visible })),
        filters: clauses,
      });
    },
    [prefs, clauses],
  );

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const query: Record<string, string> = {};
      if (tab === "queue") query.status = "submitted";
      else if (tab === "approved") query.status = "approved";
      else if (tab === "rejected") query.status = "rejected";
      else {
        query.decided_by = auth.me?.user_id ?? "";
      }
      const res = await api.api.GET("/api/v1/time-entries/approvals/queue", {
        params: { query },
      });
      const data = res.data;
      if (data === undefined) {
        setError(resultErrorText(res, "Couldn't load the approval queue."));
        setItems([]);
      } else {
        setError(null);
        // Pin wart: `entry` is the full TimeEntry on the wire (see QueueItem).
        setItems(data as unknown as QueueItem[]);
      }
    } catch (e) {
      setError(friendlyMutationText(e, "Couldn't load the approval queue."));
      setItems([]);
    } finally {
      setLoading(false);
    }
  }, [tab, auth.me?.user_id]);

  useEffect(() => {
    // Every row's actionability is a function of WHO is looking at it: your
    // own time renders with its controls disabled (`ownEntry`, below). So the
    // queue waits for the identity read. Loading it first renders a row with
    // `meId` still "", which makes your own entry actionable for as long as
    // `/api/v1/me` is in flight — a window big enough to click through.
    if (auth.me === null) return;
    setSelected(new Set());
    setRowFlags({});
    setRejectOpen(false);
    setRejectError(null);
    setCursorId(null);
    setFlaggedOnly(false);
    void reload();
  }, [auth.me, reload]);

  // The staff directory is manager-gated, and the people in a queue are staff:
  // without it a row reads "You" or "Another approver" rather than a guess.
  useEffect(() => {
    if (!isApprover) return;
    let cancelled = false;
    void (async () => {
      try {
        const { data } = await api.api.GET("/api/v1/org/users", {
          params: { query: { limit: 200 } },
        });
        if (cancelled || data === undefined) return;
        const next: Record<string, OrgUser> = {};
        for (const user of data.users) next[user.id] = user;
        setPeople(next);
      } catch {
        // A directory this approver cannot read leaves the rows nameless, which
        // is a smaller loss than a permission error on every page load.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isApprover]);

  // ---- the audit, batched once for every row on screen ------------------
  const loadedIds = useMemo(() => (items ?? []).map((it) => it.entry.id), [items]);
  const suggestions = useBillingSuggestions(isApprover ? loadedIds : []);

  /**
   * The mark for one row, and the one action that clears it.
   *
   * The server's mechanical verdicts come first — an overlap or a two-word
   * description is arithmetic, and it carries the exact remedy to apply. The
   * model's billable disagreement is the fallback: it has no fix of its own,
   * so it gets the one action that always works on it (ask the person).
   */
  const verdictFor = useCallback(
    (entry: Entry): RowVerdict | null => {
      const chip = suggestions[entry.id]?.[0];
      if (chip === undefined) return null;
      const verdict = chip.verdicts?.find((v) => v.fix !== null && v.fix !== undefined) ??
        chip.verdicts?.[0];
      if (verdict !== undefined) {
        return {
          label: verdict.label,
          fix: verdict.fix ?? null,
          chip: chip.chip,
        };
      }
      const disagrees =
        (chip.chip === "billable_false" && entry.billable) ||
        (chip.chip === "billable_true" && !entry.billable);
      if (!disagrees) return null;
      return { label: chip.label, fix: null, chip: chip.chip };
    },
    [suggestions],
  );

  /** No answers at all IS the unconfigured surface: visibly absent. The
   *  mechanical verdicts count, so an org without the audit surface still
   *  gets the overlap and the description check. */
  const auditActive = Object.keys(suggestions).length > 0;
  const flaggedIds = useMemo(() => {
    const set = new Set<string>();
    if (!auditActive) return set;
    for (const it of items ?? []) {
      if (verdictFor(it.entry) !== null) set.add(it.entry.id);
    }
    return set;
  }, [items, auditActive, verdictFor]);

  const personName = useCallback(
    (entry: Entry): string => {
      if (entry.user_id === meId) return "You";
      const user = people[entry.user_id];
      return user?.display_name ?? "Another approver";
    },
    [people, meId],
  );

  // ---- the view the table is showing ------------------------------------
  const scoped = useMemo(() => {
    const list = items ?? [];
    if (tab === "queue" && flaggedOnly && auditActive) {
      return list.filter((it) => flaggedIds.has(it.entry.id));
    }
    return list;
  }, [items, tab, flaggedOnly, auditActive, flaggedIds]);

  const filtered = useMemo(() => {
    if (clauses.length === 0) return scoped;
    return scoped.filter((it) => {
      const e = it.entry;
      for (const clause of clauses) {
        const raw =
          clause.field === "person"
            ? personName(e)
            : clause.field === "ticket"
              ? e.ticket_id ?? ""
              : clause.field === "note"
                ? e.notes ?? ""
                : hms(e.minutes);
        const value = (clause.value ?? "").toLowerCase();
        const has = raw.toLowerCase().includes(value);
        if (clause.op === "eq" && !has) return false;
        if (clause.op === "is_set" && raw.trim() === "") return false;
        if (clause.op === "is_not_set" && raw.trim() !== "") return false;
      }
      return true;
    });
  }, [scoped, clauses, personName]);

  const totalMinutes = useMemo(
    () => filtered.reduce((acc, it) => acc + it.entry.minutes, 0),
    [filtered],
  );
  const selectedMinutes = useMemo(
    () =>
      filtered
        .filter((it) => selected.has(it.entry.id))
        .reduce((acc, it) => acc + it.entry.minutes, 0),
    [filtered, selected],
  );
  /** The pass: the entries a person can sign off without opening each one. */
  const easyIds = useMemo(
    () =>
      isApprover && tab === "queue"
        ? filtered
            .filter((it) =>
              looksFine({
                own: it.entry.user_id === meId,
                minutes: it.entry.minutes,
                notes: it.entry.notes,
                flagged: flaggedIds.has(it.entry.id),
              }),
            )
            .map((it) => it.entry.id)
        : [],
    [isApprover, tab, filtered, meId, flaggedIds],
  );
  const nonBillableCount = useMemo(
    () => (items ?? []).filter((it) => !it.entry.billable).length,
    [items],
  );

  const cursor = useMemo(
    () => filtered.find((it) => it.entry.id === cursorId) ?? filtered[0] ?? null,
    [filtered, cursorId],
  );
  /** The cursor's own fix, so the pane's button applies the same remedy the
   *  row's button would. */
  const cursorFix = cursor === null ? null : (verdictFor(cursor.entry)?.fix ?? null);

  // ---- decisions --------------------------------------------------------
  function toggleSelect(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  /** Bulk decide. Applied entries leave the list immediately
   * (read-your-write from the returned snapshots); refused entries get a
   * row flag naming exactly why — never a silent guess. */
  async function decide(decision: "approve" | "reject") {
    if (selected.size === 0) return;
    if (decision === "reject" && rejectReason === "") {
      setRejectError("Pick a rejection reason — approvers reject with a typed code, not prose alone.");
      return;
    }
    setBusy(true);
    setRejectError(null);
    try {
      const res = await api.api.POST("/api/v1/time-entries/approvals/decide", {
        body: {
          entry_ids: [...selected],
          decision,
          ...(decision === "reject" && rejectReason ? { reason_code: rejectReason } : {}),
          ...(decision === "reject" && rejectNote.trim() ? { note: rejectNote.trim() } : {}),
        },
      });
      const data = res.data;
      if (data === undefined) {
        setError(resultErrorText(res));
      } else {
        const applied = new Set<string>();
        const flags: Record<string, string> = {};
        for (const item of data.results) {
          if (item.outcome === "applied") applied.add(item.entry_id);
          else flags[item.entry_id] = OUTCOME_COPY[item.outcome] ?? "Refused — refresh and retry";
        }
        setRowFlags((prev) => ({ ...prev, ...flags }));
        setItems((prev) => prev?.filter((it) => !applied.has(it.entry.id)) ?? prev);
        setSelected(new Set());
        setRejectOpen(false);
        setRejectReason("");
        setRejectNote("");
      }
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setBusy(false);
    }
  }

  /** Apply the result of one decision to the list, whatever posted it. */
  function applyOutcome(data: components["schemas"]["BulkDecisionResponse"]) {
    const applied = new Set<string>();
    const flags: Record<string, string> = {};
    for (const item of data.results) {
      if (item.outcome === "applied") applied.add(item.entry_id);
      else flags[item.entry_id] = OUTCOME_COPY[item.outcome] ?? "Refused — refresh and retry";
    }
    setRowFlags((prev) => ({ ...prev, ...flags }));
    setItems((prev) => prev?.filter((it) => !applied.has(it.entry.id)) ?? prev);
  }

  async function decideIds(ids: readonly string[], decision: "approve" | "reject") {
    if (ids.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      const res = await api.api.POST("/api/v1/time-entries/approvals/decide", {
        body: { entry_ids: [...ids], decision },
      });
      const data = res.data;
      if (data === undefined) {
        setError(resultErrorText(res));
      } else {
        applyOutcome(data);
        setSelected(new Set());
      }
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setBusy(false);
    }
  }

  /**
   * Apply the row's fix: adopt the server's factual description, or cut the
   * minutes that another entry already claimed.
   *
   * The entry stays in the queue — this clears the objection, it does not sign
   * the time off, and the human still presses Approve. The write carries the
   * instant-first envelope (an idempotency key per click) and the row updates
   * from the server's own snapshot, so what the reader sees afterwards is what
   * was stored rather than what we hoped for.
   */
  async function applyFix(entry: Entry, fix: NonNullable<RowVerdict["fix"]>) {
    // Narrowed up front so the optimistic write and the request body cannot
    // drift: a trim only ever sends the entry's NEW total, never a delta.
    const nextDescription: string | undefined =
      fix.kind === "use_suggestion" && fix.suggested_description != null
        ? fix.suggested_description
        : undefined;
    const nextMinutes: number | undefined =
      fix.kind === "trim_minutes" && fix.trim_minutes != null
        ? entry.minutes - fix.trim_minutes
        : undefined;
    if (nextDescription === undefined && nextMinutes === undefined) {
      askTech(entry);
      return;
    }
    const payload: components["schemas"]["FixEntryRequest"] = {
      idempotency_key: `fix-${entry.id}-${fix.kind}-${Date.now()}`,
      ...(nextDescription !== undefined ? { description: nextDescription } : {}),
      ...(nextMinutes !== undefined ? { minutes: nextMinutes } : {}),
    };
    setBusy(true);
    const result = await runEnvelopeMutation<
      components["schemas"]["FixEntryRequest"],
      components["schemas"]["FixEntryResponse"]
    >({
      describe: "Apply the fix",
      entityId: entry.id,
      optimistic: () =>
        setItems((prev) =>
          prev?.map((it) =>
            it.entry.id === entry.id
              ? {
                  ...it,
                  entry: {
                    ...it.entry,
                    ...(nextDescription !== undefined ? { notes: nextDescription } : {}),
                    ...(nextMinutes !== undefined ? { minutes: nextMinutes } : {}),
                  },
                }
              : it,
          ) ?? prev,
        ),
      revert: () => void reload(),
      buildBody: () => payload,
      request: (client, body) =>
        client.POST("/api/v1/time-entries/{id}/fix", {
          params: { path: { id: entry.id } },
          body,
        }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setError(
        result.reason === "time_entry_locked"
          ? "That entry is already on an invoice — nothing left to fix."
          : friendlyMutationText(result, "The fix was not applied."),
      );
      return;
    }
    await reload();
  }

  /** Send one entry back with the reason the audit implies. The one-click fix
   *  for what has no mechanical remedy — and a real decision, not an edit: a
   *  person still makes every change, and the tech is told what to fix. */
  function askTech(entry: Entry) {
    const verdict = verdictFor(entry);
    const reason = verdict?.chip === "billable_true" ? "non_billable_work" : "missing_notes";
    setSelected(new Set([entry.id]));
    setRejectReason(reason);
    setRejectNote("");
    setRejectError(null);
    setRejectOpen(true);
  }

  /** Rejected → draft so it can be corrected and resubmitted. */
  async function reopen(id: string) {
    setBusy(true);
    try {
      const res = await api.api.POST("/api/v1/time-entries/{id}/reopen", {
        params: { path: { id } },
      });
      if (res.data === undefined) {
        setError(resultErrorText(res));
      } else {
        setItems((prev) => prev?.filter((it) => it.entry.id !== id) ?? prev);
      }
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setBusy(false);
    }
  }

  async function loadHistory(id: string) {
    if (historyOpenId === id) {
      setHistoryOpenId(null);
      return;
    }
    setHistoryOpenId(id);
    if (history[id] !== undefined) return;
    try {
      const res = await api.api.GET("/api/v1/time-entries/{id}/approvals", {
        params: { path: { id } },
      });
      const records = res.data;
      if (records !== undefined) {
        setHistory((prev) => ({ ...prev, [id]: records }));
      }
    } catch {
      // History stays collapsed on failure; the row remains usable.
    }
  }

  // ---- keys: J and K move, X selects, A approves, R rejects -------------
  useEffect(() => {
    if (!isApprover || tab !== "queue") return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const target = e.target as HTMLElement | null;
      if (target !== null) {
        const tag = target.tagName;
        if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable) {
          return;
        }
      }
      const key = e.key.toLowerCase();
      if (key !== "j" && key !== "k" && key !== "x" && key !== "a" && key !== "r") return;
      if (selected.size > 1 && (key === "a" || key === "r")) return;
      e.preventDefault();
      const rows = filtered;
      if (rows.length === 0) return;
      const at = rows.findIndex((it) => it.entry.id === cursor?.entry.id);
      if (key === "j" || key === "k") {
        const next = key === "j" ? Math.min(rows.length - 1, at + 1) : Math.max(0, at - 1);
        setCursorId(rows[next]?.entry.id ?? null);
        return;
      }
      if (cursor === null) return;
      if (cursor.entry.user_id === meId) return;
      if (key === "x") {
        toggleSelect(cursor.entry.id);
        return;
      }
      if (key === "a") {
        void decideIds([cursor.entry.id], "approve");
        return;
      }
      setSelected(new Set([cursor.entry.id]));
      setRejectReason("");
      setRejectNote("");
      setRejectError(null);
      setRejectOpen(true);
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
    };
    // The handler reads the rows it is moving through, so it re-binds whenever
    // they change: a stale closure would walk a queue the user has left.
  }, [isApprover, tab, filtered, cursor, selected, meId, busy, verdictFor, items]);

  const tabs: { id: Tab; label: string; approverOnly?: boolean }[] = useMemo(() => {
    const base: { id: Tab; label: string; approverOnly?: boolean }[] = [
      { id: "queue", label: "Queue" },
    ];
    if (isApprover) {
      base.push({ id: "approved", label: "Approved" });
      base.push({ id: "decisions", label: "My decisions" });
    }
    base.push({ id: "rejected", label: "Rejected" });
    return base;
  }, [isApprover]);

  const visibleColumns = useMemo(
    () =>
      order
        .map((key) => columns.find((c) => c.key === key))
        .filter((c): c is (typeof columns)[number] => c !== undefined && c.visible),
    [order, columns],
  );

  const exportRows = useCallback(() => {
    const header = visibleColumns.map((c) => c.label);
    const body = filtered.map((it) => {
      const e = it.entry;
      const cell: Record<string, string> = {
        person: personName(e),
        ticket: e.ticket_id ?? "",
        when: formatInTimeZone(e.started_at, timeZone, "dayTime"),
        time: hms(e.minutes),
        billable: e.billable ? "Billable" : "Non-billable",
        note: e.notes ?? "",
      };
      return visibleColumns.map((c) => cell[c.key] ?? "");
    });
    downloadRowsCsv("approvals.csv", header, body);
  }, [visibleColumns, filtered, personName, timeZone]);

  return (
    <div className="m-money-page mx-auto w-full max-w-[1400px] p-4 md:p-6">
      <MoneyHeader
        title="Approvals"
        stats={
          tab === "queue" ? (
            <>
              <MoneyStatPill
                label="Unapproved"
                count={(items ?? []).length}
                tone="gold"
                testId="approvals-stat-unapproved"
              />
              <MoneyStatPill
                label="Non-billable"
                count={nonBillableCount}
                tone="bark"
                testId="approvals-stat-nonbillable"
              />
              {/* The Flagged pill exists only when the audit has answers. With
                  no grant the queue is complete and nothing suggests
                  otherwise — the pack is explicit that there is no trace. */}
              {auditActive ? (
                <MoneyStatPill
                  label="Flagged"
                  count={flaggedIds.size}
                  tone="blush"
                  pressed={flaggedOnly}
                  onClick={() => setFlaggedOnly((v) => !v)}
                  testId="approvals-stat-flagged"
                />
              ) : null}
            </>
          ) : null
        }
      />

      <DataToolbar
        label="Approval queue controls"
        views={
          <ViewSwitch
            label="Views"
            activeId={tab}
            onChange={(id) => setTab(id as Tab)}
            views={tabs.map((t) => ({
              id: t.id,
              label: t.label,
              hint: tab === t.id && items !== null ? String(items.length) : undefined,
            }))}
          />
        }
        filters={
          <FilterBar
            fields={FILTER_FIELDS}
            clauses={clauses}
            onChange={(next) => {
              setClauses(next);
              void prefs.save("approvals", {
                columns: columns.map((c) => ({ key: c.key, visible: c.visible })),
                filters: next,
              });
            }}
          />
        }
        columns={
          <ColumnChooser
            columns={columns.map((c) => ({
              key: c.key,
              label: c.label,
              visible: c.visible,
              required: c.required,
            }))}
            onChange={(next) => {
              persistColumns(next);
              setOrder(next.map((c) => c.key as ColumnKey));
            }}
          />
        }
        export={
          <ExportControl
            columns={visibleColumns.map((c) => c.label)}
            rowCount={filtered.length}
            filters={clauses}
            onExport={exportRows}
            formats={["csv"]}
          />
        }
      />

      {error !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      ) : null}

      <div className="m-money-grid">
        <MoneyCard testId="approvals-queue">
          {/* The strip above the table (pack state 02/04/06). Three states and
              no fourth: something flagged, nothing flagged, or the filter that
              is currently hiding the rest. */}
          {auditActive && tab === "queue" ? (
            flaggedOnly ? (
              <MoneyStrip
                tone="blush"
                testId="approvals-strip"
                action={
                  <Button variant="secondary" onClick={() => setFlaggedOnly(false)}>
                    Show all {(items ?? []).length}
                  </Button>
                }
              >
                Showing the <b>{filtered.length} flagged</b> entries.
              </MoneyStrip>
            ) : flaggedIds.size > 0 ? (
              <MoneyStrip
                tone="blush"
                testId="approvals-strip"
                action={
                  <Button variant="secondary" onClick={() => setFlaggedOnly(true)}>
                    Review all
                  </Button>
                }
              >
                <b>
                  {flaggedIds.size} of {(items ?? []).length} flagged.
                </b>{" "}
                Each one has a fix in its row.
              </MoneyStrip>
            ) : (
              <MoneyStrip tone="green" testId="approvals-strip">
                <b>Nothing flagged</b> in {(items ?? []).length} entries.
              </MoneyStrip>
            )
          ) : null}

          {/* The bar: how much is waiting, how much time it is, and the one
              link that clears the easy ones. */}
          {isApprover && tab === "queue" && items !== null ? (
            <div className="m-money-bar" data-testid="queue-bar">
              <span>
                <b>{filtered.length}</b> waiting · <b>{hms(totalMinutes)}</b> in all
              </span>
              <span className="m-money-bar-sp" />
              {easyIds.length > 0 && selected.size === 0 ? (
                <button
                  type="button"
                  className="m-money-linkbtn"
                  onClick={() => setSelected(new Set(easyIds))}
                  data-testid="approvals-select-easy"
                >
                  Select the {easyIds.length} that look fine
                </button>
              ) : null}
            </div>
          ) : null}

          {loading ? (
            <div className="flex justify-center p-6">
              <Spinner />
            </div>
          ) : filtered.length === 0 ? (
            /* Two different empties, and they must not read the same: a queue
               with nothing in it is good news with a next step, while a filter
               that hid everything is neither good news nor a result. */
            tab === "queue" && (items ?? []).length === 0 ? (
              <MoneyEmpty
                title="Everything is approved"
                body="The day is ready to invoice. Nothing is waiting on you."
                testId="approvals-empty"
                action={
                  <Link href="/billing">
                    <Button>Go to invoices</Button>
                  </Link>
                }
              />
            ) : tab === "rejected" ? (
              <MoneyEmpty
                title="Nothing rejected"
                body="Rejected entries wait here for the tech to fix, and come back with a reason attached."
                testId="approvals-empty"
              />
            ) : tab === "queue" ? (
              <MoneyEmpty
                title="Nothing matches this view"
                body="The queue is not empty — this filter is. Clear it to see what is waiting."
                testId="approvals-empty"
                action={
                  <>
                    {flaggedOnly ? (
                      <Button variant="secondary" onClick={() => setFlaggedOnly(false)}>
                        Show all {(items ?? []).length}
                      </Button>
                    ) : null}
                    {clauses.length > 0 ? (
                      <Button variant="secondary" onClick={() => setClauses([])}>
                        Clear filters
                      </Button>
                    ) : null}
                  </>
                }
              />
            ) : (
              <EmptyState
                message={
                  tab === "approved"
                    ? "No approved entries yet."
                    : "You haven't decided any entries yet."
                }
              />
            )
          ) : null}

          {filtered.length > 0 ? (
            <div className="m-money-scroll" style={{ overflowX: "auto" }}>
              <Table>
                {/* Column widths on a colgroup: the bulk bar replaces the header
                    row, and auto layout would drop the header's widths with it,
                    collapsing every column to an equal share. */}
                <colgroup>
                  {isApprover && tab === "queue" ? <col style={{ width: 44 }} /> : null}
                  {visibleColumns.map((column) => (
                    <col key={column.key} style={{ width: column.width }} />
                  ))}
                </colgroup>
                <thead>
                  {selected.size > 0 && isApprover && tab === "queue" ? (
                    /* The bar replaces the column header in place (pack state
                       05). It used to sit in a strip above the table with the
                       header still drawn, which pushed every row down at the
                       moment the reader aimed at one. */
                    <BulkHeaderRow
                      columnCount={visibleColumns.length + 1}
                      count={selected.size}
                      noun="entry"
                      onClear={() => setSelected(new Set())}
                      testId="bulk-bar"
                      countTestId="bulk-count"
                      leading={
                        <input
                          type="checkbox"
                          checked
                          onChange={() => setSelected(new Set())}
                          aria-label="Deselect all"
                        />
                      }
                    >
                      <b style={{ fontWeight: 600 }}>
                        {hms(selectedMinutes)} in all
                      </b>
                      <Button
                        onClick={() => void decide("approve")}
                        disabled={busy}
                        data-testid="bulk-approve"
                      >
                        Approve {selected.size}
                      </Button>
                      <Button
                        variant="secondary"
                        onClick={() => {
                          setRejectOpen(true);
                          setRejectError(null);
                        }}
                        disabled={busy}
                        data-testid="bulk-reject"
                      >
                        Reject…
                      </Button>
                    </BulkHeaderRow>
                  ) : (
                    <tr>
                      {isApprover && tab === "queue" ? (
                        <Th style={{ width: "4%" }} sortable={false} resizable={false}>
                          <input
                            type="checkbox"
                            aria-label="Select all"
                            checked={
                              filtered.length > 0 && filtered.every((it) => selected.has(it.entry.id))
                            }
                            onChange={(e) =>
                              setSelected(
                                e.target.checked
                                  ? new Set(
                                      filtered
                                        .filter((it) => it.entry.user_id !== meId)
                                        .map((it) => it.entry.id),
                                    )
                                  : new Set(),
                              )
                            }
                          />
                        </Th>
                      ) : null}
                      {visibleColumns.map((column) => (
                        <Th
                          key={column.key}
                          columnKey={column.key}
                          style={{ width: column.width, whiteSpace: "nowrap" }}
                        >
                          {column.label}
                        </Th>
                      ))}
                    </tr>
                  )}
                </thead>
                <tbody>
                  {filtered.map((it) => (
                    <QueueRow
                      key={it.entry.id}
                      item={it}
                      tab={tab}
                      isApprover={isApprover}
                      busy={busy}
                      meId={meId}
                      personName={personName(it.entry)}
                      categoryLabel={categoryLabel}
                      verdict={verdictFor(it.entry)}
                      flag={rowFlags[it.entry.id]}
                      selected={selected.has(it.entry.id)}
                      current={cursor?.entry.id === it.entry.id}
                      visibleColumns={visibleColumns.map((c) => c.key as ColumnKey)}
                      records={history[it.entry.id]}
                      historyOpen={historyOpenId === it.entry.id}
                      onCursor={() => setCursorId(it.entry.id)}
                      onToggle={() => toggleSelect(it.entry.id)}
                      onApprove={() => void decideIds([it.entry.id], "approve")}
                      onReject={() => {
                        setSelected(new Set([it.entry.id]));
                        setRejectReason("");
                        setRejectNote("");
                        setRejectOpen(true);
                        setRejectError(null);
                      }}
                      onAskTech={() => askTech(it.entry)}
                      onApplyFix={(fix) => void applyFix(it.entry, fix)}
                      onReopen={() => void reopen(it.entry.id)}
                      onHistory={() => void loadHistory(it.entry.id)}
                    />
                  ))}
                </tbody>
              </Table>
            </div>
          ) : null}
        </MoneyCard>

        <ContextPane
          item={cursor}
          meId={meId}
          personName={cursor === null ? "" : personName(cursor.entry)}
          categoryLabel={categoryLabel}
          verdict={cursor === null ? null : verdictFor(cursor.entry)}
          records={cursor === null ? undefined : history[cursor.entry.id]}
          historyOpen={cursor !== null && historyOpenId === cursor.entry.id}
          busy={busy}
          onApprove={
            cursor === null ? undefined : () => void decideIds([cursor.entry.id], "approve")
          }
          onReject={
            cursor === null
              ? undefined
              : () => {
                  setSelected(new Set([cursor.entry.id]));
                  setRejectReason("");
                  setRejectNote("");
                  setRejectOpen(true);
                  setRejectError(null);
                }
          }
          onAskTech={cursor === null ? undefined : () => askTech(cursor.entry)}
          onApplyFix={
            cursor === null || cursorFix === null
              ? undefined
              : (fix) => void applyFix(cursor.entry, fix)
          }
          onHistory={
            cursor === null ? undefined : () => void loadHistory(cursor.entry.id)
          }
        />
      </div>

      {rejectOpen && selected.size > 0 ? (
        <Dialog
          open={rejectOpen}
          onOpenChange={(open) => {
            setRejectOpen(open);
            if (!open) setRejectError(null);
          }}
        >
          <DialogContent
            title={selected.size > 1 ? `Reject ${selected.size} entries` : "Reject this entry"}
            description="Each tech is told what to fix, and the entry comes back to you when they have."
          >
            <div className="grid gap-3 p-4">
              <div>
                <Label htmlFor="reject-reason-label">Reason</Label>
                <select
                  id="reject-reason-label"
                  value={rejectReason}
                  onChange={(e) => setRejectReason(e.target.value)}
                  className="h-[var(--m-density-control)] rounded-[var(--m-radius-control)] border px-2 text-[13px]"
                  style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
                  data-testid="reject-reason"
                >
                  <option value="">Pick a reason…</option>
                  {REJECT_REASONS.map((r) => (
                    <option key={r.code} value={r.code}>
                      {r.label}
                    </option>
                  ))}
                </select>
                <FieldHint>Required, so the tech knows what to fix.</FieldHint>
              </div>
              <div>
                <Label htmlFor="reject-note-label">Note (optional)</Label>
                <Input
                  id="reject-note-label"
                  value={rejectNote}
                  onChange={(e) => setRejectNote(e.target.value)}
                  placeholder="For example, say what was done on site"
                />
              </div>
              {rejectError !== null ? (
                <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                  {rejectError}
                </p>
              ) : null}
              <FieldHint>Nothing is deleted. The entry is kept and can be resubmitted.</FieldHint>
              <div data-testid="reject-panel">
                <DialogActions>
                  <Button variant="ghost" onClick={() => setRejectOpen(false)}>
                    Cancel
                  </Button>
                  <Button
                    disabled={busy}
                    onClick={() => void decide("reject")}
                    data-testid="reject-send"
                  >
                    Reject and tell{" "}
                    {(items ?? [])
                      .filter((it) => selected.has(it.entry.id))
                      .map((it) => personName(it.entry).split(" ")[0])
                      .join(", ")}
                  </Button>
                </DialogActions>
              </div>
            </div>
          </DialogContent>
        </Dialog>
      ) : null}

      <SurfaceFooter
        shown={filtered.length}
        total={(items ?? []).length}
        noun="entry"
        nounPlural="entries"
        density={
          <DensitySwitch
            value={prefs.density}
            onChange={(step) => {
              applyDensity(step);
              void prefs.setDensity(step);
            }}
          />
        }
        hints={[
          { keys: "J K", label: "move" },
          { keys: "X", label: "select" },
          { keys: "A", label: "approve" },
          { keys: "R", label: "reject" },
        ]}
      />
    </div>
  );
}

// ---------------------------------------------------------------- rows ----

function QueueRow({
  item,
  tab,
  isApprover,
  busy,
  meId,
  personName,
  categoryLabel,
  verdict,
  flag,
  selected,
  current,
  visibleColumns,
  records,
  historyOpen,
  onCursor,
  onToggle,
  onApprove,
  onReject,
  onAskTech,
  onApplyFix,
  onReopen,
  onHistory,
}: {
  item: QueueItem;
  tab: Tab;
  isApprover: boolean;
  busy: boolean;
  meId: string;
  personName: string;
  categoryLabel: (slug: string) => string;
  verdict: RowVerdict | null;
  flag?: string;
  selected: boolean;
  current: boolean;
  visibleColumns: readonly ColumnKey[];
  records?: ApprovalRecord[];
  historyOpen: boolean;
  onCursor: () => void;
  onToggle: () => void;
  onApprove: () => void;
  onReject: () => void;
  onAskTech: () => void;
  onApplyFix: (fix: NonNullable<RowVerdict["fix"]>) => void;
  onReopen: () => void;
  onHistory: () => void;
}) {
  const { timeZone } = useTimeFormat();
  const e = item.entry;
  const ownEntry = e.user_id === meId;
  const inQueueTab = tab === "queue" && isApprover;
  const trail = (records ?? []).map(
    (r) =>
      `${ACTION_LABEL[r.action] ?? r.action} · ${formatInTimeZone(r.created_at, timeZone, "dayTime")}${
        r.reason_code ? ` · ${rejectReasonLabel(r.reason_code)}` : ""
      }${r.note ? ` · “${r.note}”` : ""}`,
  );

  const cell = (key: ColumnKey) => {
    switch (key) {
      case "person":
        return (
          <span style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <MoneyAvatar initials={initialsOf(personName)} own={ownEntry} />
            <span
              title={personName}
              style={{
                fontWeight: 500,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {personName}
            </span>
          </span>
        );
      case "ticket":
        return (
          <span style={{ display: "flex", flexDirection: "column", lineHeight: 1.3, minWidth: 0 }}>
            {/* "#18366 Disable leaver accounts" — a ticket number a person can
                read and quote. The uuid prefix is the fallback for an entry
                whose ticket the queue could not name, never the first choice. */}
            <b
              style={{
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
              title={item.ticket === null || item.ticket === undefined ? undefined : item.ticket.title}
            >
              {item.ticket === null || item.ticket === undefined ? (
                <>ticket {e.ticket_id?.slice(0, 8)}…</>
              ) : (
                <>
                  <Link
                    href={`/tickets/${item.ticket.id}`}
                    style={{ color: "var(--m-text)", textDecoration: "none" }}
                  >
                    #{item.ticket.number}
                  </Link>{" "}
                  {item.ticket.title}
                </>
              )}
            </b>
            <span
              style={{
                color: "var(--m-text-secondary)",
                fontSize: 12,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {item.ticket?.client_name ?? "—"}
              {e.work_type !== undefined && e.work_type !== null
                ? ` · ${categoryLabel(e.work_type)}`
                : ""}
              {ownEntry ? (
                <Link
                  href={`/dispatch/travel?ticket_id=${e.ticket_id}`}
                  style={{ color: "var(--m-accent-text)", marginLeft: 8 }}
                  data-testid={`approval-log-travel-${e.id}`}
                >
                  log travel
                </Link>
              ) : null}
            </span>
          </span>
        );
      case "when":
        return (
          // The pack's two lines: the day, then when it started. One wrapped
          // "Sep 30, 08:15 AM" is five lines of nothing in a 72px column.
          <span
            style={{
              display: "flex",
              flexDirection: "column",
              color: "var(--m-text-secondary)",
              whiteSpace: "nowrap",
            }}
          >
            <span>{dayLabel(e.started_at, timeZone)}</span>
            <span style={{ fontSize: 12 }}>{formatInTimeZone(e.started_at, timeZone, "time")}</span>
          </span>
        );
      case "time":
        return (
          <b style={{ fontVariantNumeric: "tabular-nums" }}>
            <Hms minutes={e.minutes} />
          </b>
        );
      case "billable":
        return e.billable ? (
          <MoneyPillTag pill={{ label: "Billable", family: "neutral" }} />
        ) : (
          <span style={{ display: "flex", flexDirection: "column", gap: 2, alignItems: "flex-start" }}>
            <MoneyPillTag pill={{ label: "Non-billable", family: "neutral" }} />
            <span
              // The reason ellipsises in a 9% column, so the full words have to
              // be reachable on hover rather than only in the pane.
              title={UNBILLABLE_REASON_LABELS[e.unbillable_reason ?? ""] ?? e.unbillable_reason ?? undefined}
              style={{
                fontSize: 11,
                color: "var(--m-text-muted)",
                maxWidth: "100%",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
              data-testid={`approval-category-${e.id}`}
            >
              {UNBILLABLE_REASON_LABELS[e.unbillable_reason ?? ""] ?? e.unbillable_reason ?? "reason pending"}
            </span>
          </span>
        );
      case "note":
        return (
          <span
            style={{
              display: "flex",
              flexDirection: "column",
              justifyContent: "center",
              gap: 2,
              minWidth: 0,
              width: "100%",
            }}
          >
            <span
              title={e.notes ?? undefined}
              style={{
                display: "block",
                width: "100%",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {e.notes ?? <span style={{ color: "var(--m-text-muted)" }}>Nothing written</span>}
            </span>
            {verdict !== null ? (
              <VerdictMark
                label={verdict.label}
                fix={verdict.fix === null ? "Ask the tech" : verdict.fix.label}
                onFix={
                  verdict.fix === null || verdict.fix.kind === "ask"
                    ? onAskTech
                    : () => void onApplyFix(verdict.fix as NonNullable<RowVerdict["fix"]>)
                }
                testId={`approval-audit-${e.id}`}
              />
            ) : null}
          </span>
        );
    }
  };

  return (
    <Tr
      className={`row-hover${selected ? " m-money-row-selected" : ""}${
        current ? " m-money-row-current" : ""
      }`}
      onClick={onCursor}
      data-testid={`approval-row-${e.id}`}
      aria-current={current ? "true" : undefined}
    >
      {inQueueTab ? (
        <Td style={{ width: "4%" }}>
          <input
            type="checkbox"
            checked={selected}
            onChange={onToggle}
            onClick={(ev) => ev.stopPropagation()}
            aria-label={`Select entry on ticket ${e.ticket_id}`}
            disabled={ownEntry}
            title={ownEntry ? "Another approver signs off your time" : undefined}
          />
        </Td>
      ) : null}
      {visibleColumns.map((key) => (
        <Td key={key} data-column={key} style={{ minWidth: 0 }}>
          {cell(key)}
          {key === "note" ? (
            /* The row's own decisions, its refusal and its trail, riding at
               the end of the last cell beside the fix button the pack already
               puts there. A column of their own pushed the table past its
               container and tripled every row's height. */
            <div
              onClick={(ev) => ev.stopPropagation()}
              style={{
                display: "flex",
                flexWrap: "wrap",
                alignItems: "center",
                gap: "6px 10px",
                marginTop: 4,
              }}
            >
              {inQueueTab ? (
                <>
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={busy || ownEntry}
                    onClick={onApprove}
                  >
                    Approve
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={busy || ownEntry}
                    onClick={onReject}
                  >
                    Reject…
                  </Button>
                </>
              ) : null}
              {tab === "rejected" && ownEntry && !item.locked ? (
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={busy}
                  onClick={onReopen}
                  data-testid={`reopen-${e.id}`}
                >
                  Reopen to draft
                </Button>
              ) : null}
              <Button size="sm" variant="ghost" onClick={onHistory} data-testid={`history-${e.id}`}>
                {historyOpen ? "Hide trail" : "Trail"}
              </Button>
              {item.locked ? <Badge tone="neutral">Locked — invoiced</Badge> : null}
              <span style={{ flex: 1 }} />
              {flag !== undefined ? (
                <span role="status" style={{ color: "var(--m-status-warning-fg)", fontSize: 12 }}>
                  {flag}
                </span>
              ) : null}
              {e.approval_reason_code && REJECTION_REASON_CODES.has(e.approval_reason_code) ? (
                <span role="alert" style={{ color: "var(--m-status-danger-fg)", fontSize: 12 }}>
                  Rejected — {rejectReasonLabel(e.approval_reason_code)}
                </span>
              ) : null}
            </div>
          ) : null}
          {key === "note" && historyOpen ? (
            <div
              className="grid gap-1 py-1 text-[12px]"
              style={{ color: "var(--m-text-secondary)" }}
              data-testid={`trail-${e.id}`}
            >
              {trail.length === 0 ? (
                <span style={{ color: "var(--m-text-muted)" }}>No recorded decisions yet.</span>
              ) : (
                trail.map((line) => <span key={line}>{line}</span>)
              )}
            </div>
          ) : null}
        </Td>
      ))}
    </Tr>
  );
}

const ACTION_LABEL: Readonly<Record<string, string>> = {
  submitted: "Submitted",
  approved: "Approved",
  rejected: "Rejected",
  reopened: "Reopened",
};

// ------------------------------------------------------------- the pane ----

/**
 * The entry in context, 420px beside the table (pack §3). The approver never
 * sees a row id: who, when, how long, billable or not and why, what the tech
 * wrote, how it was logged, and the ticket it belongs to. The decision buttons
 * sit at the bottom, where the eye ends up.
 */
function ContextPane({
  item,
  meId,
  personName,
  categoryLabel,
  verdict,
  records,
  historyOpen,
  busy,
  onApprove,
  onReject,
  onAskTech,
  onApplyFix,
  onHistory,
}: {
  item: QueueItem | null;
  meId: string;
  personName: string;
  categoryLabel: (slug: string) => string;
  verdict: RowVerdict | null;
  records?: ApprovalRecord[];
  historyOpen: boolean;
  busy: boolean;
  onApprove?: () => void;
  onReject?: () => void;
  onAskTech?: () => void;
  onApplyFix?: (fix: NonNullable<RowVerdict["fix"]>) => void;
  onHistory?: () => void;
}) {
  const { timeZone } = useTimeFormat();
  if (item === null) {
    return (
      <MoneyCard testId="approvals-pane">
        <MoneyEmpty
          title="Nothing selected"
          body="Select an entry to see it in context."
          testId="approvals-pane-empty"
        />
      </MoneyCard>
    );
  }
  const e = item.entry;
  const ownEntry = e.user_id === meId;
  const loggedWith =
    e.started_at !== null && e.started_at !== undefined && e.ended_at !== null && e.ended_at !== undefined
      ? `The timer, ${formatInTimeZone(e.started_at, timeZone, "time")} to ${formatInTimeZone(e.ended_at, timeZone, "time")}`
      : "Quick log";
  const trail = (records ?? []).map(
    (r) =>
      `${ACTION_LABEL[r.action] ?? r.action} · ${formatInTimeZone(r.created_at, timeZone, "dayTime")}${
        r.reason_code ? ` · ${rejectReasonLabel(r.reason_code)}` : ""
      }${r.note ? ` · “${r.note}”` : ""}`,
  );

  return (
    <MoneyCard testId="approvals-pane">
      <div className="m-money-pane-head">
        <MoneyAvatar initials={initialsOf(personName)} size={36} own={ownEntry} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <b style={{ display: "block" }}>{personName}</b>
          <span style={{ color: "var(--m-text-secondary)", fontSize: 13 }}>
            {formatInTimeZone(e.started_at, timeZone, "time")} to {formatInTimeZone(e.ended_at, timeZone, "time")}{" "}
            <RecordWindow
              start={e.started_at}
              end={e.ended_at}
              zone={item.user_timezone}
            />{" "}
            ·{" "}
            <b style={{ color: "var(--m-text)" }}>{hms(e.minutes)}</b> ·{" "}
            {e.work_type !== undefined && e.work_type !== null ? categoryLabel(e.work_type) : "Uncategorized"}
          </span>
        </div>
        <MoneyPillTag
          pill={e.billable ? { label: "Billable", family: "neutral" } : { label: "Non-billable", family: "neutral" }}
        />
      </div>
      <div className="m-money-pane-body">
        <div>
          <div className="m-money-lab">What they wrote</div>
          <MoneySaid>{e.notes ?? "Nothing was written."}</MoneySaid>
        </div>
        {verdict !== null ? (
          <div>
            <div className="m-money-lab">The audit says</div>
            <VerdictMark
              label={verdict.label}
              fix={verdict.fix === null ? "Ask the tech" : verdict.fix.label}
              onFix={
                verdict.fix === null || verdict.fix.kind === "ask"
                  ? onAskTech
                  : () => onApplyFix?.(verdict.fix as NonNullable<RowVerdict["fix"]>)
              }
            />
          </div>
        ) : null}
        {!e.billable ? (
          <MoneyKv label="Why it is not billed">
            {UNBILLABLE_REASON_LABELS[e.unbillable_reason ?? ""] ?? e.unbillable_reason ?? "reason pending"}
          </MoneyKv>
        ) : null}
        <MoneyKv label="Logged with">{loggedWith}</MoneyKv>
        {item.submitted_at !== null ? (
          <MoneyKv label="Submitted">{formatInTimeZone(item.submitted_at, timeZone, "dayTime")}</MoneyKv>
        ) : null}
        {item.locked ? (
          <MoneyKv label="Locked">
            <span style={{ color: "var(--m-status-warning-fg)" }}>
              Already invoiced — nothing to decide
            </span>
          </MoneyKv>
        ) : null}
        <div>
          <div className="m-money-lab">The ticket</div>
          <div className="m-money-ticket">
            <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
              <b style={{ flex: 1, minWidth: 0 }}>
                <Link
                  href={`/tickets/${e.ticket_id}`}
                  style={{ color: "var(--m-text)", textDecoration: "none" }}
                >
                  {item.ticket === null || item.ticket === undefined
                    ? `#${e.ticket_id?.slice(0, 8)}…`
                    : `#${item.ticket.number} ${item.ticket.title}`}
                </Link>
              </b>
              <Badge tone={statusTone(e.approval_status)}>{e.approval_status ?? "draft"}</Badge>
            </div>
            <MoneyKv label="Client">
              {item.ticket?.client_name === null || item.ticket?.client_name === undefined ? (
                <span style={{ color: "var(--m-text-muted)" }}>no client yet</span>
              ) : (
                item.ticket.client_name
              )}
            </MoneyKv>
            {looksFine({
              own: ownEntry,
              minutes: e.minutes,
              notes: e.notes,
              flagged: verdict !== null,
            }) ? null : (
              <MoneyKv label="Why not in the pass">
                {notFineReason({
                  own: ownEntry,
                  minutes: e.minutes,
                  notes: e.notes,
                  flagged: verdict !== null,
                })}
              </MoneyKv>
            )}
            {trail.length > 0 ? (
              <div className="m-money-act">
                {trail.map((line) => (
                  <div key={line}>
                    <span className="m-money-act-tm">trail</span>
                    <span>{line}</span>
                  </div>
                ))}
              </div>
            ) : null}
          </div>
        </div>
      </div>
      <div className="m-money-pane-foot">
        {ownEntry ? (
          <p style={{ color: "var(--m-text-secondary)", fontSize: 13, lineHeight: 1.5 }}>
            This is your own time. Another approver signs it off, so it stays out of your bulk
            pass.
          </p>
        ) : (
          <>
            <Button variant="secondary" disabled={busy} onClick={onReject} data-testid="pane-reject">
              Reject… <Kbd>R</Kbd>
            </Button>
            <Button disabled={busy} onClick={onApprove} data-testid="pane-approve">
              Approve <Kbd>A</Kbd>
            </Button>
          </>
        )}
      </div>
      {historyOpen ? (
        <div className="m-money-pane-body" style={{ borderTop: "1px solid var(--m-border)" }}>
          <button type="button" className="m-money-linkbtn" onClick={onHistory}>
            Hide trail
          </button>
          <div className="m-money-act">
            {trail.length === 0 ? (
              <span style={{ color: "var(--m-text-muted)", fontSize: 13 }}>
                No recorded decisions yet.
              </span>
            ) : (
              trail.map((line) => (
                <div key={line}>
                  <span className="m-money-act-tm">trail</span>
                  <span>{line}</span>
                </div>
              ))
            )}
          </div>
        </div>
      ) : null}
    </MoneyCard>
  );
}

function statusTone(status: string | null | undefined): "success" | "danger" | "warning" | "neutral" {
  if (status === "approved") return "success";
  if (status === "rejected") return "danger";
  if (status === "submitted") return "warning";
  return "neutral";
}
