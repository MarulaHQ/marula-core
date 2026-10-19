// /tickets — the ticket ledger, rebuilt as a Marula data surface.
//
// Purpose: the icon rail's Tickets target. A list a technician works FROM, not
// a dump of rows — so it carries the four data-surface controls (ViewSwitch,
// FilterBar, ColumnChooser, ExportControl) on one DataToolbar, bulk selection
// for the actions a person takes on twenty tickets at once, and the two facts a
// queue is judged on per row: who owns it and how long it has left.
//
// Three rules the surface obeys:
//  1. The view, the columns, the filters and the sort are the user's. They live
//     in `appearance.data_surfaces` under `tickets.list` through
//     `useDataSurfacePrefs`, so a column order survives a reload and a second
//     device.
//  2. A row is never a checkbox-only target. Selection is a leading column and
//     the row itself stays one link to the workbench.
//  3. Nothing invents a deadline. The countdown prefers the SLA clock the
//     server reports and falls back to the ticket's own due date, and a ticket
//     with neither reads "—" rather than claiming zero.
//
// Non-purpose: no editing of ticket content. The workbench owns every write
// path except the two bulk acts below, which are one PATCH each and report
// exactly how many rows took.
"use client";

import {
  Badge,
  Button,
  Card,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  EmptyState,
  ExportControl,
  FilterChips,
  FilterTrigger,
  Spinner,
  SurfaceFooter,
  SurfaceHeader,
  type ChoosableColumn,
  type ExportRequest,
  type FilterClause,
  type ViewOption,
} from "@marula-core/ui";
import {
  Segmented,
  SegmentedCount,
  SegmentedItem,
} from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { api, useRequireAuth } from "../../lib/api";
import {
  useDataSurfacePrefs,
  useSavedChoice,
  type SavedSurfaceView,
} from "../../lib/data-surface-prefs";
import {
  TicketTable,
  type LedgerColumnDef,
  type LedgerNameMaps,
} from "../../components/tickets/ledger-table";
import { useTicketSlaMap } from "../../lib/ticket-sla";
import {
  CATEGORY_LABEL,
  TICKET_FILTER_FIELDS,
  matchesClauses,
  priorityBadge,
  relativeTime,
  slaReading,
  sortTickets,
  statusTone,
  type TicketRow,
} from "../../lib/ticket-ops";

/** The D12 key this surface's display state is saved under. */
const SURFACE = "tickets.list";
const TICKET_VIEW_IDS = ["mine", "unassigned", "all"] as const;
type TicketView = (typeof TICKET_VIEW_IDS)[number];
/** One page. Deep pages go through the keyset cursor, which a bulk-selection
 *  surface cannot act on without walking every id first. */
const PAGE_LIMIT = 50;
/** One identity for "no rows yet", so effects keyed on the shown list do not
 *  see a fresh empty array on every render while a load is in flight. Never
 *  mutated — it is shared, not copied. */
const EMPTY_ROWS: TicketRow[] = [];
/** One identity for "no board statuses to offer". The bulk-status effect below
 *  sets this on the paths where no single board is selected, and that effect
 *  depends on the shown rows; a fresh `[]` per run would turn any upstream
 *  identity churn into an unbounded render loop instead of a no-op. */
const EMPTY_BOARD_STATUSES: { id: string; name: string }[] = [];
/** The pack's default queue order: soonest SLA first. One identity, because a
 *  user who has never saved a sort must not hand the memos below a fresh object
 *  on every render — that churn is what starves the App Router's transitions
 *  (see the "settles" test in tests/app/tickets-page.test.tsx). */
const DEFAULT_SORT: { field: string; dir: "asc" | "desc" } = { field: "sla", dir: "asc" };

/** The mockup's views are by ownership, not by category: the queue a person
 *  works from is "mine", then "unassigned" for the dispatcher, then everything. */
const VIEWS: ViewOption[] = [
  { id: "mine", label: "My queue" },
  { id: "unassigned", label: "Unassigned" },
  { id: "all", label: "All tickets" },
];

type ColumnDef = ChoosableColumn & { sortable: boolean };

const COLUMN_DEFS: ColumnDef[] = [
  { key: "priority", label: "P", visible: true, width: 64, sortable: true },
  { key: "number", label: "ID", visible: true, required: true, width: 104, sortable: true },
  { key: "title", label: "Ticket", visible: true, required: true, width: 320, sortable: true },
  { key: "client", label: "Client", visible: true, width: 168, sortable: false },
  { key: "category", label: "Status", visible: true, width: 116, sortable: true },
  { key: "sla", label: "SLA", visible: true, width: 116, sortable: true },
  { key: "assignee", label: "Assignee", visible: true, width: 150, sortable: false },
  /* 170, not 104: the header carries a reorder grip, the label and the sort
     mark in one nowrap line. At 104 the label and mark were clipped once the
     ledger was scaled to its container. The proportions keep the whole table
     inside the card. */
  { key: "updated_at", label: "Last activity", visible: true, width: 170, sortable: true },
  // The pack's "available but hidden" six: Contact, Board, Type and Created
  // are derivable from the row payload, so they ship as choosable columns
  // that cost their name read only while shown. Time logged and Source are
  // NOT offered — the payload carries no source field and a time-logged
  // column would mean a per-row fan-out the list never promised.
  { key: "contact", label: "Contact", visible: false, width: 150, sortable: false },
  { key: "board", label: "Board", visible: false, width: 140, sortable: true },
  { key: "type", label: "Type", visible: false, width: 140, sortable: false },
  { key: "created_at", label: "Created", visible: false, width: 110, sortable: true },
];

