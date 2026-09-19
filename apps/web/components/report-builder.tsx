// Report builder v1 (TASKLIST R6; ADR 0001): view → measures → group-by →
// filters → viz, with a live compile-on-demand preview. All identifiers
// come from the server-served field catalog (GET /api/v1/report-catalog)
// and every run re-validates server-side against the same Rust constants —
// the UI constructs specs, never SQL.
//
// Conventions: Workbench tokens, light+dark parity, transitions ≤120ms
// transform/opacity only, prefers-reduced-motion, responsive down to
// narrow viewports (config column stacks above the preview).
"use client";

import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Table,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, apiBaseUrl, loadTokens } from "../lib/api";
import { friendlyMutationText } from "../lib/error-copy";
import { downloadBuilderCsv } from "../lib/report-builder-csv";
import {
  AGG_LABELS,
  MAX_FILTERS,
  MAX_GROUP_BY,
  MAX_MEASURES,
  OP_LABELS,
  VIEW_LABELS,
  aggAccepts,
  draftProblem,
  draftToSpec,
  emptyDraft,
  labelize,
  measureLabel,
  operandType,
  type BuilderDraft,
  type FilterDraft,
  type MeasureDraft,
} from "../lib/report-builder";
import { ReportChart } from "./report-preview";

type ViewId = components["schemas"]["ViewId"];
type VizHint = components["schemas"]["VizHint"];
type Aggregate = components["schemas"]["Aggregate"];
type FilterOp = components["schemas"]["FilterOp"];
type CatalogView = components["schemas"]["CatalogView"];
type ReportDefinitionView = components["schemas"]["ReportDefinitionView"];
type ReportPage = components["schemas"]["ReportPage"];

/** Coerces a report page's unknown rows into keyed objects for rendering. */
function normalizeRows(page: ReportPage): Record<string, unknown>[] {
  return (page.rows ?? []).filter(
    (r): r is Record<string, unknown> => typeof r === "object" && r !== null,
  );
}

const VIZ_OPTIONS: { value: VizHint; label: string }[] = [
  { value: "bar", label: "Bar chart" },
  { value: "line", label: "Line chart" },
  { value: "pie", label: "Pie chart" },
  { value: "table", label: "Table" },
];

const FILTER_OPS: FilterOp[] = ["eq", "not_eq", "lt", "lt_eq", "gt", "gt_eq", "is_null", "is_not_null"];

