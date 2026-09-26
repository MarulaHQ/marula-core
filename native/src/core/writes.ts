// Typed write operations for the foundation surfaces.
//
// Purpose: declare every offline-capable write ONCE — its typed body
// builder (idempotency key + If-Match version injected), its ordering
// scope, the cache keys it invalidates, and its typed SDK call. The sync
// engine queues these; on replay it re-sends the persisted body through the
// same typed sender. Bodies are built against the generated SDK types, so a
// contract drift fails typecheck (docs/rn-groundwork.md rule 1).
//
// Non-purpose: no queue or cache state here — pure operation declarations.

import type { MeridianClient } from "@meridian/sdk";

import { MODELS, type TicketComment } from "./models";
import type { OpMethod } from "./queue";
import type { TransportResult } from "./http";
import type { ReadCache } from "./cache";

export interface MutationBase {
  idempotencyKey: string;
  version?: number;
}

/** In-session hooks for optimistic render + authoritative reconcile. */
export interface WriteContext {
  cache: ReadCache;
  uuid: () => string;
  now: () => Date;
}

export interface WriteOperation<A, B extends Record<string, unknown>> {
  kind: string;
  label: (args: A) => string;
  entityKey: (args: A) => string;
  entityId?: (args: A) => string;
  method: OpMethod;
  path: (args: A) => string;
  invalidate: (args: A) => string[];
  build: (args: A, base: MutationBase) => B;
  send: (client: MeridianClient, body: B) => Promise<TransportResult<unknown>>;
  versionOf?: (data: unknown) => number | undefined;
  /** Synchronous local render before the wire (never awaited by the UI). */
  optimistic?: (args: A, base: MutationBase, ctx: WriteContext) => Promise<void>;
  /** Undo the optimistic render on a typed rejection. */
  revert?: (args: A, base: MutationBase, ctx: WriteContext) => Promise<void>;
  /** Reconcile the optimistic render from the server's mutation snapshot. */
  apply?: (data: unknown, args: A, base: MutationBase, ctx: WriteContext) => Promise<void>;
}

export interface AddNoteArgs {
  ticketId: string;
  ticketNumber?: string;
  body: string;
  kind?: "internal" | "client_visible" | "in_house_update";
}

export type AddNoteBody = {
  /** Internal: the queue replays from the persisted body alone, so the
   * entity id rides the body. `send` never forwards it to the server. */
  __path_id: string;
  body: string;
  kind: "internal" | "client_visible" | "in_house_update";
  idempotency_key: string;
};

export const addTicketNote: WriteOperation<AddNoteArgs, AddNoteBody> = {
  kind: "ticket.add_note",
  label: (args) =>
    `Note on ${args.ticketNumber !== undefined ? args.ticketNumber : "ticket"}`,
  entityKey: (args) => `ticket:${args.ticketId}`,
  entityId: (args) => `ticket:${args.ticketId}`,
  method: "POST",
  path: (args) => `/api/v1/tickets/${args.ticketId}/comments`,
  invalidate: (args) => [
    MODELS.ticketComments(args.ticketId),
    MODELS.ticket(args.ticketId),
    MODELS.myWork,
  ],
  build: (args, base) => ({
    __path_id: args.ticketId,
    body: args.body,
    kind: args.kind ?? "internal",
    idempotency_key: base.idempotencyKey,
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/tickets/{id}/comments", {
      params: { path: { id: pathId(body) } },
      body: { body: body.body, kind: body.kind, idempotency_key: body.idempotency_key },
    }),
  versionOf: (data) => mutationVersion(data),
  optimistic: async (args, base, ctx) => {
    await ctx.cache.patch<TicketComment[]>(
      MODELS.ticketComments(args.ticketId),
      (current) => [...(current ?? []), pendingComment(args, base.idempotencyKey, ctx)],
    );
  },
  revert: async (args, base, ctx) => {
    const pendingId = pendingCommentId(base.idempotencyKey);
    await ctx.cache.patch<TicketComment[]>(MODELS.ticketComments(args.ticketId), (current) =>
      (current ?? []).filter((comment) => comment.id !== pendingId),
    );
  },
  apply: async (data, args, base, ctx) => {
    const pendingId = pendingCommentId(base.idempotencyKey);
    const authoritative = data as { id: string } | undefined;
    await ctx.cache.patch<TicketComment[]>(MODELS.ticketComments(args.ticketId), (current) => {
      const rest = (current ?? []).filter((comment) => comment.id !== pendingId);
      if (authoritative === undefined || typeof authoritative.id !== "string") return rest;
      return [...rest, data as TicketComment];
    });
  },
};

