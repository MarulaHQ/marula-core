// /boards/[id] — kanban board surface (Wave 17 lane 3).
//
// Purpose: the board's status pipeline as real kanban columns rendered
// from the IF4 board read model: hydrate once from the snapshot, then
// live through board-channel WS events as per-ticket deltas (IF5).
// Cards drag between columns; every drop rides the IF1/IF2 envelope
// (idempotency key + read-your-write version) with a synchronous
// optimistic patch and typed-rejection revert. Columns collapse, filters
// narrow the view, and column headers carry WIP/queue signals with the
// same status-tint tokens as the ticket status bar (light/dark parity).
// Non-purpose: no board administration (statuses/transitions/members are
// managed through the API's ConfigureBoard endpoints, not this page).
"use client";

import { rejectionText } from "../../../lib/error-copy";
import type { components } from "@meridian/sdk";
import {
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  StatusTag,
  Textarea,
} from "@meridian/ui";
import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { BoardNav } from "../../../components/board-nav";
import { api, useRequireAuth } from "../../../lib/api";
import {
  categoryTone,
  dueChip,
  EMPTY_FILTERS,
  filterTickets,
  groupByStatus,
  PRIORITIES,
  priorityTone,
  sortStatuses,
  transitionAllowed,
  type BoardFilters,
  type BoardStatus,
  type BoardTicket,
  type BoardTransitions,
} from "../../../lib/board-view";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import {
  boardModelKey,
  makeBoardSpec,
} from "../../../lib/instant/read-models";
import {
  useInstantChannels,
  useInstantCollection,
  useInstantWrite,
} from "../../../lib/use-instant";

type BoardMemberView = components["schemas"]["BoardMemberView"];
type Priority = components["schemas"]["Priority"];

