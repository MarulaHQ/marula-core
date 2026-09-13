// /tickets/[id] — the ticket workspace (Wave 11 lane 6 QA close-out).
//
// Purpose: three-pane workbench (list / detail / activity) composed with
// @meridian/layout so pane geometry persists per user. The DETAIL pane is
// now pod-based: a compact status bar (color + text, transitions happen
// FROM the bar) at the top, a pinned original request, a rearrangeable pod
// strip (client+contact, next steps, tasks, scheduling, classification —
// admin-configurable per RBAC level, per-user hide/order), and a tab strip
// (original request / notes / tasks / details / related). The ACTIVITY
// pane is the unified timeline (notes + key events + time, filterable by
// kind and visibility). Notes compose line-by-line with an authoring-time
// visibility selector; close/validation walks merge → time validation →
// explicit next-status confirmation → optional client email; scheduling
// links mint + track from the scheduling pod.
//
// Data flows through the IF4/IF5 instant layer: the detail and my-work
// read models hydrate once and live through WS deltas; every write renders
// optimistically then reconciles from the IF2 read-your-write envelope
// (or queues durably offline, IF6). Emails in/out have no per-ticket read
// endpoint (typed gap in the timeline); N-A task states are per-user
// panel-state (no API column); the classification display awaits the
// type/category read fields. Each gap is surfaced honestly, never faked.
// Non-purpose: no board editing and no admin actions here.
"use client";