const DEFAULT_COLUMNS: ChoosableColumn[] = COLUMN_DEFS.map((c) => ({
  key: c.key,
  label: c.label,
  visible: c.visible,
  width: c.width,
  required: c.required,
}));

/** Merge the surface's own defaults with whatever the user saved. A column the
 *  surface has since dropped stops being offered instead of becoming an
 *  unremovable ghost, and a column added since they last saved appears. */
function reconcileColumns(saved: SavedSurfaceView["columns"]): ChoosableColumn[] {
  if (saved.length === 0) return DEFAULT_COLUMNS;
  const byKey = new Map(saved.map((c) => [c.key, c]));
  const known = COLUMN_DEFS.map((def) => {
    const prior = byKey.get(def.key);
    return {
      key: def.key,
      label: def.label,
      visible: prior?.visible ?? def.visible,
      width: prior?.width ?? def.width,
      required: def.required,
    };
  });
  return known;
}

function clausesFromSaved(saved: { field: string; op: string; value?: string }[]): FilterClause[] {
  const allowed = new Set(TICKET_FILTER_FIELDS.map((f) => f.key));
  return saved
    .filter((f) => allowed.has(f.field))
    .map((f) => ({
      id: `${f.field}:${f.op}:${f.value ?? ""}`,
      field: f.field,
      op: f.op as FilterClause["op"],
      value: f.value,
    }));
}

function toSavedFilters(clauses: FilterClause[]): { field: string; op: string; value?: string }[] {
  return clauses.map((c) => ({ field: c.field, op: c.op, value: c.value }));
}

