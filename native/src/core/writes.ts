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

import type { MarulaClient } from "@marula-core/sdk";

import {
  MODELS,
  type AppointmentRow,
  type DayPack,
  type DayPackTick,
  type DispatchDay,
  type TicketComment,
  type TravelDraft,
} from "./models";
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
  send: (client: MarulaClient, body: B) => Promise<TransportResult<unknown>>;
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
  /** Uploaded-against-this-ticket attachment ids (upload-first flow: the
   * comment links files the composer staged before submit). */
  attachmentIds?: string[];
}

export type AddNoteBody = {
  /** Internal: the queue replays from the persisted body alone, so the
   * entity id rides the body. `send` never forwards it to the server. */
  __path_id: string;
  body: string;
  kind: "internal" | "client_visible" | "in_house_update";
  attachment_ids?: string[];
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
    ...(args.attachmentIds !== undefined && args.attachmentIds.length > 0
      ? { attachment_ids: args.attachmentIds }
      : {}),
    idempotency_key: base.idempotencyKey,
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/tickets/{id}/comments", {
      params: { path: { id: pathId(body) } },
      body: {
        body: body.body,
        kind: body.kind,
        ...(body.attachment_ids !== undefined ? { attachment_ids: body.attachment_ids } : {}),
        idempotency_key: body.idempotency_key,
      },
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
  billable?: boolean;
  workType?: string;
}

