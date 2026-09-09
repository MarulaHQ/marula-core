// /tickets/[id] — the ticket workspace.
//
// Purpose: three-pane workbench per design/mockups/ticket-workspace-light.png
// — ticket list (left), detail (center), activity (comments + checklist +
// timer controls, right) — composed with @meridian/layout so pane geometry
// persists per user. Data flows through the IF4/IF5 instant layer: the
// detail and my-work read models hydrate once and live through WS deltas,
// and every write renders optimistically then reconciles from the IF2
// read-your-write envelope (or queues durably offline, IF6).
// Non-purpose: no board editing and no admin actions here.
"use client";

import type { components } from "@meridian/sdk";
import { snapshotOf } from "@meridian/sdk";
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
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { TicketCustomFields } from "../../../components/ticket-custom-fields";
import { TicketUndo } from "../../../components/ticket-undo";
import {
  api,
  downloadAttachment,
  uploadAttachment,
  useRequireAuth,
} from "../../../lib/api";
import {
  makeMyWorkSpec,
  makeTicketDetailSpec,
  MY_WORK_KEY,
  ticketModelKey,
  type TicketDetailModel,
} from "../../../lib/instant/read-models";
import {
  useInstantChannels,
  useInstantCollection,
  useInstantWrite,
} from "../../../lib/use-instant";

type MyWorkItem = components["schemas"]["MyWorkItem"];
type TicketComment = components["schemas"]["TicketComment"];
type BoardStatus = components["schemas"]["BoardStatus"];
type BoardTransitionsResponse = components["schemas"]["BoardTransitionsResponse"];
type TicketSlaStatus = components["schemas"]["TicketSlaStatus"];
type Attachment = components["schemas"]["Attachment"];

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

/** Paperclip glyph: inline SVG on currentColor so both themes track tokens. */
function PaperclipIcon(): ReactNode {
  return (
    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48" />
    </svg>
  );
}

function categoryTone(category: string): "success" | "warning" | "neutral" {
  if (category === "closed") return "success";
  if (category === "waiting") return "warning";
  return "neutral";
}

/// SLA badge state: tone follows industry-standard PSA semantics — amber
/// when the clock is running low, red only for an actual breach. All
/// colors come from the shared Workbench status tokens.
function slaBadge(sla: TicketSlaStatus): { tone: "neutral" | "warning" | "danger"; label: string } | null {
  if (sla.breached) return { tone: "danger", label: "SLA breached" };
  const mins = sla.minutes_remaining;
  if (mins === null || mins === undefined) return null;
  if (mins < 0) return { tone: "danger", label: `SLA over ${formatMins(-mins)}` };
  const label = `${sla.paused ? "SLA paused · " : ""}${formatMins(mins)} left`;
  // Inside the last business hour (or already paused with little left)
  // the badge turns amber.
  if (mins <= 60 && !sla.paused) return { tone: "warning", label };
  return { tone: "neutral", label };
}

