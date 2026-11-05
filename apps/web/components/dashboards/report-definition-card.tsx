"use client";
/**
 * ReportDefinitionCard — a SAVED report on the dashboard grid.
 *
 * The catalogue's widgets are fixed report kinds with no card of their own. A
 * saved report is a stored definition, and since migration 0180 it carries the
 * card it was authored with: its window, its refresh, its size. This card draws
 * the definition through the same `GET /api/v1/report-definitions/{id}/run`
 * route the reports surface uses, and prints the stored card beside the data —
 * the window it displays and the refresh it will read at are the reviewable
 * facts, not a rendering choice.
 *
 * Sizing works exactly like a catalogue card: the panel's options come from the
 * `GET /api/report-catalog` payload and the choice is stored on the placement.
 * The card in the definition is untouched by a dashboard resize; that is a
 * separate record and a resize here is a placement decision.
 *
 * Non-purpose: no spec editing, no card editing, no second run path.
 */
import { Badge, Button, Card, EmptyState, Spinner, Table, Td, Th, Tr } from "@marula-core/ui";
import { useCallback, useEffect, useId, useState } from "react";
import { api } from "../../lib/api";
import type { CardSize, StoredCard } from "../../lib/dashboards";
import { CardContractMeta } from "./card-contract";
import { CardSizePanel, CardSizeToggle, cardHeightClass } from "./card-size-control";

/** The members of `ReportDefinitionView` this card reads. */
export interface DefinitionView {
  id: string;
  name: string;
  description?: string | null;
  card?: StoredCard | null;
  spec: { viz: string };
}

interface RunPage {
  columns: string[];
  rows: unknown[];
  summary: unknown;
}

interface Props {
  def: DefinitionView;
  size: CardSize;
  spans?: readonly string[];
  heights?: readonly string[];
  entranceDelay?: number;
  onResize?: (size: CardSize) => void;
  onMove?: (direction: -1 | 1) => void;
  canMoveEarlier?: boolean;
  canMoveLater?: boolean;
  onRemove?: () => void;
  onOpenReports?: () => void;
}

type LoadState =
  | { status: "loading" }
  | { status: "ready"; page: RunPage }
  | { status: "error"; message: string };

/** The first scalar in a run's summary, or null: a stat with no number is not a zero. */
function scalarOf(summary: unknown): string | null {
  if (typeof summary !== "object" || summary === null) return null;
  for (const value of Object.values(summary as Record<string, unknown>)) {
    if (typeof value === "number" && Number.isFinite(value)) return String(value);
    if (typeof value === "string" && value.trim() !== "") return value;
    if (typeof value === "boolean") return value ? "Yes" : "No";
  }
  return null;
}

function cellText(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return JSON.stringify(value);
}

export function ReportDefinitionCard({
  def,
  size,
  spans = [],
  heights = [],
  entranceDelay = 0,
  onResize,
  onMove,
  canMoveEarlier = false,
  canMoveLater = false,
  onRemove,
  onOpenReports,
}: Props) {
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [attempt, setAttempt] = useState(0);
  const [sizeOpen, setSizeOpen] = useState(false);
  const sizePanelId = useId();

  const load = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/report-definitions/{id}/run", {
      params: { path: { id: def.id }, query: { limit: 50 } },
    });
    if (error !== undefined || data === undefined) {
      return { ok: false as const, message: "The report did not run." };
    }
    return { ok: true as const, page: data as RunPage };
  }, [def.id]);

  useEffect(() => {
    let cancelled = false;
    setState({ status: "loading" });
    void (async () => {
      const result = await load();
      if (cancelled) return;
      if (result.ok) setState({ status: "ready", page: result.page });
      else setState({ status: "error", message: result.message });
    })();
    return () => {
      cancelled = true;
    };
  }, [load, attempt]);

  let body: React.ReactNode;
  if (state.status === "loading") {
    body = (
      <div className="flex justify-center p-4">
        <Spinner label={`Running ${def.name}`} />
      </div>
    );
  } else if (state.status === "error") {
    body = (
      <div className="grid gap-2" role="alert">
        <p className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {state.message}
        </p>
        <div>
          <Button variant="secondary" size="sm" onClick={() => setAttempt((n) => n + 1)}>
            Retry
          </Button>
        </div>
      </div>
    );
  } else if (state.page.rows.length === 0) {
    body = (
      <EmptyState message="No rows at this window. Widen the card's window or loosen a filter in the builder." />
    );
  } else if (def.spec.viz === "stat") {
    const value = scalarOf(state.page.summary);
    body =
      value === null ? (
        <EmptyState message="The run returned no headline number for this stat." />
      ) : (
        <p className="text-2xl font-semibold tabular-nums" style={{ color: "var(--m-text)" }}>
          {value}
        </p>
      );
  } else {
    const columns = state.page.columns.slice(0, 5);
    const rows = state.page.rows.slice(0, 8);
    body = (
      <Table aria-label={`${def.name} rows`}>
        <thead>
          <tr>
            {columns.map((column) => (
              <Th key={column}>{column}</Th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, index) => (
            <Tr key={index}>
              {columns.map((column) => (
                <Td key={column}>
                  {cellText((row as Record<string, unknown>)[column])}
                </Td>
              ))}
            </Tr>
          ))}
        </tbody>
      </Table>
    );
  }

  return (
    <Card
      className={`m-rise group grid gap-3 p-4 transition-shadow duration-100 hover:shadow-md ${cardHeightClass(size.height)}`}
      data-widget-id={`definition:${def.id}`}
      data-testid={`definition-widget-${def.id}`}
      data-card-span={size.span}
      data-card-height={size.height}
      style={{ animationDelay: `${entranceDelay}ms` }}
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <h3 className="truncate text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
            {def.name}
          </h3>
          <Badge tone="outline">Saved report</Badge>
        </div>
        <div className="flex items-center gap-1 opacity-0 transition-opacity duration-100 group-focus-within:opacity-100 group-hover:opacity-100">
          {onResize !== undefined ? (
            <CardSizeToggle
              open={sizeOpen}
              onToggle={() => setSizeOpen((open) => !open)}
              panelId={sizePanelId}
              title={def.name}
            />
          ) : null}
          {onOpenReports !== undefined ? (
            <Button variant="ghost" size="sm" onClick={onOpenReports}>
              Open
            </Button>
          ) : null}
          {onRemove !== undefined ? (
            <Button
              variant="ghost"
              size="sm"
              onClick={onRemove}
              aria-label={`Remove ${def.name}`}
            >
              Remove
            </Button>
          ) : null}
        </div>
      </div>

      {/* The stored card, read from the definition: window, refresh, size. */}
      <CardContractMeta
        card={def.card}
        testId={`definition-card-meta-${def.id}`}
      />

      <div id={sizePanelId}>
        {sizeOpen && onResize !== undefined ? (
          <CardSizePanel
            title={def.name}
            spans={spans}
            heights={heights}
            size={size}
            onResize={onResize}
            onMove={(direction) => onMove?.(direction)}
            canMoveEarlier={canMoveEarlier}
            canMoveLater={canMoveLater}
            testId={`card-size-definition-${def.id}`}
          />
        ) : null}
      </div>

      {body}
    </Card>
  );
}
