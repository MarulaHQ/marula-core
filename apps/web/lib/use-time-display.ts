"use client";
/**
 * useTimeDisplay — the org's default zone and the reader's second-clock switch.
 *
 * Purpose: a surface that draws a record's clock needs two values the auth
 * payload does not carry. The org default (`general.default_timezone`) is the
 * middle rung of the record-zone chain, site → org → UTC; the reader's switch
 * (`general.show_record_time`) says whether the second clock is wanted at all.
 * Both are member-readable settings, so they are read once per session and
 * shared between surfaces instead of being fetched on every render.
 *
 * Non-purpose: the reader's own zone (that is `useTimeFormat()`) and any
 * judgement about which zone a record belongs to — that is the record's own
 * field, resolved here and nowhere else. A failed read leaves the registry
 * defaults in place and never blocks a render: a clock drawn in the org's
 * default is still a true clock, while a spinner is not.
 */
import { useCallback, useEffect, useState } from "react";
import { resolveTimeZone } from "@marula-core/ui";
import { api, useAuth } from "./api";

/** The org-scope setting every record falls back to when it names no zone. */
export const ORG_TIMEZONE_KEY = "general.default_timezone";
/** The user-scope switch that shows or hides a record's own clock. */
export const SHOW_RECORD_TIME_KEY = "general.show_record_time";
/** The registry's default for the switch: the second clock is on. */
const SHOW_RECORD_TIME_DEFAULT = true;
/** The registry's default for the org zone, and the chain's last rung. */
const ORG_TIMEZONE_DEFAULT = "UTC";

export interface TimeDisplay {
  /** The org's default zone, or null before the first read lands. */
  orgTimeZone: string | null;
  /** False once the reader has turned the second clock off. */
  showRecordTime: boolean;
  /**
   * The zone to hand `DualTime`: the record's when the reader wants it, the
   * reader's otherwise. Ask for it through this rather than reading the switch
   * at each call site — one place to forget is one place to get it wrong.
   */
  zoneFor: (recordZone: string | null | undefined, viewerZone: string) => string;
  /** Re-read after a write, so the switch takes effect without a reload. */
  refresh: () => Promise<void>;
}

interface TimeSettings {
  orgTimeZone: string | null;
  showRecordTime: boolean;
}

const FALLBACK: TimeSettings = {
  orgTimeZone: null,
  showRecordTime: SHOW_RECORD_TIME_DEFAULT,
};

/**
 * One in-flight promise per account. Keyed by user id so a sign-out and a
 * sign-in of somebody else in the same tab cannot serve the first person's
 * settings to the second.
 */
const cache = new Map<string, Promise<TimeSettings>>();

async function loadSettings(): Promise<TimeSettings> {
  const [org, user] = await Promise.all([
    api.api.GET("/api/v1/settings/org"),
    api.api.GET("/api/v1/settings/user"),
  ]);
  const orgRow = (org.data ?? []).find((v) => v.key === ORG_TIMEZONE_KEY);
  const userRow = (user.data ?? []).find((v) => v.key === SHOW_RECORD_TIME_KEY);
  return {
    orgTimeZone: typeof orgRow?.value === "string" ? orgRow.value : null,
    showRecordTime:
      typeof userRow?.value === "boolean"
        ? userRow.value
        : SHOW_RECORD_TIME_DEFAULT,
  };
}

/**
 * Drops the cached read. Sign-out calls it, and the tests call it between
 * cases so one case's settings cannot answer the next one's question.
 */
export function clearTimeDisplayCache(): void {
  cache.clear();
}

function cached(userId: string): Promise<TimeSettings> {
  const hit = cache.get(userId);
  if (hit !== undefined) return hit;
  const promise = loadSettings();
  cache.set(userId, promise);
  // A rejected promise must not poison the cache for the rest of the session.
  promise.catch(() => cache.delete(userId));
  return promise;
}

export function useTimeDisplay(): TimeDisplay {
  const auth = useAuth();
  const userId = auth.me?.user_id ?? null;
  const [settings, setSettings] = useState<TimeSettings | null>(null);

  useEffect(() => {
    if (userId === null) {
      setSettings(null);
      return;
    }
    let alive = true;
    void cached(userId)
      .then((next) => {
        if (alive) setSettings(next);
      })
      .catch(() => {
        if (alive) setSettings(FALLBACK);
      });
    return () => {
      alive = false;
    };
  }, [userId]);

  const refresh = useCallback(async () => {
    if (userId === null) return;
    cache.delete(userId);
    const next = await loadSettings();
    cache.set(userId, Promise.resolve(next));
    setSettings(next);
  }, [userId]);

  const orgTimeZone = settings?.orgTimeZone ?? null;
  const showRecordTime = settings?.showRecordTime ?? SHOW_RECORD_TIME_DEFAULT;

  const zoneFor = useCallback(
    (recordZone: string | null | undefined, viewerZone: string) => {
      if (!showRecordTime) return viewerZone;
      // `resolveTimeZone` validates each rung and drops what Intl rejects, so a
      // stale site value cannot blank the reading.
      return resolveTimeZone(recordZone, orgTimeZone ?? ORG_TIMEZONE_DEFAULT);
    },
    [orgTimeZone, showRecordTime],
  );

  return { orgTimeZone, showRecordTime, zoneFor, refresh };
}
