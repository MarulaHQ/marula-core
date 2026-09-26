// Typed read models the foundation caches offline.
//
// Purpose: name every read model once (stable cache keys + a typed fetcher
// over @meridian/sdk) so screens, the sync engine, and the realtime
// invalidation map all agree. The SDK's generated `paths` types are the only
// source of truth for response shapes — nothing here hand-declares an
// entity (docs/rn-groundwork.md rule 1).
//
// Non-purpose: no cache, no hooks, no offline queue.

import { MeridianClient, type paths } from "@meridian/sdk";

import { unwrap, type TransportResult } from "./http";

type Ok<T> = T extends {
  responses: { 200: { content: { "application/json": infer R } } };
}
  ? R
  : never;

export type TicketRow = Ok<paths["/api/v1/tickets"]["get"]>[number];
export type MyWorkItem = Ok<paths["/api/v1/tickets/my-work"]["get"]>[number];
export type TicketDetail = Ok<paths["/api/v1/tickets/{id}"]["get"]>;
export type TicketComment = Ok<paths["/api/v1/tickets/{id}/comments"]["get"]>[number];
export type StockItemView = Ok<paths["/api/v1/stock-items"]["get"]>[number];
export type OnHandRow = Ok<paths["/api/v1/stock/on-hand"]["get"]>[number];
export type StockMovementView = Ok<paths["/api/v1/stock-items/{id}/movements"]["get"]>[number];
export type ToolCheckoutDetail = Ok<paths["/api/v1/tool-checkouts"]["get"]>[number];
export type SerialWhereabouts = Ok<paths["/api/v1/stock-serials"]["get"]>[number];
export type DeviceAsset = Ok<paths["/api/v1/assets/{id}"]["get"]>;
export type Notification = Ok<paths["/api/v1/notifications"]["get"]>["items"][number];
export type NotificationPage = Ok<paths["/api/v1/notifications"]["get"]>;
export type UnreadCount = Ok<paths["/api/v1/notifications/unread-count"]["get"]>;
export type Me = Ok<paths["/api/v1/me"]["get"]>;
export type TimerSession = Ok<paths["/api/v1/timers/working-set"]["get"]>[number];
export type DispatchDay = Ok<paths["/api/v1/dispatch/day"]["get"]>;
export type AppointmentRow =
  DispatchDay["technicians"][number]["appointments"][number];
export type ScanResolution = Ok<paths["/api/v1/scan/resolve"]["post"]>;
export type TimeEntry = Ok<paths["/api/v1/time-entries"]["get"]>[number];
export type TravelDraft = Ok<paths["/api/v1/dispatch/travel-drafts"]["get"]>[number];
export type OnCallBlock = Ok<paths["/api/v1/on-call-blocks"]["get"]>[number];
export type TicketAttachment =
  Ok<paths["/api/v1/tickets/{id}/attachments"]["get"]>[number];

/** Stable cache keys. Keys are persisted, so treat them as frozen. */
export const MODELS = {
  me: "me",
  myWork: "tickets.my-work",
  ticket: (id: string) => `ticket:${id}`,
  ticketComments: (id: string) => `ticket:${id}:comments`,
  activeTimer: "timer.active",
  onHand: "stock.on-hand",
  stockItem: (id: string) => `stock.item:${id}`,
  stockMovements: (id: string) => `stock.item:${id}:movements`,
  stockItems: "stock.items",
  stockSerials: "stock.serials",
  location: (id: string) => `location:${id}`,
  asset: (id: string) => `asset:${id}`,
  toolCheckouts: "tool.checkouts",
  notifications: "notifications",
  unreadCount: "notifications.unread",
  dispatchDay: (date: string) => `dispatch.day:${date}`,
  timeEntries: (ticketId: string) => `ticket:${ticketId}:time`,
  ticketAttachments: (ticketId: string) => `ticket:${ticketId}:attachments`,
  travelDrafts: "dispatch.travel-drafts",
  onCall: (fromDate: string) => `oncall:${fromDate}`,
} as const;

/** The on-call window the day surface reads (today plus two weeks). */
export const ON_CALL_WINDOW_DAYS = 14;

export const QUERY_DEFAULT_LIMIT = 100;

export async function fetchMe(client: MeridianClient): Promise<Me> {
  return unwrap(await client.api.GET("/api/v1/me"));
}

export async function fetchMyWork(client: MeridianClient): Promise<MyWorkItem[]> {
  return unwrap(await client.api.GET("/api/v1/tickets/my-work"));
}

export async function fetchTicket(client: MeridianClient, id: string): Promise<TicketDetail> {
  return unwrap(await client.api.GET("/api/v1/tickets/{id}", { params: { path: { id } } }));
}

