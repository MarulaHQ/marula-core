// Day-pack client + copy helpers (Wave 32 lane 8, `p32-today-day-pack`).
//
// Types come straight from the generated SDK (`packages/sdk`), which the
// OpenAPI re-pin carries for this lane's endpoints; the calls go through the
// typed client with no structural casts.

import type { components } from "@meridian/sdk";

import { api } from "./api";
import { friendlyMutationError, friendlyMutationText } from "./error-copy";
import { runEnvelopeMutation } from "./envelope-write";

export type DayPackSource = components["schemas"]["DayPackSourceView"];
export type DayPackHolder = components["schemas"]["DayPackHolderView"];
export type DayPackAvailability = components["schemas"]["DayPackAvailabilityView"];
export type DayPackTick = components["schemas"]["DayPackTickView"];
export type DayPackLine = components["schemas"]["DayPackLineView"];
export type DayPackKit = components["schemas"]["DayPackKitView"];
export type DayPackReminder = components["schemas"]["DayPackReminderView"];
export type DayPackStop = components["schemas"]["DayPackStopView"];
export type DayPackTrip = components["schemas"]["DayPackTripView"];
export type DayPackTomorrow = components["schemas"]["DayPackTomorrowView"];
export type DayPackCounts = components["schemas"]["DayPackCounts"];
export type DayPackView = components["schemas"]["DayPackView"];
export type PackRuleView = components["schemas"]["PackRuleView"];
export type TicketPickRow = components["schemas"]["TicketPickRowView"];

export async function fetchDayPack(date: string): Promise<DayPackView> {
  const { data, error } = await api.api.GET("/api/v1/day-pack", {
    params: { query: { date } },
  });
  if (data === undefined || error !== undefined) {
    throw new Error(friendlyMutationError(error).text);
  }
  return data;
}

/* ------------------------------------------------------------------ ticks */

export async function tickLine(
  date: string,
  line: { line_key: string; ticket_id?: string | null },
  note?: string,
): Promise<string | null> {
  const result = await runEnvelopeMutation<Record<string, unknown>, unknown>({
    describe: "tick a collected line",
    entityId: `tick:${date}:${line.line_key}`,
    buildBody: (base) => ({
      date,
      line_key: line.line_key,
      ticket_id: line.ticket_id ?? null,
      note: note ?? "Collected",
      ...base,
    }),
    optimistic: () => {},
    revert: () => {},
    request: (client, body) =>
      client.POST("/api/v1/day-pack/ticks", { body: body as never }),
  });
  return result.ok ? null : (result.friendly ?? "The tick was not saved.");
}

export async function untickLine(date: string, lineKey: string): Promise<string | null> {
  const { error } = await api.api.DELETE("/api/v1/day-pack/ticks", {
    params: { query: { date, line_key: lineKey } },
  });
  // A 404 means it was already gone (double-click, second tab) — that is
  // the same outcome the untick wanted.
  if (error !== undefined && !isNotFound(error)) {
    return friendlyMutationError(error).text;
  }
  return null;
}

function isNotFound(error: unknown): boolean {
  if (error === null || typeof error !== "object") return false;
  const status = (error as { status?: unknown }).status;
  if (status === 404) return true;
  const detail = (error as { detail?: unknown }).detail;
  return typeof detail === "string" && detail.toLowerCase().includes("not found");
}

/* ------------------------------------------------------------------ picks */

export interface PinInput {
  ticket_id: string;
  stock_item_id: string;
  qty_milli?: number;
  note?: string | null;
}

export async function fetchTicketPicks(ticketId: string): Promise<TicketPickRow[]> {
  const { data, error } = await api.api.GET("/api/v1/day-pack/picks", {
    params: { query: { ticket_id: ticketId } },
  });
  if (data === undefined || error !== undefined) {
    throw new Error(friendlyMutationError(error).text);
  }
  return data;
}

export async function pinItem(input: PinInput): Promise<string | null> {
  const result = await runEnvelopeMutation<Record<string, unknown>, unknown>({
    describe: "pin an item to the ticket",
    buildBody: (base) => ({ ...input, ...base }),
    optimistic: () => {},
    revert: () => {},
    request: (client, body) =>
      client.POST("/api/v1/day-pack/picks", { body: body as never }),
  });
  return result.ok ? null : (result.friendly ?? "The pin was not saved.");
}

export async function unpinItem(pickId: string): Promise<string | null> {
  const { error } = await api.api.DELETE("/api/v1/day-pack/picks/{id}", {
    params: { path: { id: pickId } },
  });
  if (error !== undefined && !isNotFound(error)) {
    return friendlyMutationError(error).text;
  }
  return null;
}

/* ------------------------------------------------------------------ rules */

export async function fetchPackRules(): Promise<PackRuleView[]> {
  const { data, error } = await api.api.GET("/api/v1/day-pack/rules");
  if (data === undefined || error !== undefined) {
    throw new Error(friendlyMutationError(error).text);
  }
  return data;
}

export interface RuleInput {
  scope_kind: string;
  scope_id: string;
  target_kind: string;
  stock_item_id?: string | null;
  kit_id?: string | null;
  qty_milli?: number;
  note?: string | null;
}