export interface TimerStartArgs {
  ticketId: string;
  ticketNumber?: string;
}

export type TimerStartBody = { ticket_id: string; idempotency_key: string };

export const startTimer: WriteOperation<TimerStartArgs, TimerStartBody> = {
  kind: "timer.start",
  label: (args) =>
    `Start timer on ${args.ticketNumber !== undefined ? args.ticketNumber : "ticket"}`,
  entityKey: () => "timer",
  method: "POST",
  path: () => "/api/v1/timers/start",
  invalidate: (args) => [MODELS.activeTimer, MODELS.myWork, MODELS.ticket(args.ticketId)],
  build: (args, base) => ({
    ticket_id: args.ticketId,
    idempotency_key: base.idempotencyKey,
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/timers/start", {
      body: { ticket_id: body.ticket_id, idempotency_key: body.idempotency_key },
    }),
  versionOf: (data) => mutationVersion(data),
};

export interface TimerSessionArgs {
  sessionId: string;
  ticketId?: string;
  reason?: string;
  notes?: string;
}

export type TimerStopBody = {
  session_id: string;
  notes?: string;
  reason?: string;
  idempotency_key: string;
  version?: number;
};

export const stopTimer: WriteOperation<TimerSessionArgs, TimerStopBody> = {
  kind: "timer.stop",
  label: () => "Stop timer",
  entityKey: () => "timer",
  // A stop writes the TIME ENTRY, not the session; keeping its version
  // cursor on the session key would send a stale version the server
  // rejects (the session's version is not the entry's).
  entityId: () => "time_entry",
  method: "POST",
  path: () => "/api/v1/timers/stop",
  invalidate: (args) => [
    MODELS.activeTimer,
    MODELS.myWork,
    ...(args.ticketId !== undefined ? [MODELS.ticket(args.ticketId)] : []),
  ],
  build: (args, base) => ({
    session_id: args.sessionId,
    ...(args.notes !== undefined ? { notes: args.notes } : {}),
    ...(args.reason !== undefined ? { reason: args.reason } : {}),
    idempotency_key: base.idempotencyKey,
    ...(base.version !== undefined ? { version: base.version } : {}),
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/timers/stop", {
      body: {
        session_id: body.session_id,
        ...(body.notes !== undefined ? { notes: body.notes } : {}),
        ...(body.reason !== undefined ? { reason: body.reason } : {}),
        idempotency_key: body.idempotency_key,
        version: body.version ?? null,
      },
    }),
  versionOf: (data) => mutationVersion(data),
};

export type TimerToggleBody = {
  session_id: string;
  reason?: string;
  idempotency_key: string;
};

export const pauseTimer: WriteOperation<TimerSessionArgs, TimerToggleBody> = {
  kind: "timer.pause",
  label: () => "Pause timer",
  entityKey: () => "timer",
  method: "POST",
  path: () => "/api/v1/timers/pause",
  invalidate: () => [MODELS.activeTimer],
  build: (args, base) => ({
    session_id: args.sessionId,
    ...(args.reason !== undefined ? { reason: args.reason } : {}),
    idempotency_key: base.idempotencyKey,
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/timers/pause", {
      body: {
        session_id: body.session_id,
        ...(body.reason !== undefined ? { reason: body.reason } : {}),
        idempotency_key: body.idempotency_key,
      },
    }),
};

