// /imports — the D2 import wizard (docs/money lane close-out).
//
// Purpose: source selection → CSV/JSON upload → dry-run report (typed
// per-row errors, nothing written) → field-mapping template (saved per
// source system, applied before validation on the next run) → batch
// create + chunked stage + finish + queued run with live counters, plus
// the batch run history.
//
// Non-purpose: no server-side file storage (files are chunked straight
// into the batch); no transform expressions — mapping is 1:1 column
// renaming via saved templates.
"use client";

import {
  Button,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  EmptyState,
  ExportControl,
  FieldHint,
  FilterBar,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Table,
  Tag,
  Td,
  Th,
  Tr,
  ViewSwitch,
  applyDensity,
  useDensity,
  SurfaceHeader,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import {
  BlushNote,
  HEAD_CELL_CLASS,
  InlineVerdict,
  LedgerFooter,
  NoMatches,
  PageFrame,
  SkeletonRows,
  StatPill,
  StatusPill,
  TableCard,
  UploadIcon,
  WizardStepRail,
  downloadCsv,
} from "../../components/knowledge/ledger";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";

type ImportTemplate = components["schemas"]["TemplateViewResponse"];
type BatchViewResponse = components["schemas"]["BatchViewResponse"];
type RunReportResponse = components["schemas"]["RunReportResponse"];
type RowReport = components["schemas"]["RowReport"];
type EntityKind = NonNullable<ImportTemplate["entity_kind"]>;

const ENTITY_KINDS: { value: EntityKind; label: string }[] = [
  { value: "client", label: "Clients" },
  { value: "site", label: "Sites" },
  { value: "contact", label: "Contacts" },
  { value: "board", label: "Boards" },
  { value: "ticket", label: "Tickets" },
  { value: "time_entry", label: "Time entries" },
  { value: "kb_category", label: "KB categories" },
  { value: "kb_article", label: "KB articles" },
];

const POLICIES = [
  { value: "skip", label: "Skip existing" },
  { value: "update", label: "Update existing" },
  { value: "fail", label: "Fail on conflict" },
] as const;

const CHUNK_ROWS = 500;

type ParsedCsv = { headers: string[]; rows: Record<string, string>[] };

type HistoryColumnKey =
  | "label"
  | "entity"
  | "policy"
  | "rows"
  | "created"
  | "updated"
  | "skipped"
  | "failed"
  | "finished";

interface HistoryColumn {
  key: HistoryColumnKey;
  label: string;
  visible: boolean;
  required?: boolean;
}

const HISTORY_COLUMNS: HistoryColumn[] = [
  { key: "label", label: "Batch", visible: true, required: true },
  { key: "entity", label: "Entity", visible: true },
  { key: "policy", label: "Policy", visible: true },
  { key: "rows", label: "Rows", visible: true },
  { key: "created", label: "Created", visible: true },
  { key: "updated", label: "Updated", visible: true },
  { key: "skipped", label: "Skipped", visible: false },
  { key: "failed", label: "Failed", visible: true },
  { key: "finished", label: "Finished", visible: true },
];

const RIGHT_COLUMNS = new Set<HistoryColumnKey>([
  "rows",
  "created",
  "updated",
  "skipped",
  "failed",
]);

/** One cell of the run-history ledger, used by both the table and the export so
 *  a downloaded file can never disagree with the screen it came from. */
function historyCell(key: HistoryColumnKey, b: BatchViewResponse): string {
  if (key === "label") return b.label;
  if (key === "entity") return b.entity_kind;
  if (key === "policy") return b.conflict_policy;
  if (key === "rows") return String(b.rows_total);
  if (key === "created") return String(b.rows_created);
  if (key === "updated") return String(b.rows_updated);
  if (key === "skipped") return String(b.rows_skipped);
  if (key === "failed") return String(b.rows_failed);
  return b.finished_at == null ? "—" : new Date(b.finished_at).toLocaleString();
}

function parseCsv(text: string): ParsedCsv {
  const rows: string[][] = [];
  let field = "";
  let record: string[] = [];
  let inQuotes = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (inQuotes) {
      if (c === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i++;
        } else inQuotes = false;
      } else field += c;
    } else if (c === '"') inQuotes = true;
    else if (c === ",") {
      record.push(field);
      field = "";
    } else if (c === "\n" || c === "\r") {
      if (c === "\r" && text[i + 1] === "\n") i++;
      record.push(field);
      field = "";
      if (record.length > 1 || record[0] !== "") rows.push(record);
      record = [];
    } else field += c;
  }
  record.push(field);
  if (record.length > 1 || record[0] !== "") rows.push(record);
  const clean = (s: string) => s.replace(/^\uFEFF/, "").trim();
  const headers = (rows.shift() ?? []).map(clean);
  return {
    headers,
    rows: rows.map((r) => Object.fromEntries(headers.map((h, i) => [h, r[i] ?? ""]))),
  };
}

