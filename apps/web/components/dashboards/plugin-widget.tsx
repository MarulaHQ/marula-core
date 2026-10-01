// PluginWidget — §4.6 declarative plugin widget cards (Wave 34 lane 4).
//
// Renders the org's registered plugin surface entries (slot: dashboard or
// ticket_pane) through TRUSTED components only: a stat card or a read-only
// label/value list, with per-board props validated against the entry's
// declared JSON-Schema subset before render (agent-safety §4.8 — plugin
// content never renders raw; a plugin ships declarations, not markup or
// code).
//
// Non-purpose: no plugin fetches and no arbitrary rendering — dynamic
// data widgets arrive with the WASM execution runtime and ride the same
// capability gates.
"use client";

import { Card } from "@marula-core/ui";
import { useEffect, useState } from "react";
import { api } from "../../lib/api";

type SurfaceEntry = {
  install_id: string;
  entry_key: string;
  label: string;
  definition: {
    kind?: string;
    slot?: string;
    props_schema?: unknown;
    default_props?: unknown;
  };
};

type Props = Record<string, unknown>;

/** Minimal JSON-Schema subset validation — the SAME subset SettingsSchema
 * accepts server-side (string/number/integer/boolean + enum + bounds).
 * Anything the subset does not understand fails CLOSED (the widget does
 * not render rather than rendering unvalidated). */
function validateProps(props: Props, schema: unknown): boolean {
  if (schema === null || schema === undefined) return true;
  const obj = schema as {
    type?: string;
    properties?: Record<string, { type?: string; enum?: unknown[]; minimum?: number; maximum?: number; minLength?: number; maxLength?: number }>;
    required?: string[];
  };
  if (obj.type !== "object" || typeof obj.properties !== "object") return false;
  for (const key of obj.required ?? []) {
    if (!(key in props)) return false;
  }
  for (const [key, value] of Object.entries(props)) {
    const spec = obj.properties[key];
    if (spec === undefined) return false;
    switch (spec.type) {
      case "string":
        if (typeof value !== "string") return false;
        if (spec.minLength !== undefined && value.length < spec.minLength) return false;
        if (spec.maxLength !== undefined && value.length > spec.maxLength) return false;
        break;
      case "number":
      case "integer":
        if (typeof value !== "number") return false;
        if (spec.type === "integer" && !Number.isInteger(value)) return false;
        if (spec.minimum !== undefined && value < spec.minimum) return false;
        if (spec.maximum !== undefined && value > spec.maximum) return false;
        break;
      case "boolean":
        if (typeof value !== "boolean") return false;
        break;
      default:
        return false;
    }
    if (spec.enum !== undefined && !spec.enum.includes(value)) return false;
  }
  return true;
}

function PluginWidgetCard({ entry, entranceDelay }: { entry: SurfaceEntry; entranceDelay: number }) {
  const def = entry.definition ?? {};
  const raw = (def.default_props ?? {}) as Props;
  const valid = validateProps(raw, def.props_schema);
  const kind = def.kind ?? "stat";
  return (
    <Card className="m-rise group grid gap-3 p-4" style={{ animationDelay: `${entranceDelay}ms` }}>
      <h3
        className="truncate text-[11px] font-medium uppercase tracking-wide"
        style={{ color: "var(--m-text-muted)" }}
      >
        {entry.label}
      </h3>
      {!valid ? (
        <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          This widget&apos;s configuration failed its declared schema — nothing
          renders until the plugin publishes a valid configuration.
        </p>
      ) : kind === "list" ? (
        <ul className="grid gap-1">
          {Object.entries(raw).map(([k, v]) => (
            <li key={k} className="flex justify-between gap-3 text-[13px]">
              <span style={{ color: "var(--m-text-secondary)" }}>{k.replace(/_/g, " ")}</span>
              <span className="truncate">{String(v)}</span>
            </li>
          ))}
        </ul>
      ) : (
        <div>
          <p className="text-[26px] font-semibold leading-tight" style={{ fontVariantNumeric: "tabular-nums" }}>
            {String(raw.value ?? "—")}
          </p>
          {typeof raw.caption === "string" ? (
            <p className="mt-1 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              {raw.caption}
            </p>
          ) : null}
        </div>
      )}
      <p className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
        {entry.entry_key}
      </p>
    </Card>
  );
}

/** The plugin widget section for one slot. Renders nothing while the org
 * has no registered widgets (no chrome, no empty-state noise). */
export function PluginWidgetsSection({ slot }: { slot: "dashboard" | "ticket_pane" }) {
  const [entries, setEntries] = useState<SurfaceEntry[] | null>(null);
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const res = await api.api.GET("/api/v1/plugins/surface/widgets", {});
        if (cancelled) return;
        const all = (res.data?.entries ?? []) as SurfaceEntry[];
        setEntries(all.filter((e) => e.definition?.slot === slot));
      } catch {
        if (!cancelled) setEntries([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [slot]);

  if (entries === null || entries.length === 0) return null;
  return (
    <section aria-label="Plugin widgets" className="grid gap-3" data-testid={`plugin-widgets-${slot}`}>
      {entries.map((entry, i) => (
        <PluginWidgetCard key={entry.entry_key} entry={entry} entranceDelay={Math.min(i * 60, 240)} />
      ))}
    </section>
  );
}