function formatMins(mins: number): string {
  if (mins >= 60) {
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return m === 0 ? `${h}h` : `${h}h ${m}m`;
  }
  return `${mins}m`;
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

  // IF4/IF5: hydrate-once detail snapshot + live deltas; writes reconcile
  // through the mutation envelope. Explicit refresh only re-snapshots on
  // user action (custom-field verdicts), never on navigation.
  const detailSpec = useMemo(() => makeTicketDetailSpec(api.api, ticketId), [ticketId]);
  const { data: model, loading, refresh, patch: patchModel } = useInstantCollection(
    ticketModelKey(ticketId),
    detailSpec,
  );
  const { mutate, stampEntity } = useInstantWrite();
  const [error, setError] = useState<string | null>(null);

  // Deep-link/hard-load recovery: the hydrate-once snapshot can fire before
  // the auth token is ready (401) and store a model with ticket=null. When
  // auth is ready and the ticket never arrived, re-snapshot once per ticket.
  const rehydratedFor = useRef<string | null>(null);
  useEffect(() => {
    if (auth.status !== "authed") return;
    if (loading || model?.ticket != null) return;
    if (rehydratedFor.current === ticketId) return;
    rehydratedFor.current = ticketId;
    void refresh();
  }, [auth.status, loading, model?.ticket, ticketId, refresh]);

  useInstantChannels([
    model?.ticket?.board_id !== undefined && model?.ticket?.board_id !== null
      ? `board:${model.ticket.board_id}`
      : "",
    auth.me !== null ? `user:${auth.me.user_id}` : "",
  ]);

  const ticket = model?.ticket ?? null;
  const statuses = model?.statuses ?? [];
  const workflow = model?.workflow ?? { mode: "floor" as const, edges: [] };

  // Comment composer state.
  const [commentDraft, setCommentDraft] = useState("");
  const [checklistDraft, setChecklistDraft] = useState("");
  // Attachments (2.14): files staged in the composer; already-stored
  // attachments come from the detail read model, grouped per comment.
  const [stagedFiles, setStagedFiles] = useState<File[]>([]);

  async function changeStatus(statusId: string) {
    if (ticket === null) return;
    const before = ticket;
    const target = statuses.find((s) => s.id === statusId) ?? null;
    const result = await mutate({
      describe: `Set status ${target?.name ?? statusId} on ${ticket.number}`,
      entityId: ticket.id,
      method: "POST",
      path: `/api/v1/tickets/${ticket.id}/status`,
      buildBody: (base) => ({ status_id: statusId, ...base }),
      optimistic: () => {
        if (target === null) return;
        patchDetail((m) => ({
          ...m,
          ticket: m.ticket === null ? m.ticket : { ...m.ticket, status_id: statusId, category: target.category },
        }));
      },
      captureBefore: () => before,
      revert: () => {
        patchDetail((m) => ({ ...m, ticket: m.ticket === null ? m.ticket : { ...m.ticket, ...before } }));
      },
    });
    if (result.status === "rejected") {
      setError("That status change is not allowed by this board's workflow");
      return;
    }
    setError(null);
  }

  async function startTimer() {
    await api.api.POST("/api/v1/timers/start", { body: { ticket_id: ticketId } });
  }

  async function stopTimer() {
    await api.api.POST("/api/v1/timers/stop", { body: {} });
  }

  /** Synchronous detail-model patch for optimistic/authoritative renders. */
  function patchDetail(updater: (m: TicketDetailModel) => TicketDetailModel): void {
    patchModel(updater);
  }

  async function postComment() {
    if (ticket === null) return;
    const body = commentDraft.trim();
    if (body === "") return;
    try {
      // Composer flow (2.14): upload bytes first, then link the returned
      // attachment ids in the comment create — one atomic note.
      const uploaded: string[] = [];
      for (const file of stagedFiles) {
        uploaded.push((await uploadAttachment(ticketId, file)).id);
      }
      const tempId = `pending-${Date.now()}-${Math.random().toString(36).slice(2)}`;
      const optimistic: TicketComment = {
        id: tempId,
        ticket_id: ticketId,
        author_id: auth.me?.user_id ?? "",
        author_name: auth.me?.display_name ?? "",
        body,
        kind: "internal",
        created_at: new Date().toISOString(),
        attachment_ids: uploaded,
      } as unknown as TicketComment;
      const result = await mutate({
        describe: `Comment on ${ticket.number}`,
        entityId: tempId,
        method: "POST",
        path: `/api/v1/tickets/${ticketId}/comments`,
        buildBody: (base) => ({
          body,
          kind: "internal",
          attachment_ids: uploaded,
          ...base,
        }),
        optimistic: () => {
          patchDetail((m) => ({ ...m, comments: [...m.comments, optimistic] }));
        },
        captureBefore: () => null,
        revert: () => {
          patchDetail((m) => ({
            ...m,
            comments: m.comments.filter((c) => c.id !== tempId),
          }));
        },
      });
      if (result.status === "rejected") {
        setError(result.message ?? "Could not post comment");
        return;
      }
      if (result.status === "queued") {
        setError(null);
        setCommentDraft("");
        setStagedFiles([]);
        return;
      }
      // Accepted: swap the optimistic row for the applied comment, and
      // stamp the server id's version cursor for future edits.
      const applied =
        result.mutation !== null ? snapshotOf(result.mutation).after : null;
      if (applied !== null && result.mutation !== null) {
        stampEntity(applied.id, result.mutation);
      }
      patchDetail((m) => ({
        ...m,
        comments: m.comments.map((c) =>
          c.id === tempId && applied !== null ? (applied as TicketComment) : c,
        ),
      }));
      setError(null);
      setCommentDraft("");
      setStagedFiles([]);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not post comment");
    }
  }

  /** Paperclip rows for one comment (or null = unlinked/ticket files). */
  function attachmentsFor(commentId: string | null): Attachment[] {
    return (model?.attachments ?? []).filter((a) =>
      commentId === null ? a.ticket_comment_id === null || a.ticket_comment_id === undefined : a.ticket_comment_id === commentId,
    );
  }

  function paperclipRow(attachment: Attachment): ReactNode {
    const sizeKb = Math.max(1, Math.round(attachment.size_bytes / 1024));
    return (
      <button
        key={attachment.id}
        type="button"
        className="flex items-center gap-1 rounded-[2px] px-1 py-0.5 text-left text-xs hover:underline"
        style={{ color: "var(--m-text-muted)" }}
        title={`${attachment.filename} · ${sizeKb} KB`}
        onClick={() => void downloadAttachment(attachment.id, attachment.filename)}
      >
        <PaperclipIcon />
        <span className="truncate" style={{ maxWidth: 220 }}>
          {attachment.filename}
        </span>
        <span>({sizeKb} KB)</span>
      </button>
    );
  }

  async function addChecklistItem() {
    const text = checklistDraft.trim();
    if (text === "") return;
    const tempId = `pending-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    const result = await mutate({
      describe: "Add checklist item",
      entityId: tempId,
      method: "POST",
      path: `/api/v1/tickets/${ticketId}/checklist`,
      buildBody: (base) => ({ text, ...base }),
      optimistic: () => {
        patchDetail((m) => ({
          ...m,
          checklist:
            m.checklist === null
              ? m.checklist
              : {
                  ...m.checklist,
                  items: [
                    ...m.checklist.items,
                    {
                      id: tempId,
                      text,
                      checked: false,
                      checked_at: null,
                      checked_by: null,
                      org_id: m.ticket?.org_id ?? "",
                      sort_order: m.checklist.items.length,
                      template_item_id: null,
                      ticket_id: ticketId,
                    },
                  ],
                },
        }));
      },
      captureBefore: () => null,
      revert: () => {
        patchDetail((m) => ({
          ...m,
          checklist:
            m.checklist === null
              ? m.checklist
              : { ...m.checklist, items: m.checklist.items.filter((i) => i.id !== tempId) },
        }));
      },
    });
    if (result.status === "rejected") {
      setError(result.message ?? "Could not add checklist item");
      return;
    }
    setChecklistDraft("");
    if (result.status === "accepted" && result.mutation !== null) {
      // The checklist envelope carries the applied item in `mutation.after`.
      const { after } = snapshotOf(result.mutation);
      if (after !== null) {
        stampEntity(after.id, result.mutation);
        patchDetail((m) => ({
          ...m,
          checklist:
            m.checklist === null
              ? m.checklist
              : {
                  ...m.checklist,
                  items: m.checklist.items.map((i) =>
                    i.id === tempId ? (after as components["schemas"]["ChecklistItem"]) : i,
                  ),
                },
        }));
        return;
      }
    }
    // Queued without an immediate envelope: settle on the server snapshot.
    if (result.status === "queued") void refresh();
  }

  async function toggleChecklistItem(itemId: string, checked: boolean) {
    if (itemId.startsWith("pending-")) return;
    const result = await mutate({
      describe: checked ? "Check off item" : "Uncheck item",
      entityId: itemId,
      method: "POST",
      path: `/api/v1/tickets/${ticketId}/checklist/${itemId}`,
      buildBody: (base) => ({ checked, ...base }),
      optimistic: () => {
        patchDetail((m) => ({
          ...m,
          checklist:
            m.checklist === null
              ? m.checklist
              : {
                  ...m.checklist,
                  items: m.checklist.items.map((i) =>
                    i.id === itemId ? { ...i, checked } : i,
                  ),
                },
        }));
      },
      captureBefore: () => null,
      revert: () => {
        patchDetail((m) => ({
          ...m,
          checklist:
            m.checklist === null
              ? m.checklist
              : {
                  ...m.checklist,
                  items: m.checklist.items.map((i) =>
                    i.id === itemId ? { ...i, checked: !checked } : i,
                  ),
                },
        }));
      },
    });
    if (result.status === "rejected") {
      setError(result.message ?? "Could not update checklist item");
      return;
    }
    // Authoritative toggle: the envelope carries the applied item.
    if (result.status === "accepted" && result.mutation !== null) {
      const { after } = snapshotOf(result.mutation);
      if (after !== null) {
        patchDetail((m) => ({
          ...m,
          checklist:
            m.checklist === null
              ? m.checklist
              : {
                  ...m.checklist,
                  items: m.checklist.items.map((i) =>
                    i.id === itemId ? (after as components["schemas"]["ChecklistItem"]) : i,
                  ),
                },
        }));
      }
    }
  }

  if (auth.status !== "authed") {
    return (
      <main className="flex min-h-full items-center justify-center">
        <Spinner />
      </main>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-end gap-2 px-3 py-1.5">
        <button
          type="button"
          onClick={() => {
            void controller.reset();
          }}
          title="Restore the default three-pane layout (reopens closed panes)"
          className="cursor-pointer border-none bg-transparent text-[11px] transition-opacity duration-100"
          style={{ color: "var(--m-text-muted)" }}
        >
          Reset layout
        </button>
      </div>
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
                    {(() => {
                      const badge = model?.sla == null ? null : slaBadge(model.sla);
                      return badge !== null ? (
                        <StatusTag tone={badge.tone}>{badge.label}</StatusTag>
                      ) : null;
                    })()}
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

                  {/* Custom fields (TASKLIST 2.17): definitions for this
                      board with kind-appropriate inputs; server verdicts
                      re-snapshot the detail model (never the page). */}
                  <TicketCustomFields
                    ticketId={ticket.id}
                    boardId={ticket.board_id}
                    values={ticket.custom_fields ?? []}
                    onChanged={() => void refresh()}
                  />

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

                  {/* IF7 undo (Wave 9 close-out): Ctrl/Cmd+Z steps the
                      record back one applied write; closed records offer
                      per-field reverts only. */}
                  <TicketUndo ticketId={ticket.id} onChanged={() => void refresh()} />

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
                    {(model?.comments ?? []).length === 0 && (
                      <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                        No comments yet.
                      </p>
                    )}
                    {(model?.comments ?? []).map((c) => (
                      <article key={c.id} className="rounded-[2px] border p-2 text-xs" style={{ borderColor: "var(--m-border)" }}>
                        <header className="mb-1 flex justify-between">
                          <StatusTag tone="neutral">{c.kind}</StatusTag>
                          <time style={{ color: "var(--m-text-muted)" }}>
                            {new Date(c.created_at).toLocaleString()}
                          </time>
                        </header>
                        <p className="whitespace-pre-wrap">{c.body}</p>
                        {attachmentsFor(c.id).length > 0 && (
                          <footer className="mt-1 flex flex-wrap gap-1">
                            {attachmentsFor(c.id).map((a) => paperclipRow(a))}
                          </footer>
                        )}
                      </article>
                    ))}
                    <Textarea
                      rows={3}
                      value={commentDraft}
                      onChange={(e) => setCommentDraft(e.target.value)}
                      placeholder="Add a note…"
                    />
                    <div className="flex items-center gap-2">
                      <label
                        className="flex cursor-pointer items-center gap-1 rounded-[2px] border px-2 py-1 text-xs"
                        style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}
                        title="Attach files (PDF, images, Office, zip, text)"
                      >
                        <PaperclipIcon />
                        Attach
                        <input
                          type="file"
                          multiple
                          className="hidden"
                          onChange={(e) => {
                            setStagedFiles((prev) => [...prev, ...Array.from(e.target.files ?? [])]);
                            e.target.value = "";
                          }}
                        />
                      </label>
                      {stagedFiles.map((f, i) => (
                        <button
                          key={`${f.name}-${i}`}
                          type="button"
                          className="flex items-center gap-1 rounded-[2px] px-1 text-xs"
                          style={{ color: "var(--m-text-muted)" }}
                          title="Remove"
                          onClick={() => setStagedFiles((prev) => prev.filter((_, idx) => idx !== i))}
                        >
                          <PaperclipIcon /> {f.name.length > 24 ? `${f.name.slice(0, 21)}…` : f.name} ✕
                        </button>
                      ))}
                    </div>
                    <Button size="sm" variant="primary" onClick={() => void postComment()}>
                      Comment
                    </Button>
                  </section>

                  <section className="grid gap-2">
                    <Label>Pre-close checklist</Label>
                    {model?.checklist?.enforcement_active && model.checklist.blocking.length > 0 ? (
                      <p className="text-xs" style={{ color: "var(--m-warning)" }}>
                        {model.checklist.blocking.length} item
                        {model.checklist.blocking.length === 1 ? "" : "s"} must be checked before this
                        ticket can close.
                      </p>
                    ) : null}
                    {(model?.checklist?.items ?? []).map((item) => (
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
                {loading && ticket === null ? (
                  <Spinner />
                ) : ticket === null && !loading ? (
                  <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>
                    Ticket not found in your organization
                  </p>
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

/// Left pane: the caller's my-work queue (same ordering as /today) through
/// the shared my-work read model — hydrate-once, delta-reconciled.
function MyWorkList({ selectedId, onSelect }: { selectedId: string; onSelect(id: string): void }) {
  const myWorkSpec = useMemo(() => makeMyWorkSpec(api.api), []);
  const { data: tickets, loading } = useInstantCollection(MY_WORK_KEY, myWorkSpec);

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Spinner />
      </div>
    );
  }
  if (tickets === undefined || tickets.length === 0) {
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
