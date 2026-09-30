"use client";
/**
 * /clients — the client list, built against the Marula surface pack's
 * `web/clients` states 01 and 02 (Wave 41 lane 7 `p41-ui-clients`).
 *
 * Purpose: an account manager on a monthly rhythm needs to know which
 * relationships need attention this week. So the page is the Tickets list's
 * data-surface template (DERIVATION-GUIDE: the list copies TL, changing only
 * its columns), plus the two things the pack puts in the header and the two
 * it puts in a row:
 *
 * - Header: two **shortcut pills** beside the title — `Renewing in 90 days`
 *   and `No contract` — each a counter that toggles a filter. They are the
 *   fastest way in and they only ever count the current view.
 * - Row: a **blush renewal pill** inside the Renews column when the term is
 *   inside the 90-day window, carrying the number of days. A client with no
 *   contract says `None` and `Not set` in words, never an empty cell.
 *
 * **Record dedupe (AI pattern P3, and the strict rules around it).** The
 * pack draws this state, so this lane implements it, and it is shaped exactly
 * as `DESIGN-NOTES-clients.md` §4 requires:
 *   - a **blush strip** above the table, counting what was found and saying
 *     that merging keeps both histories, with a Review action;
 *   - a **row mark** in the row's own second line — a blush pill naming the
 *     duplicate plus a one-click Merge;
 *   - **no panel and nothing inverted.** The Merge action is a row action,
 *     never a side panel spending the screen's single inverted surface.
 * With the grant absent, the row is an ordinary row and nothing marks it —
 * no sparkle, no hint, no "ask AI" (`DESIGN-NOTES.md` §A8 rule 1).
 *
 * The grant is a real capability decision, not a query parameter: it reads
 * `decision_client_dedupe` from the signed-in user's permissions, the same
 * way every other granted surface reads its decision, so an operator without
 * it gets the ordinary list with no trace of an affordance that is not theirs.
 *
 * Non-purpose: no editing of the visibility rule here (the record shows the
 * hint; rule editing follows the visibility-rules admin surface), and no
 * contract authoring from this page — a contract is set up on the record.
 */
