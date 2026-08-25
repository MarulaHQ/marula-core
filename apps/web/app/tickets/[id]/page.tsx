// /tickets/[id] — the ticket workspace.
//
// Purpose: three-pane workbench per design/mockups/ticket-workspace-light.png
// — ticket list (left), detail (center), activity (comments + checklist +
// timer controls, right) — composed with @meridian/layout so pane geometry
// persists per user. Status changes go through the semantic-category
// transition endpoint; the timer binds to this ticket.
// Non-purpose: no board editing and no admin actions here.
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
type TicketComment = components["schemas"]["TicketComment"];
type ChecklistItem = components["schemas"]["ChecklistItem"];
type BoardStatus = components["schemas"]["BoardStatus"];

const PANE_LIST = "pane-list" as const;
const PANE_DETAIL = "pane-detail" as const;
const PANE_ACTIVITY = "pane-activity" as const;

function categoryTone(category: string): "success" | "warning" | "neutral" {
  if (category === "closed") return "success";
  if (category === "waiting") return "warning";
  return "neutral";
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
  const [comments, setComments] = useState<TicketComment[]>([]);
  const [checklist, setChecklist] = useState<ChecklistItem[]>([]);
  const [commentDraft, setCommentDraft] = useState("");
  const [checklistDraft, setChecklistDraft] = useState("");
  const [error, setError] = useState<string | null>(null);

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
    setChecklist(cl.data ?? []);
    setError(null);
    if (t.data.board_id !== null && statuses.length === 0) {
      const s = await api.api.GET("/api/boards/{id}/statuses", {
        params: { path: { id: t.data.board_id } },
      });
      if (s.error === undefined) setStatuses(s.data);
    }
  }, [ticketId, statuses.length]);

  useEffect(() => {
    if (auth.status === "authed") void refreshTicket();
  }, [auth.status, refreshTicket]);

  async function changeStatus(statusId: string) {
    await api.api.POST("/api/tickets/{id}/status", {
      params: { path: { id: ticketId } },
      body: { status_id: statusId },
    });
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
      body: { body, kind: "internal" },
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
                    <StatusTag tone={categoryTone(ticket.category)}>{ticket.category}</StatusTag>
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

                  <fieldset className="grid gap-2">
                    <legend className="mb-1 text-xs font-medium">Move to status</legend>
                    <div className="flex flex-wrap gap-1">
                      {statuses.map((s) => (
                        <Button
                          key={s.id}
                          size="sm"
                          variant={s.id === ticket.status_id ? "secondary" : "ghost"}
                          disabled={s.id === ticket.status_id}
                          onClick={() => void changeStatus(s.id)}
                        >
                          {s.name}
                        </Button>
                      ))}
                    </div>
                  </fieldset>

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
              return (
                <div className="grid content-start gap-4 p-4">
                  <section className="grid gap-2">
                    <Label>Comments</Label>
                    {comments.length === 0 && (
                      <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                        No comments yet.
                      </p>
                    )}
                    {comments.map((c) => (
                      <article key={c.id} className="rounded-[2px] border p-2 text-xs" style={{ borderColor: "var(--m-border)" }}>
                        <header className="mb-1 flex justify-between">
                          <StatusTag tone="neutral">{c.kind}</StatusTag>
                          <time style={{ color: "var(--m-text-muted)" }}>
                            {new Date(c.created_at).toLocaleString()}
                          </time>
                        </header>
                        <p className="whitespace-pre-wrap">{c.body}</p>
                      </article>
                    ))}
                    <Textarea
                      rows={3}
                      value={commentDraft}
                      onChange={(e) => setCommentDraft(e.target.value)}
                      placeholder="Add a note…"
                    />
                    <Button size="sm" variant="primary" onClick={() => void postComment()}>
                      Comment
                    </Button>
                  </section>

                  <section className="grid gap-2">
                    <Label>Pre-close checklist</Label>
                    {checklist.map((item) => (
                      <label key={item.id} className="flex items-center gap-2 text-xs">
                        <input
                          type="checkbox"
                          checked={item.checked}
                          onChange={(e) => void toggleChecklistItem(item.id, e.target.checked)}
                        />
                        <span style={{ color: item.checked ? "var(--m-text-muted)" : undefined }}>
                          {item.text}
                        </span>
                      </label>
                    ))}
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
  const [tickets, setTickets] = useState<Ticket[] | null>(null);

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
