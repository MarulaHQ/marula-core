// Read models — the four IF4 collections as store specs.
//
// Purpose: per-collection cheap snapshots (hydrate), relevance predicates
// (which live events touch this collection), and delta revalidations
// (scoped refetches or payload merges — never a whole-view reload). Each
// factory binds one spec to the SDK client so tests can drive it with a
// mock transport.
//
// Non-purpose: no caching (InstantStore owns lifecycles), no React, no
// transport policy.

import type { components } from "@meridian/sdk";
import { MeridianClient } from "@meridian/sdk";
import type { InstantEvent } from "./events";
import type { ReadModelSpec } from "./instant-store";

/** The SDK client's typed query surface (openapi-fetch per-path overloads). */
export type ApiClient = MeridianClient["api"];

export type BoardTicket = components["schemas"]["Ticket"];
export type BoardStatus = components["schemas"]["BoardStatus"];
export type TicketDetail = components["schemas"]["TicketWithFields"];
export type TicketComment = components["schemas"]["TicketComment"];
export type TicketChecklistView = components["schemas"]["TicketChecklistView"];
export type TicketSlaStatus = components["schemas"]["TicketSlaStatus"];
export type Attachment = components["schemas"]["Attachment"];
export type BoardTransitionsResponse = components["schemas"]["BoardTransitionsResponse"];
export type MyWorkItem = components["schemas"]["MyWorkItem"];
export type CalendarView = components["schemas"]["CalendarViewResponse"];

/** Board read model: the status pipeline plus the board's ticket queue. */
export interface BoardModel {
  statuses: BoardStatus[];
  tickets: BoardTicket[];
}

export const boardModelKey = (boardId: string): string => `board:${boardId}`;

export function makeBoardSpec(
  client: ApiClient,
  boardId: string,
): ReadModelSpec<BoardModel> {
  const boardChannel = `board:${boardId}`;
  async function fetchTicket(id: string): Promise<BoardTicket | null> {
    const { data } = await client.GET("/api/v1/tickets/{id}", {
      params: { path: { id } },
    });
    return data ?? null;
  }
  function upsert(model: BoardModel, ticket: BoardTicket): BoardModel {
    const idx = model.tickets.findIndex((t) => t.id === ticket.id);
    if (idx < 0) return { ...model, tickets: [...model.tickets, ticket] };
    if (model.tickets[idx] === ticket) return model;
    const tickets = [...model.tickets];
    tickets[idx] = ticket;
    return { ...model, tickets };
  }
  return {
    async hydrate() {
      const [statuses, tickets] = await Promise.all([
        client.GET("/api/v1/boards/{id}/statuses", { params: { path: { id: boardId } } }),
        client.GET("/api/v1/tickets", {
          params: { query: { board_id: boardId, limit: 100 } },
        }),
      ]);
      return { statuses: statuses.data ?? [], tickets: tickets.data ?? [] };
    },
    relevant: (event) =>
      event.channel === boardChannel &&
      (event.entity === "ticket" || event.entity === "board"),
    async revalidate(model, event, ctx) {
      if (event.entity === "board") {
        const { data } = await client.GET("/api/v1/boards/{id}/statuses", {
          params: { path: { id: boardId } },
        });
        return data === undefined ? model : { ...model, statuses: data };
      }
      // ticket event on this board's channel: patch just that ticket.
      const id = event.entity_id;
      if (id === null || ctx.isPending(id)) return model;
      const ticket = await fetchTicket(id);
      if (ticket === null) {
        // Deleted/closed-out-of-scope or unknown: one scoped re-snapshot.
        const { data } = await client.GET("/api/v1/tickets", {
          params: { query: { board_id: boardId, limit: 100 } },
        });
        return data === undefined ? model : { ...model, tickets: data };
      }
      return upsert(model, ticket);
    },
    applyEntity: (model, entity) => {
      const ticket = entity as BoardTicket;
      if (typeof ticket?.id !== "string") return model;
      return upsert(model, ticket);
    },
    entityIds: (model) => model.tickets.map((t) => t.id),
  };
}

/** Ticket-detail read model: the workspace's center + activity panes. */
export interface TicketDetailModel {
  ticket: TicketDetail | null;
  comments: TicketComment[];
  checklist: TicketChecklistView | null;
  sla: TicketSlaStatus | null;
  attachments: Attachment[];
  statuses: BoardStatus[];
  workflow: BoardTransitionsResponse;
}

export const ticketModelKey = (ticketId: string): string => `ticket:${ticketId}`;