export const resumeTimer: WriteOperation<TimerSessionArgs, TimerToggleBody> = {
  kind: "timer.resume",
  label: () => "Resume timer",
  entityKey: () => "timer",
  method: "POST",
  path: () => "/api/v1/timers/resume",
  invalidate: () => [MODELS.activeTimer],
  build: (args, base) => ({
    session_id: args.sessionId,
    idempotency_key: base.idempotencyKey,
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/timers/resume", {
      body: { session_id: body.session_id, idempotency_key: body.idempotency_key },
    }),
};

export interface ConsumeStockArgs {
  itemId: string;
  itemName?: string;
  locationId: string;
  qtyMilli: number;
  /** Serialized consumption moves ONE unit. */
  serialId?: string;
  serialNumber?: string;
  ticketId?: string;
  note?: string;
}

export type ConsumeStockBody = {
  item_id: string;
  location_id: string;
  qty_milli: number;
  serial_id?: string;
  ticket_id?: string;
  note?: string;
  idempotency_key: string;
};

export const consumeStock: WriteOperation<ConsumeStockArgs, ConsumeStockBody> = {
  kind: "stock.consume",
  label: (args) =>
    `Consume ${args.itemName !== undefined ? args.itemName : "stock"}`,
  entityKey: (args) => `stock:${args.itemId}`,
  method: "POST",
  path: () => "/api/v1/stock/consume",
  invalidate: (args) => [
    MODELS.onHand,
    MODELS.stockItem(args.itemId),
    MODELS.stockMovements(args.itemId),
    MODELS.stockSerials,
    MODELS.toolsAvailability(args.itemId),
    ...(args.ticketId !== undefined ? [MODELS.ticket(args.ticketId)] : []),
  ],
  build: (args, base) => ({
    item_id: args.itemId,
    location_id: args.locationId,
    qty_milli: args.qtyMilli,
    ...(args.serialId !== undefined ? { serial_id: args.serialId } : {}),
    ...(args.ticketId !== undefined ? { ticket_id: args.ticketId } : {}),
    ...(args.note !== undefined ? { note: args.note } : {}),
    idempotency_key: base.idempotencyKey,
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/stock/consume", {
      body: {
        item_id: body.item_id,
        location_id: body.location_id,
        qty_milli: body.qty_milli,
        serial_id: body.serial_id ?? null,
        ticket_id: body.ticket_id ?? null,
        note: body.note ?? null,
        idempotency_key: body.idempotency_key,
      },
    }),
  versionOf: (data) => mutationVersion(data),
};

export interface TransferStockArgs {
  itemId: string;
  itemName?: string;
  fromLocationId: string;
  fromLocationName?: string;
  toLocationId: string;
  toLocationName?: string;
  qtyMilli: number;
  /** Serialized transfers move exactly one physical unit. */
  serialId?: string;
  serialNumber?: string;
  note?: string;
}

export type TransferStockBody = {
  item_id: string;
  from_location_id: string;
  to_location_id: string;
  qty_milli: number;
  serial_id?: string;
  note?: string;
  idempotency_key: string;
};

/** Van ↔ shelf transfer: two locations, one movement leg (Wave 32 lane 6
 * grammar; the ledger row is the trace). */
