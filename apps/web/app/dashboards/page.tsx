// /dashboards — static dashboards (TASKLIST R2), Wave 17 lane 1 polish.
//
// Purpose: per-user widget rows over the shipped R1 reports API. Defaults
// first: until the user stores an arrangement, the 'My day' preset IS the
// dashboard; 'Team load' and 'SLA health' are one click away. Rows persist
// through the EXISTING settings-registry machinery (wave 2.5): a single
// user-scope registry key (dashboards.widgets) written with the same
// GET/PUT pattern the settings pages use. Widget arrangement is fixed
// vertical rows — drag-arrangement (react-grid-layout) stays out until it
// proves wanted, per ADR 0001; the Wave 17 grid presents the same ordered
// rows as a responsive two-column layout with wide lead widgets.
//
// RBAC: org-lens widgets and org presets appear only for managers (the
// /api/v1/me permission set matching the server's owner-or-admin seam);
// everyone else gets personal-lens widgets, which the report API would
// force onto self anyway.
//
// Non-purpose: no report-spec editing (R3) and no shared/org dashboards.
"use client";

import { Button, Card, EmptyState, Spinner } from "@meridian/ui";
import { useCallback, useEffect, useMemo, useState } from "react";
import { ReportWidget } from "../../components/dashboards/report-widget";
import { PluginWidgetsSection } from "../../components/dashboards/plugin-widget";
import {
  WIDGET_CATALOG,
  WIDGET_ROWS_KEY,
  addWidget,
  decodeWidgetRows,
  defaultRows,
  encodeWidgetRows,
  isManagerPermissions,
  removeWidget,
  visiblePresets,
  widgetDef,
  widgetVisible,
  type WidgetDef,
} from "../../lib/dashboards";
import { api, useRequireAuth } from "../../lib/api";

/** Wide widgets span the full row; everything else sits two-up on xl. */
function isWide(def: WidgetDef): boolean {
  return def.id === "time_by_user:self" || def.id === "time_by_user:org" || def.id === "sla_breaches:org";
}

export default function DashboardsPage() {
  const auth = useRequireAuth();
  const manager = useMemo(
    () => isManagerPermissions(auth.me?.permissions ?? []),
    [auth.me],
  );

  const [rows, setRows] = useState<string[] | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  // Load stored rows through the settings registry (2.5 pattern); fall
  // back to the defaults-first preset when nothing usable is stored.
  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/settings/user");
      if (cancelled) return;
      const stored = (data ?? []).find((v) => v.key === WIDGET_ROWS_KEY);
      const decoded = decodeWidgetRows(stored?.value) ?? defaultRows();
      // Widgets that are no longer visible for this user (role change)
      // simply drop out of the rendered rows.
      const visible = decoded.filter((id) => {
        const def = widgetDef(id);
        return def !== undefined && widgetVisible(def, manager);
      });
      setRows(visible);
      setLoaded(true);
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status, manager]);

  const persist = useCallback(
    (next: string[]) => {
      setRows(next);
      setSaveError(null);
      void (async () => {
        const res = await api.api.PUT("/api/v1/settings/user", {
          body: { key: WIDGET_ROWS_KEY, value: encodeWidgetRows(next) },
        });
        if (res.error) {
          const detail =
            (res.error as { error?: { message?: string } }).error?.message ??
            "rejected";
          setSaveError(`Saving your layout failed: ${detail}`);
        }
      })();
    },
    [],
  );

  const visible = rows ?? [];
  const presets = visiblePresets(manager);
  const candidates = WIDGET_CATALOG.filter(
    (w) => widgetVisible(w, manager) && !visible.includes(w.id),
  );

  if (auth.status !== "authed" || !loaded || rows === null) {
    return (
      <main className="p-6" aria-busy="true">
        <Spinner label="Loading dashboards" />
      </main>
    );
  }

  return (
    <main className="mx-auto grid w-full max-w-6xl gap-5 p-6">
      <header className="m-rise grid gap-3">
        <div className="flex flex-wrap items-baseline justify-between gap-2">
          <div className="flex items-baseline gap-3">
            <h1 className="text-lg font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
              Dashboards
            </h1>
            <span
              className="text-[11px] font-medium uppercase tracking-wide"
              style={{ color: "var(--m-text-muted)" }}
            >
              {manager ? "Org-wide scope" : "Your data only"}
            </span>
          </div>
          <Button
            variant="secondary"
            size="sm"
            aria-expanded={pickerOpen}
            aria-controls="widget-picker"
            onClick={() => setPickerOpen((open) => !open)}
          >
            {pickerOpen ? "Close" : "Add widget"}
          </Button>
        </div>
        <div className="flex items-center gap-2 flex-wrap" role="group" aria-label="Default presets">
          {presets.map((preset) => {
            const active =
              preset.widgets.length === visible.length &&
              preset.widgets.every((id, i) => visible[i] === id);
            return (
              <Button
                key={preset.id}
                variant={active ? "primary" : "secondary"}
                size="sm"
                disabled={active}
                aria-label={`Apply ${preset.title} preset`}
                onClick={() => persist([...preset.widgets])}
              >
                {preset.title}
              </Button>
            );
          })}
        </div>
        {pickerOpen && (
          <Card className="grid gap-1 p-3" id="widget-picker">
            <p
              className="text-[10px] font-semibold uppercase tracking-wide"
              style={{ color: "var(--m-text-muted)" }}
            >
              Widget catalog
            </p>
            {candidates.length === 0 ? (
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Every widget available to you is already on the dashboard.
              </p>
            ) : (
              <div className="grid gap-1 sm:grid-cols-2 xl:grid-cols-3">
                {candidates.map((w) => (
                  <Button
                    key={w.id}
                    variant="ghost"
                    size="sm"
                    className="justify-start"
                    onClick={() => {
                      const next = addWidget(visible, w.id);
                      if (next !== null) persist(next);
                    }}
                  >
                    {w.title}
                    <span
                      aria-hidden="true"
                      className="ml-1 text-[10px] uppercase"
                      style={{ color: "var(--m-text-muted)" }}
                    >
                      {w.lens === "org" ? "org" : "mine"}
                    </span>
                  </Button>
                ))}
              </div>
            )}
          </Card>
        )}
        {saveError !== null && (
          <p className="text-[12px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
            {saveError}
          </p>
        )}
      </header>

      {visible.length === 0 ? (
        <Card className="p-6">
          <EmptyState
            message="Your dashboard is empty."
            action={
              <Button variant="primary" size="sm" onClick={() => persist(defaultRows())}>
                Restore My day
              </Button>
            }
          />
        </Card>
      ) : (
        <div className="grid items-start gap-4 xl:grid-cols-2">
          {visible.map((id, i) => {
            const def = widgetDef(id);
            if (def === undefined) return null;
            return (
              <div key={id} className={`min-w-0 ${isWide(def) ? "xl:col-span-2" : ""}`}>
                <ReportWidget
                  def={def}
                  meUserId={auth.me?.user_id ?? ""}
                  entranceDelay={Math.min(i * 30, 240)}
                  onRemove={() => persist(removeWidget(visible, id))}
                />
              </div>
            );
          })}
        </div>
      )}
      <PluginWidgetsSection slot="dashboard" />
    </main>
  );
}
