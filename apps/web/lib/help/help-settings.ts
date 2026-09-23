// Help + tours settings seam (Wave 26 lane 2).
//
// Purpose: the client half of the `help.*` / `tours.*` typed settings
// added to the registry — reads and writes through the ordinary audited
// settings endpoints (GET/PUT /api/v1/settings/user and /org), the same
// seam the onboarding lane owns for `onboarding.*`. Nothing here touches
// onboarding.* keys.

import { useCallback, useEffect, useState } from "react";
import { api } from "../api";

export const KEY_TOURS_DISABLED = "tours.disabled";
export const KEY_TOURS_COMPLETED = "tours.completed";
export const KEY_TOURS_ORG_DISABLED = "tours.org_disabled";
export const KEY_HELP_DOC_FEEDBACK = "help.doc_feedback";

/** Storage cap mirrored from the registry declaration (max_len 4096). */
export const COMPLETED_CAP = 100;
export const FEEDBACK_CAP = 50;

async function readSetting(
  scope: "user" | "org",
  key: string,
): Promise<string | boolean | null> {
  try {
    const { data } =
      scope === "user"
        ? await api.api.GET("/api/v1/settings/user")
        : await api.api.GET("/api/v1/settings/org");
    const row = (data ?? []).find((v) => v.key === key);
    if (row === undefined) return null;
    return row.value as string | boolean;
  } catch {
    return null;
  }
}

export async function writeSetting(
  scope: "user" | "org",
  key: string,
  value: string | boolean,
): Promise<boolean> {
  try {
    const res =
      scope === "user"
        ? await api.api.PUT("/api/v1/settings/user", { body: { key, value } })
        : await api.api.PUT("/api/v1/settings/org", { body: { key, value } });
    return !res.error;
  } catch {
    return false;
  }
}

/** Parses the capped JSON array stored in `tours.completed`. */
export function parseCompletedTours(raw: string | boolean | null): string[] {
  if (typeof raw !== "string" || raw.length === 0) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter((x): x is string => typeof x === "string")
      : [];
  } catch {
    return [];
  }
}

export interface ToursState {
  loaded: boolean;
  /** The user's own "don't show me tours" switch. */
  disabled: boolean;
  /** The org-wide kill switch (admins). */
  orgDisabled: boolean;
  /** Tour ids this account finished or skipped. */
  completed: string[];
  setDisabled(next: boolean): Promise<void>;
  markCompleted(tourId: string): Promise<void>;
  reload(): void;
}

/**
 * Tours suppression + completion state for one signed-in account.
 * Completion ALSO lands in localStorage first so the instant-first path
 * never re-nags while the wire write is in flight (or offline); the
 * server row stays the source of truth across devices.
 */
export function useToursState(enabled: boolean): ToursState {
  const [loaded, setLoaded] = useState(false);
  const [disabled, setDisabledState] = useState(false);
  const [orgDisabled, setOrgDisabled] = useState(false);
  const [completed, setCompleted] = useState<string[]>([]);
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    void (async () => {
      const [d, o, c] = await Promise.all([
        readSetting("user", KEY_TOURS_DISABLED),
        readSetting("org", KEY_TOURS_ORG_DISABLED),
        readSetting("user", KEY_TOURS_COMPLETED),
      ]);
      if (cancelled) return;
      if (typeof d === "boolean") setDisabledState(d);
      if (typeof o === "boolean") setOrgDisabled(o);
      let parsed = parseCompletedTours(c);
      // localStorage fallback: tours finished offline but not yet
      // acknowledged server-side still count as done on this device.
      try {
        const local = window.localStorage.getItem("meridian-tours-completed");
        if (local) parsed = [...new Set([...parsed, ...parseCompletedTours(local)])];
      } catch {
        // storage unavailable — server state only
      }
      setCompleted(parsed);
      setLoaded(true);
    })();
    return () => {
      cancelled = true;
    };
  }, [enabled, nonce]);

  const setDisabled = useCallback(async (next: boolean) => {
    setDisabledState(next);
    await writeSetting("user", KEY_TOURS_DISABLED, next);
  }, []);

  const markCompleted = useCallback(async (tourId: string) => {
    setCompleted((prev) => {
      const next = prev.includes(tourId) ? prev : [...prev, tourId].slice(-COMPLETED_CAP);
      try {
        window.localStorage.setItem(
          "meridian-tours-completed",
          JSON.stringify(next.slice(-COMPLETED_CAP)),
        );
      } catch {
        // storage unavailable — wire write only
      }
      return next;
    });
    const row = await readSetting("user", KEY_TOURS_COMPLETED);
    const current = parseCompletedTours(row);
    if (!current.includes(tourId)) {
      const next = [...current, tourId].slice(-COMPLETED_CAP);
      await writeSetting("user", KEY_TOURS_COMPLETED, JSON.stringify(next));
    }
  }, []);

  const reload = useCallback(() => setNonce((n) => n + 1), []);

  return { loaded, disabled, orgDisabled, completed, setDisabled, markCompleted, reload };
}

export interface DocFeedbackEntry {
  doc: string;
  up: boolean;
  at: string;
}

export function parseDocFeedback(raw: string | boolean | null): DocFeedbackEntry[] {
  if (typeof raw !== "string" || raw.length === 0) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter(
          (x): x is DocFeedbackEntry =>
            typeof x === "object" &&
            x !== null &&
            typeof (x as DocFeedbackEntry).doc === "string" &&
            typeof (x as DocFeedbackEntry).up === "boolean",
        )
      : [];
  } catch {
    return [];
  }
}

/** Records one "Was this helpful?" answer, capped, through the audited
 * settings path (help.doc_feedback). Optimistic local answer returns
 * immediately; the wire write is fire-and-forget-honest (errors surface
 * via the return value for the caller's UI state). */
export async function recordDocFeedback(
  docPath: string,
  up: boolean,
): Promise<boolean> {
  const row = await readSetting("user", KEY_HELP_DOC_FEEDBACK);
  const log = parseDocFeedback(row);
  const next = [
    ...log.filter((e) => e.doc !== docPath),
    { doc: docPath, up, at: new Date().toISOString() },
  ].slice(-FEEDBACK_CAP);
  return writeSetting("user", KEY_HELP_DOC_FEEDBACK, JSON.stringify(next));
}