export async function fetchTicketComments(
  client: MeridianClient,
  id: string,
): Promise<TicketComment[]> {
  return unwrap(
    await client.api.GET("/api/v1/tickets/{id}/comments", { params: { path: { id } } }),
  );
}

export async function fetchActiveTimer(client: MeridianClient): Promise<TimerSession | null> {
  // The pin types this response as a bare object because the Rust handler
  // returns Option<TimerSession>; at runtime it is the session or null.
  const result = await client.api.GET("/api/v1/timers/active");
  return unwrap(result) as unknown as TimerSession | null;
}

export async function fetchOnHand(client: MeridianClient): Promise<OnHandRow[]> {
  return unwrap(
    await client.api.GET("/api/v1/stock/on-hand", {
      params: { query: { limit: QUERY_DEFAULT_LIMIT } },
    }),
  );
}

export async function fetchStockItems(client: MeridianClient): Promise<StockItemView[]> {
  return unwrap(
    await client.api.GET("/api/v1/stock-items", {
      params: { query: { active: true, limit: QUERY_DEFAULT_LIMIT } },
    }),
  );
}

export async function fetchStockItem(client: MeridianClient, id: string): Promise<StockItemView> {
  return unwrap(await client.api.GET("/api/v1/stock-items/{id}", { params: { path: { id } } }));
}

export async function fetchStockMovements(
  client: MeridianClient,
  id: string,
): Promise<StockMovementView[]> {
  return unwrap(
    await client.api.GET("/api/v1/stock-items/{id}/movements", { params: { path: { id } } }),
  );
}

export async function fetchToolCheckouts(
  client: MeridianClient,
): Promise<ToolCheckoutDetail[]> {
  return unwrap(
    await client.api.GET("/api/v1/tool-checkouts", {
      params: { query: { limit: QUERY_DEFAULT_LIMIT } },
    }),
  );
}

export async function fetchStockSerials(client: MeridianClient): Promise<SerialWhereabouts[]> {
  return unwrap(await client.api.GET("/api/v1/stock-serials"));
}

export type StockLocation = Ok<paths["/api/v1/stock-locations/{id}"]["get"]>;

export async function fetchStockLocation(
  client: MeridianClient,
  id: string,
): Promise<StockLocation> {
  return unwrap(
    await client.api.GET("/api/v1/stock-locations/{id}", { params: { path: { id } } }),
  );
}

export async function fetchAsset(client: MeridianClient, id: string): Promise<DeviceAsset> {
  return unwrap(await client.api.GET("/api/v1/assets/{id}", { params: { path: { id } } }));
}

export async function fetchNotifications(client: MeridianClient): Promise<NotificationPage> {
  return unwrap(
    await client.api.GET("/api/v1/notifications", {
      params: { query: { limit: 50 } },
    }),
  );
}

export async function fetchUnreadCount(client: MeridianClient): Promise<UnreadCount> {
  return unwrap(await client.api.GET("/api/v1/notifications/unread-count"));
}

export async function fetchDispatchDay(
  client: MeridianClient,
  date?: string,
): Promise<DispatchDay> {
  return unwrap(
    await client.api.GET("/api/v1/dispatch/day", {
      params: { query: date !== undefined ? { date } : {} },
    }),
  );
}

export async function fetchTimeEntries(
  client: MeridianClient,
  ticketId: string,
): Promise<TimeEntry[]> {
  return unwrap(
    await client.api.GET("/api/v1/time-entries", {
      params: { query: { ticket_id: ticketId, limit: QUERY_DEFAULT_LIMIT } },
    }),
  );
}

export async function fetchTicketAttachments(
  client: MeridianClient,
  ticketId: string,
): Promise<TicketAttachment[]> {
  return unwrap(
    await client.api.GET("/api/v1/tickets/{id}/attachments", {
      params: { path: { id: ticketId } },
    }),
  );
}

export async function fetchTravelDrafts(client: MeridianClient): Promise<TravelDraft[]> {
  return unwrap(
    await client.api.GET("/api/v1/dispatch/travel-drafts", {
      params: { query: { limit: 100 } },
    }),
  );
}

export async function fetchOnCallBlocks(
  client: MeridianClient,
  from: Date,
  to: Date,
): Promise<OnCallBlock[]> {
  // The pin annotates this route with the calendar day/week params, so the
  // generated signature names path params the wire call never sends. The
  // route itself reads `?user_id=&from=&to=` (crates/core-api/src/routes/
  // availability.rs), with the caller as the default subject. Read back as
  // the documented query shape until the pin catches up.
  const result = (await client.api.GET("/api/v1/on-call-blocks", {
    params: { query: { from: from.toISOString(), to: to.toISOString() } },
  } as never)) as unknown as TransportResult<OnCallBlock[]>;
  return unwrap(result);
}
