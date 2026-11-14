// PluginWidget — §4.6 declarative plugin widget cards (Wave 34 lane 4).
//
// Renders the org's registered plugin surface entries (slot: dashboard or
// ticket_pane) through TRUSTED components only: a stat card or a read-only
// label/value list, with per-board props validated before render
// (agent-safety §4.8 — plugin content never renders raw; a plugin ships
// declarations, not markup or code).
//
// The validator is now the shared one. `props_schema` on a widget is judged by
// `SettingsSchema::new` + `validate_config` on the server (plugins.rs
// `PluginSurfaceProfile::validate`), which is the same subset a connector's
// `settings_schema` uses — nested objects, arrays, `pattern`, bounds. This
// component used to carry its own flat copy of that subset, so a widget with a
// nested props object failed closed here while the server accepted it. It now
// delegates to `@marula-core/ui`'s `readSettingsSchema` /
// `validateSettingsValue`: one contract, one validator, and still failing
// closed on anything the reader cannot render.
//
// Non-purpose: no plugin fetches and no arbitrary rendering — dynamic
// data widgets arrive with the WASM execution runtime and ride the same
// capability gates.
"use client";

import {
  Card,
  readSettingsSchema,
  validateSettingsValue,
  type SettingsObject,
} from "@marula-core/ui";
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

/**
 * Validates the widget's props against its declared schema with the same
 * reader and validator a connector's settings form uses. A schema outside the
 * accepted subset, or a value that fails it, fails CLOSED: the widget renders
 * the refusal instead of unvalidated content.
 */
export function validateProps(props: Props, schema: unknown): boolean {
  if (schema === null || schema === undefined) return true;
  const read = readSettingsSchema(schema);
  if (!read.ok) return false;
  const errors = validateSettingsValue(read.schema, props as SettingsObject);
  return Object.keys(errors).length === 0;
}

/** A scalar prints itself; a nested value prints as JSON, never "[object Object]". */
function displayValue(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  try {
    return JSON.stringify(value);
  } catch {
    return "—";
  }
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
              <span className="truncate">{displayValue(v)}</span>
            </li>
          ))}
        </ul>
      ) : (
        <div>
          <p className="text-[26px] font-semibold leading-tight" style={{ fontVariantNumeric: "tabular-nums" }}>
            {displayValue(raw.value)}
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