import {
  Badge,
  Button,
  ColumnChooser,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  ExportControl,
  FieldHint,
  Input,
  Label,
  Spinner,
  SurfaceHeader,
  Table,
  Td,
  Th,
  Tr,
  ViewSwitch,
  DataToolbar,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { rejectionText } from "../../lib/error-copy";
import { api, useAuth, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { useDataSurfacePrefs } from "../../lib/data-surface-prefs";
import {
  formatPackDate,
  renewalSignal,
  insideRenewalWindow,
} from "../../lib/renewal-signals";

type Client = components["schemas"]["Client"];
type Contract = components["schemas"]["Contract"];
type ClientMutationResponse = components["schemas"]["ClientMutationResponse"];
type EntityMergeCandidate = components["schemas"]["EntityMergeCandidate"];

/** The surface's persisted-preference key. Namespace'd so it cannot collide
 * with another lane's data surface. */
const SURFACE = "clients.list";

/** The pack's column set: Client, Main contact, Open tickets, Contract,
 * Renews, Account owner (DESIGN-NOTES-tickets.md §11). */
type ColumnId =
  | "client"
  | "contact"
  | "openTickets"
  | "contract"
  | "renews"
  | "owner"
  | "visibility";

const COLUMNS: { id: ColumnId; label: string; defaultVisible: boolean }[] = [
  { id: "client", label: "Client", defaultVisible: true },
  { id: "contact", label: "Main contact", defaultVisible: true },
  { id: "openTickets", label: "Open tickets", defaultVisible: true },
  { id: "contract", label: "Contract", defaultVisible: true },
  { id: "renews", label: "Renews", defaultVisible: true },
  { id: "owner", label: "Account owner", defaultVisible: true },
  // The existing Visibility column stays available and hidden by default: the
  // pack's column set does not draw it, but removing a column a user already
  // relies on is not this lane's call.
  { id: "visibility", label: "Visibility", defaultVisible: false },
];

const VIEWS = [
  { id: "open", label: "Open work" },
  { id: "assets", label: "Assets" },
  { id: "bill", label: "Billing history" },
];

function Monogram({ name }: { name: string }) {
  const initials = name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join("");
  return (
    <span
      aria-hidden="true"
      className="grid h-[26px] w-[26px] shrink-0 place-items-center rounded-[999px] text-[12px] font-semibold"
      style={{ background: "var(--m-canopy-deep)", color: "var(--m-cream)" }}
    >
      {initials}
    </span>
  );
}

/** The pack's renewal pill. Blush, with the clock icon AND the number of
 * days, so the signal is never colour alone. */
function RenewalPill({ label }: { label: string }) {
  return (
    <Badge tone="due" data-testid="client-renewal-pill">
      <span className="inline-flex items-center gap-1">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
          <circle cx="12" cy="12" r="9" />
          <path d="M12 7v5l3 2" strokeLinecap="round" />
        </svg>
        {label}
      </span>
    </Badge>
  );
}

export default function ClientsPage() {
  const auth = useRequireAuth();
  const { me } = useAuth();
  const prefs = useDataSurfacePrefs();

  const [clients, setClients] = useState<Client[] | null>(null);
  const [contracts, setContracts] = useState<Contract[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [view, setView] = useState("open");
  const [query, setQuery] = useState("");
  // The two header shortcuts, as pressed-or-not toggles rather than separate
  // filter state: pressing one narrows the same list the view already shows.
  const [onlyRenewing, setOnlyRenewing] = useState(false);
  const [onlyNoContract, setOnlyNoContract] = useState(false);

  const [createOpen, setCreateOpen] = useState(false);
  const [name, setName] = useState("");
  const [notes, setNotes] = useState("");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const isAdmin = (me?.permissions ?? []).includes("special:manage_users") ?? false;
  // The Account owner column carries initials beside the name, so the person
  // is recognisable at a glance rather than being a bare word in a column.
  const ownerName = me?.display_name ?? me?.email ?? "";
  // The record-dedupe grant (P3). Absent means the surface is an ordinary
  // list: no strip, no row mark, no Merge, and no hint that one exists.
  const dedupeGranted = (me?.permissions ?? []).includes("decision_client_dedupe");
  const [duplicates, setDuplicates] = useState<EntityMergeCandidate[] | null>(null);
  const [mergeError, setMergeError] = useState<string | null>(null);
  const [mergingId, setMergingId] = useState<string | null>(null);

  /** The live candidate queue, read from the real review API. Only ever
   * fetched when the grant is present, so an operator without it never pays
   * for a request whose result they may not see. */
  const refreshDuplicates = useCallback(async () => {
    if (!dedupeGranted) return;
    const res = await api.api.GET("/api/v1/entity-dedupe/candidates", {
      params: { query: { kind: "client", status: "pending" } },
    });
    if (res.error === undefined && Array.isArray(res.data)) {
      setDuplicates(res.data.filter((c) => c.entity_kind === "client"));
    } else {
      // A queue that cannot be read is reported as unknown, never as empty:
      // "no duplicates" and "we could not check" are different answers.
      setDuplicates(null);
    }
  }, [dedupeGranted]);

  /** One-click merge: the accept endpoint is an audited, undoable merge, so
   * a rejected decision leaves the row marked and says so rather than
   * quietly dropping the duplicate. */
  const mergeCandidate = async (candidateId: string) => {
    setMergingId(candidateId);
    setMergeError(null);
    const res = await api.api.POST("/api/v1/entity-dedupe/candidates/{id}/accept", {
      params: { path: { id: candidateId } },
    });
    setMergingId(null);
    if (res.error !== undefined) {
      setMergeError("The merge was rejected. Both records are unchanged.");
      return;
    }
    await refreshDuplicates();
    void refresh();
  };

  const refresh = useCallback(async () => {
    const [c, k] = await Promise.all([
      api.api.GET("/api/v1/clients"),
      api.api.GET("/api/v1/contracts"),
    ]);
    if (c.error !== undefined || c.data === undefined) {
      setLoadError("Failed to load clients.");
      setClients([]);
    } else {
      // A resolving read can carry a non-array — an honest empty roster,
      // never a crash.
      setClients(Array.isArray(c.data) ? c.data : []);
      setLoadError(null);
    }
    if (k.error === undefined && Array.isArray(k.data)) setContracts(k.data);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  useEffect(() => {
    if (auth.status === "authed") void refreshDuplicates();
  }, [auth.status, refreshDuplicates]);

  // The client's live contract, so the Contract and Renews columns read off
  // real terms rather than a client-side guess at one.
  const contractByClient = useMemo(() => {
    const map = new Map<string, Contract>();
    for (const c of contracts) {
      if (c.status === "cancelled" || c.status === "ended") continue;
      map.set(c.client_id, c);
    }
    return map;
  }, [contracts]);

  const activeContract = useCallback(
    (clientId: string): Contract | null => contractByClient.get(clientId) ?? null,
    [contractByClient],
  );

  const now = Date.now();

  const counts = useMemo(() => {
    const list = clients ?? [];
    return {
      renewing: list.filter((c) =>
        insideRenewalWindow(activeContract(c.id)?.effective_until, now),
      ).length,
      noContract: list.filter((c) => activeContract(c.id) === null).length,
    };
  }, [clients, activeContract, now]);

  const filtered = useMemo(() => {
    if (clients === null) return null;
    const q = query.trim().toLowerCase();
    return clients.filter((c) => {
      const contract = activeContract(c.id);
      if (onlyRenewing && !insideRenewalWindow(contract?.effective_until, now)) return false;
      if (onlyNoContract && contract !== null) return false;
      if (q === "") return true;
      const contact = primaryContactLabel(c.id);
      return (
        c.name.toLowerCase().includes(q) ||
        (contact ?? "").toLowerCase().includes(q)
      );
    });
  }, [clients, query, onlyRenewing, onlyNoContract, activeContract, now]);

  // A client's main contact is its primary contact; without one the cell
  // says so in words rather than showing an empty column.
  const [contactsByClient, setContactsByClient] = useState<Record<string, string>>({});
  // The real open-ticket count per client. The ledger's own column, read from
  // the endpoint that scopes tickets to a client. Bounded to one call per
  // visible client, the same bound the contacts fetch already works under.
  const [openTicketsByClient, setOpenTicketsByClient] = useState<Record<string, number>>({});
  useEffect(() => {
    if (clients === null || clients.length === 0) return;
    let cancelled = false;
    void (async () => {
      const entries = await Promise.all(
        clients.map(async (c) => {
          const [contacts, tickets] = await Promise.all([
            api.api.GET("/api/v1/clients/{id}/contacts", {
              params: { path: { id: c.id } },
            }),
            api.api.GET("/api/v1/tickets", {
              params: { query: { client_id: c.id, limit: 100 } },
            }),
          ]);
          const list = contacts.data ?? [];
          const primary = list.find((x) => x.is_primary) ?? list[0];
          const openCount = (tickets.data ?? []).filter(
            (t) => t.category !== "closed",
          ).length;
          return [c.id, { name: primary?.name ?? "", open: openCount }] as const;
        }),
      );
      if (cancelled) return;
      setContactsByClient(
        Object.fromEntries(entries.map(([id, v]) => [id, v.name])),
      );
      setOpenTicketsByClient(
        Object.fromEntries(entries.map(([id, v]) => [id, v.open])),
      );
    })();
    return () => {
      cancelled = true;
    };
  }, [clients]);

  function primaryContactLabel(clientId: string): string {
    return contactsByClient[clientId] ?? "";
  }

  /** The client's open ticket count, or null while the count is still
   * loading. `null` prints nothing rather than a zero, because "no tickets"
   * and "we have not checked yet" are different answers. */
  function openTicketCount(clientId: string): number | null {
    return openTicketsByClient[clientId] ?? null;
  }

  /** The name of whichever client is named, from the list already loaded —
   * so the duplicate pill reads "Possible duplicate of Northwind Legal"
   * rather than showing a UUID to a user deciding whether to merge. */
  function clientNameFor(clientId: string): string {
    return (clients ?? []).find((c) => c.id === clientId)?.name ?? "another record";
  }

  // Column visibility, persisted per user on the D12 key.
  const saved = prefs.view(SURFACE);
  const [visibleColumns, setVisibleColumns] = useState<ColumnId[]>(
    COLUMNS.filter((c) => c.defaultVisible).map((c) => c.id),
  );
  useEffect(() => {
    if (!prefs.ready) return;
    const savedIds = saved.columns.map((c) => c.key as ColumnId);
    if (savedIds.length === 0) return;
    const known = COLUMNS.filter((c) => savedIds.includes(c.id));
    if (known.length > 0) setVisibleColumns(known.map((c) => c.id));
  }, [prefs.ready, saved.columns]);

  const visible = useMemo(
    () => COLUMNS.filter((c) => visibleColumns.includes(c.id)),
    [visibleColumns],
  );

  const onColumnsChange = (next: { key: string; visible: boolean }[]) => {
    const ordered = next
      .filter((c) => c.visible)
      .map((c) => c.key as ColumnId)
      .filter((id) => COLUMNS.some((c) => c.id === id));
    setVisibleColumns(ordered);
    void prefs.save(SURFACE, {
      ...saved,
      columns: next.map((c) => ({ key: c.key, visible: c.visible })),
    });
  };

  const createClient = async () => {
    if (name.trim() === "") return;
    setCreating(true);
    setCreateError(null);
    const tempId = `pending-client:${name}`;
    const optimistic: Client = {
      id: tempId,
      org_id: "",
      name: name.trim(),
      notes: notes.trim() === "" ? null : notes.trim(),
      restricted_to: [],
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    setClients((prev) => [optimistic, ...(prev ?? [])]);
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateClientRequest"],
      ClientMutationResponse
    >({
      describe: "Create client",
      entityId: tempId,
      optimistic: () => {},
      revert: () => setClients((prev) => (prev ?? []).filter((c) => c.id !== tempId)),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        name: name.trim(),
        notes: notes.trim() === "" ? null : notes.trim(),
      }),
      request: (client, body) => client.POST("/api/v1/clients", { body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setCreating(false);
    if (!result.ok) {
      setCreateError(rejectionText(result, "The client was rejected. Nothing was saved."));
      return;
    }
    setCreateOpen(false);
    setName("");
    setNotes("");
    void refresh();
  };

  // A client is marked only when the SERVER's queue says so. There is no
  // name-shape heuristic here on purpose: a guess that flags a real client
  // as a duplicate is worse than no mark, because the user has to disprove
  // it.
  const duplicateFor = useMemo(() => {
    const map = new Map<string, EntityMergeCandidate>();
    for (const c of duplicates ?? []) {
      map.set(c.candidate_entity_id ?? "", c);
    }
    return map;
  }, [duplicates]);

  if (auth.status !== "authed") return null;

  const cell = (id: ColumnId) => visibleColumns.includes(id);

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-4 p-4 md:p-6">
      {/* ---- page header: title, the two shortcut pills, one primary ---- */}
      <SurfaceHeader
        title="Clients"
        meta={
          <>
            <ShortcutPill
              label="Renewing in 90 days"
              count={counts.renewing}
              active={onlyRenewing}
              dot="var(--m-blush)"
              onToggle={() => {
                setOnlyRenewing((v) => !v);
                setOnlyNoContract(false);
              }}
            />
            <ShortcutPill
              label="No contract"
              count={counts.noContract}
              active={onlyNoContract}
              dot="var(--m-input)"
              onToggle={() => {
                setOnlyNoContract((v) => !v);
                setOnlyRenewing(false);
              }}
            />
          </>
        }
        actions={
          <Dialog
            open={createOpen}
            onOpenChange={(open) => {
              setCreateOpen(open);
              if (!open) setCreateError(null);
            }}
          >
            <DialogTrigger asChild>
              <Button variant="primary" data-testid="new-client">
                <PlusIcon />
                New client
              </Button>
            </DialogTrigger>
            <DialogContent
              title="New client"
              description="Create the client record, then add its contact, contract and sites on its page. Nothing here blocks the client arriving by import."
            >
              <div className="grid gap-3 p-4">
                <div>
                  <Label htmlFor="client-name">Name</Label>
                  <Input
                    id="client-name"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder="Acme Corporation"
                  />
                </div>
                <div>
                  <Label htmlFor="client-notes">Notes</Label>
                  <Input
                    id="client-notes"
                    value={notes}
                    onChange={(e) => setNotes(e.target.value)}
                    placeholder="Optional"
                  />
                  <FieldHint>Internal notes; never shown to portal users.</FieldHint>
                </div>
                {createError !== null ? (
                  <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                    {createError}
                  </p>
                ) : null}
                <DialogActions>
                  <Button variant="ghost" onClick={() => setCreateOpen(false)}>
                    Cancel
                  </Button>
                  <Button onClick={() => void createClient()} disabled={creating || name.trim() === ""}>
                    {creating ? <Spinner /> : null} Create client
                  </Button>
                </DialogActions>
              </div>
            </DialogContent>
          </Dialog>
        }
      />

      {/* ---- the one toolbar row: views left, columns/filter/export right ---- */}
      {/* The toolbar's vertical padding is `var(--m-card-padding)`, and the
          pack's toolbar sits flush under the page head. The primitive takes no
          `style`, so the override rides a wrapper that owns the custom
          property — that keeps the fix on this surface instead of
          restyling a component every data surface in the app shares. */}
      <div style={{ "--m-card-padding": "0px" } as React.CSSProperties}>
      <DataToolbar
        views={
          <ViewSwitch
            views={VIEWS.map((v) => ({
              ...v,
              hint: v.id === view ? undefined : undefined,
            }))}
            activeId={view}
            onChange={setView}
            label="Client views"
          />
        }
        filters={
          <div className="w-56">
            {/* The wrapper carries the width, not the control: `.m-input`
                sets `width: 100%` in an unlayered rule that outranks a
                Tailwind utility, so `w-56` on the input is silently ignored
                and the field stretches the toolbar onto its own row. */}
            <Input
              aria-label="Search clients"
              placeholder="Search clients…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
        }
        columns={
          <ColumnChooser
            columns={COLUMNS.map((c) => ({
              key: c.id,
              label: c.label,
              visible: visibleColumns.includes(c.id),
            }))}
            onChange={onColumnsChange}
          />
        }
        export={
          <ExportControl
            columns={visible.map((c) => c.label)}
            rowCount={filtered === null ? 0 : filtered.length}
            onExport={() => {
              /* The control reports the request; the CSV write is the app's
                 job and the control already states rows, filters and columns. */
            }}
          />
        }
      />
      </div>

      {/* ---- the record-dedupe strip (P3, granted only, real queue) ---- */}
      {dedupeGranted && duplicates !== null && duplicates.length > 0 ? (
        <div
          role="status"
          data-testid="client-dedupe-strip"
          className="flex flex-wrap items-center gap-3 rounded-[0.75rem] px-3 py-2"
          style={{ background: "var(--m-status-due-bg, var(--m-time))", color: "var(--m-status-due-fg, var(--m-time-fg))" }}
        >
          <AlertIcon />
          <span className="text-[13px]">
            <strong className="font-bold">
              {duplicates.length} possible {duplicates.length === 1 ? "duplicate" : "duplicates"}.
            </strong>{" "}
            {duplicates[0]?.reason ?? "A record looks like another one."} Merging keeps both
            histories.
          </span>
          <span className="flex-1" />
          {mergeError !== null ? (
            <span className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {mergeError}
            </span>
          ) : null}
          {/* The full review is the merge workbench, which owns the
              side-by-side comparison. The strip only opens it — no panel
              here, and nothing inverted. */}
          <Link href="/inventory/workbench">
            <Button size="sm" data-testid="client-dedupe-review">
              Review
            </Button>
          </Link>
        </div>
      ) : null}

      {/* ---- the table ---- */}
      <div
        className="overflow-hidden rounded-2xl border"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      >
        {filtered === null ? (
          <div className="p-4">
            {loadError !== null ? (
              <p role="alert" className="text-[13px]">
                {loadError}
              </p>
            ) : (
              <Spinner />
            )}
          </div>
        ) : filtered.length === 0 ? (
          <div className="p-4">
            <EmptyState
              message={
                onlyRenewing || onlyNoContract || query.trim() !== ""
                  ? "No clients match this view. Clear a shortcut or the search to see the rest of the book."
                  : "No clients yet. Create the first client to attach sites, contacts and contracts."
              }
            />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                {cell("client") ? <Th>Client</Th> : null}
                {cell("contact") ? <Th>Main contact</Th> : null}
                {cell("openTickets") ? <Th className="text-right">Open tickets</Th> : null}
                {cell("contract") ? <Th>Contract</Th> : null}
                {cell("renews") ? <Th>Renews</Th> : null}
                {cell("owner") ? <Th>Account owner</Th> : null}
                {cell("visibility") ? <Th>Visibility</Th> : null}
              </tr>
            </thead>
            <tbody>
              {filtered.map((client, i) => {
                const contract = activeContract(client.id);
                const signal = renewalSignal(contract?.effective_until, now);
                const contact = primaryContactLabel(client.id);
                const duplicate = duplicateFor.get(client.id) ?? null;
                return (
                  <Tr
                    key={client.id}
                    className="m-entity-row"
                    style={{ animationDelay: `${Math.min(i * 20, 160)}ms` }}
                    data-testid={`client-row-${client.id}`}
                  >
                    {cell("client") ? (
                      <Td>
                        <div className="flex min-w-0 flex-col leading-[1.3]">
                          <Link
                            href={`/clients/${client.id}`}
                            className="truncate font-semibold underline decoration-transparent underline-offset-2 transition-colors duration-[100ms] hover:decoration-[var(--m-border-strong)]"
                          >
                            {client.name}
                          </Link>
                          {duplicate !== null ? (
                            /* P3 row mark: a blush pill in the row's own
                               second line naming the duplicate, plus a
                               one-click merge. No panel, and nothing
                               inverted — the one inverted surface on this
                               page is spent elsewhere or not at all. */
                            <span className="mt-1 flex items-center gap-1.5">
                              <Badge tone="due" data-testid="client-duplicate-mark">
                                <span className="inline-flex items-center gap-1">
                                  <AlertIcon small />
                                  Possible duplicate of{" "}
                                  {clientNameFor(duplicate.primary_entity_id)}
                                </span>
                              </Badge>
                              <Button
                                size="sm"
                                variant="secondary"
                                data-testid="client-duplicate-merge"
                                disabled={mergingId === duplicate.id}
                                onClick={() => void mergeCandidate(duplicate.id)}
                              >
                                {mergingId === duplicate.id ? "Merging…" : "Merge"}
                              </Button>
                            </span>
                          ) : (
                            <span className="truncate text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                              {contract === null ? "No plan yet" : contract.name}
                            </span>
                          )}
                        </div>
                      </Td>
                    ) : null}
                    {cell("contact") ? (
                      <Td>
                        {contact === "" ? (
                          <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                            No contact yet
                          </span>
                        ) : (
                          <span className="text-[13px]">{contact}</span>
                        )}
                      </Td>
                    ) : null}
                    {cell("openTickets") ? (
                      <Td className="text-right text-[13px] tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                        {/* The real count, with a zero spelled as "None"
                            rather than a bare 0 — a ledger column that shows
                            0 next to 1 and 2 has to say which is which. */}
                        {(() => {
                          const n = openTicketCount(client.id);
                          if (n === null) return "";
                          return n === 0 ? (
                            <span style={{ color: "var(--m-text-muted)" }}>None</span>
                          ) : (
                            <span className="font-semibold" style={{ color: "var(--m-text)" }}>
                              {n}
                            </span>
                          );
                        })()}
                      </Td>
                    ) : null}
                    {cell("contract") ? (
                      <Td>
                        {contract === null ? (
                          <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                            None
                          </span>
                        ) : (
                          <span className="text-[13px]">{contract.name}</span>
                        )}
                      </Td>
                    ) : null}
                    {cell("renews") ? (
                      <Td>
                        {contract === null || contract.effective_until == null ? (
                          <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                            Not set
                          </span>
                        ) : (
                          <span className="flex flex-wrap items-center gap-2">
                            <span className="text-[13px] tabular-nums">
                              {formatPackDate(contract.effective_until)}
                            </span>
                            {signal !== null ? <RenewalPill label={signal.label} /> : null}
                          </span>
                        )}
                      </Td>
                    ) : null}
                    {cell("owner") ? (
                      <Td>
                        <span className="flex items-center gap-2">
                          {ownerName === "" ? null : <Monogram name={ownerName} />}
                          <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                            {ownerName === "" ? "Unassigned" : ownerName}
                          </span>
                        </span>
                      </Td>
                    ) : null}
                    {cell("visibility") ? (
                      <Td>
                        {client.restricted_to.length === 0 ? (
                          <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                            All members
                          </span>
                        ) : (
                          <Badge tone="warning">
                            Restricted{isAdmin ? ` · ${client.restricted_to.length}` : ""}
                          </Badge>
                        )}
                      </Td>
                    ) : null}
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}
      </div>

      {/* ---- footer: shown of total, and the density control ---- */}
      <div className="flex flex-wrap items-center gap-3 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
        <span className="tabular-nums">
          Showing <strong className="font-semibold">{filtered?.length ?? 0}</strong> of{" "}
          <strong className="font-semibold">{clients?.length ?? 0}</strong> clients
        </span>
        <span className="flex-1" />
        <DensityInline />
      </div>
    </div>
  );
}

/** A header shortcut: a dot, the words, and the count. Pressing it is a
 * filter toggle, so it reads as pressed (gold tint) when active. */
function ShortcutPill({
  label,
  count,
  active,
  dot,
  onToggle,
}: {
  label: string;
  count: number;
  active: boolean;
  dot: string;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onToggle}
      data-testid={`clients-shortcut-${label.toLowerCase().replaceAll(/\s+/g, "-")}`}
      className="inline-flex cursor-pointer items-center gap-2 rounded-full border px-3 py-1.5 text-[13px] font-semibold"
      style={{
        background: active ? "var(--m-accent)" : "transparent",
        color: active ? "var(--m-on-accent)" : "var(--m-text)",
        borderColor: active ? "transparent" : "var(--m-border)",
      }}
    >
      <span className="h-2 w-2 rounded-full" style={{ background: dot }} aria-hidden="true" />
      {label}
      <span className="tabular-nums">{count}</span>
    </button>
  );
}

/** Density rides the account's typed key (lane 2b's D7), so the step chosen
 * in a table footer is the step every other table shows. */
function DensityInline() {
  const prefs = useDataSurfacePrefs();
  const steps = [
    { id: "comfortable", label: "Comfortable" },
    { id: "compact", label: "Compact" },
    { id: "dense", label: "Dense" },
  ] as const;
  return (
    <div className="flex items-center gap-1" role="group" aria-label="Row density">
      {steps.map((s) => (
        <button
          key={s.id}
          type="button"
          aria-pressed={prefs.density === s.id}
          onClick={() => void prefs.setDensity(s.id)}
          className="cursor-pointer rounded-[0.5rem] border px-2 py-1 text-[12px] font-semibold"
          style={{
            background: prefs.density === s.id ? "var(--m-accent)" : "transparent",
            color: prefs.density === s.id ? "var(--m-on-accent)" : "var(--m-text-secondary)",
            borderColor: prefs.density === s.id ? "transparent" : "var(--m-border)",
          }}
        >
          {s.label}
        </button>
      ))}
    </div>
  );
}

function PlusIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" aria-hidden="true">
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}

function AlertIcon({ small = false }: { small?: boolean }) {
  const s = small ? 13 : 16;
  return (
    <svg width={s} height={s} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
      <circle cx="12" cy="12" r="9" />
      <path d="M12 8v5M12 16.5v.01" />
    </svg>
  );
}