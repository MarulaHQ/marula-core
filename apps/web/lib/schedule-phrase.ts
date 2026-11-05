// lib/schedule-phrase — the deterministic half of "describe a schedule".
//
// Purpose: read one sentence a person wrote and return either the exact cadence
// vocabulary `POST /api/v1/report-schedules` accepts, or a refusal that names
// the part it could not read. Nothing here guesses, defaults silently, or picks
// a "closest match": every branch either produces fields the route will take or
// says why it will not.
//
// The wire vocabulary is the whole contract (`crates/core-api/src/routes/
// report_schedules.rs:126`): cadence `daily|weekly|monthly`, a wall clock
// `HH:MM`, `day_of_week` 0 (Monday) .. 6 (Sunday) for weekly, `day_of_month`
// 1..=28 for monthly, and an IANA zone. Two shapes the route cannot express are
// refused by name rather than bent into a neighbouring cadence: "every weekday"
// (weekly is one weekday) and "first Monday of the month" (monthly is a day
// number). The day-of-month bound is the API's, not a simplification we chose —
// `crates/core-db/migrations/0038_report_schedules.sql:54` keeps 1..28 so every
// month has the day.
//
// Non-purpose: no next-run math. DST belongs to `core-domain::cadence` on the
// server, and reimplementing it here would give the two surfaces a second
// answer to disagree with.

import type { components } from "@marula-core/sdk";

export type ScheduleCadenceSlug = components["schemas"]["ScheduleCadence"];

/** Monday-first weekday names, the order `day_of_week` uses. */
export const WEEKDAYS = [
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
  "Sunday",
] as const;

/** The body fragment the schedules create/update routes read. */
export interface ScheduleCadenceFields {
  cadence: ScheduleCadenceSlug;
  /** `HH:MM`, 24-hour, read in `timezone`. */
  time_of_day: string;
  /** Weekly only; null for every other cadence (migration 0038's CHECK). */
  day_of_week: number | null;
  /** Monthly only; null for every other cadence. */
  day_of_month: number | null;
  timezone: string;
}

/** A schedule the parser (or an edited field) resolved completely. */
export interface ScheduleProposal {
  cadence: ScheduleCadenceSlug;
  /** `HH:MM` 24-hour. */
  timeOfDay: string;
  /** 0 = Monday .. 6 = Sunday; null unless weekly. */
  dayOfWeek: number | null;
  /** 1..=28; null unless monthly. */
  dayOfMonth: number | null;
  /** IANA name, canonicalised through `Intl`. */
  timezone: string;
}

/** Which part of the sentence could not be read. */
export type UnreadPart = "cadence" | "day" | "time" | "zone" | "text";

export type RefusalCode =
  | "empty"
  | "no_cadence"
  | "no_time"
  | "no_day"
  | "ambiguous_time"
  | "time_out_of_range"
  | "unsupported_cadence"
  | "unsupported_day"
  | "day_out_of_range"
  | "no_zone"
  | "unknown_zone"
  | "conflict"
  | "unreadable";

/** A refusal that keeps the sentence a person can act on. */
export interface SchedulePhraseRefusal {
  ok: false;
  code: RefusalCode;
  part: UnreadPart;
  /** One sentence naming the part and what would make it readable. */
  message: string;
  /** The text exactly as it was typed, so the input can keep showing it. */
  raw: string;
}

export type SchedulePhraseResult =
  | { ok: true; proposal: ScheduleProposal; sentence: string }
  | SchedulePhraseRefusal;

/** Options the calling surface can supply. */
export interface SchedulePhraseOptions {
  /**
   * The zone the surface can infer — the browser's own zone, a schedule row's
   * stored zone. `null` means the surface has nothing to infer from, and a
   * sentence that does not name a zone is then refused instead of assumed.
   */
  timezone?: string | null;
}

const WEEKDAY_ALIASES: Record<string, number> = {
  monday: 0,
  mon: 0,
  tuesday: 1,
  tue: 1,
  tues: 1,
  wednesday: 2,
  wed: 2,
  thursday: 3,
  thu: 3,
  thur: 3,
  thurs: 3,
  friday: 4,
  fri: 4,
  saturday: 5,
  sat: 5,
  sunday: 6,
  sun: 6,
};

