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

import { unwrap } from "./http";

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
} as const;

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

export async function fetchStockLocations(
  client: MeridianClient,
): Promise<StockLocationRow[]> {
  return unwrap(
    await client.api.GET("/api/v1/stock-locations", {
      params: { query: { active_only: true } },
    }),
  );
}

export async function fetchTicketStockMovements(
  client: MeridianClient,
  ticketId: string,
): Promise<TicketStockMovement[]> {
  return unwrap(
    await client.api.GET("/api/v1/tickets/{id}/stock-movements", {
      params: { path: { id: ticketId }, query: { limit: QUERY_DEFAULT_LIMIT } },
    }),
  );
}

/** Per-tool-item in-stock counts, optionally narrowed to one item. */
export async function fetchToolsAvailability(
  client: MeridianClient,
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

/** The caller's day pack. No date means today in the user's timezone, which
 * is exactly the phone's question; the cache key stays stable. */
export async function fetchDayPack(client: MeridianClient): Promise<DayPack> {
  return unwrap(await client.api.GET("/api/v1/day-pack", { params: { query: {} } }));
}

export async function fetchToolKits(client: MeridianClient): Promise<ToolKit[]> {
  return unwrap(
    await client.api.GET("/api/v1/tool-kits", { params: { query: { active: true } } }),
  );
}

export async function fetchToolKit(client: MeridianClient, id: string): Promise<ToolKitDetail> {
  return unwrap(await client.api.GET("/api/v1/tool-kits/{id}", { params: { path: { id } } }));
}

export async function fetchKitAvailability(
  client: MeridianClient,
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
