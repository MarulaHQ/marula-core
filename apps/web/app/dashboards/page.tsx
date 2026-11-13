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
// Wave 46 lane 6 adds two things. Saved reports are placeable cards: their id
// is stored under `definition:`, their size comes from the card the definition
// stores (migration 0180), and the card prints the window and refresh it will
// run with. And "Describe a card" opens the shared authoring panel, which saves
// through the existing definition routes and then places the card it accepted.
//
// Non-purpose: no report-spec editing (that is /reports), no drag-to-resize and
// no drag-to-reorder. Placement is a keyboard-reachable control on each card.
"use client";

import { Button, Card, Spinner, SurfaceHeader } from "@marula-core/ui";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import { ReportWidget } from "../../components/dashboards/report-widget";
import { ReportDefinitionCard, type DefinitionView } from "../../components/dashboards/report-definition-card";
import { CardContractMeta } from "../../components/dashboards/card-contract";
import { PluginWidgetsSection } from "../../components/dashboards/plugin-widget";
import {
  cardSpanClass,
  cardHeightClass,
  sizeLabel,
} from "../../components/dashboards/card-size-control";
import { DescribeEntry } from "../../components/reports/describe-entry";
import { encodeSpecToQuery } from "../../lib/report-spec-codec";
import { SurfaceTabs } from "../../components/insight/surface-tabs";
import { InsightState } from "../../components/insight/insight-state";
import {
  WIDGET_CATALOG,
  WIDGET_ROWS_KEY,
  addPlacement,
  addWidget,
  cardVocabulary,
  defaultPlacements,
  defaultSizeFor,
  definitionSize,
  definitionWidgetId,
  definitionIdOfWidgetId,
  encodeWidgets,
  isManagerPermissions,
  isDefinitionWidgetId,
  moveWidget,
  placementsForPreset,
  placementsMatch,
  readArrangement,
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
import type { ReportDraft } from "../../lib/report-authoring";

/** Why the surface is read-only, in the two cases that can cause it. */
type ReadOnlyReason = "catalogue" | "arrangement";

const NOTICE: Record<ReadOnlyReason, string> = {
  catalogue:
    "Card sizes and the widget catalogue could not be loaded. Nothing is shown and this dashboard is read-only until they do.",
  arrangement:
    "Your saved arrangement could not be loaded. Nothing is shown and nothing has been overwritten. Retry reads it again.",
};

const PICKER_NOTICE: Record<ReadOnlyReason, string> = {
  catalogue: "No widget can be added until the card vocabulary loads.",
  arrangement: "No widget can be added until your saved arrangement loads.",
};

export default function DashboardsPage() {
  const auth = useRequireAuth();
  const manager = useMemo(
    () => isManagerPermissions(auth.me?.permissions ?? []),
    [auth.me],
  );

  // `null` while loading, and again when the surface is read-only: either the
  // catalogue is missing (a size cannot be validated) or this user's stored
  // arrangement cannot be read (writing anything would replace it).
  const [placements, setPlacements] = useState<WidgetPlacement[] | null>(null);
  const [vocabulary, setVocabulary] = useState<CardVocabulary | null>(null);
  const [readOnlyReason, setReadOnlyReason] = useState<ReadOnlyReason | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  // Saved reports, read from the definition list. Their cards are stored with
  // them, so this surface never derives a window or a size for one.
  const [definitions, setDefinitions] = useState<DefinitionView[] | null>(null);
  const [definitionsError, setDefinitionsError] = useState<string | null>(null);
  const [authoredId, setAuthoredId] = useState<string | null>(null);
  const router = useRouter();

  const refreshDefinitions = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/report-definitions");
    if (error !== undefined || data === undefined) {
      setDefinitionsError(
        "Your saved reports could not be loaded. The catalogue's widgets are still available.",
      );
      return;
    }
    setDefinitionsError(null);
    setDefinitions(data.definitions as DefinitionView[]);
  }, []);

  const resolveDashboard = useCallback(async () => {
    const [settings, catalog] = await Promise.all([
      api.api.GET("/api/v1/settings/user"),
      api.api.GET("/api/v1/report-catalog"),
    ]);
    const vocab =
      catalog.error === undefined && catalog.data !== undefined
        ? cardVocabulary(catalog.data)
        : null;
    if (vocab === null) {
      return { vocab: null, next: null, reason: "catalogue" as const };
    }
    // A failed settings read says "this user's arrangement is unknown", not
    // "this user has no arrangement". Falling through to the defaults here
    // would let the next resize overwrite the arrangement they still have.
    if (settings.error !== undefined) {
      return { vocab, next: null, reason: "arrangement" as const };
    }
    const stored = (settings.data ?? []).find((v) => v.key === WIDGET_ROWS_KEY);
    let decoded = defaultPlacements(vocab);
    if (stored !== undefined) {
      const read = readArrangement(stored.value, vocab);
      if (read.status === "read") {
        decoded = read.placements;
      } else if (read.status === "unset") {
        // Nothing has been written yet: the defaults ARE this user's
        // arrangement until they change it.
        decoded = defaultPlacements(vocab);
      } else if (read.problem === "size") {
        // The row's own rule: a stored size the payload no longer lists falls
        // back to the default arrangement instead of drawing an unmeasurable
        // grid. Every other problem leaves the document alone.
        decoded = defaultPlacements(vocab);
      } else {
        return { vocab, next: null, reason: "arrangement" as const };
      }
    }
    // Widgets no longer visible for this user (role change) drop out; the
    // stored arrangement keeps them, so a role change back restores them. A
    // saved-report placement is kept: the definition list decides whether it
    // draws or renders as unavailable, not this filter.
    const next = decoded.filter((p) => {
      if (isDefinitionWidgetId(p.id)) return true;
      const def = widgetDef(p.id);
      return def !== undefined && widgetVisible(def, manager);
    });
    return { vocab, next, reason: null };
  }, [manager]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refreshDefinitions();
  }, [auth.status, refreshDefinitions]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void (async () => {
      const { vocab, next, reason } = await resolveDashboard();
      if (cancelled) return;
      setVocabulary(vocab);
      setPlacements(next);
      setReadOnlyReason(reason);
      setLoaded(true);
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status, resolveDashboard]);

  // Retry after a failed read: the same resolve, applied directly.
  const retry = useCallback(() => {
    void (async () => {
      const { vocab, next, reason } = await resolveDashboard();
      setVocabulary(vocab);
      setPlacements(next);
      setReadOnlyReason(reason);
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

  /**
   * Place a saved report. Its size is read from the definition's own card; a
   * size the catalogue no longer lists refuses the placement rather than
   * storing a different one.
   */
  const addDefinition = useCallback(
    (def: DefinitionView) => {
      if (placements === null || vocabulary === null) return;
      const size = definitionSize(def, vocabulary);
      if (size === null) return;
      const next = addPlacement(
        placements,
        definitionWidgetId(def.id),
        size,
        vocabulary,
      );
      if (next !== null) persist(next);
    },
    [placements, vocabulary, persist],
  );

  /**
   * An accepted draft from the "describe it" entry. The card it was accepted
   * with is the card the definition now stores, so it is placed at that size —
   * not at the shape default.
   */
  const handleAuthored = useCallback(
    (result: { id: string; created: boolean; draft: ReportDraft }) => {
      setAuthoredId(result.id);
      if (placements !== null && vocabulary !== null) {
        const def: DefinitionView = {
          id: result.id,
          name: result.draft.name ?? "Saved report",
          card: result.draft.card,
          spec: result.draft.spec,
        };
        const size = definitionSize(def, vocabulary);
        if (size !== null) {
          const next = addPlacement(
            placements,
            definitionWidgetId(result.id),
            size,
            vocabulary,
          );
          if (next !== null) persist(next);
        }
      }
      void refreshDefinitions();
    },
    [placements, vocabulary, persist, refreshDefinitions],
  );

  const grid = placements ?? [];
  const presets = visiblePresets(manager);
  const candidates = WIDGET_CATALOG.filter(
    (w) => widgetVisible(w, manager) && !grid.some((p) => p.id === w.id),
  );
  const definitionCandidates = (definitions ?? []).filter(
    (d) => !grid.some((p) => p.id === definitionWidgetId(d.id)),
  );
  const authoredPlaced =
    authoredId !== null && grid.some((p) => p.id === definitionWidgetId(authoredId));

  if (auth.status !== "authed" || !loaded) {
    return (
      <main className="p-6" aria-busy="true">
        <Spinner label="Loading dashboards" />
      </main>
    );
  }

  const readOnly = readOnlyReason !== null;

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
            <div className="flex flex-wrap items-center gap-2">
              <DescribeEntry
                surface="dashboards"
                testId="dashboard-describe-entry"
                onEdit={(draft) => router.push(`/reports?spec=${encodeSpecToQuery(draft.spec)}`)}
                onAccepted={handleAuthored}
                onOpenReports={() => router.push("/reports")}
              />
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
          }
        />

        {/* Toolbar row: named arrangement views (the presets) on the left.
            Absent while read-only: applying one would write an arrangement the
            page could not read. */}
        {!readOnly && (
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
        )}

        {/* The catalogue: a popover-style card under the Add widget button.
            Each candidate shows the footprint it arrives with, read from the
            payload's shape default. */}
        {pickerOpen && (
          <Card className="grid gap-2 p-3" id="widget-picker">
            {readOnlyReason !== null ? (
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                {PICKER_NOTICE[readOnlyReason]}
              </p>
            ) : null}

            {/* Saved reports come first: they are the cards this org authored,
                and each one arrives with the card it stores. */}
            <p
              className="text-[12px] font-semibold uppercase tracking-[0.06em]"
              style={{ color: "var(--m-text-muted)" }}
            >
              Saved reports
            </p>
            {readOnlyReason !== null ? null : definitions === null ? (
              <Spinner label="Loading saved reports" />
            ) : definitionCandidates.length === 0 ? (
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                No saved reports are waiting to be placed. Describe one above, or
                save a report on the reports surface.
              </p>
            ) : (
              <div className="grid gap-1 sm:grid-cols-2 xl:grid-cols-3">
                {definitionCandidates.map((def) => {
                  const size = vocabulary === null ? null : definitionSize(def, vocabulary);
                  return (
                    <div
                      key={def.id}
                      className="grid gap-1 rounded-[var(--m-radius-control)] border p-2"
                      style={{ borderColor: "var(--m-border)" }}
                      data-testid={`definition-candidate-${def.id}`}
                    >
                      <span className="truncate text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
                        {def.name}
                      </span>
                      <CardContractMeta card={def.card} testId={`definition-candidate-meta-${def.id}`} />
                      <Button
                        variant="ghost"
                        size="sm"
                        className="justify-start"
                        disabled={size === null}
                        data-testid={`add-definition-${def.id}`}
                        aria-label={`Add ${def.name} to the dashboard`}
                        onClick={() => addDefinition(def)}
                      >
                        {size === null
                          ? "Its stored size is not in this catalogue"
                          : `Add — ${sizeLabel(size.span)} wide, ${sizeLabel(size.height)}`}
                      </Button>
                    </div>
                  );
                })}
              </div>
            )}

            <p
              className="text-[12px] font-semibold uppercase tracking-[0.06em]"
              style={{ color: "var(--m-text-muted)" }}
            >
              Widget catalogue
            </p>
            {readOnlyReason !== null ? null : candidates.length === 0 ? (
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

        {/* Save failure SURFACES: fixed-position notice in the status danger
            pair, never swallowed. */}
        {saveError !== null && (
          <div
            role="alert"
            className="flex items-center gap-2 rounded-[var(--m-radius-control)] px-3 py-2 text-[13px] font-medium"
            style={{
              background: "var(--m-status-danger-bg)",
              color: "var(--m-status-danger-fg)",
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

        {/* Read-only: no card is drawn. Either the vocabulary is missing (a
            size cannot be validated) or the stored arrangement cannot be read
            (a write would replace it). Drawing some other arrangement under
            either notice would say one thing and show another. */}
        {readOnlyReason !== null && (
          <div
            role="status"
            className="flex flex-wrap items-center gap-2 rounded-[var(--m-radius-control)] border px-3 py-2 text-[13px]"
            style={{
              borderColor: "var(--m-border)",
              background: "var(--m-surface-2)",
              color: "var(--m-text-secondary)",
            }}
            data-testid="dashboard-readonly"
            data-reason={readOnlyReason}
          >
            <span>{NOTICE[readOnlyReason]}</span>
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

      {definitionsError !== null ? (
        <div
          role="status"
          className="flex flex-wrap items-center gap-2 rounded-[var(--m-radius-control)] border px-3 py-2 text-[13px]"
          style={{
            borderColor: "var(--m-border)",
            background: "var(--m-surface-2)",
            color: "var(--m-text-secondary)",
          }}
          data-testid="dashboard-definitions-error"
        >
          <span>
            Your saved reports could not be loaded, so saved-report cards cannot be
            drawn. The catalogue&apos;s widgets are unaffected.
          </span>
          <Button variant="secondary" size="sm" onClick={() => void refreshDefinitions()}>
            Retry
          </Button>
        </div>
      ) : null}

      {authoredId !== null ? (
        <div
          role="status"
          className="rounded-[var(--m-radius-control)] border px-3 py-2 text-[13px]"
          style={{
            borderColor: "var(--m-border)",
            background: "var(--m-surface-2)",
            color: "var(--m-text-secondary)",
          }}
          data-testid="dashboard-authored-note"
        >
          {authoredPlaced
            ? "Saved. The report is on your dashboard with the card you accepted."
            : "Saved to your reports. Your dashboard is read-only right now, so it was not placed."}
        </div>
      ) : null}

      {placements === null ? null : placements.length === 0 ? (
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
            const definitionId = definitionIdOfWidgetId(placement.id);
            if (definitionId !== null) {
              const saved = (definitions ?? []).find((d) => d.id === definitionId);
              // A placement whose definition the list does not resolve is still
              // a card the user placed. Drawing nothing would leave a hole in
              // the grid with no reason and no way out, so it renders as a
              // stated state at its stored span, with the one action that clears
              // it through the same `remove()` every other card uses.
              if (saved === undefined) {
                return (
                  <div
                    key={placement.id}
                    className={`min-w-0 ${cardSpanClass(placement.size.span)}`}
                  >
                    <Card
                      className={`grid content-start gap-2 p-4 ${cardHeightClass(placement.size.height)}`}
                      data-testid={`definition-unavailable-${definitionId}`}
                      data-card-span={placement.size.span}
                      data-card-height={placement.size.height}
                    >
                      <h3 className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                        Saved report unavailable
                      </h3>
                      <p className="text-[13px] leading-[1.5]" style={{ color: "var(--m-text-secondary)" }}>
                        {definitionsError !== null
                          ? "This card's saved report could not be checked, because the saved-report list did not load."
                          : "This card's saved report is not available to you. It was deleted or is outside your scope."}
                      </p>
                      <div>
                        <Button
                          variant="secondary"
                          size="sm"
                          aria-label="Remove the unavailable saved report card"
                          onClick={() => remove(placement.id)}
                        >
                          Remove
                        </Button>
                      </div>
                    </Card>
                  </div>
                );
              }
              return (
                <div
                  key={placement.id}
                  className={`min-w-0 ${cardSpanClass(placement.size.span)}`}
                >
                  <ReportDefinitionCard
                    def={saved}
                    size={placement.size}
                    spans={vocabulary?.spans ?? []}
                    heights={vocabulary?.heights ?? []}
                    entranceDelay={Math.min(i * 30, 240)}
                    onResize={(size) => resize(placement.id, size)}
                    onMove={(direction) => move(placement.id, direction)}
                    canMoveEarlier={i > 0}
                    canMoveLater={i < placements.length - 1}
                    onRemove={() => remove(placement.id)}
                    onOpenReports={() => router.push("/reports")}
                  />
                </div>
              );
            }
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
