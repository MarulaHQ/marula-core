/**
 * Zone-aware formatting.
 *
 * Purpose: one place that answers "what time is it *there*" without the host
 * app reaching for `toLocaleString` and inheriting the OS zone. Every helper
 * takes the zone explicitly; nothing here reads the machine's zone except
 * {@link browserTimeZone}, which exists so a caller can decide.
 *
 * Non-purpose: no state, no React, no app knowledge. The account's zone lives
 * in `useTimeFormat()` (`apps/web/lib/use-time-format.ts`), not here.
 *
 * The defect this closes is recorded in `design/qa-p50-tz-display-foundation/`:
 * 157 non-test `toLocale*` sites formatted in the viewer's OS zone, and one
 * passed a hardcoded `"UTC"`. The sweep that removed them
 * (`design/qa-p50-tz-surface-rollout/`) found the pinned one was right — it read
 * a day key, which has no zone to inherit — and kept the pin as
 * {@link formatDayKey}.
 */
export const FALLBACK_TIME_ZONE = "UTC";

/**
 * The format the kit draws in.
 *
 * The zone varies by reader, and should. The format should not: two people on
 * one call comparing two screens need the same shape, and a suite can only
 * assert a reading that is pinned. Day-first with a short month name is the
 * house shape — a month *name* leaves no room for the 10/11 reading error, so
 * pinning the field order costs nothing. A caller with a real reason still
 * passes `locale`.
 */
export const DEFAULT_TIME_LOCALE = "en-GB";

/**
 * Kinds a surface actually draws. Options stay in one table so two screens
 * cannot format the same instant two ways.
 *
 * The list grew in the surface sweep (T3): the four shapes T1 needed were
 * `time`, `date`, `datetime` and `weekday`, and a survey of the remaining 151
 * call sites found eight more in use. Each name below is a shape a real surface
 * draws, not a knob — a page that wants a ninth shape should argue for it here
 * rather than reach for `Intl` and inherit the machine's zone.
 */
export type TimeKind =
  /** 14:33 */
  | "time"
  /** 18 Oct 2026 */
  | "date"
  /** 18 Oct 2026, 14:33 */
  | "datetime"
  /** Thu, 18 Oct, 14:33 */
  | "weekday"
  /** Thu, 18 Oct */
  | "day"
  /** 18 Oct */
  | "dayMonth"
  /** 18 Oct, 14:33 */
  | "dayTime"
  /** Thursday, 18 October */
  | "longDate"
  /** Oct */
  | "month"
  /** October 2026 */
  | "monthYear"
  /** Thu */
  | "weekdayOnly"
  /** Thu 18 */
  | "weekdayDay";

