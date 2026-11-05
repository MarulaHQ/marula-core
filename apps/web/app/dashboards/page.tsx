// /dashboards — the widget grid (Wave 41 lane 11, Marula rebrand; Wave 46
// lane 2 card sizes and placement).
//
// Purpose: the per-user report widget surface. The page header keeps the TL
// frame (title row with attention shortcuts, one toolbar row, 24px gutters);
// the body composes Dispatch Planning-card language: 16px-radius bordered
// cards, flat, no gradients. Widget catalogue, per-user persisted arrangement
// (settings key `dashboards.widgets`, the lane 2b pattern), role-limited
// visibility (the server is the enforcement point; the UI mirrors it), a
// save-failed state that SURFACES (a fixed blush notice above the grid, never a
// silent drop), and a per-card size and order taken from lane 1's card
// vocabulary.
//
// The vocabulary is READ, never restated: `GET /api/report-catalog` serves
// `spans`, `heights`, `shapes[].default_span`, `shapes[].default_height` and
// `card_rules`, and every control on this page renders that payload. A span or
// height the payload does not list cannot be chosen or stored.
//
// Non-purpose: no report-spec editing (that is /reports), no drag-to-resize and
// no drag-to-reorder. Placement is a keyboard-reachable control on each card.
"use client";

import { Button, Card, Spinner, SurfaceHeader } from "@marula-core/ui";
import { useCallback, useEffect, useMemo, useState } from "react";
import { ReportWidget } from "../../components/dashboards/report-widget";
import { PluginWidgetsSection } from "../../components/dashboards/plugin-widget";
import {
  cardSpanClass,
  sizeLabel,
} from "../../components/dashboards/card-size-control";
import { SurfaceTabs } from "../../components/insight/surface-tabs";
import { InsightState } from "../../components/insight/insight-state";
import {
  WIDGET_CATALOG,
  WIDGET_ROWS_KEY,
  addWidget,
  cardVocabulary,
  decodeWidgets,
  defaultPlacements,
  defaultRows,
  defaultSizeFor,
  encodeWidgets,
  isManagerPermissions,
  moveWidget,
  placementsForPreset,
  placementsMatch,
  removeWidget,
  resizeWidget,
  visiblePresets,
  widgetDef,
  widgetVisible,
} from "../../lib/dashboards";
import type {
  CardSize,
  CardVocabulary,
  WidgetPlacement,
} from "../../lib/dashboards";
import { api, useRequireAuth } from "../../lib/api";
import { friendlyMutationText } from "../../lib/error-copy";

const CATALOG_ERROR =
  "Card sizes and the widget catalogue could not be loaded, so this dashboard is read-only for now.";