export default function ImportsPage() {
  const auth = useRequireAuth();
  const [templates, setTemplates] = useState<ImportTemplate[] | null>(null);
  const [batches, setBatches] = useState<BatchViewResponse[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Step state
  const [step, setStep] = useState(1);
  const [kind, setKind] = useState<EntityKind>("client");
  const [policy, setPolicy] = useState<(typeof POLICIES)[number]["value"]>("skip");
  const [label, setLabel] = useState("");
  const [sourceSystem, setSourceSystem] = useState("");
  const [templateId, setTemplateId] = useState("none");

  const [csvText, setCsvText] = useState("");
  const [parsed, setParsed] = useState<ParsedCsv | null>(null);

  const [report, setReport] = useState<RunReportResponse | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Mapping template editor
  const [mapping, setMapping] = useState<Record<string, string>>({});
  const [templateName, setTemplateName] = useState("");
  const [savedNote, setSavedNote] = useState<string | null>(null);
  const [historyView, setHistoryView] = useState("all");
  const [historyClauses, setHistoryClauses] = useState<
    { id: string; field: string; op: never; value?: string }[]
  >([]);
  const [historyColumns, setHistoryColumns] = useState<HistoryColumn[]>(HISTORY_COLUMNS);
  const density = useDensity();

  const fileInput = useRef<HTMLInputElement>(null);

  const refresh = useCallback(async () => {
    const [t, b] = await Promise.all([
      api.api.GET("/api/v1/imports/templates"),
      api.api.GET("/api/v1/imports/batches"),
    ]);
    if (t.error !== undefined || b.error !== undefined) {
      setLoadError("Failed to load import data.");
      return;
    }
    setTemplates(t.data ?? []);
    setBatches(b.data ?? []);
    setLoadError(null);
  }, []);

  // A refresh cadence so queued runs' counters fill in live.
  useEffect(() => {
    if (auth.status !== "authed") return;
    const timer = setInterval(() => {
      void api.api.GET("/api/v1/imports/batches").then(({ data }) => {
        if (data !== undefined) setBatches(data);
      });
    }, 5000);
    return () => clearInterval(timer);
  }, [auth.status]);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const kindTemplates = useMemo(
    () => (templates ?? []).filter((t) => t.entity_kind === kind),
    [templates, kind],
  );

  const loadFile = async (file: File) => {
    const text = await file.text();
    setCsvText(text);
    try {
      const p = parseCsv(text);
      setParsed(p);
      const identity = Object.fromEntries(p.headers.map((h) => [h, h]));
      setMapping(identity);
    } catch {
      setParsed(null);
      setError("Could not parse the file as CSV.");
    }
  };

  const runDryRun = async () => {
    setBusy(true);
    setError(null);
    setReport(null);
    setSavedNote(null);
    const { data, error: e } = await api.api.POST("/api/v1/imports/dry-run", {
      body: {
        entity_kind: kind,
        conflict_policy: policy,
        label: label === "" ? null : label,
        template_id: templateId === "none" ? null : templateId,
        format: "csv",
        csv: csvText,
        json_rows: [] as unknown as Record<string, never>,
      },
    });
    setBusy(false);
    if (e !== undefined || data === undefined) {
      setError("Dry run failed — check the file and template, then retry.");
      return;
    }
    setReport(data);
    setStep(3);
  };

  const saveTemplate = async () => {
    setBusy(true);
    setError(null);
    const { error: e } = await api.api.POST("/api/v1/imports/templates", {
      body: {
        name: templateName,
        source_system: sourceSystem === "" ? "generic" : sourceSystem,
        entity_kind: kind,
        mapping: mapping as unknown as Record<string, never>,
      },
    });
    setBusy(false);
    if (e !== undefined) {
      setError("Template save failed — the name may already exist for this source.");
      return;
    }
    setSavedNote("Mapping template saved for this source system.");
    setTemplateName("");
    await refresh();
  };

  const createAndRun = async () => {
    setBusy(true);
    setError(null);
    const created = await api.api.POST("/api/v1/imports/batches", {
      body: {
        entity_kind: kind,
        conflict_policy: policy,
        label: label === "" ? null : label,
        source_note: sourceSystem === "" ? null : sourceSystem,
      },
    });
    if (created.error !== undefined || created.data === undefined) {
      setBusy(false);
      setError("Batch creation failed.");
      return;
    }
    const batchId = created.data.batch_id;
    for (let i = 0; i < parsed!.rows.length; i += CHUNK_ROWS) {
      const chunk = parsed!.rows.slice(i, i + CHUNK_ROWS);
      const staged = await api.api.POST("/api/v1/imports/batches/{batch_id}/stage", {
        params: { path: { batch_id: batchId } },
        body: {
          template_id: templateId === "none" ? null : templateId,
          format: "json",
          json_rows: chunk as unknown as Record<string, never>,
        },
      });
      if (staged.error !== undefined) {
        setBusy(false);
        setError(`Staging chunk ${Math.floor(i / CHUNK_ROWS) + 1} failed. The batch is left open — retry from run history.`);
        await refresh();
        return;
      }
    }
    await api.api.POST("/api/v1/imports/batches/{batch_id}/finish", {
      params: { path: { batch_id: batchId } },
    });
    const run = await api.api.POST("/api/v1/imports/batches/{batch_id}/run", {
      params: { path: { batch_id: batchId } },
    });
    setBusy(false);
    if (run.error !== undefined) {
      setError("The batch was staged but could not be queued — retry from run history.");
      await refresh();
      return;
    }
    setSavedNote("Batch queued. Counters fill in as the worker commits chunks.");
    await refresh();
  };

  const historyRows = useMemo(() => {
    let rows = batches ?? [];
    if (historyView === "live") rows = rows.filter((b) => b.queued_at != null && b.finished_at == null);
    if (historyView === "failed") rows = rows.filter((b) => b.rows_failed > 0);
    if (historyView === "probes") rows = rows.filter((b) => b.dry_run);
    for (const clause of historyClauses) {
      const value = (clause.value ?? "").toLowerCase();
      if (clause.field === "label") rows = rows.filter((b) => b.label.toLowerCase().includes(value));
      if (clause.field === "entity") rows = rows.filter((b) => b.entity_kind === clause.value);
      if (clause.field === "policy") rows = rows.filter((b) => b.conflict_policy === clause.value);
    }
    return rows;
  }, [batches, historyView, historyClauses]);

  const shownHistoryColumns = historyColumns.filter((c) => c.visible);

  if (auth.status !== "authed") return null;

  /* --- the stack: one step a screen, each one resumable ----------------- */
  const steps = [
    { id: "source", label: "Source" },
    { id: "upload", label: "Upload" },
    { id: "report", label: "Dry-run report" },
    { id: "mapping", label: "Mapping" },
    { id: "commit", label: "Commit" },
  ];
  // A step is reachable once its input exists: the report needs a dry-run, the
  // mapping and the commit need a parsed payload. The rail offers exactly
  // that, so the stack never shows an empty step as if it were finished.
  const reached = [
    true,
    true,
    report !== null,
    parsed !== null,
    parsed !== null,
  ];

  const stageSummary = [
    `${ENTITY_KINDS.find((k) => k.value === kind)?.label ?? kind} · ${POLICIES.find((p) => p.value === policy)?.label ?? policy}`,
    parsed === null
      ? "No payload yet"
      : `${parsed.rows.length.toLocaleString()} row${parsed.rows.length === 1 ? "" : "s"}, ${parsed.headers.length} columns`,
    report === null
      ? "Not probed yet — nothing has been written"
      : `${report.summary.ok.toLocaleString()} would be created, ${report.summary.failed.toLocaleString()} would fail`,
    parsed === null
      ? "Mapping needs a payload"
      : `${parsed.headers.length} column${parsed.headers.length === 1 ? "" : "s"} to map`,
    parsed === null
      ? "Commit needs a payload"
      : `${Math.ceil(parsed.rows.length / CHUNK_ROWS)} chunk${Math.ceil(parsed.rows.length / CHUNK_ROWS) === 1 ? "" : "s"} of up to ${CHUNK_ROWS} rows`,
  ];

  return (
    <PageFrame>
      <SurfaceHeader
        title="Imports"
        description="Probe first, then commit"
        meta={
          <>
            <StatPill tone="neutral" label="Runs" count={batches?.length ?? 0} />
            <StatPill
              tone="red"
              label="Runs with failures"
              count={(batches ?? []).filter((b) => b.rows_failed > 0).length}
            />
            <StatPill
              tone="gold"
              label="Mapping templates"
              count={templates?.length ?? 0}
            />
          </>
        }
        actions={
          (batches ?? []).some((b) => b.queued_at != null && b.finished_at == null) ? (
            <Tag tone="warning">a batch is still committing</Tag>
          ) : null
        }
      />

      {loadError !== null ? (
        <div className="mb-2">
          <BlushNote title="Import data did not load.">{loadError}</BlushNote>
        </div>
      ) : null}
      {error !== null ? (
        <div className="mb-2" role="alert" data-testid="imports-error">
          <BlushNote title="The last import action did not finish.">{error}</BlushNote>
        </div>
      ) : null}
      {savedNote !== null ? (
        <div className="mb-2" role="status" data-testid="imports-saved">
          <InlineVerdict tone="green">{savedNote}</InlineVerdict>
        </div>
      ) : null}

      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <WizardStepRail
          steps={steps}
          current={step - 1}
          onStep={(index) => setStep(index + 1)}
        />
        <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
          Steps {reached.filter(Boolean).length} of {steps.length} ready · step {step} on screen
        </span>
      </div>

      <section
        className="min-h-0 flex-1 overflow-auto rounded-[16px] p-4"
        style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)" }}
        aria-label="Import step"
      >
        {/* ---- 1 · Source ---- */}
        <div className={step === 1 ? "grid gap-3" : "hidden"}>
          <StepHeading
            word="Source"
            summary="What you are importing, and what happens when a row already exists."
          />
          <div className="grid gap-3 md:grid-cols-4">
            <div>
              <Label htmlFor="imp-kind">Entity</Label>
              <Select value={kind} onValueChange={(v) => setKind(v as EntityKind)}>
                <SelectTrigger id="imp-kind"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {ENTITY_KINDS.map((k) => (
                    <SelectItem key={k.value} value={k.value}>{k.label}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label htmlFor="imp-policy">Conflict policy</Label>
              <Select value={policy} onValueChange={(v) => setPolicy(v as typeof policy)}>
                <SelectTrigger id="imp-policy"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {POLICIES.map((p) => (
                    <SelectItem key={p.value} value={p.value}>{p.label}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label htmlFor="imp-source">Source system</Label>
              <Input
                id="imp-source"
                placeholder="e.g. connectwise"
                value={sourceSystem}
                onChange={(e) => setSourceSystem(e.target.value)}
              />
              <FieldHint>Maps and templates save under this name.</FieldHint>
            </div>
            <div>
              <Label htmlFor="imp-template">Mapping template</Label>
              <Select value={templateId} onValueChange={setTemplateId}>
                <SelectTrigger id="imp-template"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">None</SelectItem>
                  {kindTemplates.map((t) => (
                    <SelectItem key={t.id} value={t.id}>{t.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <FieldHint>
                Renames source headers to canonical columns before validation.
              </FieldHint>
            </div>
          </div>
          <div>
            <Label htmlFor="imp-label">Batch label</Label>
            <Input
              id="imp-label"
              placeholder="October client sync"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
            />
          </div>
          <StepNav onNext={() => setStep(2)} nextLabel="Next · upload" />
        </div>

        {/* ---- 2 · Upload ---- */}
        <div className={step === 2 ? "grid gap-3" : "hidden"}>
          <StepHeading
            word="Upload"
            summary="A CSV or JSON payload with a header row. Nothing is written until you commit."
          />
          <div className="flex flex-wrap items-center gap-2">
            <input
              ref={fileInput}
              type="file"
              accept=".csv,text/csv,application/json"
              className="hidden"
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (f !== undefined) void loadFile(f);
              }}
            />
            <Button variant="secondary" onClick={() => fileInput.current?.click()} data-testid="imports-choose-file">
              <UploadIcon size={16} /> Choose CSV file
            </Button>
            {parsed !== null ? (
              <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                {parsed.rows.length.toLocaleString()} row{parsed.rows.length === 1 ? "" : "s"},{" "}
                {parsed.headers.length} column{parsed.headers.length === 1 ? "" : "s"}
              </span>
            ) : null}
          </div>
          <div
            role="region"
            aria-label="CSV drop zone"
            onDragOver={(e) => e.preventDefault()}
            onDrop={(e) => {
              e.preventDefault();
              const f = e.dataTransfer.files?.[0];
              if (f !== undefined) void loadFile(f);
            }}
            className="rounded-[16px] border border-dashed p-6 text-center text-[13px]"
            style={{ borderColor: "var(--m-border)", color: "var(--m-text-secondary)" }}
          >
            Drop the file here, or paste its text below — the first row is the header row.
          </div>
          <textarea
            aria-label="Paste CSV"
            placeholder="source,site,owner… (or drop a file above)"
            value={csvText}
            onChange={(e) => {
              setCsvText(e.target.value);
              try {
                const p = parseCsv(e.target.value);
                setParsed(p);
                setMapping(Object.fromEntries(p.headers.map((h) => [h, h])));
              } catch {
                setParsed(null);
              }
            }}
            rows={5}
            className="w-full rounded-[12px] border bg-transparent p-2 text-[12px]"
            style={{ borderColor: "var(--m-border)" }}
          />
          <div className="flex flex-wrap items-center gap-2">
            <Button
              onClick={() => void runDryRun()}
              disabled={busy || parsed === null || parsed.rows.length === 0}
              data-testid="imports-dry-run"
            >
              {busy ? <Spinner /> : null} Run dry-run
            </Button>
            <StepNav onBack={() => setStep(1)} nextLabel="Next · mapping" onNext={() => setStep(4)} nextDisabled={parsed === null} />
          </div>
          <FieldHint>
            A dry-run writes nothing. It is the only safe way to see what the batch would do.
          </FieldHint>
          <StepNav onBack={() => setStep(1)} onNext={() => setStep(4)} nextLabel="Next · mapping" nextDisabled={parsed === null} />
        </div>

        {/* ---- 3 · Dry-run report ---- */}
        <div className={step === 3 ? "grid gap-3" : "hidden"} data-testid="imports-report-step">
          <StepHeading
            word="Dry-run report"
            summary="What the batch would do, row by row. Nothing has been written."
          />
          {report === null ? (
            <EmptyState message="No probe yet. Run a dry-run from the upload step first." />
          ) : (
            <>
              <div className="flex flex-wrap items-center gap-2">
                <StatusPill tone="success">{report.summary.ok.toLocaleString()} would be created</StatusPill>
                <StatusPill tone="neutral">{report.summary.would_update.toLocaleString()} updated</StatusPill>
                <StatusPill tone="neutral">{report.summary.would_skip.toLocaleString()} skipped</StatusPill>
                <StatusPill tone={report.summary.failed > 0 ? "warning" : "success"}>
                  {report.summary.failed.toLocaleString()} failed
                </StatusPill>
              </div>
              {report.rows.length > 0 ? (
                <Table data-testid="imports-report-table">
                  <thead>
                    <tr>
                      <Th className={HEAD_CELL_CLASS}>Row</Th>
                      <Th className={HEAD_CELL_CLASS}>Key</Th>
                      <Th className={HEAD_CELL_CLASS}>Outcome</Th>
                    </tr>
                  </thead>
                  <tbody>{report.rows.map((r) => <RowLine key={r.row_index} row={r} />)}</tbody>
                </Table>
              ) : (
                <EmptyState message="No rows in this payload." />
              )}
              <StepNav
                onBack={() => setStep(2)}
                onNext={() => setStep(4)}
                nextLabel="Next · mapping"
                nextDisabled={parsed === null}
              />
            </>
          )}
        </div>

        {/* ---- 4 · Field mapping ---- */}
        <div className={step === 4 ? "grid gap-3" : "hidden"}>
          <StepHeading
            word="Mapping"
            summary="Rename source columns to the canonical Marula fields, and save the mapping per source system."
          />
          {parsed === null ? (
            <EmptyState message="Mapping needs a payload. Upload one first." />
          ) : (
            <>
              <div className="grid gap-2 md:grid-cols-2">
                {parsed.headers.map((h) => (
                  <div key={h} className="flex items-center gap-2">
                    <span className="w-40 truncate text-[13px]" title={h}>{h}</span>
                    <span style={{ color: "var(--m-text-muted)" }}>→</span>
                    <Input
                      aria-label={`Canonical column for ${h}`}
                      value={mapping[h] ?? h}
                      onChange={(e) => setMapping({ ...mapping, [h]: e.target.value })}
                      className="w-56"
                    />
                  </div>
                ))}
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <Input
                  aria-label="Template name"
                  placeholder="Template name (e.g. CW clients v2)"
                  value={templateName}
                  onChange={(e) => setTemplateName(e.target.value)}
                  className="w-64"
                />
                <Button
                  variant="secondary"
                  onClick={() => void saveTemplate()}
                  disabled={busy || templateName.trim() === ""}
                >
                  Save template
                </Button>
                <StepNav onBack={() => setStep(report !== null ? 3 : 2)} onNext={() => setStep(5)} nextLabel="Next · commit" />
              </div>
            </>
          )}
        </div>

        {/* ---- 5 · Commit ---- */}
        <div className={step === 5 ? "grid gap-3" : "hidden"}>
          <StepHeading
            word="Commit"
            summary="Create the batch, stage the rows, and let the worker commit them."
          />
          {parsed === null ? (
            <EmptyState message="Commit needs a payload. Upload one first." />
          ) : (
            <>
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                {Math.ceil(parsed.rows.length / CHUNK_ROWS)} chunk
                {Math.ceil(parsed.rows.length / CHUNK_ROWS) === 1 ? "" : "s"} of up to{" "}
                {CHUNK_ROWS} rows, each row idempotent, the same write path the dry-run probed.
              </p>
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  onClick={() => void createAndRun()}
                  disabled={busy}
                  data-testid="imports-commit"
                >
                  {busy ? <Spinner /> : null} Create batch and run
                </Button>
                <StepNav onBack={() => setStep(4)} />
              </div>
            </>
          )}
        </div>
      </section>

      {/* ---- run history: a record surface, so the five controls ---- */}
      <div className="mt-4 flex min-h-0 flex-col">
        <DataToolbar
          label="Run history controls"
          views={
            <ViewSwitch
              label="Run history views"
              views={[
                { id: "all", label: "All runs", hint: String(batches?.length ?? 0) },
                { id: "live", label: "Still committing" },
                { id: "failed", label: "With failures" },
                { id: "probes", label: "Probes" },
              ]}
              activeId={historyView}
              onChange={setHistoryView}
            />
          }
          filters={
            <FilterBar
              label="Run history filters"
              emptyHint="No filters"
              fields={[
                { key: "label", label: "Label", operators: ["contains"] },
                { key: "entity", label: "Entity", operators: ["eq"] },
                { key: "policy", label: "Conflict policy", operators: ["eq"] },
              ]}
              clauses={historyClauses}
              onChange={(next) => setHistoryClauses(next.map((c) => ({ ...c, op: c.op as never })))}
            />
          }
          columns={
            <ColumnChooser
              label="Run history columns"
              columns={historyColumns.map((c) => ({
                key: c.key,
                label: c.label,
                visible: c.visible,
                required: c.required,
              }))}
              onChange={(next) =>
                setHistoryColumns(
                  next.map((c) => ({
                    key: c.key as HistoryColumnKey,
                    label: c.label,
                    visible: c.visible,
                    required: c.required,
                  })),
                )
              }
            />
          }
          export={
            <ExportControl
              label="Export"
              formats={["csv"]}
              columns={shownHistoryColumns.map((c) => c.label)}
              rowCount={historyRows.length}
              filters={historyClauses}
              onExport={() => {
                downloadCsv(
                  "import-runs.csv",
                  shownHistoryColumns.map((c) => c.label),
                  historyRows.map((b) =>
                    shownHistoryColumns.map((c) => historyCell(c.key, b)),
                  ),
                );
              }}
            />
          }
        />

        <TableCard label="Run history">
          {batches === null ? (
            <SkeletonRows sentence="Loading import runs." columns={5} />
          ) : historyRows.length === 0 ? (
            (batches?.length ?? 0) === 0 ? (
              <EmptyState message="No import runs yet. Dry-run probes are recorded here too, marked as probes." />
            ) : (
              <NoMatches
                title="No runs match these filters."
                hint="Try removing one, or switch back to All runs."
                noun="runs"
                onClear={() => {
                  setHistoryClauses([]);
                  setHistoryView("all");
                }}
              />
            )
          ) : (
            <Table data-testid="imports-history-table">
              <thead>
                <tr>
                  {shownHistoryColumns.map((c) => (
                    <Th key={c.key} className={RIGHT_COLUMNS.has(c.key) ? `${HEAD_CELL_CLASS} text-right` : HEAD_CELL_CLASS}>
                      {c.label}
                    </Th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {historyRows.map((b) => {
                  const committed =
                    b.rows_created + b.rows_updated + b.rows_skipped + b.rows_failed;
                  const pct =
                    b.dry_run || b.rows_total <= 0
                      ? null
                      : Math.min(100, Math.round((committed / b.rows_total) * 100));
                  return (
                    <Tr key={b.id} style={{ height: "var(--m-density-row)" }}>
                      {shownHistoryColumns.map((c) => (
                        <Td key={c.key} className={RIGHT_COLUMNS.has(c.key) ? "text-right tabular-nums" : undefined}>
                          {c.key === "label" ? (
                            <div className="flex min-w-0 flex-col gap-1">
                              <div className="flex items-center gap-2">
                                <span className="font-semibold" style={{ color: "var(--m-text)" }}>
                                  {b.label}
                                </span>
                                {b.dry_run ? (
                                  <StatusPill tone="neutral">probe</StatusPill>
                                ) : b.queued_at != null && b.finished_at == null ? (
                                  <StatusPill tone="warning">queued</StatusPill>
                                ) : (
                                  <StatusPill tone={b.rows_failed > 0 ? "danger" : "success"}>
                                    {b.rows_failed > 0 ? `${b.rows_failed} failed` : "done"}
                                  </StatusPill>
                                )}
                              </div>
                              {pct !== null ? (
                                <div
                                  className="h-1.5 w-40 overflow-hidden rounded-full"
                                  style={{ background: "var(--m-border)" }}
                                  role="progressbar"
                                  aria-valuenow={pct}
                                  aria-valuemin={0}
                                  aria-valuemax={100}
                                  aria-label={`Import progress ${pct}%`}
                                >
                                  <div
                                    className="h-full transition-opacity duration-100"
                                    style={{
                                      width: `${pct}%`,
                                      minWidth: 4,
                                      background:
                                        b.rows_failed > 0 && committed >= b.rows_total
                                          ? "var(--m-status-danger-fg)"
                                          : "var(--m-accent)",
                                    }}
                                  />
                                </div>
                              ) : null}
                            </div>
                          ) : null}
                          {c.key !== "label" ? historyCell(c.key, b) : null}
                        </Td>
                      ))}
                    </Tr>
                  );
                })}
              </tbody>
            </Table>
          )}
        </TableCard>

        <LedgerFooter
          shown={historyRows.length}
          total={batches?.length ?? 0}
          noun="runs"
          density={<DensitySwitch value={density} onChange={applyDensity} />}
          trailing={
            <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              {stageSummary[step - 1]}
            </span>
          }
        />
      </div>
    </PageFrame>
  );
}

function StepHeading({ word, summary }: { word: string; summary: string }): React.ReactNode {
  return (
    <div className="flex flex-wrap items-baseline gap-2">
      <h2 className="font-semibold" style={{ fontSize: 18, color: "var(--m-text)" }}>
        {word}
      </h2>
      <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
        {summary}
      </p>
    </div>
  );
}

function StepNav({
  onBack,
  onNext,
  nextLabel,
  nextDisabled,
}: {
  onBack?: () => void;
  onNext?: () => void;
  nextLabel?: string;
  nextDisabled?: boolean;
}): React.ReactNode {
  // A row of its own, hugging the right edge: the step's actions belong to the
  // step, not spread across the card's full width.
  return (
    <div className="flex w-full items-center justify-end gap-2 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
      {onBack !== undefined ? (
        <Button variant="ghost" onClick={onBack}>
          Back
        </Button>
      ) : null}
      {onNext !== undefined ? (
        <Button variant="secondary" onClick={onNext} disabled={nextDisabled ?? false}>
          {nextLabel ?? "Next"}
        </Button>
      ) : null}
    </div>
  );
}

function RowLine({ row }: { row: RowReport }) {
  const outcome = row.outcome;
  if ("status" in outcome) {
    if (outcome.status === "failed") {
      return (
        <Tr>
          <Td className="tabular-nums">{row.row_index}</Td>
          <Td>{row.external_key ?? "—"}</Td>
          <Td>
            <Tag tone="danger">failed</Tag>{" "}
            <span style={{ color: "var(--m-status-danger-fg)" }}>{outcome.errors.join("; ")}</span>
          </Td>
        </Tr>
      );
    }
    if (outcome.status === "would_skip") {
      return (
        <Tr>
          <Td className="tabular-nums">{row.row_index}</Td>
          <Td>{row.external_key ?? "—"}</Td>
          <Td>
            <Tag tone="warning">skip</Tag> {outcome.reason}
          </Td>
        </Tr>
      );
    }
    return (
      <Tr>
        <Td className="tabular-nums">{row.row_index}</Td>
        <Td>{row.external_key ?? "—"}</Td>
        <Td>
          <Tag tone={outcome.status === "would_update" ? "warning" : "success"}>
            {outcome.status === "would_update" ? "update" : "create"}
          </Tag>
        </Td>
      </Tr>
    );
  }
  return null;
}
