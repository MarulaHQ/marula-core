"use client";
/**
 * LedgerSurface — the Marula data-surface template, composed once.
 *
 * Purpose: the surface-pack's data-surface template (DESIGN-NOTES-tickets
 * §2) is the contract every record ledger in this area copies, so it is
 * composed in one place rather than re-derived on each page:
 *
 * 1. One toolbar row: the view switch left, the filter chips in the middle,
 *    Columns and Export right-aligned. It never wraps and never pushes the
 *    table down.
 * 2. Three density steps, chosen in the footer control, applied to controls
 *    and rows together through the `data-density` document attribute.
 * 3. Views, chips and the filter builder edit ONE filter state, saved per
 *    user through `useDataSurfacePrefs`.
 * 4. Columns are per user, persist, and reorder and resize by drag.
 * 5. Export says what it will export and respects the current view.
 * 6. Selection replaces the column header with an action bar, in place, so
 *    the layout never jumps. The bar is a gold tint and is deliberately NOT
 *    an inverted surface.
 * 7. Empty and error states share one shape.
 *
 * Non-purpose: this owns no data. It composes the landed primitives
 * (DataToolbar, ViewSwitch, FilterBar, ColumnChooser, ExportControl,
 * DensitySwitch, Table) and hands the page its resolved column list, so the
 * page spends its code on rows rather than on chrome.
 */
import {
  BulkHeaderRow,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  EmptyState,
  ExportControl,
  FilterBar,
  SurfaceFooter,
  Table,
  Th,
  ViewSwitch,
  type ChoosableColumn,
  type ExportRequest,
  type FilterClause,
  type FilterField,
  type ViewOption,
} from "@marula-core/ui";
import type { ReactNode } from "react";

import { cn } from "@marula-core/ui";
import { useDataSurfacePrefs } from "../../lib/data-surface-prefs";

export interface LedgerColumn {
  key: string;
  label: string;
  /** The row's identity column: always first and never switchable off. */
  required?: boolean;
  /** A column the table hides until the user asks for it. */
  hiddenByDefault?: boolean;
  /** Numeric columns read right. */
  align?: "left" | "right";
}

export interface LedgerShortcut {
  key: string;
  label: string;
  count: number;
  /** Red is breach-class only (DESIGN-NOTES-tickets §9). */
  tone: "gold" | "blush" | "red";
  active: boolean;
  onClick: () => void;
}

export interface LedgerBulkAction {
  key: string;
  label: string;
  /** A 12px mark in the pack's stroke style; the pack's bar is icon + label. */
  icon?: ReactNode;
  onClick: () => void;
  /** Rendered disabled while the action cannot apply to this selection. */
  disabled?: boolean;
  title?: string;
}

/** A resolved column, with the order the surface is showing it in. */
export type ResolvedColumn = ChoosableColumn & {
  align?: "left" | "right";
  /** The user's saved order; a column a later release adds sorts last. */
  order: number;
};

export interface LedgerSurfaceProps {
  /** Persistence key, one per surface, e.g. `purchasing-orders`. */
  surface: string;
  title: string;
  subtitle?: ReactNode;
  /** The header's right side: the primary action. */
  headerAction?: ReactNode;
  /** Counters in the title row that toggle a filter, per the pack. */
  shortcuts?: LedgerShortcut[];
  views: ViewOption[];
  activeView: string;
  onViewChange: (id: string) => void;
  columns: LedgerColumn[];
  filterFields: FilterField[];
  filterClauses: FilterClause[];
  onFilterChange: (next: FilterClause[]) => void;
  /** Rows the CURRENT view holds; the export and the footer count these. */
  rowCount: number;
  rowsShown?: number;
  sort?: { field: string; dir: "asc" | "desc" };
  onExport: (request: ExportRequest) => void | Promise<void>;
  onReorderColumns?: (from: number, to: number) => void;
  onResizeColumn?: (key: string, width: number) => void;
  /** Selectable rows render their own checkbox; this is the selection state. */
  selectedIds?: string[];
  onToggleRow?: (id: string, selected: boolean) => void;
  onToggleAll?: (selected: boolean) => void;
  allSelected?: boolean;
  someSelected?: boolean;
  bulkActions?: LedgerBulkAction[];
  onClearSelection?: () => void;
  /** The body. Receives the visible columns in the order the header shows. */
  children: (columns: ResolvedColumn[]) => ReactNode;
  emptyMessage: string;
  emptyAction?: ReactNode;
  errorMessage?: string | null;
  onRetry?: () => void;
  /**
   * Real keyboard shortcuts, rendered as kbd chips the way the pack's footer
   * does. Only shortcuts this surface ACTUALLY binds belong here — advertising
   * J/K/X that nothing listens for is a lie in the chrome. Design rationale
   * ("overdue is red, due soon is blush") belongs in a code comment, never in
   * the footer of a screen a customer reads.
   */
  keyboardHints?: { keys: string[]; label: string }[];
  testId?: string;
  /** Extra content under the table card, e.g. a received-dispatch strip. */
  childrenAfter?: ReactNode;
}

