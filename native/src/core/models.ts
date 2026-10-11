// Typed read models the foundation caches offline.
//
// Purpose: name every read model once (stable cache keys + a typed fetcher
// over @marula-core/sdk) so screens, the sync engine, and the realtime
// invalidation map all agree. The SDK's generated `paths` types are the only
// source of truth for response shapes — nothing here hand-declares an
// entity (docs/rn-groundwork.md rule 1).
//
// Non-purpose: no cache, no hooks, no offline queue.

import { MarulaClient, type paths } from "@marula-core/sdk";

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
export type TicketStockMovement =
  Ok<paths["/api/v1/tickets/{id}/stock-movements"]["get"]>[number];
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
export type ClientRow = Ok<paths["/api/v1/clients/{id}"]["get"]>;
export type ContactRow =
  Ok<paths["/api/v1/clients/{id}/contacts"]["get"]>[number];
export type TicketsListRow = Ok<paths["/api/v1/tickets"]["get"]>[number];
export type DispatchQueueView = Ok<paths["/api/v1/dispatch/queue"]["get"]>;

/** Wave 32 lane 8 day pack (Wave 33 lane 2 mirrors it on the phone). */
export type DayPack = Ok<paths["/api/v1/day-pack"]["get"]>;
export type DayPackTrip = DayPack["trips"][number];
export type DayPackLine = DayPackTrip["lines"][number];
export type DayPackKit = DayPackTrip["kits"][number];
export type DayPackTick = NonNullable<DayPackLine["tick"]>;

/** Wave 33 lane 2 field-ops reads (all public contract surfaces). */
export type StockLocationRow = Ok<paths["/api/v1/stock-locations"]["get"]>[number];
export type ToolAvailabilityRow = Ok<paths["/api/v1/tools/availability"]["get"]>[number];
export type ToolKit = Ok<paths["/api/v1/tool-kits"]["get"]>[number];
export type ToolKitDetail = Ok<paths["/api/v1/tool-kits/{id}"]["get"]>;
export type KitAvailability = Ok<paths["/api/v1/tool-kits/{id}/availability"]["get"]>;

/** Stable cache keys. Keys are persisted, so treat them as frozen. */
export const MODELS = {
  me: "me",
  myWork: "tickets.my-work",
  ticket: (id: string) => `ticket:${id}`,
  ticketComments: (id: string) => `ticket:${id}:comments`,
  ticketStockMovements: (id: string) => `ticket:${id}:stock-movements`,
  activeTimer: "timer.active",
  timerWorkingSet: "timer.working-set",
  onHand: "stock.on-hand",
  stockItem: (id: string) => `stock.item:${id}`,
  stockMovements: (id: string) => `stock.item:${id}:movements`,
  stockItems: "stock.items",
  stockSerials: "stock.serials",
  stockLocations: "stock.locations",
  toolsAvailability: (itemId?: string) =>
    itemId !== undefined ? `tools.availability:${itemId}` : "tools.availability",
  dayPack: () => "day-pack:today",
  kits: "tools.kits",
  kit: (id: string) => `tools.kit:${id}`,
  kitAvailability: (id: string, locationId?: string) =>
    locationId !== undefined ? `tools.kit:${id}:availability:${locationId}` : `tools.kit:${id}:availability`,
  location: (id: string) => `location:${id}`,
  asset: (id: string) => `asset:${id}`,
  toolCheckouts: "tool.checkouts",
  notifications: "notifications",
  unreadCount: "notifications.unread",
  dispatchDay: (date: string) => `dispatch.day:${date}`,
  timeEntries: (ticketId: string) => `ticket:${ticketId}:time`,
  ticketChecklist: (ticketId: string) => `ticket:${ticketId}:checklist`,
  myDayEntries: (userId: string) => `time:day:${userId}`,
  ticketAttachments: (ticketId: string) => `ticket:${ticketId}:attachments`,
  client: (id: string) => `client:${id}`,
  clientContacts: (id: string) => `client:${id}:contacts`,
  ticketsList: "tickets.list",
  dispatchQueue: "dispatch.queue",
  travelDrafts: "dispatch.travel-drafts",
  onCall: (fromDate: string) => `oncall:${fromDate}`,
} as const;

/** The on-call window the day surface reads (today plus two weeks). */
export const ON_CALL_WINDOW_DAYS = 14;

export const QUERY_DEFAULT_LIMIT = 100;

export async function fetchMe(client: MarulaClient): Promise<Me> {
  return unwrap(await client.api.GET("/api/v1/me"));
}

export async function fetchMyWork(client: MarulaClient): Promise<MyWorkItem[]> {
  return unwrap(await client.api.GET("/api/v1/tickets/my-work"));
}

