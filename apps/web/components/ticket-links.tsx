// TicketLinks — the ticket-detail Links tab (Wave 11 lane 10 close-out of
// the lane 9 backend, migration 0056).
//
// Purpose: typed ticket-to-ticket edges + the rollup strip + the merge
// fold. The panel renders GET /tickets/{id}/links ({from,to,rollup}) with
// resolved ticket numbers/titles, a create-link dialog (parent_child vs
// major_incident, target picked through unified search), and the merge
// flow: pick a survivor, resolve every differing field explicitly
// (source|survivor per title/priority/client/assignee; board mismatch
// resolves with the survivor moving onto the source's board), then
// confirm. Typed merge rejections — self_merge / already_merged /
// closed_survivor / board_mismatch (the key prefixes the server message) —
// render inline with the fold summary shown on success. Edits ride the
// instant-first envelope; viewing needs only tickets:view, editing
// tickets:edit.
// Non-purpose: no tag chips here (TicketTags) and no money lines
// (TicketMoney).
"use client";

import { rejectionText } from "../lib/error-copy";
import type { components } from "@meridian/sdk";
import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  Input,
  Label,
  Spinner,
  StatusTag,
  Table,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";

import { api } from "../lib/api";
import { runEnvelopeMutation } from "../lib/envelope-write";
import { money } from "../lib/format";

/** The link edge is NOT a registered openapi component (LinksView carries
 * it as value_type=Vec<Object>), so the runtime JSON shape is declared
 * here from the core-domain entity — a rename on the wire fails the
 * panel's rendering, not silently. */
interface TicketLink {
  id: string;
  org_id: string;
  from_ticket_id: string;
  to_ticket_id: string;
  link_type: string;
  created_by: string | null;
  created_at: string;
}
type LinksViewShape = { from: TicketLink[]; to: TicketLink[]; rollup: components["schemas"]["TicketRollup"] };
type TicketRollup = components["schemas"]["TicketRollup"];
type SearchResult = components["schemas"]["SearchResult"];
type Ticket = components["schemas"]["Ticket"];
type MergeConflictEntry = components["schemas"]["MergeConflictEntry"];
type MergeFoldSummary = components["schemas"]["MergeFoldSummary"];
type FieldResolution = components["schemas"]["FieldResolution"];

/** The fields the merge fold can resolve per-side, in display order. */
const RESOLVABLE_FIELDS = ["title", "priority", "client", "assignee"] as const;
type ResolvableField = (typeof RESOLVABLE_FIELDS)[number];

const FIELD_LABEL: Record<ResolvableField, string> = {
  title: "Title",
  priority: "Priority",
  client: "Client",
  assignee: "Assignee",
};

function mergeKeyOf(message: string | undefined): string | null {
  const m = message ?? "";
  for (const key of ["self_merge", "already_merged", "closed_survivor", "board_mismatch"]) {
    if (m.startsWith(`${key}:`) || m.startsWith(`${key} `)) return key;
  }
  return null;
}

const MERGE_COPY: Record<string, string> = {
  self_merge: "A ticket cannot be merged into itself.",
  already_merged: "One of these tickets has already been merged — merged tickets cannot merge again.",
  closed_survivor: "The survivor ticket is closed. Pick an open ticket to survive the merge.",
  board_mismatch:
    "These tickets live on different boards. Choose to move the survivor onto this ticket's board to proceed.",
};

