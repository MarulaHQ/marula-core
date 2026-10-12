// /tickets/[id] — Workbench v2 (Wave 15 lane 7).
//
// Purpose: the ticket detail surface rebuilt to the approved mockup
// (design/mockups/marula-ticket-workbench-html-mockup-v2.html) inside
// the lane-6 shell (nav rail + top bar + work bar). Three columns over
// the shell: the Ticket Context panel (left, collapses as the view
// narrows — never persistently visible in narrow layouts), the center
// column (header with the top color+text status bar, tabs, filter chips,
// the timeline with per-kind icon column, and the floating composer), and
// the context-aware Agent panel (right, real KB/search/proposal data).
// The workbar's Wrap up opens the real wrap-up sheet (stop timer → mint
// the time entry → apply the status; typed 409/422 inline). The classic
// three-pane workspace stays reachable at /tickets/classic/[id] for
// old-vs-new comparison during the transition.
//
// Data flows through the IF4/IF5 instant layer exactly as the classic
// workspace: hydrate-once detail snapshot, WS deltas, optimistic writes
// reconciling from the IF2 envelope (or queued durably offline). Every
// surface consumes the regen'd typed SDK; gaps render typed "not
// available yet" states — never fake data or dead buttons.
// Non-purpose: no shell rework, no board editing, no admin surfaces.

"use client";

import type { components } from "@marula-core/sdk";
import { snapshotOf } from "@marula-core/sdk";
import {
  Badge,
  Button,
  Spinner,
  StatusStepper,
} from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import { TranscribeControl } from "../../../components/attachments/transcribe-control";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { rejectionText } from "../../../lib/error-copy";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { useTimer } from "../../../lib/use-timer";
import {
  api,
  useRequireAuth,
} from "../../../lib/api";
import {
  makeTicketDetailSpec,
  ticketModelKey,
  type TicketComment,
  type TicketDetailModel,
} from "../../../lib/instant/read-models";
import {
  useInstantChannels,
  useInstantCollection,
  useInstantWrite,
} from "../../../lib/use-instant";
import {
  MoveBoardDialog,
  extractTargetStatuses,
  type MoveBoardInput,
  type MoveBoardOutcome,
} from "../../../components/ticket-workspace/move-board";
import { TasksPanel } from "../../../components/ticket-workspace/tasks-panel";
import { PopoutButton } from "../../../components/popout/popout-button";
import { SchedulingPod } from "../../../components/ticket-workspace/scheduling-pod";
import { TicketStatusBar, transitionAllowed } from "../../../components/ticket-status-bar";
import { keywordsOf } from "../../../components/workbench-v2/agent-panel";
import { WbTimeline } from "../../../components/workbench-v2/timeline";
import { WbContextPanel } from "../../../components/workbench-v2/context-panel";
import { WbAgentPanel } from "../../../components/workbench-v2/agent-panel";
import { DecisionSuggestionsCard } from "../../../components/workbench-v2/decision-card";
import { WbComposer } from "../../../components/workbench-v2/composer";
import { MergeBanner } from "../../../components/tickets/merge-banner";
import { useTabCounts } from "../../../components/ticket-workspace/tab-counts";
import {
  CustomFieldsPanel,
  TagsPanel,
  TicketLinksPanel,
} from "../../../components/ticket-workspace/ticket-fields";
import { MovementDialog, type MovementKind } from "../../../components/stock/movement-dialog";
import { PickDialog } from "../../../components/stock/pick-dialog";
import {
  fetchTicketPicks,
  qtyText,
  unpinItem,
  type TicketPickRow,
} from "../../../lib/day-pack";
import { LabelPrintButton } from "../../../components/labels/label-print-button";
import { formatQtyMilli } from "../../../lib/stock-format";
import { WrapUpSheet } from "../../../components/workbench-v2/wrap-up-sheet";
import { TasksGateDialog, type GateBlocks } from "../../../components/notes/close-gate-dialog";
import { registerWrapUpTarget } from "../../../components/workbench-v2/wrap-up-bridge";
import "../../../components/workbench-v2/workbench.css";
import "./wb-chrome.css";

type BoardStatus = components["schemas"]["BoardStatus"];
type StockMovementView = components["schemas"]["StockMovementView"];
type StockItemView = components["schemas"]["StockItemView"];

/** The panel's receipt after a recorded movement — kind + the ledger row. */
interface PartsReceipt {
  kind: MovementKind;
  movement: StockMovementView;
}

function receiptSentence(
  receipt: PartsReceipt,
  item: StockItemView | undefined,
  clientName: string | null,
): string {
  const movement = receipt.movement;
  const name = item?.name ?? "the item";
  const qty = formatQtyMilli(Math.abs(movement.qty_milli), item?.unit ?? "");
  switch (receipt.kind) {
    case "ship-out":
      return `Shipped ${qty} · ${name}${clientName !== null ? ` to ${clientName}` : ""} — the movement is on this ticket's timeline.`;
    case "receive":
      return `Returned ${qty} · ${name} to stock — the movement is on this ticket's timeline.`;
    case "consume":
      return `Used ${qty} · ${name} — the movement is on this ticket's timeline.`;
    default:
      return `Recorded ${receipt.kind} — the movement is on this ticket's timeline.`;
  }
}

export type WbTab = "activity" | "details" | "related" | "time" | "files" | "notes";

const TABS: { id: WbTab; label: string }[] = [
  { id: "activity", label: "Activity" },
  { id: "details", label: "Details" },
  { id: "related", label: "Related" },
  { id: "time", label: "Time Entries" },
  { id: "files", label: "Files" },
  { id: "notes", label: "Notes" },
];

function priorityPill(priority: string | null | undefined): ReactNode {
  if (priority == null) return null;
  const p = priority.toLowerCase();
  // The Badge priority family already carries the colour rule: P1 is the only
  // red, P2 the blush/amber "due or soon" pair, and the rest quiet. A
  // hand-rolled pill here is a second copy of that decision, and the two
  // copies would drift the first time a colour moved.
  if (p !== "p1" && p !== "p2" && p !== "p3" && p !== "p4") return null;
  return (
    <Badge priority={p} data-testid="wb-priority-pill">
      {priority.toUpperCase()}
    </Badge>
  );
}

