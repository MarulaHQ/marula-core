"use client";
/**
 * Time — the kit's clock primitives.
 *
 * Purpose: render an instant in a named zone, and render the same instant in
 * two zones side by side when the viewer's zone is not the record's zone.
 * `DualTime` is the honest treatment for a record that belongs to a site or
 * calendar in another zone: the primary reading is the viewer's, the secondary
 * is what the people who own the record see.
 *
 * Non-purpose: no fetching, no auth, no default zone. Both components take the
 * zone as a prop so a surface cannot accidentally format in the OS zone — the
 * app resolves zones in `useTimeFormat()`. See `lib/time.ts` for formatting.
 *
 * Motion law: nothing animates here. Times change in place.
 */
import { forwardRef, type ComponentProps } from "react";
import { cn } from "../lib/cn";
import {
  formatInTimeZone,
  toDate,
  zoneAbbreviation,
  type TimeKind,
} from "../lib/time";

export interface TimeProps extends Omit<ComponentProps<"time">, "children" | "dateTime"> {
  /** ISO string, epoch millis or Date. */
  value: string | number | Date;
  /** IANA zone the reading is drawn in. */
  timeZone: string;
  kind?: TimeKind;
  locale?: string;
}

/**
 * One instant, one zone. Rendered as `<time>` with the machine-readable
 * `datetime` attribute and the zone in the tooltip, so a copied value and a
 * hovered value both say where the reading came from.
 */
export const Time = forwardRef<HTMLTimeElement, TimeProps>(function Time(
  { value, timeZone, kind = "time", locale, className, title, ...rest },
  ref,
) {
  const iso = toDate(value)?.toISOString();
  const text = formatInTimeZone(value, timeZone, kind, locale);
  return (
    <time
      ref={ref}
      dateTime={iso}
      className={cn("m-time", className)}
      title={title ?? `${iso ?? "—"} · ${timeZone}`}
      {...rest}
    >
      {text}
    </time>
  );
});

Time.displayName = "Time";

export interface DualTimeProps extends Omit<ComponentProps<"time">, "children" | "dateTime"> {
  /** ISO string, epoch millis or Date. */
  value: string | number | Date;
  /** The zone the record belongs to — a site, a calendar, an SLA window. */
  zone: string;
  /** The zone the reader is in. Resolve it from `useTimeFormat()`. */
  viewerZone: string;
  kind?: TimeKind;
  locale?: string;
  /** Print the record zone's abbreviation beside the second reading. */
  showZone?: boolean;
}

/**
 * The same instant in the reader's zone and the record's zone.
 *
 * When both zones agree the component renders exactly what `Time` renders —
 * a dual reading that says the same thing twice is noise. When they differ, the
 * record's reading follows in muted type, and the whole element carries one
 * `aria-label` so a screen reader hears one coherent sentence rather than two
 * bare clock readings.
 */
export const DualTime = forwardRef<HTMLTimeElement, DualTimeProps>(function DualTime(
  { value, zone, viewerZone, kind = "time", locale, showZone = true, className, title, ...rest },
  ref,
) {
  const date = toDate(value);
  const iso = date?.toISOString();
  const primary = formatInTimeZone(value, viewerZone, kind, locale);

  if (zone === viewerZone) {
    return (
      <Time
        ref={ref}
        value={value}
        timeZone={viewerZone}
        kind={kind}
        locale={locale}
        className={className}
        title={title}
        {...rest}
      />
    );
  }

  const secondary = formatInTimeZone(value, zone, kind, locale);
  const abbreviation = showZone ? zoneAbbreviation(zone, date ?? new Date()) : "";
  const secondaryText = abbreviation === "" ? secondary : `${secondary} ${abbreviation}`;

  return (
    <time
      ref={ref}
      dateTime={iso}
      className={cn("m-dual-time", className)}
      title={title ?? `${iso ?? "—"} · recorded in ${zone}`}
      {...rest}
    >
      <span className="m-dual-time-primary">{primary}</span>
      <span className="m-dual-time-alt">
        <span className="m-visually-hidden">recorded </span>
        {secondaryText}
      </span>
    </time>
  );
});

DualTime.displayName = "DualTime";
