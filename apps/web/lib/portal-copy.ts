// The portal's vocabulary: what the client is told, in what words.
//
// Purpose: the client never sees our statuses. DESIGN-NOTES-portal.md §3
// fixes five plain words, and §9.5 hands the internal-to-client mapping to the
// implementation. This module IS that mapping, in one place, so no page can
// invent its own wording.
//
// THE FIVE WORDS (DESIGN-NOTES-portal.md §3):
//
//   Received      Logged and not yet answered
//   We're on it   A technician is working on it
//   Waiting for you  We need an answer, so it is gold and stands out
//   Scheduled     A visit is booked
//   Resolved      Fixed, with a How did we do? invitation
//
// THE MAPPING. Four of the five come straight off a ticket's category. The
// fifth, Scheduled, means a visit is booked rather than anything about the
// ticket's category, so it belongs to the schedule surface (a confirmed visit)
// and to any ticket whose service desk has booked one — not to a category. A
// ticket with no confirmed visit is never labelled Scheduled, because the pack
// promises the client never sees a claim the service desk cannot back up.
//
// NOTHING ELSE LEAKS. No priority, no board, no assignee, no SLA clock, no
// due date on the client's own list, no internal note (the API already withholds
// those, and this module keeps the words to the five).
"use client";

// The only import: the kit's zone-aware formatters. The vocabulary itself stays
// independent of the API layer.
import {
  dayKeyInTimeZone,
  formatDayKey,
  formatInTimeZone,
  shiftDayKey,
} from "@marula-core/ui";

/** The client's five words. */
export type ClientStatus = "received" | "working" | "waiting" | "scheduled" | "resolved";

/** The ticket categories the portal API returns. Declared here rather than
 *  imported so the vocabulary does not move when the API's internal type does. */
export type PortalTicketCategory = "open" | "in_progress" | "waiting" | "closed";

/** What each word means, for the code that has to decide. */
export const CLIENT_STATUS: Record<
  ClientStatus,
  { label: string; /** The one line that says why, on the row under the title. */ note: string }
> = {
  received: {
    label: "Received",
    note: "We have it, and someone is looking",
  },
  working: {
    label: "We're on it",
    note: "A technician is working on it now",
  },
  waiting: {
    label: "Waiting for you",
    note: "We need an answer from you",
  },
  scheduled: {
    label: "Scheduled",
    note: "A visit is booked",
  },
  resolved: {
    label: "Resolved",
    note: "Fixed. Tell us how it went",
  },
};

/**
 * The ticket category → the client's word. This is the whole mapping
 * (§9.5): open is logged and unanswered, in_progress has someone on it,
 * waiting needs her, closed is done.
 */
export function clientStatusFor(category: PortalTicketCategory): ClientStatus {
  switch (category) {
    case "open":
      return "received";
    case "in_progress":
      return "working";
    case "waiting":
      return "waiting";
    case "closed":
      return "resolved";
    default:
      // A category this build does not know is never shown to a client as
      // itself. "Received" is the honest floor: it is logged and unanswered.
      return "received";
  }
}

/** Money, in the client's currency, to the cent. Tabular, so a column of
 *  totals lines up. */
export function money(minor: number, currency: string): string {
  return new Intl.NumberFormat(undefined, {
    style: "currency",
    currency,
  }).format(minor / 100);
}

/** True for a `Format: date` value — a calendar day with no time of day. */
function isDayKey(value: string): boolean {
  return /^\d{4}-\d{2}-\d{2}$/.test(value);
}

/**
 * "17 Sep 2026" — the date a client reads, not an ISO string.
 *
 * Takes both kinds of value this portal receives, and they are not the same
 * kind: `issued_at` and `received_at` are instants, while `due_on` and
 * `valid_until` are `Format: date` days with no time in them. Reading a day as
 * an instant puts it at midnight UTC and shows the day before to everyone west
 * of Greenwich — an invoice due on the 18th billed as the 17th. A bare day is
 * therefore read in its own frame, where the zone argument does not apply.
 */