function slaMetaText(sla: components["schemas"]["TicketSlaStatus"] | null): string | null {
  if (sla === null || !sla.applied) return null;
  const mins = sla.minutes_remaining ?? null;
  const fmt = (m: number) => (m >= 60 ? `${Math.floor(m / 60)}h ${m % 60}m` : `${m}m`);
  if (sla.breached || (mins !== null && mins < 0)) {
    return mins !== null ? `SLA over ${fmt(-mins)}` : "SLA breached";
  }
  if (mins === null || mins === undefined) return sla.paused ? "SLA paused" : null;
  return `${sla.paused ? "Paused · " : ""}${fmt(mins)} SLA remaining`;
}

export default function TicketWorkbenchPage() {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const ticketId = params?.id ?? "";

  // IF4/IF5: hydrate-once detail snapshot + live deltas (same spec as the
  // classic workspace — one read model, two surfaces).
  const detailSpec = useMemo(() => makeTicketDetailSpec(api.api, ticketId), [ticketId]);
  const { data: model, loading, refresh, patch: patchModel } = useInstantCollection(
    ticketModelKey(ticketId),
    detailSpec,
  );
  const { mutate, stampEntity } = useInstantWrite();
  const [error, setError] = useState<string | null>(null);

  // Deep-link/hard-load recovery (same as classic).
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

  // Org directory (author names).
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

  // Header names: the client + contact are directory records, not org
  // users — resolve them for the meta row (typed gaps stay "—").
  const [clientName, setClientName] = useState<string | null>(null);
  const [contactName, setContactName] = useState<string | null>(null);
  const clientId = ticket?.client_id ?? null;
  const contactId = ticket?.contact_id ?? null;
  useEffect(() => {
    if (clientId === null) {
      setClientName(null);
      return;
    }
    let cancelled = false;
    void api.api
      .GET("/api/v1/clients/{id}", { params: { path: { id: clientId } } })
      .then(({ data }) => {
        if (!cancelled) setClientName(data?.name ?? null);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [clientId]);
  useEffect(() => {
    if (contactId === null || clientId === null) {
      setContactName(null);
      return;
    }
    let cancelled = false;
    void api.api
      .GET("/api/v1/clients/{id}/contacts", { params: { path: { id: clientId } } })
      .then(({ data }) => {
        if (!cancelled) setContactName((data ?? []).find((c) => c.id === contactId)?.name ?? null);
      })
      .catch(() => {
        if (!cancelled) setContactName(null);
      });
    return () => {
      cancelled = true;
    };
  }, [clientId, contactId]);

  // Related tickets (F-1 fix): the unified search over this ticket's title
  // words, ticket-kind hits only. Failures resolve to the typed empty
  // state — the tab never hangs in Loading.
  const [relatedTickets, setRelatedTickets] = useState<components["schemas"]["SearchResult"][]>([]);
  const [relatedLoading, setRelatedLoading] = useState(true);
  useEffect(() => {
    let cancelled = false;
    setRelatedLoading(true);
    const q = keywordsOf(ticket?.title ?? "");
    if (q === "") {
      setRelatedTickets([]);
      setRelatedLoading(false);
      return;
    }
    void api.api
      .GET("/api/v1/search", { params: { query: { q, limit: 10 } } })
      .then(({ data }) => {
        if (cancelled) return;
        setRelatedTickets(
          (data?.results ?? []).filter((r) => r.entity_kind === "ticket" && r.entity_id !== ticket?.id).slice(0, 5),
        );
        setRelatedLoading(false);
      })
      .catch(() => {
        if (!cancelled) {
          setRelatedTickets([]);
          setRelatedLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [ticket?.id, ticket?.title]);

  // Panel visibility: the context panel collapses as the view narrows —
  // persistent ≥1280px, drawer 1024–1279px, hidden below 1024px (the
  // binding constraint: never persistently visible in narrow layouts).
  const [narrow, setNarrow] = useState(false);
  const [midWidth, setMidWidth] = useState(false);
  const [contextOpen, setContextOpen] = useState(true);
  const [agentOpen, setAgentOpen] = useState(true);
  useEffect(() => {
    const update = () => {
      const w = window.innerWidth;
      setNarrow(w < 1024);
      setMidWidth(w >= 1024 && w < 1280);
      // Width-driven defaults: the context panel is persistent ≥1280px,
      // a toggleable DRAWER 1024–1279px, and overlay-only below 1024px —
      // never persistently visible in narrow layouts (binding constraint).
      setContextOpen(w >= 1280);
    };
    update();
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);

  // Day-pack pins for this ticket (best-effort; the strip hides on a
  // refused read and the pin action still writes).
  useEffect(() => {
    if (ticket !== null) void loadPicks(ticket.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ticket?.id]);

  // Tabs + writes.
  const [activeTab, setActiveTab] = useState<WbTab>("activity");

  // The pack's tab row carries figures, and Time carries a DURATION rather than
  // a count. Activity and Files are already on the read model; Time and Related
  // cost one small request each, issued once here rather than per tab open.
  const tabCounts = useTabCounts(
    ticket?.id ?? null,
    model === undefined || model === null
      ? null
      : {
          comments: Array.isArray(model.comments) ? model.comments.length : 0,
          attachments: Array.isArray(model.attachments) ? model.attachments.length : 0,
        },
  );
  const [commentBusy, setCommentBusy] = useState(false);
  const [wrapUpOpen, setWrapUpOpen] = useState(false);
  // Wave 17 close gate (typed 409 tasks_unassigned): the centered decision
  // dialog state — Option A batch + re-issue, Option B panel handoff.
  const [gateBlock, setGateBlock] = useState<GateBlocks | null>(null);
  const [moveBoardOpen, setMoveBoardOpen] = useState(false);
  const draftRef = useRef<string | null>(null);

  // "Take It" primary action: one self-assign → status → timer envelope.
  const { active: activeTimer, refresh: refreshTimers } = useTimer();
  const [takeItBusy, setTakeItBusy] = useState(false);
  const [partsOpen, setPartsOpen] = useState(false);
  const [pinOpen, setPinOpen] = useState(false);
  const [picks, setPicks] = useState<TicketPickRow[]>([]);
  const [partsReload, setPartsReload] = useState(0);
  const [partsReceipt, setPartsReceipt] = useState<PartsReceipt | null>(null);
  const [partsItems, setPartsItems] = useState<components["schemas"]["StockItemView"][]>([]);
  const [partsLocations, setPartsLocations] = useState<components["schemas"]["StockLocationView"][]>([]);
  const [partsLoadError, setPartsLoadError] = useState<string | null>(null);
  // Bumped on Take It; the Agent panel refreshes its data and reacts once.
  const [takeItNonce, setTakeItNonce] = useState(0);

  const iHaveRunningTimer =
    activeTimer !== null &&
    ticket !== null &&
    activeTimer.ticket_id === ticket.id &&
    activeTimer.stopped_at === null &&
    (activeTimer.segments ?? []).slice(-1)[0]?.ended_at == null;

  const takeItVisible = (() => {
    if (ticket === null || ticket.merged_into_ticket_id != null) return false;
    const category = ticket.category;
    if (category !== "open" && category !== "waiting") return false;
    if (takeItBusy || iHaveRunningTimer) return false;
    const meId = auth.me?.user_id ?? null;
    if (meId === null) return false;
    // Pickupable: unassigned, or already mine without a running timer.
    return ticket.assignee_id == null || ticket.assignee_id === meId;
  })();

  const mergedIntoId = ticket?.merged_into_ticket_id ?? null;
  const isMerged = mergedIntoId !== null;
  const canMoveBoard = !isMerged && ((auth.me?.permissions ?? []).includes("tickets:edit") ?? false);

  function patchDetail(updater: (m: TicketDetailModel) => TicketDetailModel): void {
    patchModel(updater);
  }

  
/** Unpack the typed 409 `tasks_unassigned` details: the exact unassigned
 * rows in checklist order (the dialog is a real list). */
function gateOf(details: unknown): GateBlocks["items"] {
  if (details !== null && typeof details === "object" && Array.isArray((details as { items?: unknown[] }).items)) {
    return (details as { items: GateBlocks["items"] }).items;
  }
  return [];
}

/** REQUIRED-item texts blocked by the pre-close gate on the same close
 * attempt — both surfaces stack in the dialog (§4.1). */
function requiredBlockedOf(details: unknown): string[] {
  if (details !== null && typeof details === "object") {
    const also = (details as { also_required?: unknown }).also_required;
    if (Array.isArray(also)) return also.map(String);
  }
  return [];
}

async function changeStatus(statusId: string): Promise<void> {
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
      if (result.reason === "tasks_unassigned" && ticket !== null) {
        // The workbar close gate: render the centered decision dialog —
        // never a bare prose error (§4.2). `details.items` carries the
        // exact unassigned rows; also stack any REQUIRED-item blockers
        // that rode the same close attempt.
        setGateBlock({
          ticketId: ticket.id,
          ticketNumber: String(ticket.number),
          items: gateOf(result.details),
          alsoRequired: requiredBlockedOf(result.details),
          statusId,
          onRetryClose: (sid) => changeStatus(sid),
          onDismiss: () => setGateBlock(null),
        });
        return;
      }
      setError(rejectionText(result, "That status change is not allowed by this board's workflow"));
      return;
    }
    setError(null);
  }

  // "Take It": one instant-first interaction — self-assign (optimistic on
  // the detail model), flip to the board's first in_progress status when
  // the workflow allows it, open the timer, then drop the eye on the
  // Activity composer. Each write is a real API call; any typed rejection
  // rolls back that step and surfaces the copy, later steps still settle.
  async function ensurePartsData(): Promise<boolean> {
    if (partsItems.length > 0 && partsLocations.length > 0) return true;
    const [itemsRes, locationsRes] = await Promise.all([
      api.api.GET("/api/v1/stock-items", { params: { query: { limit: 500 } } }),
      api.api.GET("/api/v1/stock-locations", { params: { query: {} } }),
    ]);
    if (itemsRes.data === undefined) {
      setError("Could not load stock items for the parts flow.");
      return false;
    }
    setPartsItems((itemsRes.data ?? []).filter((i) => i.active));
    setPartsLocations(locationsRes.data ?? []);
    return true;
  }

  async function openParts(): Promise<void> {
    setError(null);
    if (!(await ensurePartsData())) return;
    setPartsOpen(true);
  }

  /** Day-pack pinning: intent only, the ledger does not move here. */
  async function openPin(): Promise<void> {
    setError(null);
    if (!(await ensurePartsData())) return;
    setPinOpen(true);
  }

  async function loadPicks(ticketId: string): Promise<void> {
    try {
      setPicks(await fetchTicketPicks(ticketId));
    } catch {
      // A pins read failure never blocks the workbench; the strip stays
      // hidden and the action still writes.
    }
  }

  async function removePick(pickId: string): Promise<void> {
    const failure = await unpinItem(pickId);
    if (failure !== null) {
      setError(failure);
      return;
    }
    if (ticket !== null) void loadPicks(ticket.id);
  }

  async function takeIt(): Promise<void> {
    setTakeItBusy(true);
    try {
      // 1) Self-assign through PATCH (optimistic render + typed rollback,
      // changeStatus pattern).
      const before = ticket;
      if (ticket !== null && auth.me !== null) {
        const result = await mutate({
          describe: `Take ${ticket.number}`,
          entityId: ticket.id,
          method: "PATCH",
          path: `/api/v1/tickets/${ticket.id}`,
          buildBody: (base) => ({ assignee_id: auth.me !== null ? auth.me.user_id : null, ...base }),
          optimistic: () => {
            const meId = auth.me?.user_id;
            if (meId === undefined) return;
            patchDetail((m) => ({
              ...m,
              ticket: m.ticket === null ? m.ticket : { ...m.ticket, assignee_id: meId },
            }));
          },
          captureBefore: () => before,
          revert: () => {
            patchDetail((m) => ({
              ...m,
              ticket: m.ticket === null || before === null ? m.ticket : { ...m.ticket, ...before },
            }));
          },
        });
        if (result.status === "rejected") {
          setError(rejectionText(result, "Could not assign this ticket to you"));
          return;
        }

        // 2) Status: the board's first in_progress status, when the loaded
        // workflow allows the transition from here (skip when already
        // in_progress).
        const currentStatusId = ticket.status_id;
        const target =
          statuses.find((s) => s.category === "in_progress") ?? null;
        if (
          target !== null &&
          target.id !== currentStatusId &&
          currentStatusId !== null &&
          transitionAllowed(statuses, workflow, currentStatusId, target.id)
        ) {
          await changeStatus(target.id);
        }

        // 3) Timer: POST /timers/start through the envelope writer (the
        // timer-dock body/base pattern), then reconcile the bar state.
        const timer = await runEnvelopeMutation({
          describe: `Start timer on ${ticket.number}`,
          entityId: ticket.id,
          buildBody: (base) => ({ ticket_id: ticket.id, ...base }),
          optimistic: () => undefined,
          revert: () => undefined,
          request: (client, typed) =>
            client.POST("/api/v1/timers/start", { body: typed }),
        });
        if (!timer.ok) {
          setError(timer.reason ?? timer.friendly ?? "That timer did not start — try again.");
        }
        void refreshTimers();
      }
    } finally {
      setTakeItBusy(false);
      // 4) Writes settled — Activity tab, column scrolled to the top,
      // composer focused (motionless, geometry only).
      setActiveTab("activity");
      requestAnimationFrame(() => {
        const panel = document.querySelector<HTMLElement>('[data-testid="detail-panel-activity"]');
        if (panel !== null) panel.scrollTop = 0;
        document.querySelector<HTMLTextAreaElement>('[data-testid="notes-composer-body"]')?.focus();
      });
      // 5) Agent reaction — one real-data pointer, once.
      setTakeItNonce((n) => n + 1);
    }
  }

  async function postComment(
    body: string,
    kind: TicketComment["kind"],
    attachmentIds: string[],
    draftKey?: string | null,
    setWaitingAfterSend?: boolean,
  ): Promise<void> {
    if (ticket === null) return;
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
    setCommentBusy(true);
    try {
      const result = await mutate({
        describe: `Note on ${ticket.number}`,
        entityId: tempId,
        // The window-bus echo carries the REAL entity (a sibling window
        // re-snapshots models holding the ticket id) — the pending marker
        // matches nothing on the receiving side.
        echoEntities: [ticketId],
        method: "POST",
        path: `/api/v1/tickets/${ticketId}/comments`,
        buildBody: (base) => ({
          body,
          kind,
          attachment_ids: attachmentIds,
          draft_key: draftKey ?? null,
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
      if (result.status === "queued") return;
      const applied = result.mutation !== null ? snapshotOf(result.mutation).after : null;
      if (applied !== null && result.mutation !== null) {
        stampEntity(applied.id, result.mutation);
      }
      patchDetail((m) => ({
        ...m,
        comments: m.comments.map((c) =>
          c.id === tempId && applied !== null ? (applied as TicketComment) : c,
        ),
      }));
      // "Set to Waiting after send" (DESIGN-NOTES §4): a sent reply usually
      // means the next move belongs to the client, so the composer offers to
      // move the ticket with the note rather than making it a second
      // round trip. Only on a client-visible note — the composer does not
      // offer the box for an internal one, and this guard is what makes an
      // internal note genuinely unable to move the ticket.
      if (setWaitingAfterSend) {
        const waiting = statuses.find((s) => s.category === "waiting");
        if (waiting !== undefined && waiting.id !== applied?.status_id) {
          await changeStatus(waiting.id);
        }
      }
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not post note");
      throw e instanceof Error ? e : new Error("Could not post note");
    } finally {
      setCommentBusy(false);
    }
  }

  // Wrap-up bridge: the shell work bar asks this page first.
  useEffect(() => {
    registerWrapUpTarget((request) => {
      if (ticket === null) return;
      setWrapUpOpen(true);
      void request;
    });
    return () => registerWrapUpTarget(null);
  }, [ticket]);

  // Checklist (tasks): same tri-state semantics as the classic workspace —
  // optimistic add/check, envelope reconcile, typed rollback.
  async function addChecklistItem(text: string) {
    const tempId = `pending-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    const result = await mutate({
      describe: "Add checklist item",
      entityId: tempId,
      echoEntities: [ticketId],
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
                    ...(Array.isArray(m.checklist.items) ? m.checklist.items : []),
                    {
                      id: tempId,
                      text,
                      checked: false,
                      checked_at: null,
                      checked_by: null,
                      org_id: m.ticket?.org_id ?? "",
                      sort_order: (Array.isArray(m.checklist.items) ? m.checklist.items : []).length,
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
              : {
                  ...m.checklist,
                  items: (Array.isArray(m.checklist.items) ? m.checklist.items : []).filter(
                    (i) => i.id !== tempId,
                  ),
                },
        }));
      },
    });
    if (result.status === "rejected") {
      setError(rejectionText(result, "Could not add checklist item"));
      return;
    }
    if (result.status === "accepted" && result.mutation !== null) {
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
                  items: (Array.isArray(m.checklist.items) ? m.checklist.items : []).map((i) =>
                    i.id === tempId ? (after as components["schemas"]["ChecklistItem"]) : i,
                  ),
                },
        }));
        return;
      }
    }
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
                  items: (Array.isArray(m.checklist.items) ? m.checklist.items : []).map((i) =>
                    i.id === itemId ? { ...i, checked: state.checked, na: state.na } : i,
                  ),
                },
        }));
      },
      captureBefore: () => {
        const item = (Array.isArray(model?.checklist?.items) ? model?.checklist?.items : []).find((i) => i.id === itemId) ?? null;
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
                  items: (Array.isArray(m.checklist.items) ? m.checklist.items : []).map((i) =>
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
                  items: (Array.isArray(m.checklist.items) ? m.checklist.items : []).map((i) =>
                    i.id === itemId ? (after as components["schemas"]["ChecklistItem"]) : i,
                  ),
                },
        }));
      }
    }
    return { ok: true };
  }

  // Board move (Wave 11 lane 3 backend): optimistic render against the
  // target board's status set, authoritative reconcile from the envelope;
  // a typed `board_move_status_conflict` returns the target statuses so
  // the dialog renders an inline pick-list and retries with a status_id.
  const performBoardMove = useCallback(
    async (input: MoveBoardInput): Promise<MoveBoardOutcome> => {
      if (ticket === null) return { ok: false, message: "No ticket loaded." };
      const before = ticket;
      const beforeStatuses = statuses;
      const beforeWorkflow = workflow;
      const targetStatus =
        (input.statusId !== null
          ? (input.targetStatuses.find((s) => s.id === input.statusId) ?? null)
          : null) ?? input.autoStatus;
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
        return {
          ok: false,
          reason: result.reason,
          message: result.message,
          conflictStatuses:
            result.reason === "board_move_status_conflict"
              ? extractTargetStatuses(result.details)
              : undefined,
        };
      }
      return { ok: true };
    },
    [ticket, statuses, workflow, mutate],
  );

  if (auth.status !== "authed") {
    return (
      <main className="flex min-h-full items-center justify-center">
        <Spinner />
      </main>
    );
  }

  const sla = model?.sla ?? null;
  const current = statuses.find((s) => s.id === ticket?.status_id) ?? null;
  const statusTone =
    current?.category === "closed" ? "success" : current?.category === "waiting" ? "warning" : "neutral";
  const slaMeta = slaMetaText(sla);
  // At mid widths (1024–1279) both side panels would leave too little
  // center — the Agent panel keeps persistence, Context moves to a
  // toggleable drawer, and the header actions collapse to icons so the
  // pill row stays single-line.
  const showContext = contextOpen && !narrow && !midWidth;
  const showAgent = agentOpen && !narrow;
  const iconOnlyActions = narrow || midWidth;

  const renderCenter = (): ReactNode => {
    if (ticket === null) {
      return (
        <div className="flex h-full items-center justify-center">
          {loading ? (
            <Spinner />
          ) : (
            <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>
              Ticket not found in your organization
            </p>
          )}
        </div>
      );
    }
    return (
      <div className="m-wb-center relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        {/* Top color+text status bar (standing constraint) — transitions
            happen FROM the bar, popover grouped by semantic category. */}
        <TicketStatusBar
          number={ticket.number}
          title={ticket.title}
          statusId={ticket.status_id}
          statuses={statuses}
          workflow={workflow}
          sla={sla}
          onChangeStatus={(id) => void changeStatus(id)}
          onMoveBoard={canMoveBoard ? () => setMoveBoardOpen(true) : undefined}
        />

        {/* Header */}
        <div className="shrink-0 border-b px-5 pb-3 pt-3" style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}>
          <div className="flex items-center gap-3">
            <div
              className="flex min-w-0 items-center gap-1.5 whitespace-nowrap text-[11px]"
              style={{ color: "var(--m-text-muted)" }}
              data-testid="wb-breadcrumbs"
            >
              <Link href="/tickets" className="no-underline" style={{ color: "var(--m-text-muted)" }}>‹ Tickets</Link>
              <span>/</span>
              <strong className="" style={{ color: "var(--m-text-secondary)" }}>{ticket.number}</strong>
            </div>
            <div className="ml-auto flex flex-wrap justify-end gap-1.5">
              {takeItVisible && (
                <button
                type="button"
                data-testid="wb-take-it"
                className="m-wb-take-it flex h-[30px] shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-lg border px-2.5 leading-none text-[12px] font-semibold"
                style={{
                  borderColor: "var(--m-accent)",
                  background: "var(--m-accent)",
                  color: "var(--m-on-accent)",
                }}
                title="Assign to me, move to In Progress, start the timer"
                onClick={() => void takeIt()}
              >
                <span
                  aria-hidden
                  className="grid h-[13px] w-[13px] place-items-center rounded-full"
                  style={{ border: "1.5px solid currentColor" }}
                >
                  <span
                    aria-hidden
                    className="ml-[2px] h-0 w-0"
                    style={{
                      borderTop: "2.5px solid transparent",
                      borderBottom: "2.5px solid transparent",
                      borderLeft: "4.5px solid currentColor",
                    }}
                  />
                </span>
                {iconOnlyActions ? "Take" : "Take It"}
              </button>
              )}
              <button
                type="button"
                data-testid="wb-parts-used"
                disabled={isMerged}
                className="flex h-[30px] shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-lg border px-2.5 leading-none text-[12px] font-semibold disabled:cursor-not-allowed"
                style={{
                  borderColor: "var(--m-border)",
                  background: "var(--m-surface)",
                  color: isMerged ? "var(--m-text-muted)" : "var(--m-text-secondary)",
                }}
                title="Parts &amp; shipping — use on the job, ship to the client, return stock, print labels"
                onClick={() => void openParts()}
              >
                {iconOnlyActions ? "◫" : "Parts & shipping"}
              </button>
              <button
                type="button"
                data-testid="wb-pin-day-pack"
                disabled={isMerged}
                className="flex h-[30px] shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-lg border px-2.5 leading-none text-[12px] font-semibold disabled:cursor-not-allowed"
                style={{
                  borderColor: "var(--m-border)",
                  background: "var(--m-surface)",
                  color: isMerged ? "var(--m-text-muted)" : "var(--m-text-secondary)",
                }}
                title="Pin a tool or consumable to this ticket's day pack (intent only — the ledger does not move)"
                onClick={() => void openPin()}
              >
                {iconOnlyActions ? "⊕" : "Pin to day pack"}
              </button>
              <Link
                href={`/tickets/classic/${ticket.id}`}
                data-testid="wb-open-classic"
                title={iconOnlyActions ? "Classic workspace" : "Open the preserved classic workspace for comparison"}
                className="no-underline"
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  height: 30,
                  padding: "0 10px",
                  borderRadius: "var(--m-radius-control)",
                  border: "1px solid var(--m-border)",
                  background: "var(--m-surface)",
                  color: "var(--m-text-secondary)",
                  fontWeight: 600,
                  fontSize: 12,
                  whiteSpace: "nowrap",
                }}
              >
                {iconOnlyActions ? "◫" : "Classic"}
              </Link>
              <PopoutButton iconOnly={iconOnlyActions} params={{ id: ticket.id }} />
              <button
                type="button"
                className="flex h-[30px] shrink-0 cursor-pointer items-center whitespace-nowrap rounded-lg border px-2.5 text-[12px] font-semibold"
                style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", color: "var(--m-text-secondary)" }}
                title="More actions"
              >
                •••
              </button>
              <button
                type="button"
                className="flex h-[30px] shrink-0 cursor-pointer items-center whitespace-nowrap rounded-lg border px-2.5 text-[12px] font-semibold"
                style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", color: "var(--m-text-secondary)" }}
                onClick={() => {
                  void navigator.clipboard?.writeText(`${window.location.origin}/tickets/${ticket.id}`);
                  setError(null);
                }}
                title="Copy a link to this ticket"
              >
                {iconOnlyActions ? "↗" : "↗ Link"}
              </button>
              <button
                type="button"
                disabled={isMerged}
                className="flex h-[30px] shrink-0 cursor-pointer items-center whitespace-nowrap rounded-lg border px-2.5 text-[12px] font-semibold disabled:cursor-not-allowed"
                style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", color: isMerged ? "var(--m-text-muted)" : "var(--m-text-secondary)" }}
                title="Escalate: move to the escalation board through the board-move flow"
                onClick={() => {
                  if (canMoveBoard) setMoveBoardOpen(true);
                  else setError("Moving a ticket to another board requires ticket edit permission.");
                }}
              >
                {iconOnlyActions ? "✦" : "✦ Escalate"}
              </button>
              <button
                type="button"
                data-testid="wb-close-ticket"
                disabled={isMerged}
                onClick={() => setWrapUpOpen(true)}
                className="flex h-[30px] shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-lg border px-2.5 leading-none text-[12px] font-semibold disabled:cursor-not-allowed"
                style={{
                  borderColor: "var(--m-accent)",
                  background: isMerged ? "var(--m-accent-muted)" : "var(--m-accent)",
                  color: "var(--m-on-accent)",
                }}
              >
                <span
                  aria-hidden
                  className="grid h-[13px] w-[13px] place-items-center rounded-full"
                  style={{ border: "1.5px solid currentColor" }}
                >
                  <span className="h-[5px] w-[5px] rounded-full" style={{ background: "currentColor" }} />
                </span>
                Close
              </button>
            </div>
          </div>

          {picks.length > 0 && (
            <div
              data-testid="wb-day-pack-pins"
              className="flex flex-wrap items-center gap-1.5 text-[11px]"
              style={{ color: "var(--m-text-muted)" }}
            >
              <span className="font-semibold uppercase tracking-wide text-[10px]">Day pack</span>
              {picks.map((pick) => (
                <span
                  key={pick.id}
                  className="inline-flex items-center gap-1 rounded-[3px] border px-1.5 py-0.5"
                  style={{ borderColor: "var(--m-border)", color: "var(--m-text-secondary)" }}
                >
 <span className="tabular-nums">{qtyText(pick.qty_milli)}×</span>
                  <span>{pick.item_name ?? "Item"}</span>
                  {pick.is_tool && <span className="text-[10px]">tool</span>}
                  <button
                    type="button"
                    aria-label={`Unpin ${pick.item_name ?? "item"}`}
                    data-testid="wb-day-pack-unpin"
                    className="cursor-pointer"
                    style={{ color: "var(--m-text-muted)" }}
                    onClick={() => void removePick(pick.id)}
                  >
                    ×
                  </button>
                </span>
              ))}
            </div>
          )}

          <h1
            className="mb-0 mt-2.5 text-[26px] font-semibold leading-tight tracking-[-0.03em]"
            style={{ color: "var(--m-text)" }}
            data-testid="wb-ticket-title"
          >
            {ticket.number} · {ticket.title}
          </h1>

          {/* A merged ticket stays queryable by doctrine but is closed and
              never re-opens, so the page has to say so in the first thing the
              reader sees. Gating below (take-it, move-board) is already keyed
              off `isMerged`; this is the sentence that explains why those
              controls are missing rather than broken. */}
          {mergedIntoId !== null ? (
            <div className="mt-2.5 overflow-hidden rounded-[var(--m-radius-container)] border" style={{ borderColor: "var(--m-border)" }}>
              <MergeBanner survivorId={mergedIntoId} />
            </div>
          ) : null}

          <div className="mt-2.5 flex flex-wrap items-center gap-3">
            <span className="flex items-center gap-1.5 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              <strong style={{ color: "var(--m-text)" }}>{clientName ?? (ticket.client_id != null ? "Client" : "No client")}</strong>
            </span>
            <span aria-hidden className="hidden h-4 w-px md:block" style={{ background: "var(--m-border)" }} />
            <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              Contact: <strong style={{ color: "var(--m-text)" }}>{contactName ?? "—"}</strong>
            </span>
            {priorityPill(ticket.priority)}
            {slaMeta !== null && (
              <span className="flex items-center gap-1.5 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                ◷ <strong style={{ color: "var(--m-text)" }}>{slaMeta.replace(" SLA remaining", "")}</strong>
                <span>SLA remaining</span>
              </span>
            )}
            <span
              className="inline-flex h-[22px] items-center gap-1.5 rounded-full px-2.5 text-[10px] font-bold"
              style={{
                background: `var(--m-status-${statusTone === "success" ? "success" : statusTone === "warning" ? "warning" : "neutral"}-bg)`,
                color: `var(--m-status-${statusTone === "success" ? "success" : statusTone === "warning" ? "warning" : "neutral"}-fg)`,
              }}
            >
              <span aria-hidden className="h-1.5 w-1.5 rounded-full" style={{ background: "currentColor" }} />
              {current?.name ?? "Unknown status"}
            </span>
          </div>

          {/* The stepper the spec asks for (DESIGN-NOTES §4): the current step
              in gold, earlier steps checked, any step settable. It sits under
              the meta row rather than inside it because it is a control and
              the row above it is a fact list. */}
          {statuses.length > 1 ? (
            <div className="mt-3">
              <StatusStepper
                steps={statuses.map((s) => ({ id: s.id, label: s.name }))}
                currentId={current?.id ?? ""}
                onSelect={(id) => void changeStatus(id)}
                label="Ticket status"
              />
            </div>
          ) : null}

          {/* Decision workbench suggestion card (Wave 23 lane 3): inline
              AI triage suggestions — suggestions-by-default, pending state
              below threshold, silently absent with the decision model off. */}
          <DecisionSuggestionsCard ticketId={ticket.id} onApplied={() => void refresh()} />
        </div>

        {/* Tabs */}
        <div
          role="tablist"
          aria-label="Ticket sections"
          className="flex shrink-0 items-end gap-4 border-b px-5"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", height: 39 }}
        >
          {TABS.map((tab) => {
            const active = activeTab === tab.id;
            return (
              <button
                key={tab.id}
                type="button"
                role="tab"
                aria-selected={active}
                data-testid={`detail-tab-${tab.id}`}
                className="m-wb-tab cursor-pointer border-0 bg-transparent pb-2.5 text-[11px] font-semibold"
                style={{
                  color: active ? "var(--m-text)" : "var(--m-text-muted)",
                  borderBottom: active ? "2px solid var(--m-accent)" : "2px solid transparent",
                  marginBottom: -1,
                }}
                onClick={() => setActiveTab(tab.id)}
              >
                {tab.label}
                {tabCounts[tab.id] !== undefined ? (
                  <span className="m-wb-tab-count">{tabCounts[tab.id]}</span>
                ) : null}
              </button>
            );
          })}
        </div>

        {/* Tab panels — the floating composer overlays the bottom edge
            (mockup grammar), so every panel keeps a composer-height
            clearance before its content ends. */}
        <div
          className="min-h-0 flex-1 overflow-y-auto px-5 pt-3.5"
          style={{ paddingBottom: 176 }}
          data-testid={`detail-panel-${activeTab}`}
        >
          {activeTab === "activity" && (
            <WbTimeline
              ticketId={ticket.id}
              comments={model?.comments ?? []}
              attachments={model?.attachments ?? []}
              namesByUser={namesByUser}
              reloadToken={partsReload}
            />
          )}
          {activeTab === "details" && (
            <div className="grid content-start gap-4">
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
                <dt style={{ color: "var(--m-text-muted)" }}>Priority</dt>
                <dd className="uppercase">{ticket.priority}</dd>
                <dt style={{ color: "var(--m-text-muted)" }}>Assignee</dt>
                <dd>{ticket.assignee_id == null ? "Unassigned" : namesByUser.get(ticket.assignee_id) ?? ticket.assignee_id.slice(0, 8)}</dd>
                <dt style={{ color: "var(--m-text-muted)" }}>Created</dt>
                <dd>{new Date(ticket.created_at ?? Date.now()).toLocaleString()}</dd>
                {ticket.resolved_at != null && (
                  <>
                    <dt style={{ color: "var(--m-text-muted)" }}>Resolved</dt>
                    <dd>{new Date(ticket.resolved_at).toLocaleString()}</dd>
                  </>
                )}
              </dl>
              <TasksPanel
                testPrefix="tab-"
                items={Array.isArray(model?.checklist?.items) ? model.checklist.items : []}
                enforcementActive={model?.checklist?.enforcement_active ?? false}
                blockingCount={(Array.isArray(model?.checklist?.blocking) ? model?.checklist?.blocking : []).length ?? 0}
                onSetState={setChecklistItemState}
                onAdd={addChecklistItem}
              />
              {/* Staged run (workbench-v2 plan): the scheduling pod mounts in
                  the Details tab — it is read/send-only (no note surface), so
                  it cannot conflict with the floating composer. */}
              <SchedulingPod ticketId={ticket.id} ticketTitle={ticket.title} />
              {/* Custom fields and tags used to be a link away in the classic
                  workspace. The value payload turned out to be
                  self-describing, so the fields cost no fetch at all and the
                  tags cost one, paid only when this tab is opened. */}
              <CustomFieldsPanel fields={ticket.custom_fields} />
              <TagsPanel ticketId={ticket.id} />
            </div>
          )}
          {activeTab === "related" && (
            <div className="grid content-start gap-2">
              {/* The explicit link tree (parent/child, major-incident members)
                  and its rollup. This used to be a link away in the classic
                  workspace; similarity search below is a different question —
                  "what else mentions this" rather than "what is attached". */}
              <TicketLinksPanel ticketId={ticket.id} />
              <h2 className="text-[11px] font-bold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                Similar tickets
              </h2>
              <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                Tickets sharing this one&apos;s words, from the unified search.
              </p>
              {relatedLoading ? (
                <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>Searching…</p>
              ) : relatedTickets.length === 0 ? (
                <p className="text-xs" data-testid="wb-related-empty" style={{ color: "var(--m-text-muted)" }}>
                  No related tickets found for this ticket&apos;s words.
                </p>
              ) : (
                relatedTickets.map((h) => (
                  <Link
                    key={h.entity_id}
                    href={`/tickets/${h.entity_id}`}
                    className="block overflow-hidden rounded-lg border p-2.5 no-underline"
                    style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)" }}
                  >
                    <strong className="block text-xs" style={{ color: "var(--m-text)" }}>{h.title}</strong>
                    <span className="mt-0.5 block line-clamp-2 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                      {h.snippet}
                    </span>
                  </Link>
                ))
              )}
            </div>
          )}
          {activeTab === "time" && (
            <div>
              <WbTimeline
                ticketId={ticket.id}
                comments={[]}
                attachments={[]}
                namesByUser={namesByUser}
              />
              <p className="mt-2 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                Showing the time entries from the unified stream; filter with the Time chip on the Activity tab for full context.
              </p>
            </div>
          )}
          {activeTab === "files" && (
            <div className="grid content-start gap-2">
              {(model?.attachments ?? []).length === 0 ? (
                <p className="text-xs" data-testid="wb-files-empty" style={{ color: "var(--m-text-muted)" }}>
                  No files on this ticket yet — attach through the composer.
                </p>
              ) : (
                (model?.attachments ?? []).map((a) => (
                  <div key={a.id} className="flex items-center gap-2 text-xs" style={{ color: "var(--m-text-secondary)" }}>
                    <Icon name="clip" size={12} />
                    {a.filename} · {Math.max(1, Math.round(a.size_bytes / 1024))} KB
                    <TranscribeControl attachmentId={a.id} contentType={a.content_type} />
                  </div>
                ))
              )}
            </div>
          )}
          {activeTab === "notes" && (
            <div className="grid content-start gap-2">
              {(model?.comments ?? []).length === 0 ? (
                <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>No notes yet.</p>
              ) : (
                (model?.comments ?? []).map((c) => (
                  <article key={c.id} className="grid gap-1 rounded-lg border p-2.5 text-xs" style={{ borderColor: "var(--m-border)" }}>
                    <header className="flex flex-wrap items-center gap-2">
                      <Badge tone={c.kind === "client_visible" ? "success" : c.kind === "in_house_update" ? "warning" : "neutral"}>
                        {c.kind === "client_visible" ? "Client-visible" : c.kind === "in_house_update" ? "In-house" : "Internal"}
                      </Badge>
                      <span className="font-medium">{namesByUser.get(c.author_id ?? "") ?? "Unknown author"}</span>
                      <time className="ml-auto" style={{ color: "var(--m-text-muted)" }}>
                        {new Date(c.created_at).toLocaleString()}
                      </time>
                    </header>
                    <div className="whitespace-pre-wrap" style={{ color: "var(--m-text-secondary)" }}>{c.body}</div>
                  </article>
                ))
              )}
            </div>
          )}
          {error !== null && (
            <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
              {error}
            </p>
          )}
          {partsReceipt !== null && (
            <div
              className="grid gap-2 rounded-lg border p-2.5 text-xs"
              style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
              role="status"
              data-testid="wb-parts-receipt"
            >
              <p
                style={{ color: "var(--m-status-success-fg)" }}
                data-testid="wb-parts-notice"
              >
                {receiptSentence(
                  partsReceipt,
                  partsItems.find((i) => i.id === partsReceipt.movement.stock_item_id),
                  clientName,
                )}
              </p>
              <div className="flex flex-wrap items-center gap-2">
                {partsReceipt.kind === "ship-out" &&
                  (((partsReceipt.movement.carrier != null && partsReceipt.movement.carrier !== "") ||
                    (partsReceipt.movement.tracking_ref != null &&
                      partsReceipt.movement.tracking_ref !== "")) && (
                    <span className="" style={{ color: "var(--m-text-secondary)" }}>
                      {[partsReceipt.movement.carrier, partsReceipt.movement.tracking_ref]
                        .filter((part) => part != null && part !== "")
                        .join(" · ")}
                    </span>
                  ))}
                {partsReceipt.kind === "ship-out" && (
                  <LabelPrintButton
                    purpose={partsReceipt.movement.serial_id != null ? "serialized_unit" : "stock_item"}
                    entityId={partsReceipt.movement.serial_id ?? partsReceipt.movement.stock_item_id}
                    entityLabel={
                      partsItems.find((i) => i.id === partsReceipt.movement.stock_item_id)?.name ??
                      "Shipped item"
                    }
                  />
                )}
                <Button
                  variant="ghost"
                  size="sm"
                  data-testid="wb-parts-receipt-dismiss"
                  onClick={() => setPartsReceipt(null)}
                >
                  Dismiss
                </Button>
              </div>
            </div>
          )}
        </div>

        {/* Floating composer */}
        <WbComposer
          ticketId={ticket.id}
          disabled={isMerged}
          busy={commentBusy}
          draftRef={draftRef}
          onPost={postComment}
        />
      </div>
    );
  };

  return (
    <div className="flex h-full min-h-0">
      {/* Left: Ticket Context — collapses as the view narrows */}
      {showContext && (
        <WbContextPanel
          clientId={ticket?.client_id ?? null}
          contactId={ticket?.contact_id ?? null}
          ticketId={ticketId}
          ticketNumber={ticket?.number ?? ""}
          ticketTitle={ticket?.title ?? ""}
          open={showContext}
          onClose={() => setContextOpen(false)}
        />
      )}

      {/* Center */}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {(narrow || midWidth) && (
          <div className="flex shrink-0 items-center gap-2 border-b px-3 py-1.5" style={{ borderColor: "var(--m-border)" }}>
            <button
              type="button"
              data-testid="wb-open-context"
              className="m-wb-chip cursor-pointer text-[10.5px]"
              onClick={() => setContextOpen(true)}
            >
              ☰ Context
            </button>
            <button
              type="button"
              data-testid="wb-open-agent"
              className="m-wb-chip cursor-pointer text-[10.5px]"
              onClick={() => setAgentOpen(true)}
            >
              ✦ Agent
            </button>
          </div>
        )}
        {renderCenter()}
      </div>

      {/* Right: Agent panel */}
      {showAgent && ticket !== null && (
        <WbAgentPanel
          ticket={ticket}
          comments={model?.comments ?? []}
          namesByUser={namesByUser}
          open={showAgent}
          onClose={() => setAgentOpen(false)}
          takeItNonce={takeItNonce}
          onDraftUpdate={(body) => {
            draftRef.current = body;
          }}
        />
      )}

      {/* Drawer overlays (narrow + mid widths) */}
      {(narrow || midWidth) && contextOpen && !showContext && (
        <div className="fixed inset-0 z-40 flex" style={{ background: "var(--m-scrim)" }} onClick={() => setContextOpen(false)}>
          <div className="h-full" style={{ width: 300, background: "var(--m-surface)" }} onClick={(e) => e.stopPropagation()}>
            <WbContextPanel
              clientId={ticket?.client_id ?? null}
              contactId={ticket?.contact_id ?? null}
              ticketId={ticketId}
              ticketNumber={ticket?.number ?? ""}
          ticketTitle={ticket?.title ?? ""}
              open
              onClose={() => setContextOpen(false)}
            />
          </div>
        </div>
      )}

      {gateBlock !== null && (
        <TasksGateDialog {...gateBlock} />
      )}

      {ticket !== null && (
        <WrapUpSheet
          open={wrapUpOpen}
          onCloseSheet={() => setWrapUpOpen(false)}
          ticketId={ticket.id}
          ticketNumber={String(ticket.number)}
          statuses={statuses}
          workflow={workflow}
          statusId={ticket.status_id}
          onChangeStatus={changeStatus}
          onRefresh={() => void refresh()}
          clientName={clientName}
        />
      )}

      {ticket !== null && (
        <MovementDialog
          open={partsOpen}
          onOpenChange={setPartsOpen}
          items={partsItems}
          locations={partsLocations}
          clients={
            clientName !== null && ticket.client_id != null
              ? [{ id: ticket.client_id, name: clientName }]
              : []
          }
          initialKind="consume"
          initialClientId={ticket.client_id ?? undefined}
          ticketId={ticket.id}
          ticketLabel={`${ticket.number} · ${ticket.title}`}
          scanSurface="ticket-parts"
          searchableItems
          title="Parts & shipping"
          onRecorded={(movement, recordedKind) => {
            setPartsReceipt({ kind: recordedKind ?? "consume", movement });
            setPartsReload((n) => n + 1);
          }}
        />
      )}

      {ticket !== null && (
        <PickDialog
          open={pinOpen}
          onOpenChange={setPinOpen}
          ticketId={ticket.id}
          ticketLabel={`${ticket.number} · ${ticket.title}`}
          items={partsItems}
          onPinned={() => {
            setPinOpen(false);
            void loadPicks(ticket.id);
          }}
        />
      )}

      {ticket !== null && canMoveBoard && (
        <MoveBoardDialog
          open={moveBoardOpen}
          onOpenChange={setMoveBoardOpen}
          ticket={ticket}
          onMove={performBoardMove}
          onMoved={() => void refresh()}
        />
      )}
    </div>
  );
}