export const transferStock: WriteOperation<TransferStockArgs, TransferStockBody> = {
  kind: "stock.transfer",
  label: (args) =>
    `Move ${args.itemName !== undefined ? args.itemName : "stock"}${
      args.fromLocationName !== undefined && args.toLocationName !== undefined
        ? ` · ${args.fromLocationName} → ${args.toLocationName}`
        : ""
    }`,
  entityKey: (args) => `stock:${args.itemId}`,
  method: "POST",
  path: () => "/api/v1/stock/transfer",
  invalidate: (args) => [
    MODELS.onHand,
    MODELS.stockItem(args.itemId),
    MODELS.stockMovements(args.itemId),
    MODELS.stockSerials,
    MODELS.toolsAvailability(args.itemId),
    MODELS.location(args.fromLocationId),
    MODELS.location(args.toLocationId),
  ],
  build: (args, base) => ({
    item_id: args.itemId,
    from_location_id: args.fromLocationId,
    to_location_id: args.toLocationId,
    qty_milli: args.qtyMilli,
    ...(args.serialId !== undefined ? { serial_id: args.serialId } : {}),
    ...(args.note !== undefined ? { note: args.note } : {}),
    idempotency_key: base.idempotencyKey,
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/stock/transfer", {
      body: {
        item_id: body.item_id,
        from_location_id: body.from_location_id,
        to_location_id: body.to_location_id,
        qty_milli: body.qty_milli,
        serial_id: body.serial_id ?? null,
        note: body.note ?? null,
        idempotency_key: body.idempotency_key,
      },
    }),
  versionOf: (data) => mutationVersion(data),
};

export interface ReceiveStockArgs {
  itemId: string;
  itemName?: string;
  locationId: string;
  locationName?: string;
  qtyMilli: number;
  /** Serialized receives carry one entry per physical unit (usually one). */
  serialNumbers?: string[];
  /** The on-site job the arrival belongs to (shipment-marker confirm). */
  ticketId?: string;
  note?: string;
}

export type ReceiveStockBody = {
  item_id: string;
  location_id: string;
  qty_milli: number;
  serial_numbers?: string[];
  ticket_id?: string;
  note?: string;
  idempotency_key: string;
};

/** Confirm goods arriving on site (a shipment marker → receive movement). */
export const receiveStock: WriteOperation<ReceiveStockArgs, ReceiveStockBody> = {
  kind: "stock.receive",
  label: (args) => `Receive ${args.itemName !== undefined ? args.itemName : "stock"}`,
  entityKey: (args) => `stock:${args.itemId}`,
  method: "POST",
  path: () => "/api/v1/stock/receive",
  invalidate: (args) => [
    MODELS.onHand,
    MODELS.stockItem(args.itemId),
    MODELS.stockMovements(args.itemId),
    MODELS.stockSerials,
    MODELS.toolsAvailability(args.itemId),
    MODELS.location(args.locationId),
    ...(args.ticketId !== undefined
      ? [MODELS.ticketStockMovements(args.ticketId), MODELS.ticket(args.ticketId)]
      : []),
  ],
  build: (args, base) => ({
    item_id: args.itemId,
    location_id: args.locationId,
    qty_milli: args.qtyMilli,
    ...(args.serialNumbers !== undefined && args.serialNumbers.length > 0
      ? { serial_numbers: args.serialNumbers }
      : {}),
    ...(args.ticketId !== undefined ? { ticket_id: args.ticketId } : {}),
    ...(args.note !== undefined ? { note: args.note } : {}),
    idempotency_key: base.idempotencyKey,
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/stock/receive", {
      body: {
        item_id: body.item_id,
        location_id: body.location_id,
        qty_milli: body.qty_milli,
        serial_numbers: body.serial_numbers ?? null,
        ticket_id: body.ticket_id ?? null,
        note: body.note ?? null,
        idempotency_key: body.idempotency_key,
      },
    }),
  versionOf: (data) => mutationVersion(data),
};

export interface CreateCheckoutArgs {
  holderUserId: string;
  fromLocationId: string;
  dueAt: string;
  /** Individual units; the serial is optional (server auto-picks). */
  items: { itemId: string; serialId?: string }[];
  kitId?: string;
  notes?: string;
  /** Label override (a kit checkout says "kit", not "tools"). */
  label?: string;
}

export type CreateCheckoutBody = {
  holder_user_id: string;
  from_location_id: string;
  due_at: string;
  items: { stock_item_id: string; serial_id?: string }[];
  kit_id?: string;
  notes?: string;
  idempotency_key: string;
};