export async function createPackRule(input: RuleInput): Promise<string | null> {
  const result = await runEnvelopeMutation<Record<string, unknown>, unknown>({
    describe: "save a dispatch rule",
    buildBody: (base) => ({ ...input, ...base }),
    optimistic: () => {},
    revert: () => {},
    request: (client, body) =>
      client.POST("/api/v1/day-pack/rules", { body: body as never }),
  });
  return result.ok ? null : (result.friendly ?? "The rule was not saved.");
}

export async function setPackRuleActive(id: string, active: boolean): Promise<string | null> {
  const result = await runEnvelopeMutation<Record<string, unknown>, unknown>({
    describe: active ? "enable a dispatch rule" : "disable a dispatch rule",
    entityId: id,
    buildBody: (base) => ({ active, ...base }),
    optimistic: () => {},
    revert: () => {},
    request: (client, body) =>
      client.PATCH("/api/v1/day-pack/rules/{id}", {
        params: { path: { id } },
        body: body as never,
      }),
  });
  return result.ok ? null : (result.friendly ?? "The rule was not updated.");
}

export async function deletePackRule(id: string): Promise<string | null> {
  const { error } = await api.api.DELETE("/api/v1/day-pack/rules/{id}", {
    params: { path: { id } },
  });
  if (error !== undefined && !isNotFound(error)) {
    return friendlyMutationError(error).text;
  }
  return null;
}

/* ------------------------------------------------------------------- copy */

export function qtyText(qtyMilli: number): string {
  const value = qtyMilli / 1000;
  return Number.isInteger(value)
    ? String(value)
    : value.toFixed(2).replace(/0+$/, "").replace(/\.$/, "");
}

export function timeText(iso: string): string {
  return new Date(iso).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

/** "Van · 2" / "Shelf A-3 · 4" — where to grab it, from the read model. */
export function sourceLabel(line: { source?: DayPackSource | null }): string {
  const source = line.source;
  if (source === undefined || source === null) return "No stock recorded";
  const where = source.is_tech_vehicle ? `Van · ${source.name}` : source.name;
  return `${where} · ${qtyText(source.available_units)}`;
}

export function availabilityLabel(availability: DayPackAvailability): string {
  switch (availability.status) {
    case "available":
      return "Available";
    case "partial":
      return `Partial — ${qtyText(availability.available_units)} of ${qtyText(
        availability.required_units,
      )}`;
    case "unavailable":
      return "None available";
    case "not_configured":
      return "Not stocked";
    default:
      return availability.status;
  }
}

export function availabilityTone(
  availability: DayPackAvailability,
): "clear" | "warning" | "danger" | "muted" {
  switch (availability.status) {
    case "available":
      return "clear";
    case "partial":
      return "warning";
    case "unavailable":
      return "danger";
    default:
      return "muted";
  }
}

function dayLabel(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
}

/** "Dana has it, due Thu" — the honest unavailable state, never a bare no. */
export function holderText(availability: DayPackAvailability): string | null {
  const holder = availability.holders?.[0];
  if (holder === undefined) return null;
  const who =
    holder.display_name ?? holder.client_name ?? (holder.kind === "out" ? "Out on a job" : "Someone");
  if (holder.expected_return_at != null && holder.expected_return_at !== "") {
    const when = holder.overdue ? "overdue" : `due ${dayLabel(holder.expected_return_at)}`;
    return holder.kind === "loan" ? `${who} — ${when}` : `${who} has it — ${when}`;
  }
  if (holder.due_at != null && holder.due_at !== "") {
    const when = holder.overdue ? "overdue" : `due ${dayLabel(holder.due_at)}`;
    return `${who} has it — ${when}`;
  }
  return holder.kind === "loan" ? `${who}` : `${who} has it`;
}

export function provenanceLabel(provenance: string): string {
  switch (provenance) {
    case "pick":
      return "Pinned";
    case "rule":
      return "Default";
    case "checklist":
      return "Checklist";
    default:
      return provenance;
  }
}

export function tomorrowReasonNote(reason: string): string {
  switch (reason) {
    case "direct_to_site":
      return "Tomorrow starts on the road — take these home in the van tonight.";
    case "start_point_unknown":
      return "No start point set for tomorrow — assuming the van rides home. Set tomorrow's start under Dispatch to change this.";
    default:
      return "";
  }
}

export function startPointNote(kind?: string | null, label?: string | null): string {
  const name = label !== null && label !== undefined && label !== "" ? label : (kind ?? "");
  switch (kind) {
    case "office":
      return `Starts at the office${name === "" ? "" : ` (${name})`} — collect before you roll.`;
    case "home":
      return `Starts from home${name === "" ? "" : ` (${name})`}.`;
    case "site":
      return `Starts at a site${name === "" ? "" : ` (${name})`} — the van is already out.`;
    case "address":
      return `Starts at ${name === "" ? "an address" : name}.`;
    default:
      return "No start point for today — legs stay unestimated.";
  }
}

export { friendlyMutationText };