export function ReportBuilder({
  catalog,
  editingDef,
  canPublish,
  onSaved,
  onCancelEdit,
  runToken,
  seedSpec,
}: {
  catalog: CatalogView[] | null;
  editingDef: ReportDefinitionView | null;
  canPublish: boolean;
  onSaved: () => void;
  onCancelEdit: () => void;
  /** Bumped by the page when a saved definition should run now. */
  runToken: { def: ReportDefinitionView; nonce: number } | null;
  /** Ad-hoc spec deep link (Wave 23 lane 1 items 2+3): /reports?spec=… */
  seedSpec: { spec: components["schemas"]["ReportSpec"]; from?: string; to?: string; nonce: number } | null;
}) {
  const [draft, setDraft] = useState<BuilderDraft>(() => emptyDraft());
  const [page, setPage] = useState<ReportPage | null>(null);
  const [running, setRunning] = useState(false);
  const [runError, setRunError] = useState<string | null>(null);
  const [autoRun, setAutoRun] = useState<{ nonce: number } | null>(null);
  const ranNonceRef = useRef<number>(-1);
  const [saveOpen, setSaveOpen] = useState(false);
  const [saveName, setSaveName] = useState("");
  const [saveDescription, setSaveDescription] = useState("");
  const [saveScope, setSaveScope] = useState<"org" | "self">("self");
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const viewCatalog = useMemo(
    () => catalog?.find((v) => v.view === draft.view),
    [catalog, draft.view],
  );

  // Load a definition into the builder (edit / run-a-saved-report).
  useEffect(() => {
    if (editingDef === null) return;
    const d = editingDef.spec;
    setDraft({
      view: d.view,
      filters: (d.filters ?? []).map((f) => ({
        column: f.column,
        op: f.op,
        rawValue: filterRawValue(f.value),
      })),
      groupBy: [...(d.group_by ?? [])],
      measures: d.measures.map((m) => ({ agg: m.agg, column: m.column ?? null })),
      viz: d.viz,
      from: "",
      to: "",
    });
    setSaveName(editingDef.name);
    setSaveDescription(editingDef.description ?? "");
    setSaveScope(editingDef.scope === "org" ? "org" : "self");
  }, [editingDef]);

  // Deep-link seeding (items 2+3): a ?spec= payload loads as an ad-hoc draft
  // and runs once — the builder stays fully editable after landing.
  useEffect(() => {
    if (seedSpec === null) return;
    const d = seedSpec.spec;
    setDraft({
      view: d.view,
      filters: (d.filters ?? []).map((f) => ({
        column: f.column,
        op: f.op,
        rawValue: filterRawValue(f.value),
      })),
      groupBy: [...(d.group_by ?? [])],
      measures: d.measures.map((m) => ({ agg: m.agg, column: m.column ?? null })),
      viz: d.viz,
      from: seedSpec.from ?? "",
      to: seedSpec.to ?? "",
    });
    setAutoRun({ nonce: seedSpec.nonce });
  }, [seedSpec]);

  const buildSpec = useCallback(
    () => draftToSpec(draft, viewCatalog),
    [draft, viewCatalog],
  );

  const runPreview = useCallback(async () => {
    setRunning(true);
    setRunError(null);
    const query = {
      from: draft.from === "" ? undefined : draft.from,
      to: draft.to === "" ? undefined : draft.to,
      limit: 200,
    };
    if (editingDef !== null) {
      const { data, error } = await api.api.GET("/api/v1/report-definitions/{id}/run", {
        params: { path: { id: editingDef.id }, query },
      });
      setRunning(false);
      if (error !== undefined || !data) {
        setRunError(friendlyMutationText(error, "This report could not be run."));
        setPage(null);
        return;
      }
      setPage(data);
    } else {
      const { data, error } = await api.api.POST("/api/v1/reports/run-spec", {
        body: { spec: buildSpec(), from: query.from ?? null, to: query.to ?? null, limit: query.limit },
      });
      setRunning(false);
      if (error !== undefined || !data) {
        setRunError(friendlyMutationText(error, "This report could not be run."));
        setPage(null);
        return;
      }
      setPage(data);
    }
  }, [buildSpec, draft.from, draft.to, editingDef]);

  // runToken: the saved-reports table asked this definition to run now.
  useEffect(() => {
    if (runToken === null || editingDef === null || editingDef.id !== runToken.def.id) return;
    void runPreview();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runToken?.nonce]);

  // Deep-link auto-run (items 2+3): fires once per nonce AFTER the seeded
  // draft has committed, so the preview runs the linked spec verbatim.
  useEffect(() => {
    if (autoRun === null || editingDef !== null) return;
    if (ranNonceRef.current === autoRun.nonce) return;
    ranNonceRef.current = autoRun.nonce;
    void runPreview();
    setAutoRun(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoRun?.nonce, draft.from, draft.to, draft.view]);

  const problem = draftProblem(draft, viewCatalog);

  const handleSave = async () => {
    if (!saveName.trim()) return;
    setSaving(true);
    setSaveError(null);
    const idempotencyKey =
      typeof crypto !== "undefined" && crypto.randomUUID ? crypto.randomUUID() : `rb-${Date.now()}`;
    const body = {
      name: saveName.trim(),
      description: saveDescription.trim() === "" ? null : saveDescription.trim(),
      spec: buildSpec(),
      scope: canPublish ? saveScope : ("self" as const),
      idempotency_key: idempotencyKey,
    };
    const { error } =
      editingDef !== null
        ? await api.api.PUT("/api/v1/report-definitions/{id}", {
            params: { path: { id: editingDef.id } },
            body,
          })
        : await api.api.POST("/api/v1/report-definitions", { body });
    setSaving(false);
    if (error !== undefined) {
      setSaveError(friendlyMutationText(error, "The report could not be saved."));
      return;
    }
    setSaveOpen(false);
    onSaved();
  };

  const changeView = (view: ViewId) => {
    setDraft((d) => ({
      ...emptyDraft(view),
      viz: d.viz,
      from: d.from,
      to: d.to,
    }));
    setPage(null);
    setRunError(null);
  };

  const patch = (fn: (d: BuilderDraft) => BuilderDraft) => {
    setDraft(fn);
    setPage(null);
    setRunError(null);
  };

  const addMeasure = () =>
    patch((d) =>
      d.measures.length >= MAX_MEASURES
        ? d
        : { ...d, measures: [...d.measures, { agg: "count", column: null }] },
    );
  const addFilter = () =>
    patch((d) => {
      if (d.filters.length >= MAX_FILTERS || viewCatalog === undefined) return d;
      const first = viewCatalog.filterable[0]?.name;
      return first === undefined
        ? d
        : { ...d, filters: [...d.filters, { column: first, op: "eq" as FilterOp, rawValue: "" }] };
    });

  const noCatalog = catalog === null;

  return (
    <div className="grid gap-4 lg:grid-cols-[minmax(320px,380px)_minmax(0,1fr)] items-start">
      {/* --- config column --- */}
      <Card className="p-4 grid gap-4 min-w-0">
        <div>
          <Label htmlFor="rb-view">Data</Label>
          <Select value={draft.view} onValueChange={(v) => changeView(v as ViewId)}>
            <SelectTrigger id="rb-view" className="w-full" aria-label="Report data view">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {(catalog ?? []).map((v) => (
                <SelectItem key={v.view} value={v.view}>
                  {VIEW_LABELS[v.view]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <FieldHint>The versioned operational read model. Saved reports read views only — never raw tables.</FieldHint>
        </div>

        <div>
          <Label>Group by</Label>
          <div className="grid gap-2">
            {draft.groupBy.length === 0 ? (
              <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                No grouping — one total row.
              </p>
            ) : null}
            {draft.groupBy.map((g, i) => (
              <div key={`${g}-${i}`} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-2">
                <Select
                  value={g}
                  onValueChange={(v) =>
                    patch((d) => {
                      const groupBy = [...d.groupBy];
                      groupBy[i] = v;
                      return { ...d, groupBy };
                    })
                  }
                >
                  <SelectTrigger className="min-w-0" aria-label={`Grouping ${i + 1}`}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {(viewCatalog?.columns ?? [])
                      .filter((c) => c.name !== "org_id")
                      .map((c) => (
                        <SelectItem key={c.name} value={c.name}>
                          {labelize(c.name)}
                        </SelectItem>
                      ))}
                  </SelectContent>
                </Select>
                <Button
                  size="sm"
                  variant="ghost"
                  aria-label={`Remove grouping ${labelize(g)}`}
                  onClick={() => patch((d) => ({ ...d, groupBy: d.groupBy.filter((_, j) => j !== i) }))}
                >
                  ✕
                </Button>
              </div>
            ))}
            <Button
              size="sm"
              variant="secondary"
              disabled={draft.groupBy.length >= MAX_GROUP_BY || noCatalog}
              onClick={() =>
                patch((d) => {
                  if (d.groupBy.length >= MAX_GROUP_BY || viewCatalog === undefined) return d;
                  const used = new Set(d.groupBy);
                  const next = (
                    viewCatalog.columns.find((c) => c.name !== "org_id" && !used.has(c.name)) ??
                      viewCatalog.columns[0]
                  )?.name;
                  return next === undefined ? d : { ...d, groupBy: [...d.groupBy, next] };
                })
              }
            >
              Add grouping
            </Button>
          </div>
        </div>

        <div>
          <Label>Measures</Label>
          <div className="grid gap-2">
            {draft.measures.map((m, i) => (
              <MeasureRow
                key={i}
                measure={m}
                index={i}
                columns={viewCatalog?.columns ?? []}
                onChange={(next) =>
                  patch((d) => {
                    const measures = [...d.measures];
                    measures[i] = next;
                    return { ...d, measures };
                  })
                }
                onRemove={
                  draft.measures.length === 1
                    ? undefined
                    : () => patch((d) => ({ ...d, measures: d.measures.filter((_, j) => j !== i) }))
                }
              />
            ))}
            <Button size="sm" variant="secondary" disabled={draft.measures.length >= MAX_MEASURES} onClick={addMeasure}>
              Add measure
            </Button>
          </div>
        </div>

        <div>
          <Label>Filters</Label>
          <div className="grid gap-2">
            {draft.filters.length === 0 ? (
              <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                No filters — all rows in the window.
              </p>
            ) : null}
            {draft.filters.map((f, i) => (
              <FilterRow
                key={i}
                filter={f}
                filterable={viewCatalog?.filterable ?? []}
                onChange={(next) =>
                  patch((d) => {
                    const filters = [...d.filters];
                    filters[i] = next;
                    return { ...d, filters };
                  })
                }
                onRemove={() => patch((d) => ({ ...d, filters: d.filters.filter((_, j) => j !== i) }))}
              />
            ))}
            <Button
              size="sm"
              variant="secondary"
              disabled={draft.filters.length >= MAX_FILTERS || (viewCatalog?.filterable.length ?? 0) === 0}
              onClick={addFilter}
            >
              Add filter
            </Button>
          </div>
        </div>

        <div className="grid grid-cols-2 gap-2">
          <div>
            <Label htmlFor="rb-viz">Chart</Label>
            <Select value={draft.viz} onValueChange={(v) => patch((d) => ({ ...d, viz: v as VizHint }))}>
              <SelectTrigger id="rb-viz" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {VIZ_OPTIONS.map((o) => (
                  <SelectItem key={o.value} value={o.value}>
                    {o.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="grid grid-cols-2 gap-2">
            <div>
              <Label htmlFor="rb-from">From</Label>
              <Input
                id="rb-from"
                type="date"
                value={draft.from}
                onChange={(e) => patch((d) => ({ ...d, from: e.target.value }))}
              />
            </div>
            <div>
              <Label htmlFor="rb-to">To</Label>
              <Input
                id="rb-to"
                type="date"
                value={draft.to}
                onChange={(e) => patch((d) => ({ ...d, to: e.target.value }))}
              />
            </div>
          </div>
        </div>

        {problem !== null ? (
          <p role="status" className="text-[12px]" style={{ color: "var(--m-status-warning-fg, var(--m-text-secondary))" }}>
            {problem}
          </p>
        ) : null}

        <div className="flex flex-wrap items-center gap-2">
          <Button onClick={() => void runPreview()} disabled={running || problem !== null || noCatalog}>
            {running ? <Spinner /> : null} Run preview
          </Button>
          <Dialog
            open={saveOpen}
            onOpenChange={(open) => {
              setSaveOpen(open);
              if (open) setSaveError(null);
            }}
          >
            <DialogTrigger asChild>
              <Button variant="secondary" disabled={problem !== null || noCatalog}>
                {editingDef !== null ? "Save changes" : "Save report"}
              </Button>
            </DialogTrigger>
            <DialogContent title={editingDef !== null ? "Save report changes" : "Save report"}>
              <div className="grid gap-3 p-4">
                <div>
                  <Label htmlFor="rb-name">Name</Label>
                  <Input id="rb-name" value={saveName} onChange={(e) => setSaveName(e.target.value)} required />
                </div>
                <div>
                  <Label htmlFor="rb-desc">Description</Label>
                  <Input
                    id="rb-desc"
                    value={saveDescription}
                    onChange={(e) => setSaveDescription(e.target.value)}
                    placeholder="Optional"
                  />
                </div>
                <div>
                  <Label htmlFor="rb-scope">Visibility</Label>
                  <Select
                    value={canPublish ? saveScope : "self"}
                    onValueChange={(v) => setSaveScope(v as "org" | "self")}
                    disabled={!canPublish}
                  >
                    <SelectTrigger id="rb-scope" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="self">Private to me</SelectItem>
                      <SelectItem value="org">Publish org-wide</SelectItem>
                    </SelectContent>
                  </Select>
                  {canPublish ? null : (
                    <FieldHint>Publishing org-wide requires an owner or admin.</FieldHint>
                  )}
                </div>
                {saveError !== null ? (
                  <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                    {saveError}
                  </p>
                ) : null}
                <DialogActions>
                  <Button variant="ghost" onClick={() => setSaveOpen(false)}>
                    Cancel
                  </Button>
                  <Button onClick={() => void handleSave()} disabled={saving || !saveName.trim()}>
                    {saving ? <Spinner /> : null} Save
                  </Button>
                </DialogActions>
              </div>
            </DialogContent>
          </Dialog>
          {editingDef !== null ? (
            <Button variant="ghost" onClick={onCancelEdit}>
              Discard changes
            </Button>
          ) : null}
        </div>
      </Card>

      {/* --- preview column --- */}
      <Card className="p-4 grid gap-3 min-w-0">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="min-w-0">
            <h2 className="text-sm font-semibold">Preview</h2>
            <p className="text-[12px] truncate" style={{ color: "var(--m-text-secondary)" }}>
              {editingDef !== null
                ? `Running saved report: ${editingDef.name}`
                : "Compile-on-demand — nothing is saved until you save the report."}
            </p>
          </div>
          {editingDef !== null && page !== null ? (
            <Button size="sm" variant="secondary" onClick={() => void downloadBuilderCsv(editingDef.id, editingDef.name, draft.from, draft.to)}>
              Download CSV
            </Button>
          ) : null}
        </div>

        {runError !== null ? (
          <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {runError}
          </p>
        ) : null}

        {running ? (
          <div className="p-8 flex justify-center">
            <Spinner />
          </div>
        ) : page === null ? (
          <EmptyState message="Run the preview to see rows. The spec compiles server-side against the read-model views." />
        ) : draft.viz === "table" || draft.groupBy.length === 0 ? (
          <PreviewTable page={page} />
        ) : (
          <div className="grid gap-3">
            <ReportChart
              page={{ columns: page.columns ?? [], rows: normalizeRows(page) }}
              measures={draft.measures}
              viz={draft.viz === "pie" ? "pie" : draft.viz === "line" ? "line" : "bar"}
              ariaLabel={`${draft.viz} chart of ${VIEW_LABELS[draft.view]}`}
            />
            <PreviewTable page={page} collapsible />
          </div>
        )}
      </Card>
    </div>
  );
}

/** Renders a FilterValue back to the input's raw text when editing. */
function filterRawValue(value: components["schemas"]["FilterValue"] | undefined | null): string {
  if (value === undefined || value === null) return "";
  if ("bool" in value) return value.bool ? "true" : "false";
  if ("text" in value) return value.text;
  if ("int" in value) return String(value.int);
  if ("date" in value) return value.date;
  if ("timestamp" in value) return value.timestamp.slice(0, 16);
  return "uuid" in value ? value.uuid : "";
}

function MeasureRow({
  measure,
  index,
  columns,
  onChange,
  onRemove,
}: {
  measure: MeasureDraft;
  index: number;
  columns: { name: string; value_type: string }[];
  onChange: (m: MeasureDraft) => void;
  onRemove?: () => void;
}) {
  const eligible = (agg: Aggregate) => (agg === "count" ? true : columns.some((c) => aggAccepts(agg, c.value_type)));
  const currentType =
    measure.column !== null ? (columns.find((c) => c.name === measure.column)?.value_type ?? null) : null;
  const invalidColumn =
    measure.agg !== "count" && currentType !== null && !aggAccepts(measure.agg, currentType);
  return (
    <div className="grid gap-1">
      <div className="grid grid-cols-[130px_minmax(0,1fr)_auto] items-center gap-2">
        <Select
          value={measure.agg}
          onValueChange={(v) => {
            const agg = v as Aggregate;
            onChange({ agg, column: agg === "count" ? measure.column : measure.column ?? null });
          }}
        >
          <SelectTrigger aria-label={`Measure ${index + 1} aggregate`}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {(["count", "sum", "avg", "min", "max"] as Aggregate[])
              .filter(eligible)
              .map((agg) => (
                <SelectItem key={agg} value={agg}>
                  {AGG_LABELS[agg]}
                </SelectItem>
              ))}
          </SelectContent>
        </Select>
        <Select
          value={measure.column ?? ""}
          onValueChange={(v) => onChange({ ...measure, column: v === "" ? null : v })}
        >
          <SelectTrigger className="min-w-0" aria-label={`Measure ${index + 1} column`}>
            <SelectValue placeholder={measure.agg === "count" ? "all rows" : "pick a column"} />
          </SelectTrigger>
          <SelectContent>
            {measure.agg === "count" ? (
              <SelectItem value="">all rows</SelectItem>
            ) : null}
            {columns
              .filter((c) => aggAccepts(measure.agg, c.value_type))
              .map((c) => (
                <SelectItem key={c.name} value={c.name}>
                  {labelize(c.name)}
                </SelectItem>
              ))}
          </SelectContent>
        </Select>
        {onRemove !== undefined ? (
          <Button size="sm" variant="ghost" aria-label={`Remove measure ${index + 1}`} onClick={onRemove}>
            ✕
          </Button>
        ) : null}
      </div>
      {invalidColumn ? (
        <p className="text-[11px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {AGG_LABELS[measure.agg]} does not apply to {labelize(measure.column ?? "")}.
        </p>
      ) : null}
    </div>
  );
}

function FilterRow({
  filter,
  filterable,
  onChange,
  onRemove,
}: {
  filter: FilterDraft;
  filterable: { name: string; value_type: string }[];
  onChange: (f: FilterDraft) => void;
  onRemove: () => void;
}) {
  const valueType =
    filterable.find((c) => c.name === filter.column)?.value_type ?? "text";
  const takesValue = filter.op !== "is_null" && filter.op !== "is_not_null";
  const opType = operandType(valueType);
  return (
    <div className="grid grid-cols-[150px_120px_minmax(0,1fr)_auto] items-center gap-2">
      <Select value={filter.column} onValueChange={(v) => onChange({ ...filter, column: v, rawValue: "" })}>
        <SelectTrigger aria-label="Filter column">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {filterable.map((c) => (
            <SelectItem key={c.name} value={c.name}>
              {labelize(c.name)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Select value={filter.op} onValueChange={(v) => onChange({ ...filter, op: v as FilterOp })}>
        <SelectTrigger aria-label="Filter operator">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {FILTER_OPS.map((op) => (
            <SelectItem key={op} value={op}>
              {OP_LABELS[op]}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {takesValue ? (
        <Input
          className="min-w-0"
          type={opType === "date" ? "date" : opType === "int" ? "number" : "text"}
          value={filter.rawValue}
          aria-label={`Filter value for ${labelize(filter.column)}`}
          placeholder={opType === "uuid" ? "00000000-… uuid" : opType === "bool" ? "true / false" : undefined}
          onChange={(e) => onChange({ ...filter, rawValue: e.target.value })}
        />
      ) : (
        <div />
      )}
      <Button size="sm" variant="ghost" aria-label={`Remove filter ${labelize(filter.column)}`} onClick={onRemove}>
        ✕
      </Button>
    </div>
  );
}

function PreviewTable({ page, collapsible = false }: { page: ReportPage; collapsible?: boolean }) {
  const [expanded, setExpanded] = useState(false);
  const rows = normalizeRows(page);
  const maxRows = collapsible && !expanded ? 10 : rows.length;
  const summaryEntries = Object.entries(
    (typeof page.summary === "object" && page.summary !== null ? page.summary : {}) as Record<
      string,
      unknown
    >,
  );
  return (
    <div className="grid gap-2 min-w-0">
      {summaryEntries.length > 0 ? (
        <div className="flex flex-wrap gap-x-4 gap-y-1 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
          {summaryEntries.map(([k, v]) => (
            <span key={k}>
              {labelize(k)}:{" "}
              <span style={{ color: "var(--m-text)" }} className="tabular-nums">
                {typeof v === "object" ? "—" : String(v ?? "—")}
              </span>
            </span>
          ))}
        </div>
      ) : null}
      {rows.length === 0 ? (
        <EmptyState message="No rows matched this report in the selected window." />
      ) : (
        <>
          <div className="overflow-x-auto">
            <Table>
              <thead>
                <tr>
                  {page.columns.map((c) => (
                    <Th key={c}>{labelize(c)}</Th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {rows.slice(0, maxRows).map((row, i) => (
                  <Tr key={i}>
                    {page.columns.map((c) => {
                      const v = row[c];
                      const display =
                        v === null || v === undefined
                          ? "—"
                          : typeof v === "object"
                            ? JSON.stringify(v)
                            : String(v);
                      const numeric = typeof v === "number" || (typeof v === "string" && v !== "" && !Number.isNaN(Number(v)) && /^-?[\d.]+$/.test(v));
                      return (
                        <Td key={c} className={numeric ? "tabular-nums" : undefined}>
                          {display}
                        </Td>
                      );
                    })}
                  </Tr>
                ))}
              </tbody>
            </Table>
          </div>
          {collapsible && rows.length > 10 ? (
            <Button size="sm" variant="ghost" onClick={() => setExpanded((e) => !e)}>
              {expanded ? "Show fewer rows" : `Show all ${rows.length} rows`}
            </Button>
          ) : null}
        </>
      )}
    </div>
  );
}