export type TimerStopBody = {
  session_id: string;
  notes?: string;
  reason?: string;
  idempotency_key: string;
  version?: number;
  billable?: boolean;
  work_type?: string;
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
    ...(args.billable !== undefined ? { billable: args.billable } : {}),
    ...(args.workType !== undefined ? { work_type: args.workType } : {}),
    idempotency_key: base.idempotencyKey,
    ...(base.version !== undefined ? { version: base.version } : {}),
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/timers/stop", {
      body: {
        session_id: body.session_id,
        ...(body.notes !== undefined ? { notes: body.notes } : {}),
        ...(body.reason !== undefined ? { reason: body.reason } : {}),
        ...(body.billable !== undefined ? { billable: body.billable } : {}),
        ...(body.work_type !== undefined ? { work_type: body.work_type } : {}),
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

/** Checklist toggle args: who/what/whether. */
export interface CheckToggleArgs {
  ticketId: string;
  itemId: string;
  checked: boolean;
}

/** POST /api/v1/tickets/{id}/checklist/{item_id} — one tri-state toggle. */
export const setChecklistItemChecked: WriteOperation<CheckToggleArgs, {
  __path_id: string;
  __path_item_id: string;
  checked: boolean;
  idempotency_key: string;
}> = {
  kind: "ticket.checklist_toggle",
  label: (args) => `Checklist item on ticket`,
  entityKey: (args) => `ticket:${args.ticketId}`,
  entityId: (args) => `ticket:${args.ticketId}`,
  method: "POST",
  path: (args) => `/api/v1/tickets/${args.ticketId}/checklist/${args.itemId}`,
  invalidate: (args) => [MODELS.ticketChecklist(args.ticketId), MODELS.ticket(args.ticketId)],
  build: (args, base) => ({
    __path_id: args.ticketId,
    __path_item_id: args.itemId,
    checked: args.checked,
    idempotency_key: base.idempotencyKey,
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/tickets/{id}/checklist/{item_id}", {
      params: { path: { id: body.__path_id, item_id: body.__path_item_id } },
      body: { checked: body.checked, idempotency_key: body.idempotency_key },
    }),
  versionOf: (data) => mutationVersion(data),
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


/** The local calendar date `YYYY-MM-DD` — the day pack's tick key is a local
 * date, not a UTC instant. */
export function localDateString(now: Date = new Date()): string {
  const month = `${now.getMonth() + 1}`.padStart(2, "0");
  const day = `${now.getDate()}`.padStart(2, "0");
  return `${now.getFullYear()}-${month}-${day}`;
}

export interface TickDayPackArgs {
  lineKey: string;
  date?: string;
  ticketId?: string;
  note?: string;
}

export type TickDayPackBody = {
  line_key: string;
  date: string;
  ticket_id?: string;
  note?: string;
  idempotency_key: string;
};

function patchDayPackTick(
  cache: ReadCache,
  lineKey: string,
  tick: DayPackTick | null,
): void {
  void cache.patch<DayPack>(MODELS.dayPack(), (current) => {
    if (current === undefined) return current as unknown as DayPack;
    const patchLine = <T extends { line_key: string; tick?: DayPackTick | null }>(line: T): T =>
      line.line_key === lineKey ? { ...line, tick } : line;
    return {
      ...current,
      trips: current.trips.map((trip) => ({
        ...trip,
        lines: trip.lines.map(patchLine),
        kits: trip.kits.map(patchLine),
      })),
    };
  });
}

/** Tick a pack line. The tick is a plan note — it never touches the movement
 * ledger (Wave 32 lane 8's rule); the phone's Today list mirrors that. */
export const tickDayPack: WriteOperation<TickDayPackArgs, TickDayPackBody> = {
  kind: "daypack.tick",
  label: () => "Tick the grab list",
  entityKey: (args) => `daypack:${args.lineKey}`,
  method: "POST",
  path: () => "/api/v1/day-pack/ticks",
  invalidate: (args) => [
    MODELS.dayPack(),
    ...(args.ticketId !== undefined ? [MODELS.ticket(args.ticketId)] : []),
  ],
  build: (args, base) => ({
    line_key: args.lineKey,
    date: args.date ?? localDateString(),
    ...(args.ticketId !== undefined ? { ticket_id: args.ticketId } : {}),
    ...(args.note !== undefined ? { note: args.note } : {}),
    idempotency_key: base.idempotencyKey,
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/day-pack/ticks", {
      body: {
        line_key: body.line_key,
        date: body.date,
        ticket_id: body.ticket_id ?? null,
        note: body.note ?? null,
        idempotency_key: body.idempotency_key,
      },
    }),
  optimistic: async (args, base, ctx) => {
    patchDayPackTick(ctx.cache, args.lineKey, {
      line_key: args.lineKey,
      ticked_at: ctx.now().toISOString(),
      ...(args.note !== undefined ? { note: args.note } : {}),
    });
    void base;
  },
  revert: async (args, _base, ctx) => {
    patchDayPackTick(ctx.cache, args.lineKey, null);
  },
  apply: async (data, args, _base, ctx) => {
    const tick = (data as { tick?: DayPackTick } | undefined)?.tick;
    patchDayPackTick(ctx.cache, args.lineKey, tick ?? null);
  },
};

export interface UntickDayPackArgs {
  lineKey: string;
  date?: string;
  ticketId?: string;
}

export type UntickDayPackBody = {
  line_key: string;
  date: string;
};

/** Un-tick: the plan note the tick wrote goes away; the ledger never had it. */
export const untickDayPack: WriteOperation<UntickDayPackArgs, UntickDayPackBody> = {
  kind: "daypack.untick",
  label: () => "Un-tick the grab list",
  entityKey: (args) => `daypack:${args.lineKey}`,
  method: "DELETE",
  path: () => "/api/v1/day-pack/ticks",
  invalidate: (args) => [
    MODELS.dayPack(),
    ...(args.ticketId !== undefined ? [MODELS.ticket(args.ticketId)] : []),
  ],
  build: (args) => ({
    line_key: args.lineKey,
    date: args.date ?? localDateString(),
  }),
  send: (client, body) =>
    client.api.DELETE("/api/v1/day-pack/ticks", {
      params: { query: { date: body.date, line_key: body.line_key } },
    }),
  optimistic: async (args, _base, ctx) => {
    patchDayPackTick(ctx.cache, args.lineKey, null);
  },
  revert: async (args, _base, ctx) => {
    patchDayPackTick(ctx.cache, args.lineKey, {
      line_key: args.lineKey,
      ticked_at: ctx.now().toISOString(),
    });
  },
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

export type AppointmentStatusValue = AppointmentRow["status"];

export interface AppointmentStatusArgs {
  appointmentId: string;
  appointmentTitle?: string;
  /** Local date of the day view that owns the booking (the cache key). */
  date: string;
  status: Extract<AppointmentStatusValue, "in_progress" | "done">;
  /** State to restore if the write is rejected (optimistic revert). */
  previousStatus: AppointmentStatusValue;
}

export type AppointmentStatusBody = {
  __path_id: string;
  status: Extract<AppointmentStatusValue, "in_progress" | "done">;
};

/** Accept a dispatched booking (start it) or finish it.
 *
 * PATCH /appointments/{id} carries no IF envelope on the wire (the handler
 * takes a plain PatchAppointmentRequest), so this op sends the status alone;
 * the queue still orders it per appointment and the local day model renders
 * the flip before the server echo arrives. */
export const setAppointmentStatus: WriteOperation<AppointmentStatusArgs, AppointmentStatusBody> =
  {
    kind: "appointment.status",
    label: (args) =>
      `${args.status === "in_progress" ? "Accept" : "Finish"} ${
        args.appointmentTitle !== undefined ? args.appointmentTitle : "booking"
      }`,
    entityKey: (args) => `appointment:${args.appointmentId}`,
    entityId: (args) => `appointment:${args.appointmentId}`,
    method: "PATCH",
    path: (args) => `/api/v1/appointments/${args.appointmentId}`,
    invalidate: (args) => [MODELS.dispatchDay(args.date)],
    build: (args) => ({ __path_id: args.appointmentId, status: args.status }),
    send: (client, body) =>
      client.api.PATCH("/api/v1/appointments/{id}", {
        params: { path: { id: body.__path_id } },
        body: { status: body.status },
      }),
    optimistic: async (args, _base, ctx) => {
      await patchCached<DispatchDay>(ctx, MODELS.dispatchDay(args.date), (current) =>
        withAppointmentStatus(current, args.appointmentId, args.status),
      );
    },
    revert: async (args, _base, ctx) => {
      await patchCached<DispatchDay>(ctx, MODELS.dispatchDay(args.date), (current) =>
        withAppointmentStatus(current, args.appointmentId, args.previousStatus),
      );
    },
    apply: async (data, args, _base, ctx) => {
      const status = (data as { status?: AppointmentStatusValue } | undefined)?.status;
      if (status === undefined) return;
      await patchCached<DispatchDay>(ctx, MODELS.dispatchDay(args.date), (current) =>
        withAppointmentStatus(current, args.appointmentId, status),
      );
    },
  };

export interface TravelDraftCreateArgs {
  ticketId: string;
  ticketNumber?: string;
  appointmentId?: string;
  startedAt: string;
  endedAt: string;
  distance: number;
  distanceUnit: string;
  vehicle: string;
  billable: boolean;
  notes?: string;
}

export type TravelDraftCreateBody = {
  ticket_id: string;
  appointment_id?: string;
  started_at: string;
  ended_at: string;
  distance: number;
  distance_unit: string;
  vehicle: string;
  billable: boolean;
  notes?: string;
  idempotency_key: string;
};

export const createTravelDraft: WriteOperation<TravelDraftCreateArgs, TravelDraftCreateBody> = {
  kind: "travel.create",
  label: (args) =>
    `Log travel${args.ticketNumber !== undefined ? ` for ${args.ticketNumber}` : ""}`,
  entityKey: () => "travel",
  method: "POST",
  path: () => "/api/v1/dispatch/travel-drafts",
  invalidate: () => [MODELS.travelDrafts],
  build: (args, base) => ({
    ticket_id: args.ticketId,
    ...(args.appointmentId !== undefined ? { appointment_id: args.appointmentId } : {}),
    started_at: args.startedAt,
    ended_at: args.endedAt,
    distance: args.distance,
    distance_unit: args.distanceUnit,
    vehicle: args.vehicle,
    billable: args.billable,
    ...(args.notes !== undefined && args.notes.trim() !== "" ? { notes: args.notes } : {}),
    idempotency_key: base.idempotencyKey,
  }),
  send: (client, body) =>
    client.api.POST("/api/v1/dispatch/travel-drafts", {
      body: {
        ticket_id: body.ticket_id,
        appointment_id: body.appointment_id ?? null,
        started_at: body.started_at,
        ended_at: body.ended_at,
        distance: body.distance,
        distance_unit: body.distance_unit,
        vehicle: body.vehicle,
        billable: body.billable,
        notes: body.notes ?? null,
        idempotency_key: body.idempotency_key,
      },
    }),
  versionOf: (data) => nestedVersion(data, "draft"),
  optimistic: async (args, base, ctx) => {
    await ctx.cache.patch<TravelDraft[]>(MODELS.travelDrafts, (current) => [
      pendingTravelDraft(args, base.idempotencyKey, ctx),
      ...(current ?? []),
    ]);
  },
  revert: async (_args, base, ctx) => {
    const pendingId = pendingTravelDraftId(base.idempotencyKey);
    await ctx.cache.patch<TravelDraft[]>(MODELS.travelDrafts, (current) =>
      (current ?? []).filter((draft) => draft.id !== pendingId),
    );
  },
  apply: async (data, _args, base, ctx) => {
    const pendingId = pendingTravelDraftId(base.idempotencyKey);
    const authoritative = data as { id?: unknown } | undefined;
    await ctx.cache.patch<TravelDraft[]>(MODELS.travelDrafts, (current) => {
      const rest = (current ?? []).filter((draft) => draft.id !== pendingId);
      if (authoritative === undefined || typeof authoritative.id !== "string") return rest;
      return [data as TravelDraft, ...rest];
    });
  },
};

export interface TravelDraftIdArgs {
  draftId: string;
  version?: number;
  ticketId?: string;
  /** The row as rendered when the action fired, so a rejected delete can
   * put it back exactly as it was. */
  snapshot?: TravelDraft;
}

export type TravelDraftSubmitBody = { __path_id: string; idempotency_key: string };

export const submitTravelDraft: WriteOperation<TravelDraftIdArgs, TravelDraftSubmitBody> = {
  kind: "travel.submit",
  label: () => "Submit travel draft",
  entityKey: (args) => `traveldraft:${args.draftId}`,
  method: "POST",
  path: (args) => `/api/v1/dispatch/travel-drafts/${args.draftId}/submit`,
  invalidate: (args) => [
    MODELS.travelDrafts,
    MODELS.myWork,
    ...(args.ticketId !== undefined ? [MODELS.timeEntries(args.ticketId)] : []),
  ],
  build: (args, base) => ({ __path_id: args.draftId, idempotency_key: base.idempotencyKey }),
  send: (client, body) =>
    client.api.POST("/api/v1/dispatch/travel-drafts/{id}/submit", {
      params: { path: { id: body.__path_id } },
      body: { idempotency_key: body.idempotency_key },
    }),
  versionOf: (data) => nestedVersion(data, "entry"),
  optimistic: async (args, _base, ctx) => {
    await patchCached<TravelDraft[]>(ctx, MODELS.travelDrafts, (current) =>
      setTravelDraftStatus(current, args.draftId, "submitted"),
    );
  },
  revert: async (args, _base, ctx) => {
    await patchCached<TravelDraft[]>(ctx, MODELS.travelDrafts, (current) =>
      setTravelDraftStatus(current, args.draftId, "draft"),
    );
  },
};

export type TravelDraftDeleteBody = { __path_id: string; idempotency_key: string };

export const deleteTravelDraft: WriteOperation<TravelDraftIdArgs, TravelDraftDeleteBody> = {
  kind: "travel.delete",
  label: () => "Delete travel draft",
  entityKey: (args) => `traveldraft:${args.draftId}`,
  method: "DELETE",
  path: (args) => `/api/v1/dispatch/travel-drafts/${args.draftId}`,
  invalidate: () => [MODELS.travelDrafts],
  build: (args, base) => ({ __path_id: args.draftId, idempotency_key: base.idempotencyKey }),
  send: (client, body) =>
    client.api.DELETE("/api/v1/dispatch/travel-drafts/{id}", {
      params: { path: { id: body.__path_id } },
    }),
  optimistic: async (args, _base, ctx) => {
    await ctx.cache.patch<TravelDraft[]>(MODELS.travelDrafts, (current) =>
      (current ?? []).filter((draft) => draft.id !== args.draftId),
    );
  },
  revert: async (args, _base, ctx) => {
    if (args.snapshot === undefined) return;
    const restored = args.snapshot;
    await ctx.cache.patch<TravelDraft[]>(MODELS.travelDrafts, (current) => {
      const rows = current ?? [];
      if (rows.some((draft) => draft.id === restored.id)) return rows;
      return [restored, ...rows].sort((a, b) => b.created_at.localeCompare(a.created_at));
    });
  },
};

/** All operations the replay engine can reconstruct after relaunch. */
export const WRITE_OPERATIONS = {
  [addTicketNote.kind]: addTicketNote,
  [startTimer.kind]: startTimer,
  [stopTimer.kind]: stopTimer,
  [setChecklistItemChecked.kind]: setChecklistItemChecked,
  [pauseTimer.kind]: pauseTimer,
  [resumeTimer.kind]: resumeTimer,
  [consumeStock.kind]: consumeStock,
  [tickDayPack.kind]: tickDayPack,
  [untickDayPack.kind]: untickDayPack,
  [transferStock.kind]: transferStock,
  [receiveStock.kind]: receiveStock,
  [checkoutTools.kind]: checkoutTools,
  [checkinTools.kind]: checkinTools,
  [markNotificationRead.kind]: markNotificationRead,
  [markAllNotificationsRead.kind]: markAllNotificationsRead,
  [setAppointmentStatus.kind]: setAppointmentStatus,
  [createTravelDraft.kind]: createTravelDraft,
  [submitTravelDraft.kind]: submitTravelDraft,
  [deleteTravelDraft.kind]: deleteTravelDraft,
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

/** Patch a cached model only when it is actually in the cache. A write made
 * before the model ever loaded has nothing to render optimistically, and
 * fabricating an entry would claim server truth the app does not have. */
async function patchCached<T>(
  ctx: WriteContext,
  key: string,
  patcher: (current: T) => T,
): Promise<void> {
  if (!ctx.cache.keys().includes(key)) return;
  await ctx.cache.patch<T>(key, (current) => patcher(current as T));
}

function withAppointmentStatus(
  day: DispatchDay,
  appointmentId: string,
  status: AppointmentStatusValue,
): DispatchDay {
  return {
    ...day,
    technicians: day.technicians.map((technician) => ({
      ...technician,
      appointments: technician.appointments.map((appointment) =>
        appointment.id === appointmentId ? { ...appointment, status } : appointment,
      ),
    })),
  };
}

function pendingTravelDraftId(idempotencyKey: string): string {
  return `pending-${idempotencyKey}`;
}

function pendingTravelDraft(
  args: TravelDraftCreateArgs,
  idempotencyKey: string,
  ctx: WriteContext,
): TravelDraft {
  const now = ctx.now().toISOString();
  return {
    id: pendingTravelDraftId(idempotencyKey),
    ticket_id: args.ticketId,
    user_id: "",
    started_at: args.startedAt,
    ended_at: args.endedAt,
    distance: args.distance,
    distance_unit: args.distanceUnit,
    vehicle: args.vehicle,
    billable: args.billable,
    notes: args.notes !== undefined && args.notes.trim() !== "" ? args.notes : null,
    status: "draft",
    ...(args.appointmentId !== undefined ? { appointment_id: args.appointmentId } : {}),
    submitted_time_entry_id: null,
    expense_id: null,
    created_at: now,
    updated_at: now,
  } as TravelDraft;
}

function setTravelDraftStatus(
  drafts: TravelDraft[],
  draftId: string,
  status: string,
): TravelDraft[] {
  return drafts.map((draft) => (draft.id === draftId ? { ...draft, status } : draft));
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