/** Search-driven ticket picker used by link + merge dialogs. */
function TicketPicker({
  excludeId,
  onPick,
  pickLabel,
}: {
  excludeId: string;
  onPick(ticket: { id: string; number: string; title: string }): void;
  pickLabel: string;
}) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[] | null>(null);
  const [searching, setSearching] = useState(false);

  useEffect(() => {
    const q = query.trim();
    if (q.length < 2) {
      setResults(null);
      return;
    }
    let cancelled = false;
    setSearching(true);
    const timer = window.setTimeout(() => {
      void api.api
        .GET("/api/v1/search", { params: { query: { q, limit: 8 } } })
        .then(({ data }) => {
          if (cancelled) return;
          setResults(
            (data?.results ?? []).filter(
              (r) => r.entity_kind === "ticket" && r.entity_id !== excludeId,
            ),
          );
          setSearching(false);
        })
        .catch(() => {
          if (!cancelled) setSearching(false);
        });
    }, 250);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query, excludeId]);

  return (
    <div className="grid gap-2">
      <Input
        aria-label={pickLabel}
        placeholder="Search by number or title…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />
      {searching ? <Spinner /> : null}
      {results !== null && results.length === 0 && query.trim().length >= 2 ? (
        <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
          No tickets match “{query.trim()}”.
        </p>
      ) : null}
      <ul className="grid gap-1">
        {(results ?? []).map((r) => (
          <li key={r.entity_id}>
            <button
              type="button"
              className="grid w-full cursor-pointer gap-0.5 rounded-[var(--m-radius-container)] border p-2 text-left text-[12px] transition-opacity duration-100 hover:opacity-80"
              style={{ borderColor: "var(--m-border)" }}
              onClick={() => onPick({ id: r.entity_id, number: r.title.split(" ")[0] ?? "", title: r.title })}
            >
              <span className="font-medium">{r.title}</span>
              <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                {r.snippet.slice(0, 80)}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

export function MergedIntoBanner({
  mergedIntoTicketId,
  linkedTitle,
}: {
  mergedIntoTicketId: string;
  linkedTitle: string | null;
}): React.ReactNode {
  const router = useRouter();
  return (
    <div
      className="flex items-center gap-2 border-b px-3 py-2 text-[12px]"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      data-testid="merged-into-banner"
      role="status"
    >
      <StatusTag tone="neutral">Merged</StatusTag>
      <span style={{ color: "var(--m-text-secondary)" }}>
        This ticket was merged into{" "}
        <button
          type="button"
          className="cursor-pointer border-none bg-transparent p-0 font-medium underline transition-opacity duration-100"
          style={{ color: "var(--m-text)" }}
          onClick={() => router.push(`/tickets/${mergedIntoTicketId}`)}
        >
          {linkedTitle ?? "its survivor"}
        </button>
        . It stays queryable read-only and cannot be re-opened.
      </span>
    </div>
  );
}

export function TicketLinksPanel({
  ticketId,
  ticket,
  canWork,
  namesByUser,
  onChanged,
}: {
  ticketId: string;
  ticket: Ticket;
  canWork: boolean;
  namesByUser: Map<string, string>;
  /** Parent refresh (the merged banner/status depends on the ticket). */
  onChanged?: () => void;
}) {
  const router = useRouter();
  const [links, setLinks] = useState<LinksViewShape | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [peerTitles, setPeerTitles] = useState<Map<string, { number: string; title: string; merged_into: string | null }>>(new Map());
  const [clientNames, setClientNames] = useState<Map<string, string>>(new Map());
  const [moneyCurrency, setMoneyCurrency] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/tickets/{id}/links", {
      params: { path: { id: ticketId } },
    });
    if (error !== undefined || data === undefined) {
      setLoadError("Failed to load links.");
      return;
    }
    setLoadError(null);
    // LinksView's edge arrays are untyped in the spec (value_type Object);
    // re-type them into the declared runtime shape.
    setLinks({
      from: (data.from ?? []) as unknown as TicketLink[],
      to: (data.to ?? []) as unknown as TicketLink[],
      rollup: data.rollup,
    });
  }, [ticketId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Resolve peer ticket numbers/titles + client names lazily.
  useEffect(() => {
    if (links === null) return;
    const peerIds = new Set<string>();
    for (const edge of [...links.from, ...links.to]) {
      peerIds.add(edge.from_ticket_id === ticketId ? edge.to_ticket_id : edge.from_ticket_id);
    }
    void (async () => {
      const next = new Map(peerTitles);
      for (const id of peerIds) {
        if (next.has(id)) continue;
        const { data } = await api.api.GET("/api/v1/tickets/{id}", { params: { path: { id } } });
        if (data !== undefined && data !== null) {
          next.set(id, { number: data.number, title: data.title, merged_into: data.merged_into_ticket_id ?? null });
        }
      }
      setPeerTitles(next);
    })();
  }, [links, ticketId]);

  useEffect(() => {
    if (ticket.client_id == null) return;
    if (clientNames.has(ticket.client_id)) return;
    void api.api
      .GET("/api/v1/clients/{id}", { params: { path: { id: ticket.client_id } } })
      .then(({ data }) => {
        if (data !== undefined && data !== null) {
          setClientNames((prev) => new Map(prev).set(ticket.client_id as string, data.name));
        }
      })
      .catch(() => undefined);
  }, [ticket.client_id, clientNames]);

  // The rollup sums billable minor units across lines that each carry
  // their own ISO code; display currency is derived from the ticket's
  // first cost line / expense — never assumed.
  useEffect(() => {
    if (links === null || links.rollup.billable_cost_minor <= 0 || moneyCurrency !== null) return;
    void (async () => {
      const [cl, ex] = await Promise.all([
        api.api.GET("/api/v1/tickets/{id}/cost-lines", { params: { path: { id: ticketId } } }),
        api.api.GET("/api/v1/tickets/{id}/expenses", { params: { path: { id: ticketId } } }),
      ]);
      const currency =
        (cl.data !== undefined ? cl.data[0]?.currency : undefined) ??
        (ex.data !== undefined ? ex.data[0]?.currency : undefined);
      if (currency !== undefined) setMoneyCurrency(currency);
    })();
  }, [links, ticketId, moneyCurrency]);

  // --- create-link dialog ------------------------------------------------------
  const [createOpen, setCreateOpen] = useState(false);
  const [linkType, setLinkType] = useState<"parent_child" | "major_incident">("parent_child");
  const [linkTarget, setLinkTarget] = useState<{ id: string; title: string } | null>(null);
  const [createBusy, setCreateBusy] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const createLink = async () => {
    if (linkTarget === null) return;
    setCreateBusy(true);
    setCreateError(null);
    const before = links;
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateTicketLinkRequest"],
      unknown
    >({
      describe: "Create ticket link",
      entityId: ticketId,
      optimistic: () =>
        setLinks((prev) =>
          prev === null
            ? prev
            : {
                ...prev,
                from: [
                  ...prev.from,
                  {
                    id: `pending-${linkTarget.id}`,
                    org_id: ticket.org_id,
                    from_ticket_id: ticketId,
                    to_ticket_id: linkTarget.id,
                    link_type: linkType,
                    created_by: null,
                    created_at: new Date().toISOString(),
                  } as TicketLink,
                ],
              },
        ),
      revert: () => setLinks(before),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        to_ticket_id: linkTarget.id,
        link_type: linkType,
      }),
      request: (client, body) =>
        client.POST("/api/v1/tickets/{id}/links", {
          params: { path: { id: ticketId } },
          body,
        }) as Promise<{ data?: unknown; error?: unknown; response: Response }>,
    });
    setCreateBusy(false);
    if (!result.ok) {
      setCreateError(
        result.message?.includes("cannot link to itself")
          ? "A ticket cannot link to itself."
          : (rejectionText(result, "The link was rejected.")),
      );
      return;
    }
    setCreateOpen(false);
    setLinkTarget(null);
    void refresh();
  };

  const removeLink = async (edge: TicketLink) => {
    const before = links;
    const result = await runEnvelopeMutation<
      components["schemas"]["MutationParams"],
      unknown
    >({
      describe: "Remove ticket link",
      entityId: ticketId,
      optimistic: () =>
        setLinks((prev) =>
          prev === null
            ? prev
            : {
                ...prev,
                from: prev.from.filter((e) => e.id !== edge.id),
                to: prev.to.filter((e) => e.id !== edge.id),
              },
        ),
      revert: () => setLinks(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key }),
      request: (client, body) =>
        client.DELETE("/api/v1/tickets/{id}/links/{link_id}", {
          params: { path: { id: ticketId, link_id: edge.id } },
        }) as Promise<{ data?: unknown; error?: unknown; response: Response }>,
    });
    if (!result.ok) {
      setLoadError(rejectionText(result, "The link removal was rejected."));
      return;
    }
    void refresh();
  };

  // --- merge flow ----------------------------------------------------------------
  const [mergeOpen, setMergeOpen] = useState(false);
  const [survivor, setSurvivor] = useState<Ticket | null>(null);
  const [resolutions, setResolutions] = useState<Partial<Record<ResolvableField | "board", FieldResolution>>>({});
  const [mergeBusy, setMergeBusy] = useState(false);
  const [mergeError, setMergeError] = useState<string | null>(null);
  const [fold, setFold] = useState<{ summary: MergeFoldSummary; conflicts: MergeConflictEntry[]; survivorId: string } | null>(null);

  /** Differing resolvable fields between this ticket and the survivor. */
  const differing = useMemo(() => {
    if (survivor === null) return [];
    const out: ResolvableField[] = [];
    if (ticket.title !== survivor.title) out.push("title");
    if (ticket.priority !== survivor.priority) out.push("priority");
    if (ticket.client_id !== survivor.client_id) out.push("client");
    if (ticket.assignee_id !== survivor.assignee_id) out.push("assignee");
    return out;
  }, [ticket, survivor]);

  const boardMismatch = survivor !== null && survivor.board_id !== ticket.board_id;

  const pickSurvivor = (picked: { id: string }) => {
    setSurvivor(null);
    setResolutions({});
    setMergeError(null);
    void api.api
      .GET("/api/v1/tickets/{id}", { params: { path: { id: picked.id } } })
      .then(({ data }) => {
        if (data !== undefined && data !== null) setSurvivor(data);
        else setMergeError("That ticket could not be loaded in your scope.");
      })
      .catch(() => setMergeError("That ticket could not be loaded in your scope."));
  };

  const runMerge = async () => {
    if (survivor === null) return;
    setMergeBusy(true);
    setMergeError(null);
    const resolve: components["schemas"]["MergeResolutions"] = {};
    if (boardMismatch) resolve.board = "source";
    for (const field of differing) {
      resolve[field] = resolutions[field] ?? "survivor";
    }
    const { data, error } = await api.api.POST("/api/v1/tickets/{id}/merge", {
      params: { path: { id: ticketId } },
      body: {
        idempotency_key: `merge-${ticketId}-${survivor.id}`,
        survivor_id: survivor.id,
        resolve,
      },
    });
    setMergeBusy(false);
    if (error !== undefined || data === undefined) {
      const body = error as { error?: { message?: string } } | undefined;
      const key = mergeKeyOf(body?.error?.message);
      setMergeError(key !== null ? (MERGE_COPY[key] ?? body?.error?.message ?? null) : (body?.error?.message ?? "The merge was rejected."));
      return;
    }
    setFold({ summary: data.fold, conflicts: data.conflicts, survivorId: data.survivor_id });
    onChanged?.();
  };

  const valueLabel = (field: ResolvableField, t: Ticket | null): string => {
    if (t === null) return "—";
    switch (field) {
      case "title":
        return t.title;
      case "priority":
        return t.priority;
      case "client":
        return t.client_id == null ? "none" : (clientNames.get(t.client_id) ?? t.client_id.slice(0, 8));
      case "assignee":
        return t.assignee_id == null ? "Unassigned" : (namesByUser.get(t.assignee_id) ?? t.assignee_id.slice(0, 8));
    }
  };

  const rollup: TicketRollup | null = links?.rollup ?? null;

  if (loadError !== null && links === null) {
    return <p role="alert" className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>{loadError}</p>;
  }

  return (
    <div className="grid content-start gap-3" data-testid="links-panel">
      {/* Rollup strip */}
      <div className="flex flex-wrap gap-2" data-testid="rollup-strip">
        {rollup === null ? (
          <Spinner />
        ) : (
          <>
            <span className="rounded-[var(--m-radius-container)] border px-2 py-1 text-[11px]" style={{ borderColor: "var(--m-border)" }}>
              Children: <strong>{rollup.child_count}</strong>
              {rollup.child_open_count > 0 ? <span style={{ color: "var(--m-text-muted)" }}> ({rollup.child_open_count} open)</span> : null}
            </span>
            <span className="rounded-[var(--m-radius-container)] border px-2 py-1 text-[11px]" style={{ borderColor: "var(--m-border)" }}>
              Major-incident members: <strong>{rollup.major_incident_member_count}</strong>
            </span>
            <span className="rounded-[var(--m-radius-container)] border px-2 py-1 text-[11px]" style={{ borderColor: "var(--m-border)" }}>
              Folded time: <strong>{(rollup.time_minutes / 60).toFixed(1)} h</strong>
            </span>
            <span className="rounded-[var(--m-radius-container)] border px-2 py-1 text-[11px]" style={{ borderColor: "var(--m-border)" }}>
              Billable money:{" "}
              <strong
                title={moneyCurrency === null ? "Sum across lines of varying currency — open the Money tab for per-line detail" : undefined}
              >
                {moneyCurrency !== null ? money(rollup.billable_cost_minor, moneyCurrency) : `≈ ${(rollup.billable_cost_minor / 100).toLocaleString()} (mixed currency)`}
              </strong>
            </span>
          </>
        )}
      </div>

      {/* Edges */}
      <Card>
        <div className="flex items-center justify-between p-4 pb-2">
          <h3 className="text-[12px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
            Links
          </h3>
          {canWork ? (
            <Button size="sm" variant="secondary" onClick={() => { setCreateOpen(true); setCreateError(null); }} data-testid="create-link">
              Link ticket…
            </Button>
          ) : null}
        </div>
        {links !== null && links.from.length === 0 && links.to.length === 0 ? (
          <div className="p-4 pt-0">
            <EmptyState message="No links. Parent–child rolls child work up; major-incident folds members in." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Direction</Th>
                <Th>Type</Th>
                <Th>Ticket</Th>
                {canWork ? <Th className="text-right">Action</Th> : null}
              </tr>
            </thead>
            <tbody>
              {links?.from.map((edge) => {
                const otherId = edge.from_ticket_id === ticketId ? edge.to_ticket_id : edge.from_ticket_id;
                const peer = peerTitles.get(otherId);
                return (
                  <Tr key={edge.id}>
                    <Td className="text-[12px]">→ out</Td>
                    <Td><StatusTag tone={edge.link_type === "parent_child" ? "neutral" : "warning"}>{edge.link_type}</StatusTag></Td>
                    <Td className="text-[12px]">
                      <button
                        type="button"
                        className="cursor-pointer border-none bg-transparent p-0 text-left underline transition-opacity duration-100"
                        style={{ color: "var(--m-text)" }}
                        onClick={() => router.push(`/tickets/${otherId}`)}
                      >
                        {peer ? `${peer.number} ${peer.title}` : otherId.slice(0, 8)}
                      </button>
                    </Td>
                    {canWork ? (
                      <Td className="text-right">
                        <Button variant="ghost" size="sm" onClick={() => void removeLink(edge)}>
                          Unlink
                        </Button>
                      </Td>
                    ) : null}
                  </Tr>
                );
              })}
              {links?.to.map((edge) => {
                const otherId = edge.from_ticket_id === ticketId ? edge.to_ticket_id : edge.from_ticket_id;
                const peer = peerTitles.get(otherId);
                return (
                  <Tr key={edge.id}>
                    <Td className="text-[12px]">← in</Td>
                    <Td><StatusTag tone={edge.link_type === "parent_child" ? "neutral" : "warning"}>{edge.link_type}</StatusTag></Td>
                    <Td className="text-[12px]">
                      <button
                        type="button"
                        className="cursor-pointer border-none bg-transparent p-0 text-left underline transition-opacity duration-100"
                        style={{ color: "var(--m-text)" }}
                        onClick={() => router.push(`/tickets/${otherId}`)}
                      >
                        {peer ? `${peer.number} ${peer.title}` : otherId.slice(0, 8)}
                      </button>
                    </Td>
                    {canWork ? (
                      <Td className="text-right">
                        <Button variant="ghost" size="sm" onClick={() => void removeLink(edge)}>
                          Unlink
                        </Button>
                      </Td>
                    ) : null}
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}
      </Card>

      {/* Merge */}
      {canWork ? (
        <Card>
          <div className="flex items-center justify-between gap-3 p-4">
            <div>
              <h3 className="text-[12px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                Merge
              </h3>
              <p className="mt-1 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                Fold this ticket into a survivor: children, comments, time, costs and expenses move; this ticket closes read-only.
              </p>
            </div>
            <Button size="sm" variant="secondary" onClick={() => { setMergeOpen(true); setMergeError(null); setFold(null); }} data-testid="open-merge">
              Merge into…
            </Button>
          </div>
        </Card>
      ) : null}

      {/* Create-link dialog */}
      <Dialog open={createOpen} onOpenChange={(o) => { setCreateOpen(o); if (!o) { setLinkTarget(null); setCreateError(null); } }}>
        <DialogContent
          title="Link ticket"
          description="Parent–child rolls the child's work up to the parent. Major-incident marks this ticket as folded into the incident."
        >
          <div className="grid gap-3 p-4">
            <div>
              <Label>Link type</Label>
              <div className="flex gap-2">
                <Button variant={linkType === "parent_child" ? "primary" : "secondary"} size="sm" onClick={() => setLinkType("parent_child")}>
                  Parent–child
                </Button>
                <Button variant={linkType === "major_incident" ? "primary" : "secondary"} size="sm" onClick={() => setLinkType("major_incident")}>
                  Major incident
                </Button>
              </div>
            </div>
            <div>
              <Label>{linkType === "parent_child" ? "Parent ticket" : "Major incident"}</Label>
              {linkTarget !== null ? (
                <div className="flex items-center justify-between gap-2 rounded-[var(--m-radius-container)] border p-2 text-[12px]" style={{ borderColor: "var(--m-accent)" }}>
                  <span className="font-medium">{linkTarget.title}</span>
                  <Button variant="ghost" size="sm" onClick={() => setLinkTarget(null)}>Change</Button>
                </div>
              ) : (
                <TicketPicker
                  excludeId={ticketId}
                  pickLabel="Search for the ticket to link"
                  onPick={(t) => setLinkTarget({ id: t.id, title: t.title })}
                />
              )}
            </div>
            {createError !== null ? (
              <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>{createError}</p>
            ) : null}
            <DialogActions>
              <Button variant="ghost" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button variant="primary" onClick={() => void createLink()} disabled={createBusy || linkTarget === null}>
                {createBusy ? <Spinner /> : null} Create link
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      {/* Merge dialog */}
      <Dialog open={mergeOpen} onOpenChange={(o) => { setMergeOpen(o); if (!o) { setSurvivor(null); setResolutions({}); setMergeError(null); setFold(null); } }}>
        <DialogContent
          title="Merge this ticket into a survivor"
          description="The survivor keeps its identity; everything this ticket carries folds into it. Typed conflicts render inline."
        >
          <div className="grid gap-3 p-4">
            {fold !== null ? (
              <>
                <p className="text-[13px]" data-testid="merge-fold">
                  Merged. Fold summary: {fold.summary.children_relinked} children relinked ·{" "}
                  {fold.summary.comments_folded} comments · {fold.summary.time_entries_folded} time entries (
                  {fold.summary.time_minutes_folded} min) · {fold.summary.cost_lines_folded} cost lines ·{" "}
                  {fold.summary.expenses_folded} expenses.
                </p>
                {fold.conflicts.length > 0 ? (
                  <ul className="grid gap-1 text-[12px]" data-testid="merge-conflicts-applied">
                    {fold.conflicts.map((c) => (
                      <li key={c.field}>
                        <strong>{c.field}</strong> resolved to <strong>{c.resolution}</strong>
                      </li>
                    ))}
                  </ul>
                ) : null}
                <DialogActions>
                  <Button onClick={() => { setMergeOpen(false); setFold(null); router.push(`/tickets/${fold.survivorId}`); }}>
                    Go to survivor
                  </Button>
                </DialogActions>
              </>
            ) : (
              <>
                {survivor === null ? (
                  <TicketPicker excludeId={ticketId} pickLabel="Search for the survivor ticket" onPick={pickSurvivor} />
                ) : (
                  <>
                    <div className="rounded-[var(--m-radius-container)] border p-2 text-[12px]" style={{ borderColor: "var(--m-accent)" }} data-testid="merge-survivor">
                      Survivor: <strong>{survivor.number} {survivor.title}</strong>{" "}
                      <StatusTag tone={survivor.category === "closed" ? "danger" : "neutral"}>{survivor.category}</StatusTag>
                      <Button variant="ghost" size="sm" onClick={() => setSurvivor(null)}>Change</Button>
                    </div>
                    {boardMismatch ? (
                      <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-warning-fg)" }}>
                        {MERGE_COPY.board_mismatch ?? ""}
                      </p>
                    ) : null}
                    {differing.length > 0 ? (
                      <div className="grid gap-2" data-testid="merge-resolutions">
                        <Label>Resolve differing fields (this ticket vs survivor)</Label>
                        <Table>
                          <thead>
                            <tr>
                              <Th>Field</Th>
                              <Th>This ticket</Th>
                              <Th>Survivor</Th>
                              <Th>Winner</Th>
                            </tr>
                          </thead>
                          <tbody>
                            {differing.map((field) => (
                              <Tr key={field}>
                                <Td className="text-[12px]">{FIELD_LABEL[field]}</Td>
                                <Td className="max-w-40 truncate text-[12px]">{valueLabel(field, ticket)}</Td>
                                <Td className="max-w-40 truncate text-[12px]">{valueLabel(field, survivor)}</Td>
                                <Td>
                                  <div className="flex gap-1">
                                    <Button
                                      size="sm"
                                      variant={(resolutions[field] ?? "survivor") === "source" ? "primary" : "secondary"}
                                      onClick={() => setResolutions((r) => ({ ...r, [field]: "source" }))}
                                    >
                                      Source
                                    </Button>
                                    <Button
                                      size="sm"
                                      variant={(resolutions[field] ?? "survivor") === "survivor" ? "primary" : "secondary"}
                                      onClick={() => setResolutions((r) => ({ ...r, [field]: "survivor" }))}
                                    >
                                      Survivor
                                    </Button>
                                  </div>
                                </Td>
                              </Tr>
                            ))}
                          </tbody>
                        </Table>
                      </div>
                    ) : null}
                  </>
                )}
                {mergeError !== null ? (
                  <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }} data-testid="merge-error">
                    {mergeError}
                  </p>
                ) : null}
                <DialogActions>
                  <Button variant="ghost" onClick={() => setMergeOpen(false)}>Cancel</Button>
                  <Button variant="primary" onClick={() => void runMerge()} disabled={mergeBusy || survivor === null}>
                    {mergeBusy ? <Spinner /> : null} Confirm merge
                  </Button>
                </DialogActions>
              </>
            )}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
