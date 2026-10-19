"use client";
/**
 * Data-surface preferences — the app-side seam for the D12 controls and the
 * D7 density step.
 *
 * Purpose: one place that reads `appearance.data_surfaces` and writes it back,
 * so a surface's view, columns, filters and sort are the user's rather than
 * the component's, and the document is validated against the same shapes the
 * database will hold. Two rules the controls depend on:
 *
 * 1. A write is OPTIMISTIC and local-first, because a column order that waits
 *    for a round trip to feel responsive is a column order nobody uses. A
 *    rejected write puts the previous state back and says so.
 * 2. A document that fails to parse is DISCARDED, not repaired. The saved
 *    shape is closed; a document the parser refuses is a document some other
 *    version wrote, and guessing at it would write our guess back over it.
 *
 * Non-purpose: this holds no query. The document describes what the user is
 * LOOKING at; the surface still asks its own endpoint for the rows.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, useAuth } from "../lib/api";
import {
  applyDensity,
  DEFAULT_DENSITY,
  isDensity,
  type Density,
} from "@marula-core/ui";

const DATA_SURFACES_KEY = "appearance.data_surfaces";
const DENSITY_KEY = "appearance.density";

/** One surface's saved state, as the D12 document holds it. */
export interface SavedSurfaceView {
  view?: string;
  columns: { key: string; visible: boolean; width?: number }[];
  filters: { field: string; op: string; value?: string }[];
  /** How the clauses combine. Absent means "all", the pack's default. */
  match?: "all" | "any";
  sort?: { field: string; dir: "asc" | "desc" };
}

type Document_ = Record<string, SavedSurfaceView>;

const EMPTY: Document_ = {};

/** One identity for "this surface has nothing saved". A caller that reads an
 *  unsaved surface has to get a STABLE object: a fresh `columns: []` /
 *  `filters: []` on every render churns every memo keyed on it, and on
 *  /tickets one of those memos drives a setState — which is a render loop that
 *  starves the router's transitions. */
const EMPTY_VIEW: SavedSurfaceView = { columns: [], filters: [] };

/**
 * Parse a stored document, dropping any surface whose shape is wrong rather
 * than failing the whole document: one bad surface should not cost a user
 * every other surface they saved.
 */
function parseDocument(raw: unknown): Document_ {
  if (typeof raw !== "string" || raw.trim() === "") return EMPTY;
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return EMPTY;
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) return EMPTY;
  const out: Document_ = {};
  for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
    if (typeof entry !== "object" || entry === null) continue;
    const view = entry as Partial<SavedSurfaceView>;
    if (!Array.isArray(view.columns)) continue;
    out[key] = {
      view: typeof view.view === "string" ? view.view : undefined,
      columns: view.columns.filter(
        (c): c is { key: string; visible: boolean; width?: number } =>
          typeof c === "object" &&
          c !== null &&
          typeof (c as { key?: unknown }).key === "string",
      ),
      filters: Array.isArray(view.filters) ? view.filters : [],
      match: view.match === "any" ? "any" : view.match === "all" ? "all" : undefined,
      sort: view.sort,
    };
  }
  return out;
}

function serialize(doc: Document_): string {
  // Sorted keys so two users who made the same changes write the same string;
  // the settings audit log is worth reading.
  const ordered: Document_ = {};
  for (const key of Object.keys(doc).sort()) {
    const entry = doc[key];
    if (entry !== undefined) ordered[key] = entry;
  }
  return JSON.stringify(ordered);
}

export interface DataSurfacePrefs {
  ready: boolean;
  error: string | null;
  /** The saved state for one surface, or an empty one. */
  view: (surface: string) => SavedSurfaceView;
  /** Replace one surface's saved state and persist. */
  save: (surface: string, next: SavedSurfaceView) => Promise<void>;
  /** Forget one surface entirely (an emptied chooser, a renamed table). */
  clear: (surface: string) => Promise<void>;
  density: Density;
  setDensity: (step: Density) => Promise<void>;
}

/**
 * A named view (or tab, or lifecycle step) that follows the saved document.
 *
 * The initial state cannot read the document: `useDataSurfacePrefs` resolves it
 * from the network, so on the first paint every surface sees the empty view. A
 * plain `useState` initialiser therefore captures the fallback and keeps it,
 * which is how a returning user got their columns and filters back but not the
 * view they left on. This applies the saved choice once, when the document is
 * ready, and never again — so a later click is not undone by a late read.
 */