/** Words a person writes around the parts, carrying no cadence of their own. */
const FILLER = new Set([
  "a",
  "an",
  "and",
  "at",
  "each",
  "every",
  "for",
  "from",
  "in",
  "it",
  "its",
  "next",
  "o",
  "clock",
  "oclock",
  "of",
  "on",
  "please",
  "repeat",
  "repeats",
  "run",
  "runs",
  "schedule",
  "scheduled",
  "starting",
  "the",
  "time",
  "to",
]);

const UNSUPPORTED_INTERVAL =
  /\b(every other|bi-?weekly|fortnightly|alternate|every (two|2) weeks?|every other week)\b/;
const UNSUPPORTED_CADENCE_WORD =
  /\b(hourly|every hour|quarterly|every quarter|yearly|annually|every year|once a year)\b/;
const UNSUPPORTED_WEEKDAYS =
  /\b(every )?(weekdays?|workdays?|business days?)\b|\bweekday\b/;
const UNSUPPORTED_NTH_WEEKDAY =
  /\b(first|second|third|fourth|fifth|last)\s+(monday|tuesday|wednesday|thursday|friday|saturday|sunday|mon|tue|tues|wed|thu|thur|thurs|fri|sat|sun)\b/;
const UNSUPPORTED_LAST_DAY = /\blast day( of (the )?month)?\b/;
const NO_CADENCE_PHRASE = /\b(whenever|when it'?s busy|when busy|as needed|on demand|ad ?hoc)\b/;

/**
 * Read one sentence into the schedules API's cadence vocabulary.
 *
 * Pure and deterministic: the same text and options always produce the same
 * answer, and a sentence with no zone in it is refused when `options.timezone`
 * is absent rather than read in some default zone.
 */
export function parseSchedulePhrase(
  raw: string,
  options: SchedulePhraseOptions = {},
): SchedulePhraseResult {
  const refusal = (
    code: RefusalCode,
    part: UnreadPart,
    message: string,
  ): SchedulePhraseRefusal => ({ ok: false, code, part, message, raw });

  const text = normalise(raw);
  if (text === "") {
    return refusal(
      "empty",
      "text",
      "Type a schedule first — for example “every Monday at 09:00, Europe/London”.",
    );
  }

  const zone = extractZone(text);
  if (zone.kind === "invalid") {
    return refusal(
      "unknown_zone",
      "zone",
      `“${zone.token}” is not a time zone I can read. Use an IANA name such as “Europe/London” or “America/New_York”.`,
    );
  }
  let rest = zone.rest;

  // Unsupported shapes are checked before the supported ones they resemble, so
  // "every other Tuesday" can never be read as "every Tuesday".
  if (UNSUPPORTED_INTERVAL.test(rest)) {
    return refusal(
      "unsupported_cadence",
      "cadence",
      "Schedules fire every week or every month, never every other week. Pick one weekday, or create two schedules a week apart.",
    );
  }
  if (UNSUPPORTED_CADENCE_WORD.test(rest)) {
    return refusal(
      "unsupported_cadence",
      "cadence",
      "Schedules fire daily, weekly or monthly. This one has no hourly, quarterly or yearly cadence to save it as.",
    );
  }
  if (UNSUPPORTED_WEEKDAYS.test(rest)) {
    return refusal(
      "unsupported_day",
      "day",
      "A weekly schedule fires on one weekday. Pick Monday through Sunday, or create one schedule per weekday.",
    );
  }
  if (UNSUPPORTED_NTH_WEEKDAY.test(rest) || UNSUPPORTED_LAST_DAY.test(rest)) {
    return refusal(
      "unsupported_day",
      "day",
      "A monthly schedule fires on a day number from 1 to 28, not on a counted weekday. Give a date instead, for example “monthly on the 15th”.",
    );
  }
  if (NO_CADENCE_PHRASE.test(rest)) {
    return refusal(
      "no_cadence",
      "cadence",
      "This schedule fires on a fixed cadence — daily, weekly or monthly. “Whenever it is busy” is not one of the three.",
    );
  }

  // The cadence is settled before the time, so a sentence with neither names the
  // part a person has to add first.
  const weekday = extractWeekday(rest);
  const cadenceWord = extractCadenceWord(rest);
  if (weekday !== null && cadenceWord === "daily") {
    return refusal(
      "conflict",
      "cadence",
      `“Daily” already fires every day, so “${WEEKDAYS[weekday]}” contradicts it. Drop one of the two.`,
    );
  }
  if (weekday !== null && cadenceWord === "monthly") {
    return refusal(
      "conflict",
      "day",
      "A monthly schedule fires on a day number and a weekly one on a weekday. These cannot be the same schedule.",
    );
  }
  const cadence: ScheduleCadenceSlug | null =
    cadenceWord ?? (weekday === null ? null : "weekly");
  if (cadence === null) {
    return refusal(
      "no_cadence",
      "cadence",
      "Name the cadence: “daily”, “every Monday”, or “monthly on the 15th”.",
    );
  }

  const time = extractTime(rest);
  if (time.kind === "missing") {
    return refusal(
      "no_time",
      "time",
      "Say the time of day, for example “every Monday at 09:00” or “daily at 6pm”.",
    );
  }
  if (time.kind === "ambiguous") {
    return refusal(
      "ambiguous_time",
      "time",
      `“${time.token}” has no am or pm, and 8 in the morning and 8 in the evening are both 8. Write “8am”, “8pm”, or “08:00”.`,
    );
  }
  if (time.kind === "range") {
    return refusal(
      "time_out_of_range",
      "time",
      "A time of day runs from 00:00 to 23:59. Pick an hour inside that range.",
    );
  }
  rest = time.rest;

  if (cadence === "weekly" && weekday === null) {
    return refusal(
      "no_day",
      "day",
      "A weekly schedule needs a weekday, for example “every Monday at 09:00”.",
    );
  }

  let dayOfMonth: number | null = null;
  if (cadence === "monthly") {
    const day = extractDayOfMonth(rest);
    if (day.kind === "missing") {
      return refusal(
        "no_day",
        "day",
        "A monthly schedule needs a day from 1 to 28, for example “monthly on the 15th at 07:00”.",
      );
    }
    if (day.kind === "range") {
      return refusal(
        "day_out_of_range",
        "day",
        `Monthly schedules fire on days 1 to 28 so every month has the day; the ${day.value}${ordinalSuffix(day.value)} would skip February. Pick a day from 1 to 28.`,
      );
    }
    dayOfMonth = day.value;
  }

  const leftover = residue(rest);
  if (leftover !== null) {
    return refusal(
      "unreadable",
      "text",
      `I could not read “${leftover}”. Describe the cadence and time, for example “every Monday at 09:00, Europe/London”.`,
    );
  }

  const timezone = resolveZone(zone.zone, options.timezone);
  if (timezone.kind === "missing") {
    return refusal(
      "no_zone",
      "zone",
      "Name the time zone the wall clock reads in, for example “Europe/London”. Without one, 09:00 is a different moment for every reader.",
    );
  }
  if (timezone.kind === "invalid") {
    return refusal(
      "unknown_zone",
      "zone",
      `“${timezone.token}” is not a time zone I can read. Use an IANA name such as “Europe/London”.`,
    );
  }

  const proposal: ScheduleProposal = {
    cadence,
    timeOfDay: time.value,
    dayOfWeek: cadence === "weekly" ? weekday : null,
    dayOfMonth,
    timezone: timezone.value,
  };
  return { ok: true, proposal, sentence: formatScheduleSentence(proposal) };
}

/**
 * The sentence the fields spell out. Re-rendering from the fields after an edit
 * is what keeps the proposal and what will be saved the same statement.
 */
export function formatScheduleSentence(proposal: ScheduleProposal): string {
  const time = proposal.timeOfDay;
  const body =
    proposal.cadence === "daily"
      ? `Every day at ${time}`
      : proposal.cadence === "weekly" && proposal.dayOfWeek !== null
        ? `Every ${WEEKDAYS[proposal.dayOfWeek]} at ${time}`
        : proposal.cadence === "monthly" && proposal.dayOfMonth !== null
          ? `Every month on the ${proposal.dayOfMonth}${ordinalSuffix(proposal.dayOfMonth)} at ${time}`
          : `Every ${proposal.cadence} at ${time}`;
  return proposal.timezone === "" ? body : `${body}, ${proposal.timezone}`;
}

/**
 * The create/update body fragment, with the day field each cadence does not
 * carry nulled as migration 0038's CHECK requires. Callers gate on
 * [`proposalProblem`] first; this function does not silently repair a proposal.
 */
export function toCadenceFields(proposal: ScheduleProposal): ScheduleCadenceFields {
  return {
    cadence: proposal.cadence,
    time_of_day: proposal.timeOfDay,
    day_of_week: proposal.cadence === "weekly" ? proposal.dayOfWeek : null,
    day_of_month: proposal.cadence === "monthly" ? proposal.dayOfMonth : null,
    timezone: proposal.timezone,
  };
}

/**
 * Why an edited proposal cannot be saved, or null when every field is one the
 * route accepts. Same rules the parser enforces, applied to fields a person
 * typed by hand.
 */
export function proposalProblem(proposal: ScheduleProposal): string | null {
  if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(proposal.timeOfDay)) {
    return "Write the time as HH:MM, from 00:00 to 23:59.";
  }
  if (proposal.cadence === "weekly") {
    if (proposal.dayOfWeek === null || proposal.dayOfWeek < 0 || proposal.dayOfWeek > 6) {
      return "A weekly schedule needs a weekday.";
    }
  } else if (proposal.cadence === "monthly") {
    if (proposal.dayOfMonth === null || proposal.dayOfMonth < 1 || proposal.dayOfMonth > 28) {
      return "A monthly schedule needs a day from 1 to 28.";
    }
  }
  if (canonicalZone(proposal.timezone) === null) {
    return "That is not a time zone the schedule can read. Use an IANA name such as “Europe/London”.";
  }
  return null;
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

function normalise(raw: string): string {
  return raw
    .toLowerCase()
    .replace(/[“”"'’]/g, " ")
    .replace(/[,;!?]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

type ZoneExtraction = { kind: "ok"; rest: string; zone: string | null };

function extractZone(text: string): ZoneExtraction | { kind: "invalid"; token: string } {
  const kept: string[] = [];
  let zone: string | null = null;
  for (const token of text.split(" ")) {
    if (token === "") continue;
    if (zone === null && (token === "utc" || token === "gmt")) {
      zone = "UTC";
      continue;
    }
    if (zone === null && looksLikeIanaZone(token)) {
      const canonical = canonicalZone(token);
      if (canonical === null) return { kind: "invalid", token };
      zone = canonical;
      continue;
    }
    kept.push(token);
  }
  return { kind: "ok", rest: kept.join(" "), zone };
}

function looksLikeIanaZone(token: string): boolean {
  return /^[a-z]+(?:_[a-z]+)*(?:\/[a-z]+(?:_[a-z]+)*)+$/.test(token);
}

/** `Intl` is the only zone authority in the browser; a bad name throws here. */
function canonicalZone(token: string): string | null {
  if (token.trim() === "") return null;
  try {
    return new Intl.DateTimeFormat("en-US", { timeZone: token }).resolvedOptions().timeZone;
  } catch {
    return null;
  }
}

type TimeExtraction =
  | { kind: "ok"; rest: string; value: string }
  | { kind: "missing" }
  | { kind: "ambiguous"; rest: string; token: string }
  | { kind: "range"; rest: string };

const CLOCK_24 = /\b([01]?\d|2[0-3]):([0-5]\d)\b/;
const CLOCK_12 = /\b(1[0-2]|0?[1-9])(?::([0-5]\d))?\s?(am|pm)\b/;
const BARE_HOUR = /\b(?:at|@)\s?(\d{1,2})\b/;
const OUT_OF_RANGE = /\b(2[4-9]|[3-9]\d):([0-5]\d)\b/;

function extractTime(text: string): TimeExtraction {
  let rest = text;

  const words = /\b(noon|midday|midnight)\b/.exec(rest);
  if (words !== null) {
    const value = words[1] === "midnight" ? "00:00" : "12:00";
    rest = splice(rest, words.index, words[0].length);
    return { kind: "ok", rest, value };
  }

  const twelve = CLOCK_12.exec(rest);
  if (twelve !== null) {
    const hour = Number.parseInt(twelve[1], 10) % 12 + (twelve[3] === "pm" ? 12 : 0);
    const minute = twelve[2] === undefined ? 0 : Number.parseInt(twelve[2], 10);
    rest = splice(rest, twelve.index, twelve[0].length);
    return { kind: "ok", rest, value: hhmm(hour, minute) };
  }

  const outOfRange = OUT_OF_RANGE.exec(rest);
  if (outOfRange !== null) {
    return { kind: "range", rest: splice(rest, outOfRange.index, outOfRange[0].length) };
  }

  const twentyFour = CLOCK_24.exec(rest);
  if (twentyFour !== null) {
    const value = hhmm(Number.parseInt(twentyFour[1], 10), Number.parseInt(twentyFour[2], 10));
    rest = splice(rest, twentyFour.index, twentyFour[0].length);
    return { kind: "ok", rest, value };
  }

  const bare = BARE_HOUR.exec(rest);
  if (bare !== null) {
    const hour = Number.parseInt(bare[1], 10);
    const token = bare[0].trim();
    rest = splice(rest, bare.index, bare[0].length);
    if (hour > 23) return { kind: "range", rest };
    return { kind: "ambiguous", rest, token };
  }

  return { kind: "missing" };
}

function extractWeekday(text: string): number | null {
  for (const token of text.split(" ")) {
    const bare = token.replace(/s$/, "");
    if (token in WEEKDAY_ALIASES) return WEEKDAY_ALIASES[token] ?? null;
    if (bare in WEEKDAY_ALIASES) return WEEKDAY_ALIASES[bare] ?? null;
  }
  return null;
}

type CadenceWord = ScheduleCadenceSlug | null;

function extractCadenceWord(text: string): CadenceWord {
  if (/\b(daily|every day|each day|once a day)\b/.test(text)) return "daily";
  if (/\b(monthly|every month|each month|once a month)\b/.test(text)) return "monthly";
  if (/\b(weekly|every week|each week|once a week)\b/.test(text)) return "weekly";
  return null;
}

type DayExtraction =
  | { kind: "ok"; value: number }
  | { kind: "missing" }
  | { kind: "range"; value: number };

function extractDayOfMonth(text: string): DayExtraction {
  const match = /\b(\d{1,2})(?:st|nd|rd|th)?\b/.exec(text);
  if (match === null) return { kind: "missing" };
  const value = Number.parseInt(match[1], 10);
  if (value < 1 || value > 31) return { kind: "range", value };
  if (value > 28) return { kind: "range", value };
  return { kind: "ok", value };
}

/**
 * What is left after every recognised part is struck out. A word this parser
 * does not know is reported, not ignored: dropping it is how "every other
 * Tuesday" becomes "every Tuesday".
 */
function residue(text: string): string | null {
  const stripped = text
    .replace(CLOCK_12, " ")
    .replace(CLOCK_24, " ")
    .replace(/\b(noon|midday|midnight)\b/g, " ")
    .replace(/\b(daily|every day|each day|once a day)\b/g, " ")
    .replace(/\b(monthly|every month|each month|once a month)\b/g, " ")
    .replace(/\b(weekly|every week|each week|once a week)\b/g, " ")
    .replace(/\b(\d{1,2})(?:st|nd|rd|th)?\b/g, " ")
    .replace(/\b(am|pm)\b/g, " ");

  for (const token of stripped.split(" ")) {
    if (token === "") continue;
    if (token in WEEKDAY_ALIASES) continue;
    if (token.replace(/s$/, "") in WEEKDAY_ALIASES) continue;
    if (FILLER.has(token)) continue;
    if (/^\d+$/.test(token)) continue;
    return token;
  }
  return null;
}

type ZoneResolution =
  | { kind: "ok"; value: string }
  | { kind: "missing" }
  | { kind: "invalid"; token: string };

function resolveZone(
  fromPhrase: string | null,
  fromSurface: string | null | undefined,
): ZoneResolution {
  const candidate = fromPhrase ?? (fromSurface === null || fromSurface === undefined ? null : fromSurface);
  if (candidate === null || candidate.trim() === "") return { kind: "missing" };
  const canonical = canonicalZone(candidate);
  if (canonical === null) return { kind: "invalid", token: candidate };
  return { kind: "ok", value: canonical };
}

function splice(text: string, start: number, length: number): string {
  return `${text.slice(0, start)} ${text.slice(start + length)}`.replace(/\s+/g, " ").trim();
}

function hhmm(hour: number, minute: number): string {
  return `${hour.toString().padStart(2, "0")}:${minute.toString().padStart(2, "0")}`;
}

function ordinalSuffix(value: number): string {
  if (value % 100 >= 11 && value % 100 <= 13) return "th";
  switch (value % 10) {
    case 1:
      return "st";
    case 2:
      return "nd";
    case 3:
      return "rd";
    default:
      return "th";
  }
}
