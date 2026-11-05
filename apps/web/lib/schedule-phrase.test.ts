// Table-driven suite for lib/schedule-phrase.
//
// The parser is the lane's whole promise: either the exact cadence vocabulary
// `POST /api/v1/report-schedules` accepts, or a refusal that names the part it
// could not read. Every case below asserts one of those two outcomes, and the
// refusal cases assert the named part as well as the code — a refusal that
// blames the wrong field sends the reader to the wrong place.
import { describe, expect, it } from "vitest";
import {
  formatScheduleSentence,
  parseSchedulePhrase,
  proposalProblem,
  toCadenceFields,
  WEEKDAYS,
  type ScheduleProposal,
} from "./schedule-phrase";

const LONDON = { timezone: "Europe/London" };

interface AcceptedCase {
  text: string;
  options?: { timezone?: string | null };
  cadence: "daily" | "weekly" | "monthly";
  time: string;
  dayOfWeek?: number;
  dayOfMonth?: number;
  timezone: string;
  sentence: string;
}

const ACCEPTED: AcceptedCase[] = [
  {
    text: "daily at 6pm",
    options: LONDON,
    cadence: "daily",
    time: "18:00",
    timezone: "Europe/London",
    sentence: "Every day at 18:00, Europe/London",
  },
  {
    text: "Every Monday at 09:00",
    options: LONDON,
    cadence: "weekly",
    time: "09:00",
    dayOfWeek: 0,
    timezone: "Europe/London",
    sentence: "Every Monday at 09:00, Europe/London",
  },
  {
    text: "monthly on the 15th at 7am",
    options: LONDON,
    cadence: "monthly",
    time: "07:00",
    dayOfMonth: 15,
    timezone: "Europe/London",
    sentence: "Every month on the 15th at 07:00, Europe/London",
  },
  {
    // The zone is in the sentence, so the surface has nothing to infer.
    text: "every sunday at 12:00 in America/New_York",
    options: { timezone: null },
    cadence: "weekly",
    time: "12:00",
    dayOfWeek: 6,
    timezone: "America/New_York",
    sentence: "Every Sunday at 12:00, America/New_York",
  },
  {
    // A zone written in lowercase is canonicalised, not echoed.
    text: "every tuesday at 18:30 europe/london",
    options: { timezone: null },
    cadence: "weekly",
    time: "18:30",
    dayOfWeek: 1,
    timezone: "Europe/London",
    sentence: "Every Tuesday at 18:30, Europe/London",
  },
  {
    text: "daily at noon",
    options: LONDON,
    cadence: "daily",
    time: "12:00",
    timezone: "Europe/London",
    sentence: "Every day at 12:00, Europe/London",
  },
  {
    text: "daily at midnight",
    options: LONDON,
    cadence: "daily",
    time: "00:00",
    timezone: "Europe/London",
    sentence: "Every day at 00:00, Europe/London",
  },
  {
    text: "each day at 08:00 UTC",
    options: { timezone: null },
    cadence: "daily",
    time: "08:00",
    timezone: "UTC",
    sentence: "Every day at 08:00, UTC",
  },
  {
    text: "every friday at 16:45",
    options: LONDON,
    cadence: "weekly",
    time: "16:45",
    dayOfWeek: 4,
    timezone: "Europe/London",
    sentence: "Every Friday at 16:45, Europe/London",
  },
  {
    text: "monthly on the 28th at 06:00",
    options: LONDON,
    cadence: "monthly",
    time: "06:00",
    dayOfMonth: 28,
    timezone: "Europe/London",
    sentence: "Every month on the 28th at 06:00, Europe/London",
  },
  {
    // "the 1st of every month" in the other order reads the same.
    text: "the 1st of every month at 9:15am",
    options: LONDON,
    cadence: "monthly",
    time: "09:15",
    dayOfMonth: 1,
    timezone: "Europe/London",
    sentence: "Every month on the 1st at 09:15, Europe/London",
  },
];