export function useSavedChoice<T extends string>(
  ready: boolean,
  saved: string | undefined,
  allowed: readonly T[],
  fallback: T,
): [T, (next: T) => void] {
  const isAllowed = (value: string | undefined): value is T =>
    value !== undefined && (allowed as readonly string[]).includes(value);
  const [value, setValue] = useState<T>(() => (isAllowed(saved) ? saved : fallback));
  const restored = useRef(false);
  useEffect(() => {
    if (!ready || restored.current) return;
    if (isAllowed(saved)) {
      restored.current = true;
      setValue(saved);
    } else if (saved === undefined) {
      // Nothing was saved; the fallback is the whole answer.
      restored.current = true;
    }
    // A value that is not allowed YET (a tab the user's role may take) stays
    // pending, so the effect applies it when the role resolves. A value this
    // surface can never render simply stays unapplied.
  }, [ready, saved, allowed]);
  return [value, setValue];
}

export function useDataSurfacePrefs(): DataSurfacePrefs {
  const auth = useAuth();
  const authed = auth.status === "authed";
  const [doc, setDoc] = useState<Document_>(EMPTY);
  const [density, setDensityState] = useState<Density>(DEFAULT_DENSITY);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The pre-save value, so a rejected write can put back exactly what was
  // there rather than a guess.
  const previous = useRef<Document_>(EMPTY);

  useEffect(() => {
    if (!authed) return;
    let cancelled = false;
    void (async () => {
      try {
        const { data } = await api.api.GET("/api/v1/settings/user");
        if (cancelled) return;
        const rows = data ?? [];
        const surfaceRow = rows.find((v) => v.key === DATA_SURFACES_KEY);
        const densityRow = rows.find((v) => v.key === DENSITY_KEY);
        const nextDoc = parseDocument(surfaceRow?.value);
        previous.current = nextDoc;
        setDoc(nextDoc);
        const step = densityRow?.value;
        if (isDensity(step)) {
          setDensityState(step);
          applyDensity(step);
        }
      } catch {
        // A failed read leaves every surface on its own defaults, which is
        // the same state a fresh account is in.
      } finally {
        if (!cancelled) setReady(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [authed]);

  const save = useCallback(
    async (surface: string, next: SavedSurfaceView) => {
      const before = doc;
      const optimistic = { ...doc, [surface]: next };
      previous.current = before;
      setDoc(optimistic);
      setError(null);
      try {
        const res = await api.api.PUT("/api/v1/settings/user", {
          body: { key: DATA_SURFACES_KEY, value: serialize(optimistic) },
        });
        if (res.error) throw new Error("The preference did not save.");
      } catch (e) {
        setDoc(previous.current);
        setError(e instanceof Error ? e.message : "The preference did not save.");
      }
    },
    [doc],
  );

  const clear = useCallback(
    async (surface: string) => {
      const optimistic = { ...doc };
      delete optimistic[surface];
      previous.current = doc;
      setDoc(optimistic);
      setError(null);
      try {
        const res = await api.api.PUT("/api/v1/settings/user", {
          body: { key: DATA_SURFACES_KEY, value: serialize(optimistic) },
        });
        if (res.error) throw new Error("The preference did not save.");
      } catch (e) {
        setDoc(previous.current);
        setError(e instanceof Error ? e.message : "The preference did not save.");
      }
    },
    [doc],
  );

  const setDensity = useCallback(async (step: Density) => {
    const before = density;
    setDensityState(step);
    applyDensity(step);
    setError(null);
    try {
      const res = await api.api.PUT("/api/v1/settings/user", {
        body: { key: DENSITY_KEY, value: step },
      });
      if (res.error) throw new Error("The density did not save.");
    } catch (e) {
      setDensityState(before);
      applyDensity(before);
      setError(e instanceof Error ? e.message : "The density did not save.");
    }
  }, [density]);

  return useMemo(
    () => ({
      ready,
      error,
      view: (surface: string) => doc[surface] ?? EMPTY_VIEW,
      save,
      clear,
      density,
      setDensity,
    }),
    [ready, error, doc, save, clear, density, setDensity],
  );
}