export function makeTicketDetailSpec(
  client: ApiClient,
  ticketId: string,
): ReadModelSpec<TicketDetailModel> {
  async function fetchTicketBundle(): Promise<
    Pick<TicketDetailModel, "ticket" | "sla">
  > {
    const [t, sla] = await Promise.all([
      client.GET("/api/v1/tickets/{id}", { params: { path: { id: ticketId } } }),
      client.GET("/api/v1/tickets/{id}/sla", { params: { path: { id: ticketId } } }),
    ]);
    return { ticket: t.data ?? null, sla: sla.data ?? null };
  }
  return {
    async hydrate() {
      const [bundle, comments, checklist, attachments] = await Promise.all([
        fetchTicketBundle(),
        client.GET("/api/v1/tickets/{id}/comments", {
          params: { path: { id: ticketId } },
        }),
        client.GET("/api/v1/tickets/{id}/checklist", {
          params: { path: { id: ticketId } },
        }),
        client.GET("/api/v1/tickets/{id}/attachments", {
          params: { path: { id: ticketId } },
        }),
      ]);
      const boardId = bundle.ticket?.board_id ?? null;
      let statuses: BoardStatus[] = [];
      let workflow: BoardTransitionsResponse = { mode: "floor", edges: [] };
      if (boardId !== null) {
        const [s, tr] = await Promise.all([
          client.GET("/api/v1/boards/{id}/statuses", { params: { path: { id: boardId } } }),
          client.GET("/api/v1/boards/{id}/transitions", { params: { path: { id: boardId } } }),
        ]);
        if (s.error === undefined) statuses = s.data ?? [];
        if (tr.error === undefined && tr.data !== undefined) workflow = tr.data;
      }
      return {
        ...bundle,
        comments: comments.data ?? [],
        checklist: checklist.data ?? null,
        attachments: attachments.data ?? [],
        statuses,
        workflow,
      };
    },
    relevant: (event) => {
      if (event.entity === "ticket" && event.entity_id === ticketId) return true;
      if (event.entity === "ticket_comment") {
        const payload = event.payload as
          | { ticket_id?: string; comment?: { ticket_id?: string } }
          | null;
        return payload?.ticket_id === ticketId || payload?.comment?.ticket_id === ticketId;
      }
      if (event.entity === "attachment") {
        return (
          (event.payload as { ticket_id?: string } | null)?.ticket_id === ticketId
        );
      }
      return false;
    },
    async revalidate(model, event, ctx) {
      if (event.entity === "ticket") {
        // Checklist items ride ticket.* events with their full payload;
        // the checklist view itself refetches on checklist_updated.
        const isChecklistUpdate = event.type === "ticket.checklist_updated";
        if (!isChecklistUpdate) {
          if (ctx.isPending(ticketId)) return model;
          const bundle = await fetchTicketBundle();
          return { ...model, ...bundle };
        }
        const { data } = await client.GET("/api/v1/tickets/{id}/checklist", {
          params: { path: { id: ticketId } },
        });
        return { ...model, checklist: data ?? null };
      }
      if (event.entity === "ticket_comment") {
        // comment_added carries the full comment — merge without a fetch.
        const comment = (event.payload as { comment?: TicketComment } | null)?.comment;
        if (comment !== undefined && typeof comment?.id === "string") {
          if (model.comments.some((c) => c.id === comment.id)) return model;
          return { ...model, comments: [...model.comments, comment] };
        }
        const { data } = await client.GET("/api/v1/tickets/{id}/comments", {
          params: { path: { id: ticketId } },
        });
        return { ...model, comments: data ?? model.comments };
      }
      // attachment event
      const { data } = await client.GET("/api/v1/tickets/{id}/attachments", {
        params: { path: { id: ticketId } },
      });
      return { ...model, attachments: data ?? model.attachments };
    },
    applyEntity: (model, entity) => {
      const ticket = entity as TicketDetail;
      if (typeof ticket?.id !== "string" || model.ticket === null) return model;
      // Merge the applied snapshot over the enriched detail shape.
      return { ...model, ticket: { ...model.ticket, ...ticket } };
    },
    entityIds: (model) => (model.ticket !== null ? [model.ticket.id] : []),
  };
}

/** My-work list read model: the cross-board queue behind /today and the workspace pane. */
export const MY_WORK_KEY = "my-work";

export function makeMyWorkSpec(client: ApiClient): ReadModelSpec<MyWorkItem[]> {
  return {
    async hydrate() {
      const { data } = await client.GET("/api/v1/tickets/my-work");
      return data ?? [];
    },
    relevant: (event) => event.entity === "ticket",
    async revalidate(items) {
      const { data } = await client.GET("/api/v1/tickets/my-work");
      return data ?? items;
    },
    entityIds: (items) => items.map((i) => i.id),
  };
}

/** Calendar-week read model: one scope/week per cache key. */
export const calendarModelKey = (scope: string, week: string): string =>
  `calendar:${scope}:${week}`;

export function makeCalendarSpec(
  client: ApiClient,
  params: { mode: "personal" | "team"; date: string; from: string; to: string },
): ReadModelSpec<CalendarView | null> {
  return {
    async hydrate() {
      if (params.mode === "personal") {
        const { data } = await client.GET("/api/v1/calendar/week", {
          params: { query: { date: params.date } },
        });
        return (data as CalendarView | undefined) ?? null;
      }
      const { data } = await client.GET("/api/v1/calendar/team", {
        params: { query: { from: params.from, to: params.to } },
      });
      return (data as CalendarView | undefined) ?? null;
    },
    relevant: (event) =>
      event.entity === "appointment" || event.entity === "availability",
    async revalidate() {
      // Appointments change per week window; a scoped refetch of this
      // view is the delta (the store coalesces bursts).
      if (params.mode === "personal") {
        const { data } = await client.GET("/api/v1/calendar/week", {
          params: { query: { date: params.date } },
        });
        return (data as CalendarView | undefined) ?? null;
      }
      const { data } = await client.GET("/api/v1/calendar/team", {
        params: { query: { from: params.from, to: params.to } },
      });
      return (data as CalendarView | undefined) ?? null;
    },
    entityIds: () => [],
  };
}

/** Narrows an event payload's ticket id, when present. */
export function payloadTicketId(event: InstantEvent): string | null {
  const id = (event.payload as { ticket_id?: string } | null)?.ticket_id;
  return typeof id === "string" ? id : null;
}
