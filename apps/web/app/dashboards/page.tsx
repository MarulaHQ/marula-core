// /dashboards — the widget grid (Wave 41 lane 11, Marula rebrand).
//
// Purpose: the per-user report widget surface. The page header keeps the TL
// frame (title row with attention shortcuts, one toolbar row, 24px gutters);
// the body composes Dispatch Planning-card language: 16px-radius bordered
// cards, flat, no gradients. Widget catalogue, per-user persisted row order
// (settings key `dashboards.widgets`, the lane 2b pattern), role-limited
// visibility (the server is the enforcement point; the UI mirrors it), and
// a save-failed state that SURFACES — a fixed blush notice above the grid,
// never a silent drop.
//
// Non-purpose: no report-spec editing (that is /reports), no drag-to-resize
// of widget cards (drag-to-reorder of ROWS stays in a later wave if wanted).
"use client";

import {
  Button,
  Card,
  EmptyState,
  Spinner,
} from "@marula-core/ui";
import { useCallback, useEffect, useMemo, useState } from "react";
import { ReportWidget } from "../../components/dashboards/report-widget";
import { PluginWidgetsSection } from "../../components/dashboards/plugin-widget";
import { SurfaceTabs } from "../../components/insight/surface-tabs";
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
import { friendlyMutationText } from "../../lib/error-copy";

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
        try {
          const res = await api.api.PUT("/api/v1/settings/user", {
            body: { key: WIDGET_ROWS_KEY, value: encodeWidgetRows(next) },
          });
          if (res.error) {
            const detail = friendlyMutationText(
              res.error,
              "the server refused the change",
            );
            setSaveError(`Your dashboard did not save — ${detail}. Your arrangement is still on screen; try again.`);
          }
        } catch (e) {
          setSaveError(
            friendlyMutationText(e, "Your dashboard did not save — the network dropped. Your arrangement is still on screen; try again."),
          );
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
        {/* Page header row, TL frame: title 26px semibold + scope pill, action right. */}
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-3">
            <h1
              className="text-[26px] font-semibold tracking-[-0.01em]"
              style={{ color: "var(--m-text)" }}
            >
              Dashboards
            </h1>
            <span
              className="inline-flex h-6 items-center rounded-full px-2.5 text-[12px] font-semibold"
              style={{
                background: "var(--m-surface-2)",
                color: "var(--m-text-secondary)",
              }}
            >
              {manager ? "Org-wide scope" : "Your data only"}
            </span>
          </div>
          <Button
            variant={pickerOpen ? "secondary" : "primary"}
            size="sm"
            aria-expanded={pickerOpen}
            aria-controls="widget-picker"
            onClick={() => setPickerOpen((open) => !open)}
          >
            {pickerOpen ? "Close" : "Add widget"}
          </Button>
        </div>

        {/* Toolbar row: named arrangement views (the presets) on the left. */}
        <SurfaceTabs
          tabs={presets.map((p) => ({ id: p.id, label: p.title, ariaLabel: `Apply ${p.title} preset` }))}
          activeId={
            presets.find(
              (p) =>
                p.widgets.length === visible.length &&
                p.widgets.every((id, i) => visible[i] === id),
            )?.id ?? ""
          }
          onChange={(id) => {
            const preset = presets.find((p) => p.id === id);
            if (preset !== undefined) persist([...preset.widgets]);
          }}
          label="Saved arrangements"
        />

        {/* The catalogue: a popover-style card under the Add widget button. */}
        {pickerOpen && (
          <Card className="grid gap-1 p-3" id="widget-picker">
            <p
              className="text-[12px] font-semibold uppercase tracking-[0.06em]"
              style={{ color: "var(--m-text-muted)" }}
            >
              Widget catalogue
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

        {/* Save failure SURFACES: fixed-position blush notice, never swallowed. */}
        {saveError !== null && (
          <div
            role="alert"
            className="flex items-center gap-2 rounded-[var(--m-radius-control)] px-3 py-2 text-[13px] font-medium"
            style={{
              background: "var(--m-status-due-bg)",
              color: "var(--m-status-due-fg)",
            }}
            data-testid="dashboard-save-error"
          >
            <svg
              aria-hidden="true"
              className="size-4 shrink-0"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
            >
              <path d="M4 17l6-6 4 4 6-8" />
              <path d="M4 21h16" opacity="0.4" />
            </svg>
            {saveError}
          </div>
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
        <div className="grid items-start gap-4 xl:grid-cols-[1.5fr_1fr]">
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