export const checkoutTools: WriteOperation<CreateCheckoutArgs, CreateCheckoutBody> = {
  kind: "tool.checkout",
  label: (args) => args.label ?? "Check out tools",
  entityKey: () => "tools",
  method: "POST",
  path: () => "/api/v1/tool-checkouts",
  invalidate: (args) => [
    MODELS.toolCheckouts,
    MODELS.onHand,
    MODELS.stockSerials,
    ...(args.kitId !== undefined ? [MODELS.kit(args.kitId), MODELS.kitAvailability(args.kitId)] : []),
    ...args.items.map((item) => MODELS.toolsAvailability(item.itemId)),
  ],
  build: (args, base) => ({
    holder_user_id: args.holderUserId,
    from_location_id: args.fromLocationId,
    due_at: args.dueAt,
    items: args.items.map((item) => ({
      stock_item_id: item.itemId,
      ...(item.serialId !== undefined ? { serial_id: item.serialId } : {}),
    })),
    ...(args.kitId !== undefined ? { kit_id: args.kitId } : {}),
    ...(args.notes !== undefined ? { notes: args.notes } : {}),
    idempotency_key: base.idempotencyKey,
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/tool-checkouts", {
      body: {
        holder_user_id: body.holder_user_id,
        from_location_id: body.from_location_id,
        due_at: body.due_at,
        items: body.items.map((item) => ({
          stock_item_id: item.stock_item_id,
          serial_id: item.serial_id ?? null,
        })),
        kit_id: body.kit_id ?? null,
        notes: body.notes ?? null,
        idempotency_key: body.idempotency_key,
      },
    }),
  versionOf: (data) => nestedVersion(data, "checkout"),
};

export interface CheckinLine {
  /** The tool-checkout line this scan returns. */
  lineId: string;
  /** good | fair | damaged (the field-ops default is good). */
  condition?: "good" | "fair" | "damaged";
  notes?: string;
}

export interface CheckinArgs {
  checkoutId: string;
  toLocationId?: string;
  notes?: string;
  /** Empty/absent = every outstanding unit (the whole-transaction return). */
  lines?: CheckinLine[];
}

export type CheckinBody = {
  __path_id: string;
  to_location_id?: string;
  notes?: string;
  lines?: CheckinLine[];
  idempotency_key: string;
};

export const checkinTools: WriteOperation<CheckinArgs, CheckinBody> = {
  kind: "tool.checkin",
  label: () => "Return tools",
  entityKey: (args) => `toolcheckout:${args.checkoutId}`,
  method: "POST",
  path: (args) => `/api/v1/tool-checkouts/${args.checkoutId}/checkin`,
  invalidate: () => [MODELS.toolCheckouts, MODELS.onHand, MODELS.stockSerials, MODELS.toolsAvailability()],
  build: (args, base) => ({
    __path_id: args.checkoutId,
    ...(args.toLocationId !== undefined ? { to_location_id: args.toLocationId } : {}),
    ...(args.notes !== undefined ? { notes: args.notes } : {}),
    ...(args.lines !== undefined && args.lines.length > 0 ? { lines: args.lines } : {}),
    idempotency_key: base.idempotencyKey,
  }),
  send: (client, body) => {
    const id = checkoutIdFromBody(body);
    return client.api.POST("/api/v1/tool-checkouts/{id}/checkin", {
      params: { path: { id } },
      body: {
        to_location_id: body.to_location_id ?? null,
        notes: body.notes ?? null,
        idempotency_key: body.idempotency_key,
        lines: (body.lines ?? []).map((line) => ({
          line_id: line.lineId,
          return_condition: line.condition ?? "good",
          return_notes: line.notes ?? null,
        })),
      },
    });
  },
  versionOf: (data) => nestedVersion(data, "checkout"),
};

export interface NotificationReadArgs {
  notificationId: string;
}

export type NotificationReadBody = { __path_id: string; idempotency_key: string };

