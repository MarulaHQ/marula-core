"use client";
/**
 * LocalTime — one instant, drawn in the reader's own zone.
 *
 * Purpose: the answer for the ~140 surfaces that print a time to the person
 * looking at it — a log row, a table cell, "last run at". Each of them used to
 * call `toLocaleString()`, which formats in whatever zone the machine is in; the
 * reader's zone is the one their account stores (`useTimeFormat()`), and the
 * device's only when the account has none — which is also the right answer on a
 * share link a stranger opens.
 *
 * Why a wrapper. The kit's `<Time>` takes the zone as a prop on purpose, so a
 * surface cannot inherit the machine's by accident. That forces the app to make
 * the decision once, and `useTimeFormat()` is the decision. This component is
 * where it is spent, so a call site reads as what it means: this reading is mine.
 *
 * Non-purpose: no window of two instants (`components/record-window.tsx` draws
 * the approvals window). `RecordTime`, below, is the single-instant form.
 */
import { DualTime, Time, type TimeKind } from "@marula-core/ui";
import type { ComponentProps } from "react";
import { useTimeDisplay } from "../lib/use-time-display";
import { useTimeFormat } from "../lib/use-time-format";

export interface LocalTimeProps extends Omit<ComponentProps<"time">, "children" | "dateTime"> {
  /** ISO string, epoch millis or Date. A missing value draws the dash. */
  value: string | number | Date | null | undefined;
  /** The shape to draw. Defaults to the full date and clock. */
  kind?: TimeKind;
}

export function LocalTime({ value, kind = "datetime", className, ...rest }: LocalTimeProps) {
  const { timeZone } = useTimeFormat();

  // `new Date(null)` is the epoch, so a missing value has to be turned away
  // before it reaches the formatter — a blank cell is honest, 1970 is not.
  if (value === null || value === undefined || value === "") {
    return (
      <time className={className} {...rest}>
        —
      </time>
    );
  }

  return <Time value={value} timeZone={timeZone} kind={kind} className={className} {...rest} />;
}

export interface RecordTimeProps extends Omit<ComponentProps<"time">, "children" | "dateTime"> {
  /** ISO string, epoch millis or Date. */
  value: string | number | Date | null | undefined;
  /** The zone the record belongs to — a site, a calendar, a run schedule. */
  zone: string | null | undefined;
  kind?: TimeKind;
  /** Print the record zone's abbreviation beside the second reading. */
  showZone?: boolean;
}

/**
 * RecordTime — one instant that belongs to a record, drawn against the reader.
 *
 * The record's zone arrives raw from the read model and is resolved here
 * through the chain T2 built: the record's own value, then the org default, then
 * UTC. The reader's switch decides whether the second clock appears at all, and
 * `DualTime` says nothing when the two zones agree — so a row that used to draw
 * one clock still draws one clock, and a row that needs two now gets them.
 *
 * Non-purpose: no window of two instants (see `components/record-window.tsx`),
 * and no fetching — `useTimeDisplay()` already holds the org default.
 */
export function RecordTime({
  value,
  zone,
  kind = "datetime",
  showZone,
  className,
  ...rest
}: RecordTimeProps) {
  const time = useTimeFormat();
  const display = useTimeDisplay();
  const recordZone = display.zoneFor(zone, time.timeZone);

  if (value === null || value === undefined || value === "") {
    return (
      <time className={className} {...rest}>
        —
      </time>
    );
  }

  return (
    <DualTime
      value={value}
      zone={recordZone}
      viewerZone={time.timeZone}
      kind={kind}
      showZone={showZone}
      className={className}
      {...rest}
    />
  );
}