function csvCell(value: string): string {
  return /[",\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value;
}

export default function TicketsListPage() {
  const auth = useRequireAuth();
  const prefs = useDataSurfacePrefs();
  const saved = prefs.view(SURFACE);

  /* The document resolves after the first paint; the hook applies the saved
     view once it does. A bare initialiser read the empty view and kept "mine"
     for every returning user. */
  const [view, setView] = useSavedChoice(prefs.ready, saved.view, TICKET_VIEW_IDS, "mine");
  const [rows, setRows] = useState<TicketRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [people, setPeople] = useState<Record<string, string>>({});
  const [clients, setClients] = useState<Record<string, string>>({});
  // The optional columns' name maps. Each loads only while its column is
  // shown, so a column nobody turned on costs nothing.
  const [contacts, setContacts] = useState<Record<string, string>>({});
  const [boardNames, setBoardNames] = useState<Record<string, string>>({});
  const [typeNames, setTypeNames] = useState<Record<string, string>>({});
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [bulkBusy, setBulkBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [bulkPriority, setBulkPriority] = useState<components["schemas"]["Priority"]>("p2");
  const [bulkAssignee, setBulkAssignee] = useState("");
  const [bulkStatus, setBulkStatus] = useState("");
  const [bulkBoard, setBulkBoard] = useState("");
  /** The statuses of the single board a multi-board selection shares, if
   *  any: the bulk Set status only means something when the selected rows
   *  all live on one board (a status id is board-scoped). */
  const [bulkBoardStatuses, setBulkBoardStatuses] = useState<{ id: string; name: string }[]>([]);
  const [reloadKey, setReloadKey] = useState(0);
  /** Whether another page exists. A full page means there may be more — the
   *  same rule the server uses for its own `x-next-cursor` header, but decided
   *  from the row count, because a cross-origin response's custom headers are
   *  not readable by the page unless the API exposes them. */
  const [hasMore, setHasMore] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  /** The builder's popover, shared by the chips' `+ Add filter` chip and the
   *  toolbar's Filter button — the template draws both, and both open this. */
  const [filterOpen, setFilterOpen] = useState(false);
  /** The attention shortcut the mockup puts in the title row: null, "risk" or "breach". */
  const [attention, setAttention] = useState<null | "risk" | "breach">(null);

  const columns = useMemo(() => reconcileColumns(saved.columns), [saved.columns]);
  const clauses = useMemo(() => clausesFromSaved(saved.filters), [saved.filters]);
  // A user who never saved a view still gets the pack's default: the queue reads
  // in SLA order, breach first.
  const sort = saved.sort ?? DEFAULT_SORT;
  const visible = useMemo(() => columns.filter((c) => c.visible), [columns]);
  const visibleDefs = useMemo<LedgerColumnDef[]>(
    () =>
      visible.map((c) => ({
        ...c,
        sortable: COLUMN_DEFS.find((d) => d.key === c.key)?.sortable === true,
      })),
    [visible],
  );

  // --- data ---------------------------------------------------------------
  // One fetch per page; the view (mine / unassigned / all) and the attention
  // shortcut are applied to the loaded rows, so switching a view does not
  // refetch. Older rows are reached by walking the list a page at a time, which
  // is what the footer's "Load 50 more" does — before this, the surface stopped
  // at the first page and told the reader to narrow the view instead, which left
  // every older ticket unreachable.
  //
  // The page is asked for by `offset`, not by the keyset cursor the endpoint also
  // offers: the cursor arrives in an `x-next-cursor` response header, and a page
  // on another origin cannot read a custom response header the API does not
  // expose. `offset` is a supported query parameter
  // (crates/core-api/src/routes/tickets.rs:427) and the app's other ledgers page
  // the same way.
  const fetchPage = useCallback(
    async (offset: number): Promise<{ rows: TicketRow[]; full: boolean } | null> => {
      try {
        const result = await api.api.GET("/api/v1/tickets", {
          params: { query: { limit: PAGE_LIMIT, offset } },
        });
        if (result.error !== undefined || result.data === undefined) return null;
        const rows = result.data as TicketRow[];
        return { rows, full: rows.length === PAGE_LIMIT };
      } catch {
        // A transport failure (an aborted request, a dropped connection) rejects
        // rather than returning an error field. Without this the surface stayed
        // on its loading splash forever instead of showing the error state.
        return null;
      }
    },
    [],
  );

  /** Append the next page. Selection survives, because a bulk act has to be
   *  able to reach a row that arrived on page two, and the append de-duplicates
   *  by id so an offset page that overlaps cannot double a row. */
  const loadMore = useCallback(async () => {
    if (!hasMore) return;
    const offset = rows?.length ?? 0;
    setLoadingMore(true);
    const page = await fetchPage(offset);
    setLoadingMore(false);
    if (page === null) {
      setNotice("The next page of tickets did not load. Try again.");
      return;
    }
    setRows((prev) => {
      const seen = new Set((prev ?? []).map((r) => r.id));
      return [...(prev ?? []), ...page.rows.filter((r) => !seen.has(r.id))];
    });
    setHasMore(page.full);
  }, [fetchPage, hasMore, rows]);

  const refresh = useCallback(
    async () => {
      setRows(null);
      setError(null);
      const page = await fetchPage(0);
      if (page === null) {
        setError("The ticket list did not load. Check your connection and retry.");
        setRows([]);
        setHasMore(false);
        return;
      }
      const next = [...page.rows].sort((a, b) => b.updated_at.localeCompare(a.updated_at));
      setRows(next);
      setHasMore(page.full);
    },
    [fetchPage],
  );

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh, reloadKey]);

  // The SLA clock the customer is promised, per row. The read itself is shared
  // and cached by ticket id (lib/ticket-sla), so this page pays one capped
  // fan-out per ticket per SESSION rather than fifty calls every time the
  // ledger is opened, and the workbench pays none for a ticket already seen.
  const slaIds = useMemo(() => (rows ?? []).map((r) => r.id), [rows]);
  // A merged row links to its survivor, which is usually a row already on
  // this page — so the banner can name it instead of claiming it is out of
  // scope.
  const numbersById = useMemo(() => {
    const map: Record<string, string> = {};
    for (const row of rows ?? []) map[row.id] = row.number;
    return map;
  }, [rows]);
  const { clocks: sla } = useTicketSlaMap(slaIds);

  // Assignee names. The staff directory is manager-scoped, so its absence
  // degrades the column to "You" / "Assigned" rather than breaking the surface.
  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void (async () => {
      try {
        const [res, clientRes] = await Promise.all([
          api.api.GET("/api/v1/org/users", { params: { query: { limit: 200 } } }),
          api.api.GET("/api/v1/clients"),
        ]);
        if (cancelled) return;
        if (res.data !== undefined) {
          const map: Record<string, string> = {};
          for (const user of res.data.users) map[user.id] = user.display_name;
          setPeople(map);
        }
        if (clientRes.data !== undefined) {
          const cmap: Record<string, string> = {};
          for (const client of clientRes.data) cmap[client.id] = client.name;
          setClients(cmap);
        }
      } catch {
        if (!cancelled) setPeople({});
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status]);

  // --- derived rows -------------------------------------------------------
  const myId = auth.status === "authed" ? auth.me?.user_id ?? null : null;

  // The optional columns, read only while shown. Board and Type names are
  // one org-scoped read each; a Contact needs its client's directory, which
  // is bounded by the distinct clients on this page and cached in state.
  const showsOptional = useMemo(
    () => ({
      contact: visible.some((c) => c.key === "contact"),
      board: visible.some((c) => c.key === "board"),
      type: visible.some((c) => c.key === "type"),
    }),
    [visible],
  );

  /** The maps as the table and the export take them, gated per column. */
  const optionalNames = useCallback(
    (): LedgerNameMaps => ({
      contacts: showsOptional.contact ? contacts : undefined,
      boards: showsOptional.board ? boardNames : undefined,
      types: showsOptional.type ? typeNames : undefined,
    }),
    [showsOptional, contacts, boardNames, typeNames],
  );

  useEffect(() => {
    if (auth.status !== "authed" || !showsOptional.board) return;
    let cancelled = false;
    void (async () => {
      const res = await api.api.GET("/api/v1/boards/nav");
      if (cancelled || res.data === undefined) return;
      const map: Record<string, string> = {};
      for (const board of res.data) map[board.id] = board.name;
      setBoardNames(map);
    })().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [auth.status, showsOptional.board]);

  useEffect(() => {
    if (auth.status !== "authed" || !showsOptional.type) return;
    let cancelled = false;
    void (async () => {
      const res = await api.api.GET("/api/v1/taxonomy/types", { params: { query: {} } });
      if (cancelled || res.data === undefined) return;
      const map: Record<string, string> = {};
      for (const node of res.data) map[node.id] = node.name;
      setTypeNames(map);
    })().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [auth.status, showsOptional.type]);

  useEffect(() => {
    if (auth.status !== "authed" || !showsOptional.contact) return;
    const clientIds = [...new Set((rows ?? []).map((r) => r.client_id).filter((id): id is string => id !== null && id !== undefined))];
    if (clientIds.length === 0) return;
    let cancelled = false;
    void (async () => {
      const map: Record<string, string> = {};
      await Promise.all(
        clientIds.map(async (clientId) => {
          const res = await api.api.GET("/api/v1/clients/{id}/contacts", {
            params: { path: { id: clientId } },
          });
          for (const contact of res.data ?? []) map[contact.id] = contact.name;
        }),
      );
      if (!cancelled) setContacts(map);
    })().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [auth.status, showsOptional.contact, rows]);

  // Counts for the view switch, computed from the loaded rows.
  const viewCounts = useMemo(() => {
    const all = rows ?? [];
    return {
      mine: all.filter((r) => r.assignee_id === myId).length,
      unassigned: all.filter((r) => !r.assignee_id).length,
      all: all.length,
    };
  }, [rows, myId]);


  // The two attention shortcuts the mockup puts in the title row. "Changed
  // since you looked" needs an unread flag the list payload does not carry, so
  // it is not built yet.
  const attentionCounts = useMemo(() => {
    let risk = 0;
    let breach = 0;
    for (const r of rows ?? []) {
      const tone = slaReading(r, sla[r.id] ?? null)?.tone;
      if (tone === "danger") breach += 1;
      else if (tone === "warning") risk += 1;
    }
    return { risk, breach };
  }, [rows, sla]);

  const filtered = useMemo(() => {
    if (rows === null) return null;
    let base = rows;
    if (view === "mine") base = base.filter((r) => r.assignee_id === myId);
    else if (view === "unassigned") base = base.filter((r) => !r.assignee_id);
    if (attention !== null) {
      base = base.filter((r) => {
        const tone = slaReading(r, sla[r.id] ?? null)?.tone;
        return attention === "breach" ? tone === "danger" : tone === "warning";
      });
    }
    // Two filter fields take a human word where the row carries an id or an
    // enum, so they are translated before matching: a Client clause names the
    // client and a Status clause names the status label. That is what keeps
    // the chip reading "Client is Acme Corporation" / "Status is not Closed"
    // — the pack's chip wording — instead of a UUID or an enum slug.
    const resolved = clauses.map((c) => {
      if (c.field === "client_id") {
        const id = Object.entries(clients).find(
          ([, name]) => name.toLowerCase() === (c.value ?? "").trim().toLowerCase(),
        );
        return { ...c, value: id?.[0] ?? "\u0000no-such-client" };
      }
      if (c.field === "category") {
        const hit = (Object.entries(CATEGORY_LABEL) as [string, string][]).find(
          ([, label]) => label.toLowerCase() === (c.value ?? "").trim().toLowerCase(),
        );
        return { ...c, value: hit?.[0] ?? "\u0000no-such-status" };
      }
      return c;
    });
    const ordered = sortTickets(base.filter((r) => matchesClauses(r, resolved)), sort);
    // SLA is not a row field: it comes from the SLA map, so it sorts here.
    // Soonest first by default, which is the mockup's order; a ticket with no
    // policy sorts last rather than claiming first place. The sentinel is a
    // finite number and ties compare 0: Infinity - Infinity is NaN, and a
    // NaN comparator scrambles V8's order (the captured ledger read neither
    // soonest-first nor number order until this was fixed).
    if (sort?.field === "sla") {
      const NO_SLA = Number.MAX_SAFE_INTEGER;
      const rank = (id: string) => sla[id]?.minutes_remaining ?? NO_SLA;
      const dir = sort.dir === "asc" ? 1 : -1;
      return [...ordered].sort((a, b) => {
        const diff = (rank(a.id) - rank(b.id)) * dir;
        return diff === 0 ? a.number.localeCompare(b.number) : diff;
      });
    }
    return ordered;
  }, [rows, clauses, sort, view, myId, attention, sla, clients]);

  // A stable identity matters: an effect keyed on `shown` (the bulk status
  // read below) would otherwise see a fresh `[]` on every render while the
  // rows are still loading and loop.
  const shown = useMemo(() => filtered ?? EMPTY_ROWS, [filtered]);

  // A selection that outlives its row would let a bulk act on a ticket that is
  // no longer on screen, so it is pruned against what is actually rendered.
  const liveIds = useMemo(() => new Set(shown.map((r) => r.id)), [shown]);
  useEffect(() => {
    setSelected((prev) => {
      const next = new Set([...prev].filter((id) => liveIds.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [liveIds]);

  const allSelected = shown.length > 0 && shown.every((r) => selected.has(r.id));

  const toggleAll = useCallback(() => {
    setSelected(allSelected ? new Set() : new Set(shown.map((r) => r.id)));
  }, [allSelected, shown]);

  const toggleOne = useCallback((id: string, on: boolean) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  }, []);

  /** The row's one-click Take it: one PATCH against the caller's own id,
   *  reported in the surface's notice exactly like a bulk act. */
  const takeRow = useCallback(
    async (row: TicketRow) => {
      if (auth.me === null) return;
      setNotice(null);
      try {
        const res = await api.api.PATCH("/api/v1/tickets/{id}", {
          params: { path: { id: row.id } },
          body: { assignee_id: auth.me.user_id },
        });
        if (res.error !== undefined) {
          setNotice(`${row.number} was not taken — try again.`);
          return;
        }
        setNotice(`Took ${row.number}. It is in My queue now.`);
        setReloadKey((k) => k + 1);
      } catch {
        setNotice(`${row.number} was not taken — try again.`);
      }
    },
    [auth.me],
  );

  // --- persistence --------------------------------------------------------
  const persist = useCallback(
    (patch: {
      view?: string;
      columns?: SavedSurfaceView["columns"];
      filters?: SavedSurfaceView["filters"];
      sort?: SavedSurfaceView["sort"];
    }) => {
      void prefs.save(SURFACE, {
        view: patch.view ?? view,
        columns: patch.columns ?? columns.map((c) => ({ key: c.key, visible: c.visible, width: c.width })),
        filters: patch.filters ?? saved.filters,
        sort: patch.sort ?? saved.sort,
      });
    },
    [prefs, view, columns, saved.filters, saved.sort],
  );

  const onView = useCallback(
    (id: TicketView) => {
      setView(id);
      setSelected(new Set());
      persist({ view: id });
    },
    [persist],
  );

  const onColumns = useCallback(
    (next: ChoosableColumn[]) => {
      persist({ columns: next.map((c) => ({ key: c.key, visible: c.visible, width: c.width })) });
    },
    [persist],
  );

  const onClauses = useCallback(
    (next: FilterClause[]) => {
      persist({ filters: toSavedFilters(next) });
    },
    [persist],
  );

  const onSort = useCallback(
    (field: string) => {
      const dir = sort?.field === field && sort.dir === "asc" ? "desc" : "asc";
      persist({ sort: { field, dir } });
    },
    [persist, sort],
  );

  // --- bulk ---------------------------------------------------------------
  const runBulk = useCallback(
    async (
      patch: {
        priority?: components["schemas"]["Priority"];
        assignee_id?: string | null;
        category_id?: string | null;
      },
      verb: string,
    ) => {
      const ids = [...selected];
      if (ids.length === 0) return;
      setBulkBusy(true);
      setNotice(null);
      let ok = 0;
      const failures: string[] = [];
      await Promise.all(
        ids.map(async (id) => {
          try {
            const res = await api.api.PATCH("/api/v1/tickets/{id}", {
              params: { path: { id } },
              body: patch,
            });
            if (res.error === undefined) ok += 1;
            else failures.push(id);
          } catch {
            failures.push(id);
          }
        }),
      );
      setBulkBusy(false);
      setNotice(
        failures.length === 0
          ? `${verb} on ${ok} ${ok === 1 ? "ticket" : "tickets"}.`
          : `${verb} on ${ok} of ${ids.length}. ${failures.length} could not be saved — retry those rows.`,
      );
      setSelected(new Set());
      setReloadKey((k) => k + 1);
    },
    [selected],
  );

  /** Move a selection to another board. Each ticket remaps its own status
   *  through the semantic categories (the server's rule); the ones it cannot
   *  place without a human choice are named in the notice rather than moved
   *  on a guess, and a rejected row keeps its board. */
  const runBulkMove = useCallback(
    async (targetBoardId: string) => {
      const ids = [...selected];
      if (ids.length === 0) return;
      setBulkBusy(true);
      setNotice(null);
      let ok = 0;
      let conflicts = 0;
      let rejected = 0;
      await Promise.all(
        ids.map(async (id) => {
          try {
            const res = await api.api.POST("/api/v1/tickets/{id}/move-board", {
              params: { path: { id } },
              body: { target_board_id: targetBoardId },
            });
            if (res.error === undefined) ok += 1;
            else if (res.response.status === 409) conflicts += 1;
            else rejected += 1;
          } catch {
            rejected += 1;
          }
        }),
      );
      setBulkBusy(false);
      const parts = [`Moved ${ok} of ${ids.length}.`];
      if (conflicts > 0) {
        parts.push(
          `${conflicts} ${conflicts === 1 ? "needs" : "need"} a status choice — open ${conflicts === 1 ? "it" : "them"} to pick one.`,
        );
      }
      if (rejected > 0) {
        parts.push(`${rejected} could not move — nothing changed on those rows.`);
      }
      setNotice(parts.join(" "));
      setSelected(new Set());
      setBulkBoard("");
      setReloadKey((k) => k + 1);
    },
    [selected],
  );

  // The target board list for the bulk move, read once while a selection
  // exists (one org-scoped read, then the bar owns the pick).
  const [boards, setBoards] = useState<{ id: string; name: string }[]>([]);
  useEffect(() => {
    if (auth.status !== "authed" || selected.size === 0) return;
    let cancelled = false;
    void (async () => {
      const res = await api.api.GET("/api/v1/boards/nav");
      if (cancelled || res.data === undefined) return;
      setBoards(res.data.map((b) => ({ id: b.id, name: b.name })));
    })().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [auth.status, selected.size]);

  // While a selection exists on ONE board, that board's statuses can be
  // offered to the Set status picker (a status id is board-scoped, so a
  // mixed-board selection has no meaningful status to set).
  useEffect(() => {
    if (selected.size === 0) {
      setBulkBoardStatuses((prev) => (prev.length === 0 ? prev : EMPTY_BOARD_STATUSES));
      setBulkStatus("");
      return;
    }
    const boardIds = new Set(shown.filter((r) => selected.has(r.id)).map((r) => r.board_id));
    if (boardIds.size !== 1) {
      setBulkBoardStatuses((prev) => (prev.length === 0 ? prev : EMPTY_BOARD_STATUSES));
      setBulkStatus("");
      return;
    }
    const boardId = [...boardIds][0];
    let cancelled = false;
    void (async () => {
      const res = await api.api.GET("/api/v1/boards/{id}/statuses", { params: { path: { id: boardId ?? "" } } });
      if (cancelled || res.data === undefined) return;
      setBulkBoardStatuses(res.data.map((s) => ({ id: s.id, name: s.name })));
    })().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [selected, shown]);

  // --- export -------------------------------------------------------------
  const onExport = useCallback(
    (request: ExportRequest) => {
      const header = request.columns;
      const lines = [header.map(csvCell).join(",")];
      // The optional columns' name maps, so an exported Contact/Board/Type
      // reads as a name rather than the id the row carries.
      const names = optionalNames();
      for (const row of shown) {
        lines.push(header.map((key) => csvCell(exportValue(row, key, sla, people, auth.me?.user_id ?? null, names))).join(","));
      }
      const blob = new Blob([lines.join("\n")], { type: "text/csv;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = `tickets-${view}-${new Date().toISOString().slice(0, 10)}.csv`;
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
      URL.revokeObjectURL(url);
      setNotice(`Exported ${shown.length} ${shown.length === 1 ? "row" : "rows"} as CSV.`);
    },
    [shown, view, sla, people, auth.me, contacts, boardNames, typeNames, showsOptional],
  );

  /** The bulk bar's "Export N": the selection only, never the whole view. */
  const onExportSelected = useCallback(() => {
    const rows = shown.filter((r) => selected.has(r.id));
    const names = optionalNames();
    const keys = visible.map((c) => c.key);
    const lines = [keys.map(csvCell).join(",")];
    for (const row of rows) {
      lines.push(keys.map((key) => csvCell(exportValue(row, key, sla, people, auth.me?.user_id ?? null, names))).join(","));
    }
    const blob = new Blob([lines.join("\n")], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `tickets-${view}-selection-${new Date().toISOString().slice(0, 10)}.csv`;
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    URL.revokeObjectURL(url);
    setNotice(`Exported ${rows.length} selected ${rows.length === 1 ? "row" : "rows"} as CSV.`);
  }, [shown, selected, visible, sla, people, auth.me?.user_id, view, optionalNames]);

  if (auth.status !== "authed") return null;

  const mergedCount = shown.filter((r) => r.merged_into_ticket_id !== null && r.merged_into_ticket_id !== undefined).length;

  /* The pack keeps the footer in every state, because that is where the density
     control lives: a reader who cannot see the table is exactly the one trying
     to widen it. One element, rendered under whichever body is showing, rather
     than a footer that exists only once there are rows. */
  const surfaceFooter = (
    <SurfaceFooter
      shown={shown.length}
      total={viewCounts.all}
      noun="ticket"
      onLoadMore={hasMore ? () => void loadMore() : undefined}
      loadMoreCount={PAGE_LIMIT}
      loadingMore={loadingMore}
      density={
        <DensitySwitch
          value={prefs.density}
          onChange={prefs.setDensity}
          variant="icons"
          label="Row density"
        />
      }
      trailing={
        mergedCount === 0 ? undefined : (
          <span className="tabular-nums">{mergedCount} merged</span>
        )
      }
    />
  );

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-4 p-4 md:p-6">
      <SurfaceHeader
        title="Tickets"
        meta={
          /* The mockup's attention shortcuts ride the title row, doubling as
             filters. Blush for "at risk", red for "breached"; the count is the
             number of rows that match. "Changed since you looked" is the pack's
             third shortcut and still unbuilt (the list payload carries no unread
             flag). The title row's "New ticket" primary is likewise unbuilt:
             the only create flow in the product is the board's dialog, so the
             slot stays empty rather than shipping a dead button. */
          <div className="flex flex-wrap items-center gap-2" role="group" aria-label="Attention shortcuts">
            {([
              { id: "risk", label: "At risk", colour: "var(--m-blush)", count: attentionCounts.risk },
              { id: "breach", label: "Breached", colour: "var(--m-status-danger-fg)", count: attentionCounts.breach },
            ] as const).map((s) => {
              const active = attention === s.id;
              return (
                <button
                  key={s.id}
                  type="button"
                  aria-pressed={active}
                  onClick={() => setAttention((a) => (a === s.id ? null : s.id))}
                  className="inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs"
                  style={{
                    borderColor: active ? s.colour : "var(--m-border)",
                    color: "var(--m-text-secondary)",
                    background: active ? "var(--m-surface-2)" : "var(--m-surface)",
                  }}
                >
                  <span className="inline-block h-2 w-2 rounded-full" style={{ background: s.colour }} aria-hidden="true" />
                  {s.label}
                  <span className="tabular-nums font-medium" style={{ color: "var(--m-text)" }}>{s.count}</span>
                </button>
              );
            })}
          </div>
        }
      />

      <DataToolbar
        label="Ticket list controls"
        views={
          <Segmented type="single" value={view} onValueChange={(v) => v && onView(v as TicketView)} aria-label="Ticket views">
            {VIEWS.map((v) => (
              <SegmentedItem key={v.id} value={v.id}>
                {v.label}
                <SegmentedCount>{viewCounts[v.id as keyof typeof viewCounts]}</SegmentedCount>
              </SegmentedItem>
            ))}
          </Segmented>
        }
        chips={
          <FilterChips
            fields={TICKET_FILTER_FIELDS}
            clauses={clauses}
            onChange={onClauses}
            label="Ticket filters"
            emptyHint="No filters"
            onAdd={() => setFilterOpen(true)}
          />
        }
        filter={
          <FilterTrigger
            fields={TICKET_FILTER_FIELDS}
            clauses={clauses}
            onChange={onClauses}
            open={filterOpen}
            onOpenChange={setFilterOpen}
            label="Ticket filters"
          />
        }
        columns={<ColumnChooser columns={columns} onChange={onColumns} label="Ticket columns" />}
        export={
          <ExportControl
            columns={visible.map((c) => c.key)}
            rowCount={shown.length}
            sort={sort}
            filters={clauses}
            onExport={onExport}
          />
        }
      />

      {notice !== null ? (
        <p className="text-[13px]" role="status" style={{ color: "var(--m-text-secondary)" }}>
          {notice}
        </p>
      ) : null}
      {prefs.error !== null ? (
        <p className="text-[13px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
          {prefs.error}
        </p>
      ) : null}

      {rows === null ? (
        <Card>
          <div className="p-4">
            <Spinner label="Loading tickets" />
          </div>
          {surfaceFooter}
        </Card>
      ) : error !== null ? (
        <Card>
          <div className="p-6">
            <EmptyState
              message={
                "Couldn't load tickets. Filters and columns are saved, so nothing is lost."
              }
              illustration={<ErrorMark />}
              action={
                // The pack's error state carries a second action, "Check
                // status". This product has no status page to point it at, so
                // the slot stays empty rather than shipping a dead button.
                <Button size="sm" variant="primary" onClick={() => void refresh()}>
                  Retry
                </Button>
              }
            />
          </div>
          {surfaceFooter}
        </Card>
      ) : shown.length === 0 ? (
        <Card>
          <div className="p-6">
            {clauses.length > 0 ? (
              <EmptyState
                message="No tickets match these filters. Remove one to widen the search."
                illustration={<MagnifierMark />}
                action={
                  <Button size="sm" variant="secondary" onClick={() => onClauses([])}>
                    Clear filters
                  </Button>
                }
              />
            ) : (
              <EmptyState
                message={
                  attention === "breach"
                    ? "Nothing is breaching its SLA. Good."
                    : attention === "risk"
                      ? "Nothing is at risk right now."
                      : view === "mine"
                        ? "Nothing is assigned to you right now."
                        : view === "unassigned"
                          ? "Everything open has an owner."
                          : "There are no tickets yet."
                }
                illustration={<SparklesMark />}
                action={
                  view === "mine" && viewCounts.unassigned > 0 ? (
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => onView("unassigned")}
                    >
                      See unassigned ({viewCounts.unassigned})
                    </Button>
                  ) : undefined
                }
              />
            )}
          </div>
          {surfaceFooter}
        </Card>
      ) : (
        <Card className="overflow-hidden">
          <TicketTable
            columns={visibleDefs}
            rows={shown}
            sort={sort}
            sla={sla}
            people={people}
            clients={clients}
            names={optionalNames()}
            numbersById={numbersById}
            myId={auth.me?.user_id ?? null}
            selected={selected}
            onSort={onSort}
            onReorderColumns={(from, to) => {
              const next = columns.slice();
              const [moved] = next.splice(from, 1);
              if (moved !== undefined) next.splice(to, 0, moved);
              onColumns(next);
            }}
            onResizeColumn={(key, width) =>
              onColumns(columns.map((c) => (c.key === key ? { ...c, width } : c)))
            }
            onToggleRow={toggleOne}
            onTake={(row) => void takeRow(row)}
            bulk={
              selected.size > 0
                ? {
                    count: selected.size,
                    onClear: () => setSelected(new Set()),
                    children: (
                      <>
                        {/* The pickers each set a value and the button the
                            picker feeds carries the primary treatment: the two
                            are a pair, so the control the picker feeds is the
                            one that reads as "this is why you are here". */}
                        <select
                          className="m-input"
                          aria-label="User to assign the selected tickets to"
                          value={bulkAssignee}
                          onChange={(e) => setBulkAssignee(e.target.value)}
                        >
                          <option value="">Assign to…</option>
                          {Object.entries(people).map(([id, name]) => (
                            <option key={id} value={id}>
                              {name}
                            </option>
                          ))}
                        </select>
                        <Button
                          size="sm"
                          variant="primary"
                          disabled={bulkBusy || bulkAssignee === ""}
                          onClick={() => void runBulk({ assignee_id: bulkAssignee }, "Assigned")}
                        >
                          Assign
                        </Button>
                        {bulkBoardStatuses.length > 0 ? (
                          <>
                            <select
                              className="m-input"
                              aria-label="Status to set on the selected tickets"
                              value={bulkStatus}
                              onChange={(e) => setBulkStatus(e.target.value)}
                            >
                              <option value="">Set status…</option>
                              {bulkBoardStatuses.map((s) => (
                                <option key={s.id} value={s.id}>
                                  {s.name}
                                </option>
                              ))}
                            </select>
                            <Button
                              size="sm"
                              variant="primary"
                              disabled={bulkBusy || bulkStatus === ""}
                              onClick={() => void runBulk({ category_id: bulkStatus }, "Status set")}
                            >
                              Set status
                            </Button>
                          </>
                        ) : null}
                        <select
                          className="m-input"
                          aria-label="Priority to set on the selected tickets"
                          value={bulkPriority}
                          onChange={(e) => setBulkPriority(e.target.value as components["schemas"]["Priority"])}
                        >
                          <option value="p1">P1</option>
                          <option value="p2">P2</option>
                          <option value="p3">P3</option>
                          <option value="p4">P4</option>
                        </select>
                        <Button
                          size="sm"
                          variant="primary"
                          disabled={bulkBusy}
                          onClick={() => void runBulk({ priority: bulkPriority }, `Set ${bulkPriority.toUpperCase()}`)}
                        >
                          Set priority
                        </Button>
                        <select
                          className="m-input"
                          aria-label="Board to move the selected tickets to"
                          value={bulkBoard}
                          onChange={(e) => setBulkBoard(e.target.value)}
                        >
                          <option value="">Move to board…</option>
                          {boards.map((b) => (
                            <option key={b.id} value={b.id}>
                              {b.name}
                            </option>
                          ))}
                        </select>
                        <Button
                          size="sm"
                          variant="primary"
                          disabled={bulkBusy || bulkBoard === ""}
                          onClick={() => void runBulkMove(bulkBoard)}
                        >
                          Move
                        </Button>
                        <Button size="sm" variant="secondary" disabled={bulkBusy} onClick={onExportSelected}>
                          Export {selected.size}
                        </Button>
                      </>
                    ),
                  }
                : undefined
            }
          />
          {surfaceFooter}
        </Card>
      )}
    </div>
  );
}

// --- one row ---------------------------------------------------------------

function exportValue(
  row: TicketRow,
  key: string,
  sla: Record<string, components["schemas"]["TicketSlaStatus"] | null>,
  people: Record<string, string>,
  myId: string | null,
  names?: LedgerNameMaps,
): string {
  switch (key) {
    case "category":
      return CATEGORY_LABEL[row.category] ?? row.category;
    case "priority":
      return row.priority.toUpperCase();
    case "assignee":
      if (!row.assignee_id) return "Unassigned";
      return people[row.assignee_id] ?? (row.assignee_id === myId ? "You" : "Assigned");
    case "sla":
      return slaReading(row, sla[row.id] ?? null)?.label ?? "";
    case "contact":
      return row.contact_id ? names?.contacts?.[row.contact_id] ?? "" : "";
    case "board":
      return names?.boards?.[row.board_id] ?? "";
    case "type":
      return row.type_id ? names?.types?.[row.type_id] ?? "" : "";
    case "updated_at":
      return row.updated_at;
    default: {
      const raw = (row as unknown as Record<string, unknown>)[key];
      return raw === null || raw === undefined ? "" : String(raw);
    }
  }
}

// --- the empty states' marks ------------------------------------------------
//
// The pack's three empty shapes each carry their own small line drawing, so a
// reader tells the cause apart before reading a word: a magnifier for "your
// filters matched nothing", sparkles for "this queue is quiet", and a blush
// cross for "we could not load it" — the cross is blush, never red, because a
// failed read is not a breach. All three are stroked in text-secondary on the
// same 48px grid the kit's default mark uses.

function EmptyMarkBase({ children }: { children: ReactNode }): React.JSX.Element {
  return (
    <svg
      width="48"
      height="48"
      viewBox="0 0 48 48"
      fill="none"
      stroke="var(--m-text-secondary)"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

function MagnifierMark(): React.JSX.Element {
  return (
    <EmptyMarkBase>
      <circle cx="21" cy="21" r="12" />
      <path d="m30 30 9 9" />
      <path d="M16 21h10" />
    </EmptyMarkBase>
  );
}

function SparklesMark(): React.JSX.Element {
  return (
    <EmptyMarkBase>
      <path d="M24 12v0M24 12c.8 4.5 3.5 7.2 8 8-4.5.8-7.2 3.5-8 8-.8-4.5-3.5-7.2-8-8 4.5-.8 7.2-3.5 8-8Z" />
      <path d="M35 32c.5 2.6 2 4.1 4.6 4.6-2.6.5-4.1 2-4.6 4.6-.5-2.6-2-4.1-4.6-4.6 2.6-.5 4.1-2 4.6-4.6Z" />
    </EmptyMarkBase>
  );
}

function ErrorMark(): React.JSX.Element {
  return (
    <svg
      width="48"
      height="48"
      viewBox="0 0 48 48"
      fill="none"
      stroke="var(--m-blush)"
      strokeWidth="2"
      strokeLinecap="round"
      aria-hidden="true"
    >
      <circle cx="24" cy="24" r="13" />
      <path d="M24 16v9" />
      <path d="M24 29v.4" />
    </svg>
  );
}
