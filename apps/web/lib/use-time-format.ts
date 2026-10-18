"use client";
/**
 * useTimeFormat — the zone a surface should draw in, and the formatters for it.
 *
 * Purpose: one answer to "which zone is this reader in?" for every surface that
 * prints a time. The account's stored zone wins; when it is missing or not a
 * zone Intl knows, the device's zone is the fallback; UTC is the last resort.
 *
 * Non-purpose: no picker and no persistence — `/settings/preferences` writes
 * the account zone through `PATCH /api/v1/users/me`, and this hook reads what
 * `AuthProvider` holds.
 *
 * Mount safety: the device zone is read in an effect, not during render, so the
 * server's UTC draw and the first client draw match during hydration. Until the
 * effect runs, `source` is `"pending"` and `timeZone` is the stored zone or UTC.
 */
import { useEffect, useMemo, useState } from "react";
import {
  FALLBACK_TIME_ZONE,
  browserTimeZone,
  dayKeyInTimeZone,
  formatInTimeZone,
  isTimeZone,
  type TimeKind,
} from "@marula-core/ui";
import { useAuth } from "./api";

/** Where the resolved zone came from. `pending` is one render, before mount. */
export type TimeZoneSource = "user" | "browser" | "pending";

export interface TimeFormat {
  /** The zone to pass to `<Time>` / `<DualTime>` and to the formatters. */
  timeZone: string;
  source: TimeZoneSource;
  /** The device's zone once mounted, so a settings hint can name it. */
  deviceTimeZone: string | null;
  format(value: string | number | Date, kind?: TimeKind): string;
  /** `YYYY-MM-DD` in {@link timeZone} — grouping and "today" both need it. */
  dayKey(value: string | number | Date): string;
}

export function useTimeFormat(): TimeFormat {
  const me = useAuth().me;
  const [deviceTimeZone, setDeviceTimeZone] = useState<string | null>(null);

  useEffect(() => {
    setDeviceTimeZone(browserTimeZone());
  }, []);

  const stored = me?.timezone ?? null;
  const storedIsZone = isTimeZone(stored);
  const timeZone = storedIsZone ? stored : (deviceTimeZone ?? FALLBACK_TIME_ZONE);
  const source: TimeZoneSource = storedIsZone
    ? "user"
    : deviceTimeZone === null
      ? "pending"
      : "browser";

  return useMemo(
    () => ({
      timeZone,
      source,
      deviceTimeZone,
      format: (value: string | number | Date, kind: TimeKind = "time") =>
        formatInTimeZone(value, timeZone, kind),
      dayKey: (value: string | number | Date) => dayKeyInTimeZone(value, timeZone),
    }),
    [timeZone, source, deviceTimeZone],
  );
}
