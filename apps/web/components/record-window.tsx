"use client";
/**
 * RecordWindow — the window a record happened in, in the record's own zone.
 *
 * Purpose: approvals are read in the approver's frame (their day, their clock),
 * but the work was logged in the member's. This draws that second reading as a
 * compact `recorded 09:00–11:00 UTC` aside, and draws nothing at all when the
 * two zones agree or the reader has turned record time off — the same collapse
 * `DualTime` performs for a single instant.
 *
 * Non-purpose: no formatting of the primary reading (the surface owns its own
 * frame) and no fetching — `useTimeDisplay()` already carries the org default
 * and the reader's switch.
 */
import { formatInTimeZone, zoneAbbreviation } from "@marula-core/ui";
import { useTimeDisplay } from "../lib/use-time-display";
import { useTimeFormat } from "../lib/use-time-format";

export function RecordWindow({
  start,
  end,
  zone,
}: {
  start: string | null | undefined;
  end: string | null | undefined;
  /** The record's own zone, from the row's read model. */
  zone: string | null | undefined;
}) {
  const time = useTimeFormat();
  const display = useTimeDisplay();
  const record = display.zoneFor(zone, time.timeZone);
  if (record === time.timeZone || !start || !end) return null;
  return (
    <span className="m-dual-time-alt" data-testid="record-window">
      <span className="m-visually-hidden">recorded </span>
      {formatInTimeZone(start, record, "time")}–{formatInTimeZone(end, record, "time")}{" "}
      {zoneAbbreviation(record)}
    </span>
  );
}