const KIND_OPTIONS: Record<TimeKind, Intl.DateTimeFormatOptions> = {
  time: { hour: "2-digit", minute: "2-digit", hourCycle: "h23" },
  date: { day: "2-digit", month: "short", year: "numeric" },
  datetime: {
    day: "2-digit",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  },
  weekday: { weekday: "short", day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit", hourCycle: "h23" },
  day: { weekday: "short", day: "2-digit", month: "short" },
  dayMonth: { day: "2-digit", month: "short" },
  dayTime: { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit", hourCycle: "h23" },
  longDate: { weekday: "long", day: "numeric", month: "long" },
  month: { month: "short" },
  monthYear: { month: "long", year: "numeric" },
  weekdayOnly: { weekday: "short" },
  weekdayDay: { weekday: "short", day: "numeric" },
};

/**
 * `Intl.supportedValuesOf` is ES2022 but absent from older Safari and from the
 * kit's `es2022` lib typings, so it is feature-detected rather than assumed.
 */
const intlWithValues = Intl as typeof Intl & { supportedValuesOf?: (key: string) => string[] };

/**
 * Zones offered when `supportedValuesOf` is missing. Deliberately short: the
 * picker falls back to a list a user can still find their city in, and the
 * stored value is validated by Intl either way.
 */
const FALLBACK_ZONES = [
  "UTC",  "Africa/Cairo",
  "Africa/Johannesburg",
  "Africa/Lagos",
  "Africa/Nairobi",
  "America/Chicago",
  "America/Denver",
  "America/Los_Angeles",
  "America/New_York",
  "America/Sao_Paulo",
  "America/Toronto",
  "Asia/Dubai",
  "Asia/Hong_Kong",
  "Asia/Kolkata",
  "Asia/Shanghai",
  "Asia/Singapore",
  "Asia/Tokyo",
  "Australia/Perth",
  "Australia/Sydney",
  "Europe/Amsterdam",
  "Europe/Berlin",
  "Europe/London",
  "Europe/Madrid",
  "Europe/Paris",
  "Europe/Rome",
  "Europe/Stockholm",
  "Europe/Warsaw",
  "Pacific/Auckland",
];

/** The zone this device reports. Safe on a server too — it just answers UTC. */
export function browserTimeZone(): string {
  try {
    return new Intl.DateTimeFormat().resolvedOptions().timeZone || FALLBACK_TIME_ZONE;
  } catch {
    return FALLBACK_TIME_ZONE;
  }
}

/** Built once: `supportedValuesOf` allocates its list on every call. */
let zoneCache: readonly string[] | null = null;

/**
 * Every zone the picker may offer.
 *
 * `Intl.supportedValuesOf("timeZone")` omits `UTC` — the spec's list is CLDR's
 * canonical zones, and UTC is not one of them. It is still a zone a user may
 * legitimately store (it is every account's default), so it is prepended when
 * the platform leaves it out.
 */
export function supportedTimeZones(): readonly string[] {
  if (zoneCache !== null) return zoneCache;
  const fromIntl = intlWithValues.supportedValuesOf?.("timeZone");
  const zones = fromIntl !== undefined && fromIntl.length > 0 ? fromIntl : FALLBACK_ZONES;
  zoneCache = zones.includes(FALLBACK_TIME_ZONE) ? zones : [FALLBACK_TIME_ZONE, ...zones];
  return zoneCache;
}

/**
 * True when Intl can format in this zone. Aliases (`Asia/Calcutta`,
 * `US/Eastern`) pass even though `supportedValuesOf` omits them, which is what
 * a stored value from an older client needs.
 */
export function isTimeZone(value: string | null | undefined): value is string {
  if (typeof value !== "string" || value.trim() === "") return false;
  if (value === FALLBACK_TIME_ZONE) return true;
  if (supportedTimeZones().includes(value)) return true;
  try {
    new Intl.DateTimeFormat(DEFAULT_TIME_LOCALE, { timeZone: value }).format(0);
    return true;
  } catch {
    return false;
  }
}

/**
 * The zone a surface should draw in: the account's when it is real, then the
 * caller's fallback (usually the browser), then UTC. Never returns a value Intl
 * would reject.
 */
export function resolveTimeZone(
  preferred: string | null | undefined,
  fallback?: string | null,
): string {
  if (isTimeZone(preferred)) return preferred;
  if (isTimeZone(fallback)) return fallback;
  const browser = browserTimeZone();
  return isTimeZone(browser) ? browser : FALLBACK_TIME_ZONE;
}

export function toDate(value: string | number | Date): Date | null {
  const date = value instanceof Date ? value : new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

/** Formats one instant in one zone. `—` for an unparseable value or zone. */
export function formatInTimeZone(
  value: string | number | Date,
  timeZone: string,
  kind: TimeKind = "time",
  locale?: string,
): string {
  const date = toDate(value);
  if (date === null) return "—";
  try {
    return new Intl.DateTimeFormat(locale ?? DEFAULT_TIME_LOCALE, {
      ...KIND_OPTIONS[kind],
      timeZone,
    }).format(date);
  } catch {
    return "—";
  }
}

/**
 * Formats a bare `YYYY-MM-DD` day key.
 *
 * A day key is a calendar day, not an instant, so it has no zone of its own and
 * must not be read in the reader's. "Thursday" is a property of `2026-10-15`,
 * and an afternoon west of UTC does not make it Wednesday. The key is therefore
 * read in one neutral frame — midnight UTC, which is the same calendar date in
 * every zone from `UTC-12` to `UTC+12`.
 *
 * Surfaces that group by "today" want {@link dayKeyInTimeZone} instead: that one
 * answers which day an *instant* falls on, and there the zone is the point.
 */
export function formatDayKey(
  key: string | null | undefined,
  kind: TimeKind = "day",
  locale?: string,
): string {
  if (typeof key !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(key)) return "—";
  return formatInTimeZone(`${key}T00:00:00Z`, FALLBACK_TIME_ZONE, kind, locale);
}

/** `SAST`, `GMT+2`, `EDT` — whatever this zone calls itself at that instant. */
export function zoneAbbreviation(timeZone: string, at: Date = new Date()): string {
  try {
    const parts = new Intl.DateTimeFormat(DEFAULT_TIME_LOCALE, {
      timeZone,
      timeZoneName: "short",
    }).formatToParts(at);
    return parts.find((part) => part.type === "timeZoneName")?.value ?? "";
  } catch {
    return "";
  }
}

/** The zone's offset from UTC at that instant, in minutes east of UTC. */
export function timeZoneOffsetMinutes(timeZone: string, at: Date = new Date()): number {
  try {
    const parts = new Intl.DateTimeFormat(DEFAULT_TIME_LOCALE, {
      timeZone,
      hourCycle: "h23",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    }).formatToParts(at);
    const get = (type: Intl.DateTimeFormatPartTypes): number =>
      Number(parts.find((part) => part.type === type)?.value ?? "0");
    const wallClock = Date.UTC(
      get("year"),
      get("month") - 1,
      get("day"),
      get("hour"),
      get("minute"),
      get("second"),
    );
    return Math.round((wallClock - at.getTime()) / 60_000);
  } catch {
    return 0;
  }
}

/** `UTC+02:00`, `UTC-03:30`. */
export function offsetLabel(minutes: number): string {
  const sign = minutes < 0 ? "-" : "+";
  const absolute = Math.abs(minutes);
  const hours = String(Math.floor(absolute / 60)).padStart(2, "0");
  const rest = String(absolute % 60).padStart(2, "0");
  return `UTC${sign}${hours}:${rest}`;
}

export interface TimeZoneOption {
  value: string;
  /** The stored IANA id — what the API takes. */
  label: string;
  /** `UTC+02:00` at the instant the list was built, for the picker row. */
  offset: string;
}

/**
 * Picker rows, alphabetical by IANA id. `current` is appended when the stored
 * zone is a valid alias the platform does not list, so the control never shows
 * a value it cannot render as an option.
 */
export function timeZoneOptions(at: Date = new Date(), current?: string | null): TimeZoneOption[] {
  const zones = [...supportedTimeZones()];
  if (isTimeZone(current) && !zones.includes(current)) zones.push(current);
  return zones
    .map((zone) => ({
      value: zone,
      label: zone,
      offset: offsetLabel(timeZoneOffsetMinutes(zone, at)),
    }))
    .sort((a, b) => a.label.localeCompare(b.label));
}

/** `YYYY-MM-DD` in the given zone — the key grouping and "today" both need. */
export function dayKeyInTimeZone(value: string | number | Date, timeZone: string): string {
  const date = toDate(value);
  if (date === null) return "";
  try {
    const parts = new Intl.DateTimeFormat(DEFAULT_TIME_LOCALE, {
      timeZone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    }).formatToParts(date);
    const get = (type: Intl.DateTimeFormatPartTypes): string =>
      parts.find((part) => part.type === type)?.value ?? "";
    return `${get("year")}-${get("month")}-${get("day")}`;
  } catch {
    return "";
  }
}