export default function DashboardsPage() {
  const auth = useRequireAuth();
  const manager = useMemo(
    () => isManagerPermissions(auth.me?.permissions ?? []),
    [auth.me],
  );

  // `null` while loading, and again when no catalogue arrived: the size of a
  // card cannot be validated without the vocabulary the server owns.
  const [placements, setPlacements] = useState<WidgetPlacement[] | null>(null);
  const [vocabulary, setVocabulary] = useState<CardVocabulary | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const resolveDashboard = useCallback(async () => {
    const [settings, catalog] = await Promise.all([
      api.api.GET("/api/v1/settings/user"),
      api.api.GET("/api/v1/report-catalog"),
    ]);
    const vocab =
      catalog.error === undefined && catalog.data !== undefined
        ? cardVocabulary(catalog.data)
        : null;
    let next: WidgetPlacement[] | null = null;
    if (vocab !== null) {
      const stored = (settings.data ?? []).find((v) => v.key === WIDGET_ROWS_KEY);
      const decoded = decodeWidgets(stored?.value, vocab) ?? defaultPlacements(vocab);
      // Widgets no longer visible for this user (role change) drop out; the
      // stored arrangement keeps them, so a role change back restores them.
      next = decoded.filter((p) => {
        const def = widgetDef(p.id);
        return def !== undefined && widgetVisible(def, manager);
      });
    }
    return { vocab, next };
  }, [manager]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void (async () => {
      const { vocab, next } = await resolveDashboard();
      if (cancelled) return;
      setVocabulary(vocab);
      setPlacements(next);
      setLoaded(true);
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status, resolveDashboard]);

  // Retry after a failed catalogue read: the same resolve, applied directly.
  const retry = useCallback(() => {
    void (async () => {
      const { vocab, next } = await resolveDashboard();
      setVocabulary(vocab);
      setPlacements(next);
      setLoaded(true);
    })();
  }, [resolveDashboard]);

  const persist = useCallback(
    (next: WidgetPlacement[]) => {
      setPlacements(next);
      setSaveError(null);
      void (async () => {
        try {
          const res = await api.api.PUT("/api/v1/settings/user", {
            body: { key: WIDGET_ROWS_KEY, value: encodeWidgets(next) },
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

  const resize = useCallback(
    (id: string, size: CardSize) => {
      if (placements === null || vocabulary === null) return;
      const next = resizeWidget(placements, id, size, vocabulary);
      if (next !== null) persist(next);
    },
    [placements, vocabulary, persist],
  );

  const move = useCallback(
    (id: string, direction: -1 | 1) => {
      if (placements === null) return;
      persist(moveWidget(placements, id, direction));
    },
    [placements, persist],
  );

  const remove = useCallback(
    (id: string) => {
      if (placements === null) return;
      persist(removeWidget(placements, id));
    },
    [placements, persist],
  );

  const add = useCallback(
    (id: string) => {
      if (placements === null || vocabulary === null) return;
      const next = addWidget(placements, id, vocabulary);
      if (next !== null) persist(next);
    },
    [placements, vocabulary, persist],
  );

  const grid = placements ?? [];
  const presets = visiblePresets(manager);
  const candidates = WIDGET_CATALOG.filter(
    (w) => widgetVisible(w, manager) && !grid.some((p) => p.id === w.id),
  );

  if (auth.status !== "authed" || !loaded) {
    return (
      <main className="p-6" aria-busy="true">
        <Spinner label="Loading dashboards" />
      </main>
    );
  }

  const readOnly = vocabulary === null;

  return (
    <main className="mx-auto grid w-full max-w-6xl gap-5 p-6">
      <div className="m-rise grid gap-3">
        <SurfaceHeader
          title="Dashboards"
          meta={
            <span
              className="inline-flex h-6 items-center rounded-full px-2.5 text-[12px] font-semibold"
              style={{
                background: "var(--m-surface-2)",
                color: "var(--m-text-secondary)",
              }}
            >
              {manager ? "Org-wide scope" : "Your data only"}
            </span>
          }
          actions={
            <Button
              variant={pickerOpen ? "secondary" : "primary"}
              size="sm"
              aria-expanded={pickerOpen}
              aria-controls="widget-picker"
              onClick={() => setPickerOpen((open) => !open)}
            >
              {pickerOpen ? "Close" : "Add widget"}
            </Button>
          }
        />

        {/* Toolbar row: named arrangement views (the presets) on the left. */}
        <SurfaceTabs
          tabs={presets.map((p) => ({ id: p.id, label: p.title, ariaLabel: `Apply ${p.title} preset` }))}
          activeId={presets.find((p) => placementsMatch(grid, p.widgets))?.id ?? ""}
          onChange={(id) => {
            const preset = presets.find((p) => p.id === id);
            if (preset === undefined || vocabulary === null) return;
            const next = placementsForPreset(preset.widgets, grid, vocabulary);
            if (next !== null) persist(next);
          }}
          label="Saved arrangements"
        />

        {/* The catalogue: a popover-style card under the Add widget button.
            Each candidate shows the footprint it arrives with, read from the
            payload's shape default. */}
        {pickerOpen && (
          <Card className="grid gap-2 p-3" id="widget-picker">
            <p
              className="text-[12px] font-semibold uppercase tracking-[0.06em]"
              style={{ color: "var(--m-text-muted)" }}
            >
              Widget catalogue
            </p>
            {readOnly ? (
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                No widget can be added until the card vocabulary loads.
              </p>
            ) : candidates.length === 0 ? (
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Every widget available to you is already on the dashboard.
              </p>
            ) : (
              <div className="grid gap-1 sm:grid-cols-2 xl:grid-cols-3">
                {candidates.map((w) => {
                  const size = vocabulary === null ? null : defaultSizeFor(w, vocabulary);
                  return (
                    <Button
                      key={w.id}
                      variant="ghost"
                      size="sm"
                      className="justify-start"
                      data-testid={`add-widget-${w.id}`}
                      aria-label={`Add ${w.title}`}
                      onClick={() => add(w.id)}
                    >
                      <span className="truncate">{w.title}</span>
                      <span
                        aria-hidden="true"
                        className="ml-1 text-[10px] uppercase"
                        style={{ color: "var(--m-text-muted)" }}
                      >
                        {w.lens === "org" ? "org" : "mine"}
                      </span>
                      <span
                        className="ml-auto shrink-0 text-[10px]"
                        style={{ color: "var(--m-text-muted)" }}
                      >
                        {size === null
                          ? "no default size"
                          : `${sizeLabel(size.span)} wide, ${sizeLabel(size.height)}`}
                      </span>
                    </Button>
                  );
                })}
              </div>
            )}
            {vocabulary !== null && vocabulary.rules.length > 0 ? (
              <details className="text-[12px]">
                <summary
                  className="cursor-pointer text-[11px] font-semibold uppercase tracking-[0.06em]"
                  style={{ color: "var(--m-text-muted)" }}
                >
                  Card rules
                </summary>
                <ul className="mt-1 grid list-disc gap-1 pl-4">
                  {vocabulary.rules.map((rule) => (
                    <li key={rule} style={{ color: "var(--m-text-secondary)" }}>
                      {rule}
                    </li>
                  ))}
                </ul>
              </details>
            ) : null}
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

        {/* No vocabulary: the cards still render, stacked, with no size
            controls and no writes, because a size cannot be validated. */}
        {readOnly && (
          <div
            role="status"
            className="flex flex-wrap items-center gap-2 rounded-[var(--m-radius-control)] border px-3 py-2 text-[13px]"
            style={{
              borderColor: "var(--m-border)",
              background: "var(--m-surface-2)",
              color: "var(--m-text-secondary)",
            }}
            data-testid="dashboard-catalog-error"
          >
            <span>{CATALOG_ERROR}</span>
            <Button
              variant="secondary"
              size="sm"
              onClick={retry}
            >
              Retry
            </Button>
          </div>
        )}
      </div>

      {placements === null ? (
        <div className="grid gap-4">
          {defaultRows().map((id, i) => {
            const def = widgetDef(id);
            if (def === undefined) return null;
            return (
              <div key={id} className="min-w-0">
                <ReportWidget
                  def={def}
                  meUserId={auth.me?.user_id ?? ""}
                  entranceDelay={Math.min(i * 30, 240)}
                />
              </div>
            );
          })}
        </div>
      ) : placements.length === 0 ? (
        <Card className="p-6">
          <InsightState
            title="Your dashboard is empty"
            hint="Add widgets from the catalogue above, or restore the default arrangement."
            primary={
              <Button
                variant="primary"
                size="sm"
                onClick={() => {
                  if (vocabulary !== null) persist(defaultPlacements(vocabulary));
                }}
              >
                Restore My day
              </Button>
            }
          />
        </Card>
      ) : (
        <div className="grid grid-cols-12 items-start gap-4" data-testid="dashboard-grid">
          {placements.map((placement, i) => {
            const def = widgetDef(placement.id);
            if (def === undefined) return null;
            return (
              <div
                key={placement.id}
                className={`min-w-0 ${cardSpanClass(placement.size.span)}`}
              >
                <ReportWidget
                  def={def}
                  meUserId={auth.me?.user_id ?? ""}
                  size={placement.size}
                  spans={vocabulary?.spans ?? []}
                  heights={vocabulary?.heights ?? []}
                  entranceDelay={Math.min(i * 30, 240)}
                  onResize={(size) => resize(placement.id, size)}
                  onMove={(direction) => move(placement.id, direction)}
                  canMoveEarlier={i > 0}
                  canMoveLater={i < placements.length - 1}
                  onRemove={() => remove(placement.id)}
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
