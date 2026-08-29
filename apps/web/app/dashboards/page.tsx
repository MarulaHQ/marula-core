// /dashboards — static dashboards (TASKLIST R2).
//
// Purpose: per-user widget rows over the shipped R1 reports API. Defaults
// first: until the user stores an arrangement, the 'My day' preset IS the
// dashboard; 'Team load' and 'SLA health' are one click away. Rows persist
// through the EXISTING settings-registry machinery (wave 2.5): a single
// user-scope registry key (dashboards.widgets) written with the same
// GET/PUT pattern the settings pages use. Widget arrangement is fixed
// vertical rows — drag-arrangement (react-grid-layout) stays out until it
// proves wanted, per ADR 0001.
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
import type { ReactNode } from "react";
import { ReportWidget } from "../../components/dashboards/report-widget";
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
} from "../../lib/dashboards";
import { api, useRequireAuth } from "../../lib/api";

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
    <main className="p-6 grid gap-4 max-w-3xl">
      <header className="grid gap-2">
        <div className="flex items-baseline gap-2">
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Dashboards
          </h1>
          <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            {manager ? "Org-wide scope" : "Your data only"}
          </span>
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
          <Button
            variant="ghost"
            size="sm"
            aria-expanded={pickerOpen}
            aria-controls="widget-picker"
            onClick={() => setPickerOpen((open) => !open)}
          >
            {pickerOpen ? "Close" : "Add widget"}
          </Button>
        </div>
        {pickerOpen && (
          <Card className="p-3 grid gap-1" id="widget-picker">
            {candidates.length === 0 ? (
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Every widget available to you is already on the dashboard.
              </p>
            ) : (
              candidates.map((w) => (
                <Button
                  key={w.id}
                  variant="ghost"
                  size="sm"
                  className="justify-self-start"
                  onClick={() => {
                    const next = addWidget(visible, w.id);
                    if (next !== null) persist(next);
                  }}
                >
                  {w.title}
                </Button>
              ))
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
        <div className="grid gap-4">
          {visible.map((id) => {
            const def = widgetDef(id);
            if (def === undefined) return null;
            return (
              <ReportWidget
                key={id}
                def={def}
                meUserId={auth.me?.user_id ?? ""}
                onRemove={() => persist(removeWidget(visible, id))}
              />
            );
          })}
        </div>
      )}
    </main>
  );
}
