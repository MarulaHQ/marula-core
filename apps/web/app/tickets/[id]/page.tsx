// /tickets/[id] — the ticket workspace.
//
// Purpose: three-pane workbench per design/mockups/ticket-workspace-light.png
// — ticket list (left), detail (center), activity (comments + checklist +
// timer controls, right) — composed with @meridian/layout so pane geometry
// persists per user. Status changes go through the semantic-category
// transition endpoint; the timer binds to this ticket. The note composer
// picks the structured taxonomy (internal / client-visible / in-house
// update) and the checklist panel surfaces required-item progress plus the
// exact blockers the server reports on an enforced close (typed 409).
// Non-purpose: no board editing, no template administration here.
"use client";

import type { components } from "@meridian/sdk";
import {
  Button,
  EmptyState,
  Input,
  Label,
  Spinner,
  StatusTag,
  Table,
  Td,
  Th,
  Textarea,
  Tr,
} from "@meridian/ui";
import { PanelLayout, usePanelLayout } from "@meridian/layout";
import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { AppHeader } from "../../../components/app-header";
import { api, useRequireAuth } from "../../../lib/api";

type Ticket = components["schemas"]["Ticket"];
type MyWorkItem = components["schemas"]["MyWorkItem"];
type TicketComment = components["schemas"]["TicketComment"];
type ChecklistItem = components["schemas"]["ChecklistItem"];
type ChecklistProgress = components["schemas"]["ChecklistProgress"];
type TicketChecklistView = components["schemas"]["TicketChecklistView"];
type BoardStatus = components["schemas"]["BoardStatus"];
type BoardTransitionsResponse = components["schemas"]["BoardTransitionsResponse"];

const PANE_LIST = "pane-list" as const;
const PANE_DETAIL = "pane-detail" as const;
const PANE_ACTIVITY = "pane-activity" as const;

// Semantic-category groups in lifecycle order. Colors are industry-standard
// PSA semantics expressed ONLY through the Workbench status tokens (amber
// for waiting, green for closed; red stays reserved for breaches) — light
// and dark parity come from the shared token set.
const CATEGORY_GROUPS: {
  category: NonNullable<BoardStatus["category"]>;
  label: string;
  tone: "neutral" | "warning" | "success";
}[] = [
  { category: "open", label: "Open", tone: "neutral" },
  { category: "in_progress", label: "In Progress", tone: "neutral" },
  { category: "waiting", label: "Waiting", tone: "warning" },
  { category: "closed", label: "Closed", tone: "success" },
];

function categoryTone(category: string): "success" | "warning" | "neutral" {
  if (category === "closed") return "success";
  if (category === "waiting") return "warning";
  return "neutral";
}

// Note-kind taxonomy: squared tags, functional tones only (green = goes to
// the client, amber = internal coordination variant), labels spelled out.
const NOTE_KINDS: {
  value: NonNullable<TicketComment["kind"]>;
  label: string;
  tone: "neutral" | "success" | "warning";
}[] = [
  { value: "internal", label: "Internal", tone: "neutral" },
  { value: "client_visible", label: "Client-visible", tone: "success" },
  { value: "in_house_update", label: "In-house update", tone: "warning" },
];

function noteKindMeta(kind: NonNullable<TicketComment["kind"]>): (typeof NOTE_KINDS)[number] {
  return NOTE_KINDS.find((k) => k.value === kind) ?? NOTE_KINDS[0]!;
}

/// Client mirror of the server's transition verdict (core-domain `workflow`):
/// explicit matrix wins; otherwise the semantic floor governs — any→any
/// except identity and closed→open. The API remains authoritative; this only
/// disables buttons the server would reject anyway.
function transitionAllowed(
  statuses: BoardStatus[],
  workflow: BoardTransitionsResponse,
  fromId: string,
  toId: string,
): boolean {
  if (fromId === toId) return false;
  if ((workflow.mode ?? "floor") === "explicit") {
    return (workflow.edges ?? []).some(
      (e) => e.from_status_id === fromId && e.to_status_id === toId,
    );
  }
  const catOf = new Map(statuses.map((s) => [s.id, s.category]));
  const fromCat = catOf.get(fromId);
  const toCat = catOf.get(toId);
  if (fromCat === undefined || toCat === undefined) return false;
  return !(fromCat === "closed" && toCat === "open");
}

/// Extracts the missing-item list from the typed checklist-blocked 409
/// body (`{ error: { code: 409, message, missing: string[] } }`). Returns
/// null for any other error shape so callers can fall back to generic copy.
function extractMissingItems(errorBody: unknown): string[] | null {
  const err = (errorBody as { error?: { code?: number; missing?: unknown } })?.error;
  if (err?.code !== 409 || !Array.isArray(err.missing)) return null;
  const items = err.missing.filter((t): t is string => typeof t === "string");
  return items.length > 0 ? items : [];
}