export async function fetchTicket(client: MarulaClient, id: string): Promise<TicketDetail> {
  return unwrap(await client.api.GET("/api/v1/tickets/{id}", { params: { path: { id } } }));
}

export async function fetchClient(client: MarulaClient, id: string): Promise<ClientRow> {
  return unwrap(await client.api.GET("/api/v1/clients/{id}", { params: { path: { id } } }));
}

export async function fetchClientContacts(
  client: MarulaClient,
  id: string,
): Promise<ContactRow[]> {
  return unwrap(
    await client.api.GET("/api/v1/clients/{id}/contacts", { params: { path: { id } } }),
  );
}

/** The board's All scope: open tickets across boards (the phone reads one page). */
export async function fetchTicketsList(client: MarulaClient): Promise<TicketsListRow[]> {
  return unwrap(
    await client.api.GET("/api/v1/tickets", {
      params: { query: { limit: QUERY_DEFAULT_LIMIT } },
    }),
  );
}

/** The board's Unassigned scope: the dispatch queue in dispatch order. */
export async function fetchDispatchQueue(client: MarulaClient): Promise<DispatchQueueView> {
  return unwrap(await client.api.GET("/api/v1/dispatch/queue", { params: { query: {} } }));
}

export async function fetchTicketComments(
  client: MarulaClient,
  id: string,
): Promise<TicketComment[]> {
  return unwrap(
    await client.api.GET("/api/v1/tickets/{id}/comments", { params: { path: { id } } }),
  );
}

export async function fetchActiveTimer(client: MarulaClient): Promise<TimerSession | null> {
  // The pin types this response as a bare object because the Rust handler
  // returns Option<TimerSession>; at runtime it is the session or null.
  const result = await client.api.GET("/api/v1/timers/active");
  return unwrap(result) as unknown as TimerSession | null;
}

/**
 * Recent timer sessions, including PAUSED ones — `/timers/active` answers
 * only for a running session (`is_running` is false once a segment closes),
 * so without this read a paused timer is unreachable: the card vanishes and
 * nothing can resume or stop it. The working set keeps the paused session
 * visible (the pack's paused card), most recent first.
 */
export async function fetchTimerWorkingSet(client: MarulaClient): Promise<TimerSession[]> {
  return unwrap(
    await client.api.GET("/api/v1/timers/working-set"),
  ) as unknown as TimerSession[];
}

export async function fetchOnHand(client: MarulaClient): Promise<OnHandRow[]> {
  return unwrap(
    await client.api.GET("/api/v1/stock/on-hand", {
      params: { query: { limit: QUERY_DEFAULT_LIMIT } },
    }),
  );
}

export async function fetchStockItems(client: MarulaClient): Promise<StockItemView[]> {
  return unwrap(
    await client.api.GET("/api/v1/stock-items", {
      params: { query: { active: true, limit: QUERY_DEFAULT_LIMIT } },
    }),
  );
}

export async function fetchStockItem(client: MarulaClient, id: string): Promise<StockItemView> {
  return unwrap(await client.api.GET("/api/v1/stock-items/{id}", { params: { path: { id } } }));
}

export async function fetchStockMovements(
  client: MarulaClient,
  id: string,
): Promise<StockMovementView[]> {
  return unwrap(
    await client.api.GET("/api/v1/stock-items/{id}/movements", { params: { path: { id } } }),
  );
}

export async function fetchToolCheckouts(
  client: MarulaClient,
): Promise<ToolCheckoutDetail[]> {
  return unwrap(
    await client.api.GET("/api/v1/tool-checkouts", {
      params: { query: { limit: QUERY_DEFAULT_LIMIT } },
    }),
  );
}

export async function fetchStockSerials(client: MarulaClient): Promise<SerialWhereabouts[]> {
  return unwrap(await client.api.GET("/api/v1/stock-serials"));
}

export type StockLocation = Ok<paths["/api/v1/stock-locations/{id}"]["get"]>;

export async function fetchStockLocation(
  client: MarulaClient,
  id: string,
): Promise<StockLocation> {
  return unwrap(
    await client.api.GET("/api/v1/stock-locations/{id}", { params: { path: { id } } }),
  );
}

export async function fetchAsset(client: MarulaClient, id: string): Promise<DeviceAsset> {
  return unwrap(await client.api.GET("/api/v1/assets/{id}", { params: { path: { id } } }));
}

export async function fetchNotifications(client: MarulaClient): Promise<NotificationPage> {
  return unwrap(
    await client.api.GET("/api/v1/notifications", {
      params: { query: { limit: 50 } },
    }),
  );
}

export async function fetchUnreadCount(client: MarulaClient): Promise<UnreadCount> {
  return unwrap(await client.api.GET("/api/v1/notifications/unread-count"));
}