export function longDate(value: string, timeZone: string): string {
  return isDayKey(value)
    ? formatDayKey(value, "date")
    : formatInTimeZone(value, timeZone, "date");
}

/** "Thu 1 Oct" — the short form a card or a schedule row uses. */
export function shortDate(iso: string, timeZone: string): string {
  return formatInTimeZone(iso, timeZone, "day");
}

/** "09:00" in 24-hour clock, which is what a service desk publishes slots in. */
export function clockTime(iso: string, timeZone: string): string {
  return formatInTimeZone(iso, timeZone, "time");
}

/** "Today, 09:14" · "Yesterday, 16:20" · "Thu 1 Oct, 09:00".
 *
 *  The stamp every list row carries. Relative for today and yesterday, because
 *  that is how a person reads "when did this happen", and a date beyond that.
 *  24-hour clock, matching the schedule slots the portal publishes.
 *  (portal/sand/states/02-tickets.png)
 *
 *  Today and yesterday are compared as day keys in the reader's zone: a client
 *  in Sydney reading at 09:00 is looking at yesterday's London work and should
 *  see the word that is true for them.
 */
export function stamp(iso: string, timeZone: string, now: number = Date.now()): string {
  const time = formatInTimeZone(iso, timeZone, "time");
  const key = dayKeyInTimeZone(iso, timeZone);
  const today = dayKeyInTimeZone(now, timeZone);
  if (key === today) return `Today, ${time}`;
  if (key === shiftDayKey(today, -1)) return `Yesterday, ${time}`;
  if (key === shiftDayKey(today, 1)) return `Tomorrow, ${time}`;
  return `${formatInTimeZone(iso, timeZone, "day")}, ${time}`;
}

/** "today at 11:38" · "yesterday at 16:20" · "Mon 28 Sep at 09:14".
 *
 *  The same idea as `stamp`, in the form a sentence wants: lowercased and
 *  joined with "at", because it is read inside a line that already starts
 *  "Raised". `stamp` keeps the capitalised comma form for list rows, where it
 *  stands on its own. (portal/sand/states/04-ticket-nothing-yet.png) */
export function raisedAt(iso: string, timeZone: string, now: number = Date.now()): string {
  const time = formatInTimeZone(iso, timeZone, "time");
  const key = dayKeyInTimeZone(iso, timeZone);
  const today = dayKeyInTimeZone(now, timeZone);
  if (key === today) return `today at ${time}`;
  if (key === shiftDayKey(today, -1)) return `yesterday at ${time}`;
  return `${formatInTimeZone(iso, timeZone, "day")} at ${time}`;
}

/** Whole days between today and a date, negative once it has passed. */
export function daysUntil(iso: string, now: Date = new Date()): number {
  const target = new Date(iso);
  const startOfTarget = Date.UTC(target.getUTCFullYear(), target.getUTCMonth(), target.getUTCDate());
  const startOfToday = Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate());
  return Math.round((startOfTarget - startOfToday) / 86_400_000);
}

/** "Overdue 12 days" — the number is the fact; the colour only points at it. */
export function overdueDays(iso: string, now: Date = new Date()): number | null {
  const days = daysUntil(iso, now);
  return days < 0 ? -days : null;
}

/** "09:00 to 10:30" for a visit row. */
export function timeRange(startIso: string, endIso: string, timeZone: string): string {
  return `${clockTime(startIso, timeZone)} to ${clockTime(endIso, timeZone)}`;
}

/** Initials for an avatar, from a display name. */
export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  const first = parts[0];
  if (first === undefined) return "?";
  if (parts.length === 1) return first.slice(0, 2).toUpperCase();
  const last = parts[parts.length - 1];
  return `${first.charAt(0)}${last === undefined ? "" : last.charAt(0)}`.toUpperCase();
}