import { rejectionText } from "../../../lib/error-copy";
import type { components } from "@meridian/sdk";
import { snapshotOf } from "@meridian/sdk";
import {
  Button,
  EmptyState,
  Spinner,
  StatusTag,
  Table,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import { PanelLayout, usePanelLayout } from "@meridian/layout";
import Link from "next/link";
import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { TicketCustomFields } from "../../../components/ticket-custom-fields";
import { TicketUndo } from "../../../components/ticket-undo";
import {
  api,
  downloadAttachment,
  uploadAttachment,
  useRequireAuth,
} from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import {
  makeMyWorkSpec,
  makeTicketDetailSpec,
  MY_WORK_KEY,
  ticketModelKey,
  type TicketDetailModel,
  type TicketComment,
} from "../../../lib/instant/read-models";
import {
  useInstantChannels,
  useInstantCollection,
  useInstantWrite,
} from "../../../lib/use-instant";
import {
  orderedPods,
  podVisibleFor,
  loadPodState,
  saveUserState,
  rbacLevelFor,
  type PodUserState,
  type PodVisibilityConfig,
  type PodId,
} from "../../../components/ticket-workspace/pod-state";
import {
  PodAdminSettings,
  PodUserControls,
  canConfigurePods,
} from "../../../components/ticket-workspace/pod-settings";
import { TicketStatusBar } from "../../../components/ticket-status-bar";
import { PresenceRoster } from "../../../components/instant/presence-roster";
import {
  MoveBoardDialog,
  extractTargetStatuses,
  type MoveBoardInput,
  type MoveBoardOutcome,
} from "../../../components/ticket-workspace/move-board";
import { ClientContactPod } from "../../../components/ticket-workspace/client-contact-pod";
import { NextStepsPod } from "../../../components/ticket-workspace/next-steps-pod";
import { TasksPanel } from "../../../components/ticket-workspace/tasks-panel";
import { NotesComposer } from "../../../components/ticket-workspace/notes-composer";
import { TicketTimeline } from "../../../components/ticket-workspace/timeline";
import { SchedulingPod } from "../../../components/ticket-workspace/scheduling-pod";
import { CloseFlow } from "../../../components/ticket-workspace/close-flow";
import { DetailTabs, OriginalRequest, DETAIL_TABS, type DetailTabId } from "../../../components/ticket-workspace/detail-tabs";
import { TicketTags } from "../../../components/ticket-tags";
import { TicketLinksPanel, MergedIntoBanner } from "../../../components/ticket-links";
import { TicketMoney } from "../../../components/ticket-money";

type BoardStatus = components["schemas"]["BoardStatus"];
type BoardTransitionsResponse = components["schemas"]["BoardTransitionsResponse"];
type Attachment = components["schemas"]["Attachment"];
type OrgUser = components["schemas"]["OrgUser"];

const PANE_LIST = "pane-list" as const;
const PANE_DETAIL = "pane-detail" as const;
const PANE_ACTIVITY = "pane-activity" as const;

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
        { kind: "pane", id: PANE_ACTIVITY, tabs: [{ id: "activity", title: "Timeline" }], activeTabId: "activity", minWidthPx: 320 },
      ],
      sizes: [0.26, 0.46, 0.28],
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

  // Merged tickets: banner title resolution + mutation suppression.
  const mergedIntoId = ticket?.merged_into_ticket_id ?? null;
  const [mergedIntoTitle, setMergedIntoTitle] = useState<string | null>(null);
  useEffect(() => {
    if (mergedIntoId === null) {
      setMergedIntoTitle(null);
      return;
    }
    let cancelled = false;
    void api.api
      .GET("/api/v1/tickets/{id}", { params: { path: { id: mergedIntoId } } })
      .then(({ data }) => {
        if (!cancelled && data !== undefined && data !== null) setMergedIntoTitle(`${data.number} ${data.title}`);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [mergedIntoId]);
  const isMerged = mergedIntoId !== null;
  const canMoveBoard = !isMerged && (auth.me?.permissions.includes("tickets:edit") ?? false);

  // --- org directory (author names for notes/timeline) -----------------------
  const [namesByUser, setNamesByUser] = useState<Map<string, string>>(new Map());
  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/org/users", { params: { query: { limit: 200 } } });
      if (cancelled || data === undefined) return;
      const map = new Map<string, string>();
      for (const u of data.users) map.set(u.id, u.display_name);
      setNamesByUser(map);
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status]);

  // --- pod state (admin + per-user) -------------------------------------------
  const rbac = rbacLevelFor(auth.me?.permissions ?? []);
  const [podAdmin, setPodAdmin] = useState<PodVisibilityConfig>({});
  const [podUser, setPodUser] = useState<PodUserState>({ hidden: [], order: [] });
  const [podAdminError, setPodAdminError] = useState<string | null>(null);
  const podStateLoaded = useRef(false);
  useEffect(() => {
    if (auth.status !== "authed" || auth.me === null) return;
    if (podStateLoaded.current) return;
    podStateLoaded.current = true;
    // Admin layer is org server truth (GET /api/org/pod-visibility —
    // lane 6); the user layer stays per-user device state.
    void Promise.all([
      api.api.GET("/api/v1/org/pod-visibility"),
      loadPodState(auth.me.org_id, auth.me.user_id),
    ]).then(([vis, { user }]) => {
      setPodAdmin(
        (vis.data?.config ?? {}) as PodVisibilityConfig,
      );
      setPodUser(user);
    });
  }, [auth.status, auth.me]);

  const onPodAdminChange = useCallback(
    (next: PodVisibilityConfig) => {
      const previous = podAdmin;
      setPodAdmin(next);
      setPodAdminError(null);
      if (auth.me === null) return;
      // Optimistic org write (Admin:edit); a typed rejection rolls the
      // local matrix back and surfaces the reason inline.
      void runEnvelopeMutation({
        describe: "Update pod visibility matrix",
        buildBody: (base) => ({ config: next, ...base }),
        optimistic: () => {},
        revert: () => setPodAdmin(previous),
        request: (client, body) =>
          client.PUT("/api/v1/org/pod-visibility", {
            body: body as components["schemas"]["PutPodVisibilityRequest"],
          }),
      }).then((result) => {
        if (result.ok === false) {
          setPodAdminError(rejectionText(result, "Could not save the pod visibility matrix"));
        }
      });
    },
    [auth.me, podAdmin],
  );
  const onPodUserChange = useCallback(
    (next: PodUserState) => {
      setPodUser(next);
      if (auth.me !== null) void saveUserState(auth.me.org_id, auth.me.user_id, next);
    },
    [auth.me],
  );

  const visiblePods = orderedPods(podUser.order).filter(
    (pod) => podVisibleFor(pod, rbac, podAdmin, podUser.hidden),
  );

  // --- detail tabs --------------------------------------------------------------
  const [activeTab, setActiveTab] = useState<DetailTabId>("original");
  const [moveBoardOpen, setMoveBoardOpen] = useState(false);

  // --- write paths ----------------------------------------------------------------
  const [commentDraftsBusy, setCommentDraftsBusy] = useState(false);

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
      // Reconcile copy, never an error toast: a typed version_conflict
      // reads as "someone else's change landed — refresh and re-apply"
      // (the optimistic patch is already reverted).
      setError(rejectionText(result, "That status change is not allowed by this board's workflow"));
      return;
    }
    setError(null);
  }

  async function patchTicket(
    describe: string,
    body: Record<string, unknown>,
    optimistic: (m: TicketDetailModel) => TicketDetailModel,
  ): Promise<void> {
    if (ticket === null) return;
    const before = ticket;
    const result = await mutate({
      describe,
      entityId: ticket.id,
      method: "PATCH",
      path: `/api/v1/tickets/${ticket.id}`,
      buildBody: (base) => ({ ...body, ...base }),
      optimistic: () => {
        patchDetail(optimistic);
      },
      captureBefore: () => before,
      revert: () => {
        patchDetail((m) => ({ ...m, ticket: m.ticket === null ? m.ticket : { ...m.ticket, ...before } }));
      },
    });
    if (result.status === "rejected") {
      throw new Error(rejectionText(result, "Change rejected"));
    }
  }

  /// Board move (Wave 11 lane 3 backend): optimistic render against the
  /// target board's status set, authoritative reconcile from the IF2
  /// envelope; a typed `board_move_status_conflict` returns the target
  /// board's statuses so the dialog can render an inline pick-list and
  /// retry with an explicit `status_id`.
  async function performBoardMove(input: MoveBoardInput): Promise<MoveBoardOutcome> {
    if (ticket === null) return { ok: false, message: "No ticket loaded." };
    const before = ticket;
    // The optimistic render swaps the board's status set + workflow too, so a
    // rejection must restore the WHOLE slice — otherwise a typed conflict
    // leaves the target board's statuses in place and the ticket's own status
    // renders as "Unknown status".
    const beforeStatuses = statuses;
    const beforeWorkflow = workflow;
    const pinned =
      input.statusId !== null
        ? (input.targetStatuses.find((s) => s.id === input.statusId) ?? null)
        : null;
    const targetStatus = pinned ?? input.autoStatus;
    const result = await mutate({
      describe: `Move ${ticket.number} to another board`,
      entityId: ticket.id,
      method: "POST",
      path: `/api/v1/tickets/${ticket.id}/move-board`,
      buildBody: (base) => ({
        target_board_id: input.targetBoardId,
        status_id: input.statusId,
        reset_sla: input.resetSla,
        ...base,
      }),
      optimistic: () => {
        patchDetail((m) => {
          if (m.ticket === null) return m;
          return {
            ...m,
            statuses: input.targetStatuses.length > 0 ? input.targetStatuses : m.statuses,
            workflow: { mode: "floor", edges: [] },
            ticket: {
              ...m.ticket,
              board_id: input.targetBoardId,
              ...(targetStatus !== null
                ? { status_id: targetStatus.id, category: targetStatus.category }
                : {}),
            },
          };
        });
      },
      captureBefore: () => before,
      revert: () => {
        patchDetail((m) => ({
          ...m,
          ticket: m.ticket === null ? m.ticket : { ...m.ticket, ...before },
          statuses: beforeStatuses,
          workflow: beforeWorkflow,
        }));
      },
    });
    if (result.status === "rejected") {
      const conflictStatuses =
        result.reason === "board_move_status_conflict"
          ? extractTargetStatuses(result.details)
          : undefined;
      return {
        ok: false,
        reason: result.reason,
        message: result.message,
        conflictStatuses,
      };
    }
    return { ok: true };
  }

  async function saveCustomField(fieldId: string, value: string): Promise<void> {
    if (ticket === null) return;
    await patchTicket(
      "Set next steps",
      { custom_fields: [{ field_id: fieldId, value }] },
      (m) => ({
        ...m,
        ticket:
          m.ticket === null
            ? m.ticket
            : {
                ...m.ticket,
                custom_fields: m.ticket.custom_fields.some((v) => v.field_id === fieldId)
                  ? m.ticket.custom_fields.map((v) => (v.field_id === fieldId ? { ...v, value } : v))
                  : [...m.ticket.custom_fields, { field_id: fieldId, name: "Next steps", kind: "text", required: false, sort_order: 0, value }],
              },
      }),
    );
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

  async function postComment(body: string, kind: TicketComment["kind"], attachmentIds: string[]): Promise<void> {
    if (ticket === null) return;
    try {
      const tempId = `pending-${Date.now()}-${Math.random().toString(36).slice(2)}`;
      const optimistic: TicketComment = {
        id: tempId,
        ticket_id: ticketId,
        author_id: auth.me?.user_id ?? "",
        body,
        kind,
        created_at: new Date().toISOString(),
        attachment_ids: attachmentIds,
      } as unknown as TicketComment;
      const result = await mutate({
        describe: `Note on ${ticket.number}`,
        entityId: tempId,
        method: "POST",
        path: `/api/v1/tickets/${ticketId}/comments`,
        buildBody: (base) => ({
          body,
          kind,
          attachment_ids: attachmentIds,
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
        throw new Error(rejectionText(result, "Could not post note"));
      }
      if (result.status === "queued") {
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
    } catch (e) {
      throw e instanceof Error ? e : new Error("Could not post note");
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
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
          <path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48" />
        </svg>
        <span className="truncate" style={{ maxWidth: 220 }}>
          {attachment.filename}
        </span>
        <span>({sizeKb} KB)</span>
      </button>
    );
  }

  async function addChecklistItem(text: string) {
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
      setError(rejectionText(result, "Could not add checklist item"));
      return;
    }
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

  async function setChecklistItemState(
    itemId: string,
    state: { checked: boolean; na: boolean },
  ): Promise<{ ok: boolean; message?: string }> {
    if (itemId.startsWith("pending-")) return { ok: false };
    let prior: { checked: boolean; na: boolean } | null = null;
    const result = await mutate({
      describe: state.na ? "Mark item N-A" : state.checked ? "Check off item" : "Uncheck item",
      entityId: itemId,
      method: "POST",
      path: `/api/v1/tickets/${ticketId}/checklist/${itemId}`,
      buildBody: (base) => ({ checked: state.checked, na: state.na, ...base }),
      optimistic: () => {
        patchDetail((m) => ({
          ...m,
          checklist:
            m.checklist === null
              ? m.checklist
              : {
                  ...m.checklist,
                  items: m.checklist.items.map((i) =>
                    i.id === itemId ? { ...i, checked: state.checked, na: state.na } : i,
                  ),
                },
        }));
      },
      captureBefore: () => {
        const item = model?.checklist?.items.find((i) => i.id === itemId) ?? null;
        if (item !== null) prior = { checked: item.checked, na: item.na ?? false };
        return prior;
      },
      revert: () => {
        const snapshot = prior;
        if (snapshot === null) return;
        patchDetail((m) => ({
          ...m,
          checklist:
            m.checklist === null
              ? m.checklist
              : {
                  ...m.checklist,
                  items: m.checklist.items.map((i) =>
                    i.id === itemId ? { ...i, checked: snapshot.checked, na: snapshot.na } : i,
                  ),
                },
        }));
      },
    });
    if (result.status === "rejected") {
      const message = rejectionText(result, "Could not update checklist item");
      setError(message);
      return { ok: false, message };
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
    return { ok: true };
  }

  // --- close flow ----------------------------------------------------------------
  const [closeOpen, setCloseOpen] = useState(false);
  const [nextStepsDraft, setNextStepsDraft] = useState<string>("");
  useEffect(() => {
    const v = ticket?.custom_fields.find((f) => f.name.toLowerCase() === "next steps");
    setNextStepsDraft(v != null && v.value != null ? String(v.value) : "");
  }, [ticket?.custom_fields]);

  // Close-email recipients: the ticket's client and contact names pulled
  // from the main records (intelligent pull — contact preferred, client
  // fallback), fetched only while the close flow may need them.
  const [clientName, setClientName] = useState<string | null>(null);
  const [contactName, setContactName] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    setClientName(null);
    setContactName(null);
    const clientId = ticket?.client_id ?? null;
    if (clientId === null) return;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/clients/{id}", { params: { path: { id: clientId } } });
      if (cancelled) return;
      if (data !== undefined && data !== null) setClientName(data.name);
      const { data: contacts } = await api.api.GET("/api/v1/clients/{id}/contacts", {
        params: { path: { id: clientId } },
      });
      if (cancelled || contacts === undefined) return;
      const contactId = ticket?.contact_id ?? null;
      const match = contactId !== null ? contacts.find((c) => c.id === contactId) : undefined;
      if (match !== undefined) setContactName(match.name);
      else if (contacts.length === 1 && contacts[0] !== undefined) setContactName(contacts[0].name);
    })();
    return () => {
      cancelled = true;
    };
  }, [ticket?.client_id, ticket?.contact_id]);

  if (auth.status !== "authed") {
    return (
      <main className="flex min-h-full items-center justify-center">
        <Spinner />
      </main>
    );
  }

  const checklistItems = model?.checklist?.items ?? [];

  const renderNotesPane = (): ReactNode => (
    <div className="grid content-start gap-3 p-3">
      <NotesComposer
        busy={commentDraftsBusy}
        onPost={async (body, kind, attachmentIds) => {
          setCommentDraftsBusy(true);
          try {
            await postComment(body, kind, attachmentIds);
          } finally {
            setCommentDraftsBusy(false);
          }
        }}
        onUpload={(file) => uploadAttachment(ticketId, file).then((r) => r.id)}
      />
      <ul className="grid gap-2">
        {(model?.comments ?? []).length === 0 && (
          <li className="text-xs" style={{ color: "var(--m-text-muted)" }}>
            No notes yet.
          </li>
        )}
        {(model?.comments ?? []).map((c) => (
          <article key={c.id} className="grid gap-1 rounded-[2px] border p-2 text-xs" style={{ borderColor: "var(--m-border)" }}>
            <header className="flex flex-wrap items-center gap-2">
              <StatusTag tone={c.kind === "client_visible" ? "success" : c.kind === "in_house_update" ? "warning" : "neutral"}>
                {c.kind === "client_visible" ? "Client-visible" : c.kind === "in_house_update" ? "In-house" : "Internal"}
              </StatusTag>
              <span className="font-medium">{namesByUser.get(c.author_id ?? "") ?? "Unknown author"}</span>
              <time className="ml-auto" style={{ color: "var(--m-text-muted)" }}>
                {new Date(c.created_at).toLocaleString()}
              </time>
            </header>
            <div className="grid gap-0.5">
              {c.body.split("\n").map((line, idx) => (
                <p key={idx} className="whitespace-pre-wrap" style={{ color: line.startsWith("- ") ? undefined : undefined }}>
                  {line}
                </p>
              ))}
            </div>
            {attachmentsFor(c.id).length > 0 && (
              <footer className="flex flex-wrap gap-1">
                {attachmentsFor(c.id).map((a) => paperclipRow(a))}
              </footer>
            )}
          </article>
        ))}
      </ul>
</div>
  );

  const renderTasksTab = (): ReactNode => (
    <div className="p-3">
    <TasksPanel
      testPrefix="tab-"
      items={checklistItems}
      enforcementActive={model?.checklist?.enforcement_active ?? false}
      blockingCount={model?.checklist?.blocking.length ?? 0}
      onSetState={setChecklistItemState}
      onAdd={addChecklistItem}
    />
    </div>
  );

  const renderDetailsTab = (): ReactNode => (
    <div className="grid content-start gap-4 p-3">
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
        <dt style={{ color: "var(--m-text-muted)" }}>Priority</dt>
        <dd className="uppercase">{ticket?.priority}</dd>
        <dt style={{ color: "var(--m-text-muted)" }}>Assignee</dt>
        <dd>{ticket?.assignee_id == null ? "Unassigned" : namesByUser.get(ticket.assignee_id) ?? ticket.assignee_id.slice(0, 8)}</dd>
        <dt style={{ color: "var(--m-text-muted)" }}>Created</dt>
        <dd>{ticket ? new Date(ticket.created_at ?? Date.now()).toLocaleString() : null}</dd>
        {ticket?.resolved_at != null && (
          <>
            <dt style={{ color: "var(--m-text-muted)" }}>Resolved</dt>
            <dd>{new Date(ticket.resolved_at).toLocaleString()}</dd>
          </>
        )}
      </dl>
      {ticket !== null && (
        <TicketCustomFields
          ticketId={ticket.id}
          boardId={ticket.board_id}
          values={ticket.custom_fields ?? []}
          onChanged={() => void refresh()}
        />
      )}
      <div className="grid gap-1 text-xs">
        <dt className="font-medium">Board</dt>
        <dd className="flex flex-wrap items-center gap-2" data-testid="details-board">
          {canMoveBoard ? (
            <Button
              size="sm"
              variant="secondary"
              data-testid="details-move-board"
              onClick={() => setMoveBoardOpen(true)}
            >
              Move to another board…
            </Button>
          ) : (
            <span style={{ color: "var(--m-text-muted)" }}>
              Moving a ticket to another board requires ticket edit permission.
            </span>
          )}
        </dd>
      </div>
      <div className="grid gap-1 text-xs">
        <dt className="font-medium">Classification</dt>
        <dd style={{ color: "var(--m-text-muted)" }} data-testid="classification-gap">
          Type/category read-back is not exposed on the ticket read yet —{" "}
          <Link href="/settings/taxonomy" className="underline">
            manage the trees in Settings → Taxonomy
          </Link>
          .
        </dd>
      </div>
      {ticket !== null && <TicketUndo ticketId={ticket.id} onChanged={() => void refresh()} />}
    </div>
  );

  const renderRelatedTab = (): ReactNode => (
    <div className="grid content-start gap-3 p-3">
      <section className="grid gap-1">
        <h2 className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>Attachments</h2>
        {(model?.attachments ?? []).length === 0 && (
          <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>No attachments.</p>
        )}
        <div className="flex flex-wrap gap-1">
          {(model?.attachments ?? []).map((a) => paperclipRow(a))}
        </div>
      </section>
    </div>
  );

  const renderLinksTab = (): ReactNode =>
    ticket === null ? null : (
      <div className="p-3">
        <TicketLinksPanel
          ticketId={ticket.id}
          ticket={ticket}
          canWork={!isMerged && (auth.me?.permissions.includes("tickets:edit") ?? false)}
          namesByUser={namesByUser}
          onChanged={() => void refresh()}
        />
      </div>
    );

  const renderMoneyTab = (): ReactNode => {
    const canViewFinancials = auth.me?.permissions.includes("special:view_financials") ?? false;
    if (!canViewFinancials) {
      return (
        <div className="p-3">
          <p className="text-xs" data-testid="money-gap" style={{ color: "var(--m-text-muted)" }}>
            Cost lines and expenses are financial data — your role does not include financial visibility.
          </p>
        </div>
      );
    }
    return (
      <div className="p-3">
        <TicketMoney ticketId={ticketId} />
      </div>
    );
  };

  const renderPod = (podId: PodId): ReactNode => {
    if (ticket === null) return null;
    switch (podId) {
      case "client-contact":
        return (
          <ClientContactPod
            key="client-contact"
            clientId={ticket.client_id ?? null}
            contactId={ticket.contact_id ?? null}
            onChange={async (patch) => {
              await patchTicket(
                "Change client/contact",
                patch,
                (m) =>
                  m.ticket === null
                    ? m
                    : { ...m, ticket: { ...m.ticket, ...patch } as typeof m.ticket },
              );
            }}
            onClientChanged={() => {
              void refresh();
            }}
          />
        );
      case "next-steps":
        return (
          <NextStepsPod
            key="next-steps"
            boardId={ticket.board_id}
            values={ticket.custom_fields ?? []}
            canManageBoards={auth.me?.permissions.includes("special:manage_boards") ?? false}
            onSaveValue={saveCustomField}
            onDefinitionCreated={() => {
              void refresh();
            }}
          />
        );
      case "task-lists":
        return (
          <section
            key="task-lists"
            className="rounded-[2px] border p-3"
            style={{ borderColor: "var(--m-border)" }}
            data-testid="pod-tasks-inline"
            aria-label="Tasks"
          >
            <TasksPanel
              testPrefix="pod-"
              items={checklistItems}
              enforcementActive={model?.checklist?.enforcement_active ?? false}
              blockingCount={model?.checklist?.blocking.length ?? 0}
              onSetState={setChecklistItemState}
              onAdd={addChecklistItem}
            />
          </section>
        );
      case "scheduling":
        return <SchedulingPod key="scheduling" ticketId={ticket.id} ticketTitle={ticket.title} />;
      case "taxonomy":
        return (
          <section
            key="taxonomy"
            className="grid content-start gap-2 rounded-[2px] border p-3"
            style={{ borderColor: "var(--m-border)" }}
            data-testid="pod-taxonomy"
            aria-label="Classification"
          >
            <h2 className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Classification
            </h2>
            <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              This ticket&apos;s type/category read-back is not on the ticket view yet;{" "}
              <Link href="/settings/taxonomy" className="underline">
                manage the org&apos;s trees in Settings → Taxonomy
              </Link>
              .
            </p>
          </section>
        );
    }
  };

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-end gap-2 px-3 py-1.5">
        {podAdminError !== null && (
          <p className="text-[11px]" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="pod-admin-error">
            {podAdminError}
          </p>
        )}
        {canConfigurePods(auth.me?.permissions ?? []) && (
          <PodAdminSettings
            level={rbac}
            config={podAdmin}
            onConfigure={onPodAdminChange}
            onReset={onPodAdminChange.bind(null, {})}
          />
        )}
        <PodUserControls
          visiblePods={visiblePods.map((p) => ({ id: p.id, title: p.title, rearrangeable: p.rearrangeable }))}
          userState={podUser}
          onChange={onPodUserChange}
        />
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
                <div className="grid content-start">
                  {isMerged && mergedIntoId !== null ? (
                    <MergedIntoBanner mergedIntoTicketId={mergedIntoId} linkedTitle={mergedIntoTitle} />
                  ) : null}
                  <TicketStatusBar
                    number={ticket.number}
                    title={ticket.title}
                    statusId={ticket.status_id}
                    statuses={statuses}
                    workflow={workflow}
                    sla={model?.sla ?? null}
                    onChangeStatus={(id) => void changeStatus(id)}
                    onMoveBoard={canMoveBoard ? () => setMoveBoardOpen(true) : undefined}
                  />
                  <PresenceRoster ticketId={ticket.id} />
                  <div className="flex flex-wrap items-center justify-between gap-2 px-3 pt-2">
                    <TicketTags
                      ticketId={ticket.id}
                      canEdit={!isMerged && (auth.me?.permissions.includes("tickets:edit") ?? false)}
                      onChanged={() => void refresh()}
                    />
                    <div className="flex justify-end gap-2">
                      <Button size="sm" variant="primary" data-testid="validate-close" onClick={() => setCloseOpen(true)} disabled={isMerged}>
                        Validate for close
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => void startTimer()} disabled={isMerged}>
                        Start timer
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => void stopTimer()} disabled={isMerged}>
                        Stop timer
                      </Button>
                    </div>
                  </div>
                  <div className="grid gap-3 p-3">
                    <OriginalRequest ticket={ticket} />
                    <DetailTabs active={activeTab} onChange={setActiveTab} />
                    <div role="tabpanel" data-testid={`detail-panel-${activeTab}`} className="grid gap-3">
                      {activeTab === "notes" && renderNotesPane()}
                      {activeTab === "tasks" && renderTasksTab()}
                      {activeTab === "details" && renderDetailsTab()}
                      {activeTab === "related" && renderRelatedTab()}
                      {activeTab === "links" && renderLinksTab()}
                      {activeTab === "money" && renderMoneyTab()}
                      {activeTab === "original" && (
                        <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                          The original request is pinned above and always visible.
                        </p>
                      )}
                    </div>
                    <div className="grid gap-3" data-testid="pod-strip">
                      {visiblePods.map((p) => renderPod(p.id))}
                    </div>
                    {error !== null && (
                      <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
                        {error}
                      </p>
                    )}
                  </div>
                  <CloseFlow
                    open={closeOpen}
                    onCloseDialog={() => setCloseOpen(false)}
                    ticketId={ticket.id}
                    ticketNumber={String(ticket.number)}
                    ticketTitle={ticket.title}
                    userId={auth.me?.user_id ?? "anonymous"}
                    checklist={checklistItems}
                    nextSteps={nextStepsDraft}
                    clientName={null}
                    contactName={null}
                    statuses={statuses}
                    workflow={workflow}
                    statusId={ticket.status_id}
                    onPostNote={async (body, kind) => {
                      await postComment(body, kind, []);
                    }}
                    onChangeStatus={changeStatus}
                  />
                  {canMoveBoard ? (
                    <MoveBoardDialog
                      open={moveBoardOpen}
                      onOpenChange={setMoveBoardOpen}
                      ticket={ticket}
                      onMove={performBoardMove}
                      onMoved={() => void refresh()}
                    />
                  ) : null}
                </div>
              );
            }
            if (pane.id === PANE_ACTIVITY && ticket !== null) {
              return (
                <div className="grid content-start gap-3 p-3">
                  <TicketTimeline ticketId={ticket.id} comments={model?.comments ?? []} namesByUser={namesByUser} />
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
/// the shared my-work read model — hydrate-once, delta-reconciled — plus
/// the tag filter: when tags are picked the pane switches to the full
/// ticket list through GET /tickets?tags=… (AND semantics — a ticket must
/// carry EVERY selected tag, surfaced in the UI).
function MyWorkList({ selectedId, onSelect }: { selectedId: string; onSelect(id: string): void }) {
  const myWorkSpec = useMemo(() => makeMyWorkSpec(api.api), []);
  const { data: tickets, loading } = useInstantCollection(MY_WORK_KEY, myWorkSpec);

  // Tag filter state (B1): org tags + the AND-selected subset.
  const [orgTags, setOrgTags] = useState<components["schemas"]["Tag"][]>([]);
  const [pickedTags, setPickedTags] = useState<string[]>([]);
  const [tagResults, setTagResults] = useState<components["schemas"]["TicketWithFields"][] | null>(null);
  const [filterExpanded, setFilterExpanded] = useState(false);
  const [tagLoading, setTagLoading] = useState(false);
  const [tagError, setTagError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void api.api
      .GET("/api/v1/tags")
      .then(({ data }) => {
        if (!cancelled && data !== undefined) setOrgTags(data.map((tv) => tv.tag));
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (pickedTags.length === 0) {
      setTagResults(null);
      setTagError(null);
      return;
    }
    let cancelled = false;
    setTagLoading(true);
    void api.api
      .GET("/api/v1/tickets", { params: { query: { tags: pickedTags.join(","), limit: 200 } } })
      .then(({ data, error }) => {
        if (cancelled) return;
        setTagLoading(false);
        if (error !== undefined || data === undefined) {
          setTagError("The tag-filtered list could not be loaded.");
          return;
        }
        setTagResults(data);
      })
      .catch(() => {
        if (!cancelled) {
          setTagLoading(false);
          setTagError("The tag-filtered list could not be loaded.");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [pickedTags]);

  const activeRows: { id: string; number: string; title: string }[] | null =
    pickedTags.length > 0
      ? (tagResults ?? []).map((t) => ({ id: t.id, number: t.number, title: t.title }))
      : (tickets ?? []).map((t) => ({ id: t.id, number: t.number, title: t.title }));

  if (pickedTags.length > 0 && tagLoading && tagResults === null) {
    return (
      <div className="flex h-full items-center justify-center">
        <Spinner />
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      {orgTags.length > 0 ? (
        <div
          className="shrink-0 border-b px-2 py-1.5"
          style={{ borderColor: "var(--m-border)" }}
          data-testid="tags-filter"
        >
          <div className="flex items-center gap-1 text-[10px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
            Tags
            {pickedTags.length > 1 ? <span title="A ticket must carry every selected tag">· matches ALL</span> : null}
          </div>
          <div className="mt-1 flex flex-wrap gap-1">
            {(filterExpanded ? orgTags : orgTags.slice(0, 10)).map((tag) => {
              const picked = pickedTags.includes(tag.id);
              const color = tag.color ?? "var(--m-text-muted)";
              return (
                <button
                  key={tag.id}
                  type="button"
                  aria-pressed={picked}
                  className="cursor-pointer rounded-[2px] border px-1.5 py-0.5 text-[10px] transition-opacity duration-100"
                  style={{
                    borderColor: color,
                    color: picked ? "var(--m-surface)" : color,
                    background: picked ? color : "transparent",
                  }}
                  onClick={() =>
                    setPickedTags((prev) => (prev.includes(tag.id) ? prev.filter((t) => t !== tag.id) : [...prev, tag.id]))
                  }
                >
                  {tag.name}
                </button>
              );
            })}
            {orgTags.length > 10 ? (
              <button
                type="button"
                className="cursor-pointer border-none bg-transparent text-[10px] underline"
                style={{ color: "var(--m-text-muted)" }}
                onClick={() => setFilterExpanded((v) => !v)}
              >
                {filterExpanded ? "less" : `+${orgTags.length - 10} more`}
              </button>
            ) : null}
            {pickedTags.length > 0 ? (
              <button
                type="button"
                className="cursor-pointer border-none bg-transparent text-[10px] underline"
                style={{ color: "var(--m-text-muted)" }}
                onClick={() => setPickedTags([])}
              >
                clear
              </button>
            ) : null}
          </div>
        </div>
      ) : null}
      <div className="min-h-0 flex-1 overflow-auto">
        {tagError !== null ? (
          <p className="p-2 text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
            {tagError}
          </p>
        ) : activeRows === null || (pickedTags.length === 0 && loading) ? (
          <div className="flex h-full items-center justify-center">
            <Spinner />
          </div>
        ) : activeRows.length === 0 ? (
          <EmptyState message={pickedTags.length > 0 ? "No tickets carry every selected tag." : "Your open tickets appear here."} />
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Number</Th>
                <Th>Title</Th>
              </tr>
            </thead>
            <tbody>
              {activeRows.map((t) => (
                <Tr key={t.id} selected={t.id === selectedId} onClick={() => onSelect(t.id)}>
                  <Td>{t.number}</Td>
                  <Td>{t.title}</Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </div>
    </div>
  );
}