/// New-ticket composer: the only staff create surface, wired to
/// POST /api/v1/tickets through the IF1/IF2 envelope (idempotency key,
/// read-your-write version). The board channel's `ticket.created` delta
/// lands the row in the list — no local insert needed.
function NewTicketDialog({
  boardId,
  canCreate,
}: {
  boardId: string;
  canCreate: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [priority, setPriority] = useState<Priority>("p3");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!canCreate) return null;

  const reset = () => {
    setTitle("");
    setPriority("p3");
    setBody("");
    setError(null);
  };

  const create = async () => {
    if (title.trim() === "") return;
    setBusy(true);
    setError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateTicketRequest"],
      components["schemas"]["TicketMutationResponse"]
    >({
      describe: "Create ticket",
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        board_id: boardId,
        title: title.trim(),
        priority,
        body: body.trim() === "" ? null : body,
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, requestBody) =>
        client.POST("/api/v1/tickets", { body: requestBody }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setError(rejectionText(result, "The ticket was rejected. Nothing was created."));
      return;
    }
    setOpen(false);
    reset();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) reset();
      }}
    >
      <DialogTrigger asChild>
        <Button size="sm">New ticket</Button>
      </DialogTrigger>
      <DialogContent
        title="New ticket"
        description="Opens on this board's entry state with SLA clocks running."
      >
        <div className="grid gap-3 p-4">
          <div>
            <Label htmlFor="new-ticket-title">Title</Label>
            <Input
              id="new-ticket-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              autoFocus
            />
          </div>
          <div>
            <Label htmlFor="new-ticket-priority">Priority</Label>
            <Select
              value={priority}
              onValueChange={(v) => setPriority(v as Priority)}
            >
              <SelectTrigger aria-label="Priority" className="w-32">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {PRIORITIES.map((p) => (
                  <SelectItem key={p} value={p}>
                    {p.toUpperCase()}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label htmlFor="new-ticket-body">Description</Label>
            <Textarea
              id="new-ticket-body"
              rows={4}
              value={body}
              onChange={(e) => setBody(e.target.value)}
            />
            <FieldHint>Optional. Visible to clients on the portal.</FieldHint>
          </div>
          {error !== null ? (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {error}
            </p>
          ) : null}
          <DialogActions>
            <Button variant="ghost" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button onClick={() => void create()} disabled={busy || title.trim() === ""}>
              {busy ? <Spinner /> : null} Create ticket
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}

const ROLES = ["viewer", "member", "admin"] as const;

/// Compact board-admin roster: existing members with inline role change.
/// Adding NEW members waits on an org user-directory endpoint (deferred;
/// see stage notes) — re-roling works fully today.
function MembersPanel({ boardId }: { boardId: string }) {
  const [members, setMembers] = useState<BoardMemberView[] | null>(null);
  const [hidden, setHidden] = useState(false);
  const [open, setOpen] = useState(false);

  const refresh = useCallback(async () => {
    const res = await api.api.GET("/api/v1/boards/{id}/members", {
      params: { path: { id: boardId } },
    });
    if (res.response.status === 403) {
      setHidden(true);
      return;
    }
    if (res.error !== undefined) return;
    setMembers(res.data);
  }, [boardId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function changeRole(userId: string, role: string) {
    await api.api.PUT("/api/v1/boards/{id}/members", {
      params: { path: { id: boardId } },
      body: { user_id: userId, role: role as (typeof ROLES)[number] },
    });
    void refresh();
  }

  if (hidden || members === null) {
    return null;
  }
  return (
    <section aria-label="Board members" className="flex flex-col gap-1">
      <button
        type="button"
        className="flex items-center gap-1 text-[10px] font-semibold uppercase tracking-wide"
        style={{ color: "var(--m-text-muted)" }}
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        Members ({members.length})
        <span aria-hidden>{open ? "▾" : "▸"}</span>
      </button>
      {open &&
        members.map((m) => (
          <div key={m.user_id} className="row-hover flex h-6 items-center gap-2 text-xs">
            <span className="truncate">{m.display_name}</span>
            <span className="ml-auto" />
            <select
              aria-label={`Role for ${m.display_name}`}
              className="rounded-[var(--m-radius-control)] border px-1 py-0 text-xs"
              style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
              value={m.role}
              onChange={(e) => void changeRole(m.user_id, e.target.value)}
            >
              {ROLES.map((r) => (
                <option key={r} value={r}>
                  {r}
                </option>
              ))}
            </select>
          </div>
        ))}
    </section>
  );
}

/** Chevron glyph for collapse triggers. */
function ChevronIcon(): ReactNode {
  return (
    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

interface KanbanCardProps {
  ticket: BoardTicket;
  statuses: BoardStatus[];
  workflow: BoardTransitions | null;
  compact: boolean;
  dragging: boolean;
  onDragStart(id: string): void;
  onDragEnd(): void;
  onMove(ticket: BoardTicket, toStatusId: string): void;
}

/// One kanban card: number, title, priority chip, category + due chips,
/// assigned marker. Drag handle is the whole card; the Move affordance is
/// keyboard-reachable and honors the same transition matrix as drops.
function KanbanCard({
  ticket,
  statuses,
  workflow,
  compact,
  dragging,
  onDragStart,
  onDragEnd,
  onMove,
}: KanbanCardProps): ReactNode {
  const router = useRouter();
  const [menuOpen, setMenuOpen] = useState(false);
  const tone = categoryTone(ticket.category);
  const due = dueChip(ticket.due_at, Date.now());
  const current = statuses.find((s) => s.id === ticket.status_id) ?? null;
  const targets = sortStatuses(statuses).filter(
    (s) =>
      ticket.status_id !== null &&
      transitionAllowed(statuses, workflow, ticket.status_id, s.id),
  );

  return (
    <div
      data-testid={`kanban-card-${ticket.number}`}
      className={`kanban-card relative rounded-[var(--m-radius-container)] border ${dragging ? "kanban-card-dragging" : ""}`}
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData("text/plain", ticket.id);
        e.dataTransfer.effectAllowed = "move";
        onDragStart(ticket.id);
      }}
      onDragEnd={onDragEnd}
    >
      <span
        aria-hidden
        className="absolute left-0 top-3 bottom-3 w-[3px] rounded-full"
        style={{
          background: `var(--m-status-${tone === "success" ? "success" : tone === "warning" ? "warning" : "neutral"}-fg)`,
        }}
      />
      <div className="grid gap-1.5 px-3 py-2.5 pl-4">
        <div className="flex items-center gap-2">
          <span className="font-mono text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            {ticket.number}
          </span>
          <StatusTag tone={priorityTone(ticket.priority)}>
            {ticket.priority.toUpperCase()}
          </StatusTag>
          <span className="ml-auto flex items-center gap-1">
            {(ticket.assignee_id ?? null) !== null && (
              <span
                data-testid={`kanban-assigned-${ticket.number}`}
                title="Assigned"
                className="grid h-4 w-4 place-items-center rounded-full text-[9px] font-semibold"
                style={{ background: "var(--m-accent-soft)", color: "var(--m-accent)" }}
                aria-label="Assigned"
              >
                A
              </span>
            )}
          </span>
        </div>
        <button
          type="button"
          className="text-left text-[13px] font-medium leading-snug hover:underline"
          style={{ color: "var(--m-text)" }}
          onClick={() => router.push(`/tickets/${ticket.id}`)}
        >
          {ticket.title}
        </button>
        {!compact && (
          <div className="flex flex-wrap items-center gap-1.5">
            <StatusTag tone={tone}>{ticket.category}</StatusTag>
            {due !== null && <StatusTag tone={due.tone}>{due.label}</StatusTag>}
            <span className="ml-auto" />
            <span className="relative">
              <Button
                size="sm"
                variant="ghost"
                aria-haspopup="menu"
                aria-expanded={menuOpen}
                aria-label={`Move ${ticket.number}`}
                data-testid={`kanban-move-${ticket.number}`}
                onClick={() => setMenuOpen((v) => !v)}
              >
                Move <ChevronIcon />
              </Button>
              {menuOpen && (
                <div
                  role="menu"
                  aria-label={`Move ${ticket.number} to status`}
                  className="absolute right-0 top-full z-30 mt-1 grid max-h-64 min-w-44 content-start gap-1 overflow-y-auto rounded-[var(--m-radius-container)] border p-2 shadow-md"
                  style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                >
                  <p className="px-1 text-[10px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                    {current?.name ?? "Unknown"} →
                  </p>
                  {targets.length === 0 && (
                    <p className="px-1 text-xs" style={{ color: "var(--m-text-muted)" }}>
                      No allowed transitions
                    </p>
                  )}
                  {targets.map((s) => (
                    <Button
                      key={s.id}
                      size="sm"
                      variant="ghost"
                      onClick={() => {
                        setMenuOpen(false);
                        if (ticket.status_id !== null) onMove(ticket, s.id);
                      }}
                    >
                      {s.name}
                    </Button>
                  ))}
                </div>
              )}
            </span>
          </div>
        )}
      </div>
    </div>
  );
}

interface KanbanColumnProps {
  status: BoardStatus;
  tickets: BoardTicket[];
  statuses: BoardStatus[];
  workflow: BoardTransitions | null;
  compact: boolean;
  collapsed: boolean;
  dropActive: boolean;
  draggingId: string | null;
  onToggleCollapse(): void;
  onDragOverColumn(statusId: string): void;
  onDropOnColumn(statusId: string): void;
  onDragStart(id: string): void;
  onDragEnd(): void;
  onMove(ticket: BoardTicket, toStatusId: string): void;
}

/// One kanban column: status-tinted header with count + WIP/queue signal,
/// collapse support, and the drop target for card moves. Tints come from
/// the shared status tokens so light and dark match the ticket status bar.
function KanbanColumn({
  status,
  tickets,
  statuses,
  workflow,
  compact,
  collapsed,
  dropActive,
  draggingId,
  onToggleCollapse,
  onDragOverColumn,
  onDropOnColumn,
  onDragStart,
  onDragEnd,
  onMove,
}: KanbanColumnProps): ReactNode {
  const tone = categoryTone(status.category);
  const toneFg = `var(--m-status-${tone === "success" ? "success" : tone === "warning" ? "warning" : "neutral"}-fg)`;
  const toneBg = `var(--m-status-${tone === "success" ? "success" : tone === "warning" ? "warning" : "neutral"}-bg)`;
  const signal =
    status.category === "in_progress" && tickets.length > 0
      ? `WIP ${tickets.length}`
      : status.category === "waiting" && tickets.length > 0
        ? `Queue ${tickets.length}`
        : null;

  if (collapsed) {
    return (
      <div
        data-testid={`kanban-column-${status.name}`}
        className="flex w-10 shrink-0 flex-col items-center gap-2 rounded-[var(--m-radius-container)] border py-3"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      >
        <button
          type="button"
          aria-label={`Expand ${status.name} column`}
          aria-expanded={false}
          data-testid={`kanban-collapse-${status.name}`}
          className="grid h-6 w-6 place-items-center rounded-full hover:opacity-80"
          style={{ background: toneBg, color: toneFg }}
          onClick={onToggleCollapse}
        >
          <span className="rotate-180"><ChevronIcon /></span>
        </button>
        <span className="kanban-rail-label text-xs font-medium" style={{ color: "var(--m-text-secondary)" }}>
          {status.name} ({tickets.length})
        </span>
      </div>
    );
  }

  return (
    <section
      data-testid={`kanban-column-${status.name}`}
      aria-label={`${status.name} column`}
      className={`flex min-w-64 w-64 shrink-0 flex-col rounded-[var(--m-radius-container)] border ${dropActive ? "kanban-column-drop" : ""}`}
      style={{ borderColor: "var(--m-border)", background: "var(--m-canvas)" }}
      onDragOver={(e) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        onDragOverColumn(status.id);
      }}
      onDrop={(e) => {
        e.preventDefault();
        onDropOnColumn(status.id);
      }}
    >
      <header
        className="flex items-center gap-2 rounded-t-[var(--m-radius-container)] px-3 py-2"
        style={{ background: toneBg }}
      >
        <span aria-hidden className="h-2 w-2 shrink-0 rounded-full" style={{ background: toneFg }} />
        <h3 className="truncate text-xs font-semibold" style={{ color: toneFg }}>
          {status.name}
        </h3>
        <span
          className="rounded-full px-1.5 text-[10px] font-semibold tabular-nums"
          style={{ background: "var(--m-surface)", color: "var(--m-text-secondary)" }}
          data-testid={`kanban-count-${status.name}`}
        >
          {tickets.length}
        </span>
        {signal !== null && (
          <StatusTag tone={tone}>{signal}</StatusTag>
        )}
        <span className="ml-auto" />
        <button
          type="button"
          aria-label={`Collapse ${status.name} column`}
          aria-expanded={true}
          data-testid={`kanban-collapse-${status.name}`}
          className="grid h-5 w-5 place-items-center rounded hover:opacity-70"
          style={{ color: toneFg }}
          onClick={onToggleCollapse}
        >
          <ChevronIcon />
        </button>
      </header>
      <div className="flex min-h-16 flex-col gap-2 p-2">
        {tickets.map((t) => (
          <KanbanCard
            key={t.id}
            ticket={t}
            statuses={statuses}
            workflow={workflow}
            compact={compact}
            dragging={draggingId === t.id}
            onDragStart={onDragStart}
            onDragEnd={onDragEnd}
            onMove={onMove}
          />
        ))}
        {tickets.length === 0 && (
          <p className="px-1 py-2 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            Nothing here
          </p>
        )}
      </div>
    </section>
  );
}

export default function BoardPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const params = useParams<{ id: string }>();
  const boardId = typeof params?.id === "string" ? params.id : "";

  // IF4/IF5: hydrate-once board snapshot, deltas over the board channel.
  const boardSpec = useMemo(() => makeBoardSpec(api.api, boardId), [boardId]);
  const { data: board, loading, patch } = useInstantCollection(boardModelKey(boardId), boardSpec);
  const { mutate } = useInstantWrite();
  useInstantChannels(boardId === "" ? [] : [`board:${boardId}`]);

  const [workflow, setWorkflow] = useState<BoardTransitions | null>(null);
  const [filters, setFilters] = useState<BoardFilters>(EMPTY_FILTERS);
  const [compact, setCompact] = useState(false);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const [moveError, setMoveError] = useState<string | null>(null);

  // The transition matrix drives both drops and the Move menus. Failure
  // (e.g. no permission) falls back to the semantic floor client-mirror.
  useEffect(() => {
    if (boardId === "") return;
    let cancelled = false;
    void api.api
      .GET("/api/v1/boards/{id}/transitions", { params: { path: { id: boardId } } })
      .then((res) => {
        if (!cancelled && res.data !== undefined) setWorkflow(res.data);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [boardId]);

  const statuses = board?.statuses ?? [];
  const ordered = useMemo(() => sortStatuses(statuses), [statuses]);
  const visible = useMemo(
    () => filterTickets(board?.tickets ?? [], filters),
    [board?.tickets, filters],
  );
  const groups = useMemo(() => groupByStatus(visible, ordered), [visible, ordered]);
  const totalTickets = board?.tickets.length ?? 0;

  const toggleCollapse = useCallback((statusId: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(statusId)) next.delete(statusId);
      else next.add(statusId);
      return next;
    });
  }, []);

  const moveTicket = useCallback(
    (ticket: BoardTicket, toStatusId: string) => {
      if (ticket.status_id === null || ticket.status_id === toStatusId) return;
      const target = statuses.find((s) => s.id === toStatusId);
      if (target === undefined) return;
      const before = { status_id: ticket.status_id, category: ticket.category };
      setMoveError(null);
      void mutate({
        describe: `Set status ${target.name} on ${ticket.number}`,
        entityId: ticket.id,
        method: "POST",
        path: `/api/v1/tickets/${ticket.id}/status`,
        buildBody: (base) => ({ status_id: toStatusId, ...base }),
        optimistic: () => {
          patch((m) => ({
            ...m,
            tickets: m.tickets.map((t) =>
              t.id === ticket.id ? { ...t, status_id: toStatusId, category: target.category } : t,
            ),
          }));
        },
        captureBefore: () => before,
        revert: () => {
          patch((m) => ({
            ...m,
            tickets: m.tickets.map((t) => (t.id === ticket.id ? { ...t, ...before } : t)),
          }));
        },
      }).then((result) => {
        if (result.status === "rejected") {
          setMoveError(
            result.message ??
              `That move is not allowed by this board's workflow (${ticket.number})`,
          );
        }
      });
    },
    [mutate, patch, statuses],
  );

  const handleDrop = useCallback(
    (statusId: string) => {
      setDropTarget(null);
      if (draggingId === null) return;
      const ticket = (board?.tickets ?? []).find((t) => t.id === draggingId);
      setDraggingId(null);
      if (ticket === undefined || ticket.status_id === null) return;
      if (!transitionAllowed(statuses, workflow, ticket.status_id, statusId)) {
        const from = statuses.find((s) => s.id === ticket.status_id)?.name ?? "this status";
        const to = statuses.find((s) => s.id === statusId)?.name ?? "that status";
        setMoveError(`Workflow does not allow ${from} → ${to} (${ticket.number})`);
        return;
      }
      moveTicket(ticket, statusId);
    },
    [board?.tickets, draggingId, moveTicket, statuses, workflow],
  );

  if (auth.status !== "authed") {
    return (
      <main className="flex min-h-full items-center justify-center">
        <Spinner />
      </main>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex min-h-0 flex-1">
        <BoardNav />
        <main
          className="flex min-w-0 flex-1 flex-col gap-3 overflow-auto p-4 md:p-6"
          style={{ borderLeft: "1px solid var(--m-border)" }}
        >
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="text-sm font-semibold">Board</h1>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => router.push(`/boards/${boardId}/fields`)}
            >
              Custom fields
            </Button>
            <span className="ml-auto" />
            <MembersPanel boardId={boardId} />
            <NewTicketDialog
              boardId={boardId}
              canCreate={auth.me?.permissions.includes("tickets:create") ?? false}
            />
          </div>

          <div
            role="toolbar"
            aria-label="Board filters"
            className="flex flex-wrap items-center gap-2 rounded-[var(--m-radius-container)] border p-2"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
          >
            <Input
              aria-label="Search tickets"
              placeholder="Search title or number…"
              value={filters.query}
              onChange={(e) => setFilters((f) => ({ ...f, query: e.target.value }))}
              className="h-7 w-56 text-xs"
              data-testid="board-search"
            />
            <div className="flex items-center gap-1" role="group" aria-label="Priority filter">
              {PRIORITIES.map((p) => {
                const active = filters.priorities.includes(p);
                return (
                  <button
                    key={p}
                    type="button"
                    aria-pressed={active}
                    data-testid={`board-filter-${p}`}
                    className="rounded-full border px-2 py-0.5 text-[11px] font-semibold tabular-nums"
                    style={
                      active
                        ? { borderColor: "var(--m-accent)", color: "var(--m-accent)", boxShadow: "0 0 0 1px var(--m-accent)" }
                        : { borderColor: "var(--m-border)", color: "var(--m-text-secondary)" }
                    }
                    onClick={() =>
                      setFilters((f) => ({
                        ...f,
                        priorities: active
                          ? f.priorities.filter((x) => x !== p)
                          : [...f.priorities, p],
                      }))
                    }
                  >
                    {p.toUpperCase()}
                  </button>
                );
              })}
            </div>
            <button
              type="button"
              aria-pressed={filters.unassignedOnly}
              data-testid="board-filter-unassigned"
              className="rounded-full border px-2 py-0.5 text-[11px]"
              style={
                filters.unassignedOnly
                  ? { borderColor: "var(--m-accent)", color: "var(--m-accent)", boxShadow: "0 0 0 1px var(--m-accent)" }
                  : { borderColor: "var(--m-border)", color: "var(--m-text-secondary)" }
              }
              onClick={() => setFilters((f) => ({ ...f, unassignedOnly: !f.unassignedOnly }))}
            >
              Unassigned
            </button>
            <span className="ml-auto" />
            <button
              type="button"
              aria-pressed={compact}
              data-testid="board-density"
              className="rounded-full border px-2 py-0.5 text-[11px]"
              style={
                compact
                  ? { borderColor: "var(--m-accent)", color: "var(--m-accent)", boxShadow: "0 0 0 1px var(--m-accent)" }
                  : { borderColor: "var(--m-border)", color: "var(--m-text-secondary)" }
              }
              onClick={() => setCompact((v) => !v)}
            >
              {compact ? "Compact" : "Comfortable"}
            </button>
          </div>

          {moveError !== null && (
            <p
              role="alert"
              data-testid="board-move-error"
              className="rounded-[var(--m-radius-control)] px-3 py-1.5 text-xs"
              style={{ background: "var(--m-status-danger-bg)", color: "var(--m-status-danger-fg)" }}
            >
              {moveError}
            </p>
          )}

          {loading ? (
            <div className="flex flex-1 items-center justify-center">
              <Spinner />
            </div>
          ) : ordered.length === 0 ? (
            <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              This board has no statuses yet.
            </p>
          ) : (
            <>
              {totalTickets === 0 && (
                <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                  No tickets on this board yet.
                </p>
              )}
              <div
                data-testid="kanban-board"
                className="flex min-h-0 flex-1 items-start gap-3 overflow-x-auto pb-2"
              >
                {ordered.map((s) => (
                  <KanbanColumn
                    key={s.id}
                    status={s}
                    tickets={groups.get(s.id) ?? []}
                    statuses={statuses}
                    workflow={workflow}
                    compact={compact}
                    collapsed={collapsed.has(s.id)}
                    dropActive={dropTarget === s.id && draggingId !== null}
                    draggingId={draggingId}
                    onToggleCollapse={() => toggleCollapse(s.id)}
                    onDragOverColumn={setDropTarget}
                    onDropOnColumn={handleDrop}
                    onDragStart={setDraggingId}
                    onDragEnd={() => {
                      setDraggingId(null);
                      setDropTarget(null);
                    }}
                    onMove={moveTicket}
                  />
                ))}
              </div>
            </>
          )}
        </main>
      </div>
    </div>
  );
}