export function LedgerSurface({
  surface,
  title,
  subtitle,
  headerAction,
  shortcuts,
  views,
  activeView,
  onViewChange,
  columns: defaultColumns,
  filterFields,
  filterClauses,
  onFilterChange,
  rowCount,
  rowsShown,
  sort,
  onExport,
  onReorderColumns,
  onResizeColumn,
  selectedIds,
  onToggleRow,
  onToggleAll,
  allSelected,
  someSelected,
  bulkActions,
  onClearSelection,
  children,
  emptyMessage,
  emptyAction,
  errorMessage,
  onRetry,
  keyboardHints,
  testId,
  childrenAfter,
}: LedgerSurfaceProps): React.JSX.Element {
  const prefs = useDataSurfacePrefs();
  const saved = prefs.view(surface);

  // The saved document wins where it speaks and the surface's own definition
  // where it does not, so a column a later release adds still appears.
  const resolved: ResolvedColumn[] = defaultColumns.map((column) => {
    const savedColumn = saved.columns.find((c) => c.key === column.key);
    const savedIndex = saved.columns.findIndex((c) => c.key === column.key);
    return {
      key: column.key,
      label: column.label,
      visible: savedColumn?.visible ?? !column.hiddenByDefault,
      required: column.required === true,
      width: savedColumn?.width,
      align: column.align,
      order: savedIndex === -1 ? Number.MAX_SAFE_INTEGER : savedIndex,
    };
  });
  const ordered = [...resolved].sort((a, b) => a.order - b.order);
  const visible = ordered.filter((column) => column.visible);
  const selectable = onToggleRow != null;

  function persist(view: string, nextColumns: ChoosableColumn[]): void {
    void prefs.save(surface, {
      view,
      columns: nextColumns.map((column) => ({
        key: column.key,
        visible: column.visible,
        width: column.width,
      })),
      filters: filterClauses.map((clause) => ({
        field: clause.field,
        op: clause.op,
        value: clause.value,
      })),
      sort,
    });
  }

  const selectedCount = selectedIds?.length ?? 0;
  const barInPlace = selectedCount > 0;

  return (
    <div className="m-ledger-page" data-testid={testId}>
      <header className="m-ledger-header">
        <div className="m-ledger-title-row">
          <h1 className="m-ledger-title">{title}</h1>
          {shortcuts != null && shortcuts.length > 0 ? (
            <div className="m-ledger-shortcuts" role="group" aria-label={`${title} shortcuts`}>
              {shortcuts.map((shortcut) => (
                <button
                  key={shortcut.key}
                  type="button"
                  className={cn(
                    "m-ledger-shortcut",
                    shortcut.tone === "gold" && "is-gold",
                    shortcut.tone === "blush" && "is-blush",
                    shortcut.tone === "red" && "is-red",
                    shortcut.active && "is-active",
                  )}
                  aria-pressed={shortcut.active}
                  data-testid={`ledger-shortcut-${shortcut.key}`}
                  onClick={shortcut.onClick}
                >
                  <span
                    className={cn(
                      "m-ledger-dot",
                      shortcut.tone === "gold" && "is-gold",
                      shortcut.tone === "blush" && "is-blush",
                      shortcut.tone === "red" && "is-red",
                    )}
                  />
                  <span>{shortcut.label}</span>
                  <span className="m-ledger-shortcut-count tabular-nums">{shortcut.count}</span>
                </button>
              ))}
            </div>
          ) : null}
          <div className="m-ledger-header-action">{headerAction}</div>
        </div>
        {subtitle != null ? <p className="m-ledger-subtitle">{subtitle}</p> : null}
      </header>

      <DataToolbar
        label={`${title} controls`}
        views={
          <ViewSwitch
            views={views}
            activeId={activeView}
            onChange={(id) => {
              onViewChange(id);
              persist(id, ordered);
            }}
            label={`${title} views`}
          />
        }
        filters={
          <FilterBar
            fields={filterFields}
            clauses={filterClauses}
            onChange={(next) => {
              onFilterChange(next);
              persist(activeView, ordered);
            }}
            label={`${title} filters`}
            emptyHint="No filters"
          />
        }
        columns={
          <ColumnChooser
            columns={ordered.map((column) => ({
              key: column.key,
              label: column.label,
              visible: column.visible,
              required: column.required,
              width: column.width,
            }))}
            onChange={(next) => persist(activeView, next)}
            label={`${title} columns`}
          />
        }
        export={
          <ExportControl
            columns={visible.map((column) => column.key)}
            rowCount={rowCount}
            sort={sort}
            filters={filterClauses}
            onExport={onExport}
            label="Export"
          />
        }
      />

      <section className="m-ledger-card" aria-label={title}>
        {errorMessage != null ? (
          <div className="m-ledger-state" data-testid={`${testId ?? title}-error`}>
            <EmptyState
              message={errorMessage}
              action={
                onRetry != null ? (
                  <button
                    type="button"
                    className="m-ledger-primary"
                    data-testid={`${testId ?? title}-retry`}
                    onClick={onRetry}
                  >
                    Retry
                  </button>
                ) : null
              }
            />
          </div>
        ) : rowCount === 0 ? (
          <div className="m-ledger-state">
            <EmptyState message={emptyMessage} action={emptyAction} />
          </div>
        ) : (
          <div className="m-ledger-table-wrap">
            <Table>
              {/* The widths live on a colgroup so they survive the bulk bar
                  replacing the header row (auto layout would otherwise drop them
                  with the header and stretch every column equally). */}
              <colgroup>
                {selectable ? <col style={{ width: 44 }} /> : null}
                {visible.map((column) => (
                  <col key={column.key} style={{ width: column.width }} />
                ))}
              </colgroup>
              <thead>
                {barInPlace ? (
                  <BulkHeaderRow
                    columnCount={visible.length + (selectable ? 1 : 0)}
                    count={selectedCount}
                    noun="row"
                    onClear={() => onClearSelection?.()}
                    testId="ledger-bulk-bar"
                    leading={
                      selectable ? (
                        /*
                          The pack's bar leads with the selection state itself, not
                          with a number: the count then reads as a consequence of
                          what is ticked rather than as a figure you have to trust.
                          The select column keeps its own cell so the checkbox does
                          not move out from under the column it belongs to.
                        */
                        <input
                          type="checkbox"
                          className="m-ledger-checkbox"
                          aria-label={allSelected === true ? "Clear selection" : "Select every row"}
                          checked={allSelected === true}
                          ref={
                            someSelected === true
                              ? (node) => {
                                  if (node) node.indeterminate = true;
                                }
                              : undefined
                          }
                          onChange={(event) => onToggleAll?.(event.target.checked)}
                          data-testid="ledger-bulk-select-all"
                        />
                      ) : undefined
                    }
                  >
                    {(bulkActions ?? []).map((action) => (
                      <button
                        key={action.key}
                        type="button"
                        className="m-ledger-bulk-action"
                        disabled={action.disabled === true}
                        title={action.title}
                        data-testid={`ledger-bulk-${action.key}`}
                        onClick={action.onClick}
                      >
                        {action.icon != null ? (
                          <span className="m-ledger-bulk-action-icon" aria-hidden="true">
                            {action.icon}
                          </span>
                        ) : null}
                        {action.label}
                      </button>
                    ))}
                  </BulkHeaderRow>
                ) : (
                  <tr>
                    {selectable ? (
                      <Th className="m-ledger-select-col" data-select="true" sortable={false}>
                        <input
                          type="checkbox"
                          className="m-ledger-checkbox"
                          aria-label={allSelected === true ? "Clear selection" : "Select every row"}
                          checked={allSelected === true}
                          ref={
                            someSelected === true
                              ? (node) => {
                                  if (node) node.indeterminate = true;
                                }
                              : undefined
                          }
                          onChange={(event) => onToggleAll?.(event.target.checked)}
                          data-testid="ledger-select-all"
                        />
                      </Th>
                    ) : null}
                    {visible.map((column, index) => (
                      <Th
                        key={column.key}
                        columnKey={column.key}
                        index={index}
                        columnCount={visible.length}
                        sortable={onReorderColumns != null}
                        resizable={onResizeColumn != null}
                        width={column.width}
                        onReorder={onReorderColumns}
                        onResize={onResizeColumn}
                        data-align={column.align === "right" ? "right" : "left"}
                        data-column-key={column.key}
                      >
                        {column.label}
                      </Th>
                    ))}
                  </tr>
                )}
              </thead>
              <tbody>{children(visible)}</tbody>
            </Table>
          </div>
        )}

        <SurfaceFooter
          shown={rowsShown ?? rowCount}
          total={rowCount}
          noun="row"
          className="m-ledger-footer"
          density={
            <DensitySwitch
              value={prefs.density}
              onChange={(step) => void prefs.setDensity(step)}
              label={`${title} density`}
            />
          }
          hints={
            keyboardHints == null
              ? undefined
              : keyboardHints.map((hint) => ({ keys: hint.keys.join(" "), label: hint.label }))
          }
        />
      </section>

      {childrenAfter}
    </div>
  );
}
