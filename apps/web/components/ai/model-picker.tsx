"use client";

// AI model picker — the sticky per-surface pick inside chat / ticket
// note assist / doc drafting / email drafting entry points.
//
// Purpose: one picker pattern every AI-assisted surface shares. It
// delegates the sticky pick to the server (PUT /api/v1/ai/me/model-
// picks/{surface_key}; DELETE clears it), precedence user → org inside
// the admin allowlist — "user pick sticks" survives refreshes and
// devices because the server records it, and off-list asks are refused
// typed (`ai_model_not_allowed`), never redirected. The allowlist
// pairs + the currently-recorded pick arrive on /api/v1/ai/me/surfaces.
//
// Posture: degrade-quiet. A surface that is ungranted renders its
// typed not-available note, never a dead control (visibly-absent).
//
// Tokens: docs/theme.md only; motion ≤120ms transform/opacity.

import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";

type AiAllowedModelDto = components["schemas"]["AiAllowedModelDto"];

export interface SurfacePickState {
  provider: string | null;
  model: string | null;
  loading: boolean;
  set: (provider: string, model: string) => Promise<boolean>;
  clear: () => Promise<boolean>;
  allowed: AiAllowedModelDto[];
  granted: boolean;
}

/**
 * Loads me/surfaces once and exposes the sticky pick state for one
 * surface. `null` = surfaces not yet loaded (picker keeps its own
 * fetch); `{granted, models}` overrides.
 */
export function useLazySurfacePick(
  surfaceKey: string,
  surfacesFromMatrix?: { granted: boolean; models: AiAllowedModelDto[]; picked?: AiAllowedModelDto | null } | null,
): SurfacePickState {
  const [state, setState] = useState<{ provider: string | null; model: string | null; loading: boolean }>({
    provider: null,
    model: null,
    loading: true,
  });
  const [allowed, setAllowed] = useState<AiAllowedModelDto[]>(surfacesFromMatrix?.models ?? []);
  const granted = surfacesFromMatrix ? surfacesFromMatrix.granted : true;

  const load = useCallback(async () => {
    setState((s) => ({ ...s, loading: true }));
    const { data } = await api.api.GET("/api/v1/ai/me/surfaces");
    const row = data?.surfaces.find((s) => s.surface_key === surfaceKey);
    const surfaceAllowlist = surfacesFromMatrix?.models ?? (row?.models ?? []);
    setAllowed(surfaceAllowlist);
    setState({
      provider: surfacesFromMatrix?.picked?.provider ?? row?.picked?.provider ?? null,
      model: surfacesFromMatrix?.picked?.model ?? row?.picked?.model ?? null,
      loading: false,
    });
  }, [surfaceKey, surfacesFromMatrix]);

  useEffect(() => {
    void load();
  }, [load]);

  const set = useCallback(
    async (provider: string, model: string) => {
      const previous = state;
      setState({ ...state, provider, model, loading: true });
      const { error } = await api.api.PUT("/api/v1/ai/me/model-picks/{surface_key}", {
        params: { path: { surface_key: surfaceKey } },
        body: { provider, model },
      });
      if (error !== undefined) {
        setState(previous);
        return false;
      }
      return true;
    },
    [state, surfaceKey],
  );

  const clear = useCallback(async () => {
    const previous = state;
    setState({ ...state, provider: null, model: null, loading: false });
    const { error } = await api.api.DELETE("/api/v1/ai/me/model-picks/{surface_key}", {
      params: { path: { surface_key: surfaceKey } },
    });
    if (error !== undefined) {
      setState(previous);
      return false;
    }
    return true;
  }, [state, surfaceKey]);

  return { ...state, set, clear, allowed, granted };
}

/** The picker dropdown itself. `granted=false` renders the typed note. */
export function ModelPicker({
  pickState,
  compact,
  onNote,
}: {
  pickState: SurfacePickState;
  compact?: boolean;
  onNote?: (message: string) => void;
}) {
  if (pickState.loading) {
    return <Spinner />;
  }
  if (!pickState.granted) {
    return (
      <span
        className="text-[12px]"
        style={{ color: "var(--m-status-neutral-fg)" }}
        aria-live="polite"
        data-testid="ai-model-picker-unavailable"
      >
        Not available — this AI surface is not granted to you.
      </span>
    );
  }
  const value =
    pickState.provider != null && pickState.model != null
      ? `${pickState.provider}/${pickState.model}`
      : "";
  const commit = async (next: string) => {
    if (next === value) return;
    if (next === "") {
      await pickState.clear();
      return;
    }
    const [provider, model] = next.split("/", 2) as [string, string];
    const ok = await pickState.set(provider ?? "", model ?? "");
    if (!ok) onNote?.("The model pick was not accepted — it is outside the admin allowlist.");
  };
  return (
    <Select value={value !== "" ? value : undefined} onValueChange={commit}>
      <SelectTrigger
        data-testid="ai-model-picker"
        className={compact ? "h-[var(--m-density-control)] text-[12px]" : "text-[12px]"}
        style={{ borderRadius: "var(--m-radius-control)" }}
        aria-label="Model used by this AI surface"
      >
        <SelectValue placeholder="Org default model" />
      </SelectTrigger>
      <SelectContent>
        {pickState.allowed.map((m) => (
          <SelectItem key={`${m.provider}/${m.model}`} value={`${m.provider}/${m.model}`}>
            {m.model} — {m.label}
          </SelectItem>
        ))}
        <SelectItem value="">Org default model</SelectItem>
      </SelectContent>
    </Select>
  );
}