interface RefusedCase {
  text: string;
  options?: { timezone?: string | null };
  code: string;
  part: "cadence" | "day" | "time" | "zone" | "text";
  /** A word the message must carry, so the refusal points at the right thing. */
  says: RegExp;
}

const REFUSED: RefusedCase[] = [
  { text: "   ", code: "empty", part: "text", says: /Type a schedule/ },
  {
    text: "whenever it's busy",
    options: LONDON,
    code: "no_cadence",
    part: "cadence",
    says: /daily, weekly or monthly/,
  },
  {
    text: "asdf",
    options: LONDON,
    code: "no_cadence",
    part: "cadence",
    says: /Name the cadence/,
  },
  {
    text: "every other Tuesday at 9am",
    options: LONDON,
    code: "unsupported_cadence",
    part: "cadence",
    says: /never every other week/,
  },
  {
    text: "biweekly on Monday at 09:00",
    options: LONDON,
    code: "unsupported_cadence",
    part: "cadence",
    says: /every week or every month/,
  },
  {
    text: "every 2 weeks on Monday at 09:00",
    options: LONDON,
    code: "unsupported_cadence",
    part: "cadence",
    says: /every week or every month/,
  },
  {
    text: "hourly at 09:00",
    options: LONDON,
    code: "unsupported_cadence",
    part: "cadence",
    says: /no hourly, quarterly or yearly/,
  },
  {
    text: "every weekday at 8:30",
    options: LONDON,
    code: "unsupported_day",
    part: "day",
    says: /one weekday/,
  },
  {
    text: "every workday at 09:00",
    options: LONDON,
    code: "unsupported_day",
    part: "day",
    says: /one weekday/,
  },
  {
    // The row named this case; the API's monthly shape is a day number, so it
    // is refused rather than read as "the first Monday".
    text: "first Monday of the month at 9",
    options: LONDON,
    code: "unsupported_day",
    part: "day",
    says: /day number from 1 to 28/,
  },
  {
    text: "last day of the month at 9am",
    options: LONDON,
    code: "unsupported_day",
    part: "day",
    says: /day number from 1 to 28/,
  },
  {
    text: "monthly on the 31st at 9am",
    options: LONDON,
    code: "day_out_of_range",
    part: "day",
    says: /1 to 28/,
  },
  {
    text: "monthly at 9am",
    options: LONDON,
    code: "no_day",
    part: "day",
    says: /day from 1 to 28/,
  },
  {
    text: "weekly at 9am",
    options: LONDON,
    code: "no_day",
    part: "day",
    says: /needs a weekday/,
  },
  {
    text: "every Monday",
    options: LONDON,
    code: "no_time",
    part: "time",
    says: /Say the time of day/,
  },
  {
    // "at 8" is 8 in the morning or 8 in the evening; the parser will not pick.
    text: "every Monday at 8",
    options: LONDON,
    code: "ambiguous_time",
    part: "time",
    says: /8am/,
  },
  {
    text: "daily at 25:00",
    options: LONDON,
    code: "time_out_of_range",
    part: "time",
    says: /00:00 to 23:59/,
  },
  {
    // The surface has no zone to infer, and the sentence names none.
    text: "every Monday at 09:00",
    options: { timezone: null },
    code: "no_zone",
    part: "zone",
    says: /Name the time zone/,
  },
  {
    text: "daily at 08:00 Mars/Olympus",
    options: LONDON,
    code: "unknown_zone",
    part: "zone",
    says: /not a time zone/,
  },
  {
    text: "daily on Monday at 9am",
    options: LONDON,
    code: "conflict",
    part: "cadence",
    says: /contradicts/,
  },
  {
    text: "every Monday morning at 9am",
    options: LONDON,
    code: "unreadable",
    part: "text",
    says: /could not read/,
  },
];