export async function fetchDispatchDay(
  client: MarulaClient,
  date?: string,
): Promise<DispatchDay> {
  return unwrap(
    await client.api.GET("/api/v1/dispatch/day", {
      params: { query: date !== undefined ? { date } : {} },
    }),
  );
}

export async function fetchTimeEntries(
  client: MarulaClient,
  ticketId: string,
): Promise<TimeEntry[]> {
  return unwrap(
    await client.api.GET("/api/v1/time-entries", {
      params: { query: { ticket_id: ticketId, limit: QUERY_DEFAULT_LIMIT } },
    }),
  );
}

/**
 * One person's day of time (the Time tab's read model): their entries since
 * midnight, newest first. `user_id` + `since` is the server contract — the
 * per-ticket read stays for a ticket's own history.
 */
export async function fetchMyDayEntries(
  client: MarulaClient,
  userId: string,
  sinceIso: string,
): Promise<TimeEntry[]> {
  return unwrap(
    await client.api.GET("/api/v1/time-entries", {
      params: { query: { user_id: userId, since: sinceIso, limit: QUERY_DEFAULT_LIMIT } },
    }),
  );
}

/** Local midnight as an ISO instant — the day window the Time tab asks for. */
export function startOfLocalDay(now = new Date()): string {
  return new Date(now.getFullYear(), now.getMonth(), now.getDate(), 0, 0, 0, 0).toISOString();
}

/** The ticket's checklist (read view: items + progress + close-gate state). */
export type TicketChecklistView = Ok<paths["/api/v1/tickets/{id}/checklist"]["get"]>;

export async function fetchTicketChecklist(
  client: MarulaClient,
  ticketId: string,
): Promise<TicketChecklistView> {
  return unwrap(
    await client.api.GET("/api/v1/tickets/{id}/checklist", { params: { path: { id: ticketId } } }),
  );
}

export async function fetchStockLocations(
  client: MarulaClient,
): Promise<StockLocationRow[]> {
  return unwrap(
    await client.api.GET("/api/v1/stock-locations", {
      params: { query: { active_only: true } },
    }),
  );
}

export async function fetchTicketAttachments(
  client: MarulaClient,
  ticketId: string,
): Promise<TicketAttachment[]> {
  return unwrap(
    await client.api.GET("/api/v1/tickets/{id}/attachments", {
      params: { path: { id: ticketId } },
    }),
  );
}

export async function fetchTicketStockMovements(
  client: MarulaClient,
  ticketId: string,
): Promise<TicketStockMovement[]> {
  return unwrap(
    await client.api.GET("/api/v1/tickets/{id}/stock-movements", {
      params: { path: { id: ticketId }, query: { limit: QUERY_DEFAULT_LIMIT } },
    }),
  );
}

export async function fetchTravelDrafts(client: MarulaClient): Promise<TravelDraft[]> {
  return unwrap(
    await client.api.GET("/api/v1/dispatch/travel-drafts", {
      params: { query: { limit: 100 } },
    }),
  );
}

/** Per-tool-item in-stock counts, optionally narrowed to one item. */
export async function fetchToolsAvailability(
  client: MarulaClient,
  itemId?: string,
): Promise<ToolAvailabilityRow[]> {
  return unwrap(
    await client.api.GET("/api/v1/tools/availability", {
      params: {
        query:
          itemId !== undefined
            ? { item_id: itemId, limit: QUERY_DEFAULT_LIMIT }
            : { limit: QUERY_DEFAULT_LIMIT },
      },
    }),
  );
}

export async function fetchOnCallBlocks(
  client: MarulaClient,
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

/** The caller's day pack. No date means today in the user's timezone, which
 * is exactly the phone's question; the cache key stays stable. */
export async function fetchDayPack(client: MarulaClient): Promise<DayPack> {
  return unwrap(await client.api.GET("/api/v1/day-pack", { params: { query: {} } }));
}

export async function fetchToolKits(client: MarulaClient): Promise<ToolKit[]> {
  return unwrap(
    await client.api.GET("/api/v1/tool-kits", { params: { query: { active: true } } }),
  );
}

export async function fetchToolKit(client: MarulaClient, id: string): Promise<ToolKitDetail> {
  return unwrap(await client.api.GET("/api/v1/tool-kits/{id}", { params: { path: { id } } }));
}

export async function fetchKitAvailability(
  client: MarulaClient,
  id: string,
  locationId?: string,
): Promise<KitAvailability> {
  return unwrap(
    await client.api.GET("/api/v1/tool-kits/{id}/availability", {
      params: {
        path: { id },
        query: locationId !== undefined ? { location_id: locationId } : {},
      },
    }),
  );
}