export const markNotificationRead: WriteOperation<NotificationReadArgs, NotificationReadBody> = {
  kind: "notification.read",
  label: () => "Mark notification read",
  entityKey: (args) => `notification:${args.notificationId}`,
  method: "POST",
  path: (args) => `/api/v1/notifications/${args.notificationId}/read`,
  invalidate: () => [MODELS.notifications, MODELS.unreadCount],
  build: (args, base) => ({ __path_id: args.notificationId, idempotency_key: base.idempotencyKey }),
  send: (client, body) => {
    const id = notificationIdFromBody(body);
    return client.api.POST("/api/v1/notifications/{id}/read", {
      params: { path: { id } },
    });
  },
};

export const markAllNotificationsRead: WriteOperation<Record<string, never>, { idempotency_key: string }> =
  {
    kind: "notification.read_all",
    label: () => "Mark all notifications read",
    entityKey: () => "notifications",
    method: "POST",
    path: () => "/api/v1/notifications/read-all",
    invalidate: () => [MODELS.notifications, MODELS.unreadCount],
    build: (_args, base) => ({ idempotency_key: base.idempotencyKey }),
    send: (client) => client.api.POST("/api/v1/notifications/read-all"),
  };

/** All operations the replay engine can reconstruct after relaunch. */
export const WRITE_OPERATIONS = {
  [addTicketNote.kind]: addTicketNote,
  [startTimer.kind]: startTimer,
  [stopTimer.kind]: stopTimer,
  [pauseTimer.kind]: pauseTimer,
  [resumeTimer.kind]: resumeTimer,
  [consumeStock.kind]: consumeStock,
  [transferStock.kind]: transferStock,
  [receiveStock.kind]: receiveStock,
  [checkoutTools.kind]: checkoutTools,
  [checkinTools.kind]: checkinTools,
  [markNotificationRead.kind]: markNotificationRead,
  [markAllNotificationsRead.kind]: markAllNotificationsRead,
} as const;

export type WriteOperationKind = keyof typeof WRITE_OPERATIONS;

function mutationVersion(data: unknown): number | undefined {
  const version = (data as { mutation?: { version?: unknown } } | undefined)?.mutation
    ?.version;
  return typeof version === "number" ? version : undefined;
}

/** The optimistic placeholder a queued note renders until the server
 * snapshot replaces it (docs/instant-first.md: optimistic render,
 * authoritative reconcile). */
function pendingCommentId(idempotencyKey: string): string {
  return `pending-${idempotencyKey}`;
}

function pendingComment(
  args: AddNoteArgs,
  idempotencyKey: string,
  ctx: WriteContext,
): TicketComment {
  return {
    id: pendingCommentId(idempotencyKey),
    org_id: "",
    ticket_id: args.ticketId,
    author_id: null,
    body: args.body,
    kind: args.kind ?? "internal",
    created_at: ctx.now().toISOString(),
    updated_at: null,
  } as TicketComment;
}

function nestedVersion(data: unknown, key: string): number | undefined {
  const nested = (data as Record<string, unknown> | undefined)?.[key];
  const version = (nested as { mutation?: { version?: unknown } } | undefined)?.mutation
    ?.version;
  if (typeof version === "number") return version;
  return mutationVersion(data);
}

/** Persisted bodies carry their entity id (`__path_id`) so replay can
 * rebuild the path without the original args. `send` strips it. */
function pathId(body: Record<string, unknown>): string {
  const explicit = body["__path_id"];
  if (typeof explicit === "string") return explicit;
  throw new Error("queued ticket note op is missing __path_id");
}

function checkoutIdFromBody(body: Record<string, unknown>): string {
  const id = body["__path_id"];
  if (typeof id === "string") return id;
  throw new Error("queued tool.checkin op is missing __path_id");
}

function notificationIdFromBody(body: Record<string, unknown>): string {
  const id = body["__path_id"];
  if (typeof id === "string") return id;
  throw new Error("queued notification.read op is missing __path_id");
}