describe("parseSchedulePhrase — accepted sentences", () => {
  it.each(ACCEPTED.map((c) => [c.text, c] as const))("reads %s", (_label, c) => {
    const result = parseSchedulePhrase(c.text, c.options ?? {});
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.proposal.cadence).toBe(c.cadence);
    expect(result.proposal.timeOfDay).toBe(c.time);
    expect(result.proposal.dayOfWeek).toBe(c.dayOfWeek ?? null);
    expect(result.proposal.dayOfMonth).toBe(c.dayOfMonth ?? null);
    expect(result.proposal.timezone).toBe(c.timezone);
    expect(result.sentence).toBe(c.sentence);
  });
});

describe("parseSchedulePhrase — refusals name the part", () => {
  it.each(REFUSED.map((c) => [c.text, c] as const))("refuses %s", (_label, c) => {
    const result = parseSchedulePhrase(c.text, c.options ?? {});
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.code).toBe(c.code);
    expect(result.part).toBe(c.part);
    expect(result.raw).toBe(c.text);
    expect(result.message).toMatch(c.says);
  });

  it("keeps the raw sentence exactly as typed, for the input to keep showing", () => {
    const raw = "Every other Tuesday at 9am";
    const result = parseSchedulePhrase(raw, LONDON);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.raw).toBe(raw);
  });
});

describe("fields the route reads", () => {
  it("nulls the day field the cadence does not carry (migration 0038)", () => {
    const weekly = parseSchedulePhrase("every Monday at 09:00", LONDON);
    if (!weekly.ok) throw new Error("expected a proposal");
    expect(toCadenceFields(weekly.proposal)).toEqual({
      cadence: "weekly",
      time_of_day: "09:00",
      day_of_week: 0,
      day_of_month: null,
      timezone: "Europe/London",
    });

    const monthly = parseSchedulePhrase("monthly on the 15th at 07:00", LONDON);
    if (!monthly.ok) throw new Error("expected a proposal");
    expect(toCadenceFields(monthly.proposal)).toEqual({
      cadence: "monthly",
      time_of_day: "07:00",
      day_of_week: null,
      day_of_month: 15,
      timezone: "Europe/London",
    });

    const daily = parseSchedulePhrase("daily at 6pm", LONDON);
    if (!daily.ok) throw new Error("expected a proposal");
    expect(toCadenceFields(daily.proposal)).toEqual({
      cadence: "daily",
      time_of_day: "18:00",
      day_of_week: null,
      day_of_month: null,
      timezone: "Europe/London",
    });
  });

  it("reports an edited proposal that the route would refuse", () => {
    const base: ScheduleProposal = {
      cadence: "weekly",
      timeOfDay: "09:00",
      dayOfWeek: 0,
      dayOfMonth: null,
      timezone: "Europe/London",
    };
    expect(proposalProblem(base)).toBeNull();
    expect(proposalProblem({ ...base, dayOfWeek: null })).toMatch(/weekday/);
    expect(proposalProblem({ ...base, timeOfDay: "9:00" })).toMatch(/HH:MM/);
    expect(proposalProblem({ ...base, timeOfDay: "24:00" })).toMatch(/HH:MM/);
    expect(proposalProblem({ ...base, timezone: "Mars/Olympus" })).toMatch(/time zone/);
    expect(proposalProblem({ ...base, cadence: "monthly", dayOfMonth: 31 })).toMatch(/1 to 28/);
    expect(proposalProblem({ ...base, cadence: "monthly", dayOfWeek: null, dayOfMonth: 0 })).toMatch(
      /1 to 28/,
    );
  });

  it("renders the sentence from fields alone, so an edit rewrites it", () => {
    expect(
      formatScheduleSentence({
        cadence: "weekly",
        timeOfDay: "06:15",
        dayOfWeek: 3,
        dayOfMonth: null,
        timezone: "UTC",
      }),
    ).toBe(`Every ${WEEKDAYS[3]} at 06:15, UTC`);
  });
});