export default function TicketWorkspacePage() {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const ticketId = params?.id ?? "";

  // Layout: persisted three-pane row split.
  const controller = usePanelLayout({
    userId: auth.me?.user_id ?? "anonymous",
    fallback: {
      kind: "split",
      id: "root",
      direction: "row",
      children: [
        { kind: "pane", id: PANE_LIST, tabs: [{ id: "tickets", title: "Tickets" }], activeTabId: "tickets", minWidthPx: 260 },
        { kind: "pane", id: PANE_DETAIL, tabs: [{ id: "detail", title: "Detail" }], activeTabId: "detail", minWidthPx: 380 },
        { kind: "pane", id: PANE_ACTIVITY, tabs: [{ id: "activity", title: "Activity" }], activeTabId: "activity", minWidthPx: 320 },
      ],
      sizes: [0.28, 0.42, 0.3],
    },
  });

  const [ticket, setTicket] = useState<Ticket | null>(null);
  const [statuses, setStatuses] = useState<BoardStatus[]>([]);
  const [workflow, setWorkflow] = useState<BoardTransitionsResponse>({ mode: "floor", edges: [] });
  const [comments, setComments] = useState<TicketComment[]>([]);
  const [checklist, setChecklist] = useState<TicketChecklistView | null>(null);
  const [commentDraft, setCommentDraft] = useState("");
  const [commentKind, setCommentKind] = useState<NonNullable<TicketComment["kind"]>>("internal");
  const [checklistDraft, setChecklistDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  // Blockers reported by the server's typed 409 on an enforced close.
  const [closeBlockers, setCloseBlockers] = useState<string[] | null>(null);

  const refreshTicket = useCallback(async () => {
    if (ticketId === "") return;
    const [t, c, cl] = await Promise.all([
      api.api.GET("/api/tickets/{id}", { params: { path: { id: ticketId } } }),
      api.api.GET("/api/tickets/{id}/comments", { params: { path: { id: ticketId } } }),
      api.api.GET("/api/tickets/{id}/checklist", { params: { path: { id: ticketId } } }),
    ]);
    if (t.error !== undefined) {
      setError("Ticket not found in your organization");
      return;
    }
    setTicket(t.data);
    setComments(c.data ?? []);
    setChecklist(cl.data ?? null);
    setError(null);
    if (t.data.board_id !== null && statuses.length === 0) {
      // Statuses + the board's effective workflow arrive together so the
      // status control can group by category and pre-disable illegal moves.
      const [s, tr] = await Promise.all([
        api.api.GET("/api/boards/{id}/statuses", {
          params: { path: { id: t.data.board_id } },
        }),
        api.api.GET("/api/boards/{id}/transitions", {
          params: { path: { id: t.data.board_id } },
        }),
      ]);
      if (s.error === undefined) setStatuses(s.data);
      if (tr.error === undefined) setWorkflow(tr.data);
    }
  }, [ticketId, statuses.length]);

  useEffect(() => {
    if (auth.status === "authed") void refreshTicket();
  }, [auth.status, refreshTicket]);

  async function changeStatus(statusId: string) {
    setCloseBlockers(null);
    const res = await api.api.POST("/api/tickets/{id}/status", {
      params: { path: { id: ticketId } },
      body: { status_id: statusId },
    });
    if (res.error !== undefined) {
      // Typed 409: pre-close checklist enforcement names every unchecked
      // REQUIRED item so the panel can render exact blockers.
      const missing = extractMissingItems(res.error);
      if (missing !== null) {
        setCloseBlockers(missing);
        setError(null);
        void refreshTicket();
        return;
      }
      setError("That status change is not allowed by this board's workflow");
      return;
    }
    setError(null);
    void refreshTicket();
  }

  async function startTimer() {
    await api.api.POST("/api/timers/start", { body: { ticket_id: ticketId } });
  }

  async function stopTimer() {
    await api.api.POST("/api/timers/stop", { body: {} });
  }

  async function postComment() {
    const body = commentDraft.trim();
    if (body === "") return;
    await api.api.POST("/api/tickets/{id}/comments", {
      params: { path: { id: ticketId } },
      body: { body, kind: commentKind },
    });
    setCommentDraft("");
    void refreshTicket();
  }

  async function addChecklistItem() {
    const text = checklistDraft.trim();
    if (text === "") return;
    await api.api.POST("/api/tickets/{id}/checklist", {
      params: { path: { id: ticketId } },
      body: { text },
    });
    setChecklistDraft("");
    void refreshTicket();
  }

  async function toggleChecklistItem(itemId: string, checked: boolean) {
    await api.api.POST("/api/tickets/{id}/checklist/{item_id}", {
      params: { path: { id: ticketId, item_id: itemId } },
      body: { checked },
    });
    void refreshTicket();
  }

  if (auth.status !== "authed") {
    return (
      <main className="flex min-h-screen items-center justify-center">
        <Spinner />
      </main>
    );
  }

  return (
    <div className="flex h-screen flex-col">
      <AppHeader />
      <main className="min-h-0 flex-1">
        <PanelLayout
          controller={controller}
          label="Ticket workspace"
          renderPaneContent={({ pane }) => {
            if (pane.id === PANE_LIST) {
              return (
                <MyWorkList selectedId={ticketId} onSelect={(id) => router.push(`/tickets/${id}`)} />
              );
            }
            if (pane.id === PANE_DETAIL && ticket !== null) {
              return (
                <div className="grid content-start gap-4 p-4">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-xs" style={{ color: "var(--m-text-muted)" }}>
                      {ticket.number}
                    </span>
                    <StatusTag tone={categoryTone(ticket.category)}>
                      {statuses.find((s) => s.id === ticket.status_id)?.name ?? ticket.category}
                    </StatusTag>
                  </div>
                  <h1 className="text-base font-semibold">{ticket.title}</h1>
                  {ticket.body !== null && (
                    <p className="whitespace-pre-wrap text-sm" style={{ color: "var(--m-text-secondary)" }}>
                      {ticket.body}
                    </p>
                  )}
                  <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
                    <dt style={{ color: "var(--m-text-muted)" }}>Priority</dt>
                    <dd className="uppercase">{ticket.priority}</dd>
                    <dt style={{ color: "var(--m-text-muted)" }}>Assignee</dt>
                    <dd>{ticket.assignee_id == null ? "Unassigned" : ticket.assignee_id.slice(0, 8)}</dd>
                    <dt style={{ color: "var(--m-text-muted)" }}>Created</dt>
                    <dd>{new Date(ticket.created_at ?? Date.now()).toLocaleString()}</dd>
                    {ticket.resolved_at != null && (
                      <>
                        <dt style={{ color: "var(--m-text-muted)" }}>Resolved</dt>
                        <dd>{new Date(ticket.resolved_at).toLocaleString()}</dd>
                      </>
                    )}
                  </dl>

                  <fieldset className="grid gap-3">
                    <legend className="mb-1 text-xs font-medium">Move to status</legend>
                    {CATEGORY_GROUPS.map(({ category, label, tone }) => {
                      const group = statuses.filter((s) => s.category === category);
                      if (group.length === 0) return null;
                      return (
                        <div key={category} className="grid gap-1">
                          <StatusTag tone={tone}>{label}</StatusTag>
                          <div className="flex flex-wrap gap-1">
                            {group.map((s) => {
                              const current = s.id === ticket.status_id;
                              const allowed =
                                current ||
                                (ticket.status_id !== null &&
                                  transitionAllowed(
                                    statuses,
                                    workflow,
                                    ticket.status_id,
                                    s.id,
                                  ));
                              return (
                                <Button
                                  key={s.id}
                                  size="sm"
                                  variant={current ? "secondary" : "ghost"}
                                  disabled={!allowed}
                                  title={
                                    allowed
                                      ? undefined
                                      : "This board's workflow does not allow that transition"
                                  }
                                  onClick={() => void changeStatus(s.id)}
                                >
                                  {s.name}
                                </Button>
                              );
                            })}
                          </div>
                        </div>
                      );
                    })}
                  </fieldset>

                  {error !== null && (
                    <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
                      {error}
                    </p>
                  )}

                  <div className="flex gap-2">
                    <Button variant="primary" size="sm" onClick={() => void startTimer()}>
                      Start timer
                    </Button>
                    <Button size="sm" onClick={() => void stopTimer()}>
                      Stop timer
                    </Button>
                  </div>
                </div>
              );
            }
            if (pane.id === PANE_ACTIVITY && ticket !== null) {
              const progress: ChecklistProgress | undefined = checklist?.progress;
              return (
                <div className="grid content-start gap-4 p-4">
                  <section className="grid gap-2">
                    <Label>Comments</Label>
                    {comments.length === 0 && (
                      <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                        No comments yet.
                      </p>
                    )}
                    {comments.map((c) => {
                      const meta =
                        c.kind === null || c.kind === undefined
                          ? (NOTE_KINDS[0]!)
                          : noteKindMeta(c.kind);
                      return (
                        <article
                          key={c.id}
                          className="rounded-[2px] border p-2 text-xs"
                          style={{ borderColor: "var(--m-border)" }}
                        >
                          <header className="mb-1 flex justify-between">
                            <StatusTag tone={meta.tone}>{meta.label}</StatusTag>
                            <time style={{ color: "var(--m-text-muted)" }}>
                              {new Date(c.created_at).toLocaleString()}
                            </time>
                          </header>
                          <p className="whitespace-pre-wrap">{c.body}</p>
                        </article>
                      );
                    })}
                    <div className="flex flex-wrap items-center gap-1 pt-1">
                      {NOTE_KINDS.map((k) => (
                        <button
                          key={k.value}
                          type="button"
                          aria-pressed={commentKind === k.value}
                          title={
                            k.value === "client_visible"
                              ? "Visible to the client on portal surfaces"
                              : k.value === "in_house_update"
                                ? "Staff-only coordination update (never shown to clients)"
                                : "Private to your team"
                          }
                          onClick={() => setCommentKind(k.value)}
                          className="cursor-pointer rounded-[2px] border px-2 py-1 text-xs"
                          style={{
                            borderColor:
                              commentKind === k.value ? "var(--m-accent)" : "var(--m-border)",
                            background:
                              commentKind === k.value ? "var(--m-accent-soft)" : "transparent",
                          }}
                        >
                          {k.label}
                        </button>
                      ))}
                    </div>
                    <Textarea
                      rows={3}
                      value={commentDraft}
                      onChange={(e) => setCommentDraft(e.target.value)}
                      placeholder={`Add a note (${noteKindMeta(commentKind).label})…`}
                    />
                    <Button size="sm" variant="primary" onClick={() => void postComment()}>
                      Comment
                    </Button>
                  </section>

                  <section className="grid gap-2">
                    <div className="flex items-center justify-between">
                      <Label>Pre-close checklist</Label>
                      {progress !== undefined && (
                        <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                          {progress.checked}/{progress.total} done
                          {(checklist?.enforcement_active ?? false) &&
                            ` · ${progress.required_open} required open`}
                        </span>
                      )}
                    </div>
                    {closeBlockers !== null && closeBlockers.length > 0 && (
                      <div
                        role="alert"
                        className="rounded-[2px] border p-2 text-xs"
                        style={{
                          borderColor: "var(--m-status-danger-fg)",
                          background: "var(--m-status-danger-bg, transparent)",
                        }}
                      >
                        <p className="font-medium">Close blocked — required items incomplete:</p>
                        <ul className="ml-4 list-disc">
                          {closeBlockers.map((text) => (
                            <li key={text}>{text}</li>
                          ))}
                        </ul>
                      </div>
                    )}
                    {(checklist?.items ?? []).map((item) => {
                      const required = item.template_item_id != null;
                      return (
                        <label key={item.id} className="flex items-center gap-2 text-xs">
                          <input
                            type="checkbox"
                            checked={item.checked}
                            onChange={(e) => void toggleChecklistItem(item.id, e.target.checked)}
                          />
                          <span
                            style={{
                              color: item.checked ? "var(--m-text-muted)" : undefined,
                              textDecoration: item.checked ? "line-through" : undefined,
                            }}
                          >
                            {item.text}
                          </span>
                          {required && (
                            <StatusTag tone={item.checked ? "success" : "warning"}>required</StatusTag>
                          )}
                        </label>
                      );
                    })}
                    <div className="flex gap-2">
                      <Input
                        value={checklistDraft}
                        onChange={(e) => setChecklistDraft(e.target.value)}
                        placeholder="Add checklist item…"
                      />
                      <Button size="sm" onClick={() => void addChecklistItem()}>
                        Add
                      </Button>
                    </div>
                  </section>
                </div>
              );
            }
            return (
              <div className="flex h-full items-center justify-center">
                {error !== null ? (
                  <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>{error}</p>
                ) : (
                  <Spinner />
                )}
              </div>
            );
          }}
        />
      </main>
    </div>
  );
}

/// Left pane: the caller's my-work queue (same ordering as /today).
function MyWorkList({ selectedId, onSelect }: { selectedId: string; onSelect(id: string): void }) {
  const [tickets, setTickets] = useState<MyWorkItem[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    void api.api.GET("/api/tickets/my-work").then((res) => {
      if (!cancelled && res.error === undefined) setTickets(res.data);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  if (tickets === null) {
    return (
      <div className="flex h-full items-center justify-center">
        <Spinner />
      </div>
    );
  }
  if (tickets.length === 0) {
    return <EmptyState message="Your open tickets appear here." />;
  }
  return (
    <Table>
      <thead>
        <tr>
          <Th>Number</Th>
          <Th>Title</Th>
        </tr>
      </thead>
      <tbody>
        {tickets.map((t) => (
          <Tr key={t.id} selected={t.id === selectedId} onClick={() => onSelect(t.id)}>
            <Td>{t.number}</Td>
            <Td>{t.title}</Td>
          </Tr>
        ))}
      </tbody>
    </Table>
  );
}
