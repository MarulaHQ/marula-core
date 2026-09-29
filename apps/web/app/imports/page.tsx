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
  Card,
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
  Tag,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
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

  if (auth.status !== "authed") return null;

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-8 p-4 md:p-6">
      <div>
        <h1 className="text-lg font-semibold">Imports</h1>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Bring CSV or JSON data into Marula: dry-run first (nothing is written), then commit as
          a batch. Field mappings save per source system so the next import maps itself.
        </p>
      </div>

      {loadError !== null ? (
        <p role="alert" className="text-[13px]">{loadError}</p>
      ) : null}
      {savedNote !== null ? (
        <p role="status" className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>{savedNote}</p>
      ) : null}
      {error !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>{error}</p>
      ) : null}

      <Card>
        <h2 className="p-4 pb-2 text-sm font-semibold">1 · Source</h2>
        <div className="grid gap-3 px-4 pb-4 md:grid-cols-4">
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
            <Input id="imp-source" placeholder="e.g. connectwise" value={sourceSystem} onChange={(e) => setSourceSystem(e.target.value)} />
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
            <FieldHint>Saved per source system; renames source headers to canonical columns before validation.</FieldHint>
          </div>
        </div>
      </Card>

      <Card>
        <h2 className="p-4 pb-2 text-sm font-semibold">2 · Upload</h2>
        <div className="grid gap-3 px-4 pb-4">
          <div className="flex items-center gap-2">
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
            <Button variant="secondary" onClick={() => fileInput.current?.click()}>
              Choose CSV file
            </Button>
            {parsed !== null ? (
              <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                {parsed.rows.length} row{parsed.rows.length === 1 ? "" : "s"}, {parsed.headers.length} columns
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
            className="rounded-md border border-dashed text-center text-[12px]"
            style={{ borderColor: "var(--m-border)", padding: "12px 8px", color: "var(--m-text-secondary)" }}
          >
            …or drop a CSV file here — or paste its text below (header row required)
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
            className="w-full rounded-md border bg-transparent p-2 font-mono text-[12px]"
            style={{ borderColor: "var(--m-border)" }}
          />
          <div>
            <Button onClick={() => void runDryRun()} disabled={busy || parsed === null || parsed.rows.length === 0}>
              {busy ? <Spinner /> : null} Run dry-run
            </Button>
          </div>
        </div>
      </Card>

      {step >= 3 && report !== null ? (
        <Card>
          <h2 className="p-4 pb-2 text-sm font-semibold">3 · Dry-run report</h2>
          <p className="px-4 pb-2 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            {report.summary.ok} would be created · {report.summary.would_update} updated ·{" "}
            {report.summary.would_skip} skipped ·{" "}
            <span style={{ color: report.summary.failed > 0 ? "var(--m-status-danger-fg)" : undefined }}>
              {report.summary.failed} failed
            </span>
          </p>
          {report.rows.length > 0 ? (
            <div className="max-h-72 overflow-auto px-4 pb-4">
              <Table>
                <thead>
                  <tr>
                    <Th>Row</Th>
                    <Th>Key</Th>
                    <Th>Outcome</Th>
                  </tr>
                </thead>
                <tbody>
                  {report.rows.map((r) => (
                    <RowLine key={r.row_index} row={r} />
                  ))}
                </tbody>
              </Table>
            </div>
          ) : (
            <div className="px-4 pb-4">
              <EmptyState message="No rows in this payload." />
            </div>
          )}
        </Card>
      ) : null}

      {step >= 3 && parsed !== null ? (
        <Card>
          <h2 className="p-4 pb-2 text-sm font-semibold">4 · Field mapping template</h2>
          <p className="px-4 pb-2 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Map source columns to canonical Marula fields (typed free-form; the next dry-run
            validates targets against the field catalog). Save per source system to reuse.
          </p>
          <div className="grid gap-2 px-4 pb-4 md:grid-cols-2">
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
          <div className="flex flex-wrap items-center gap-2 px-4 pb-4">
            <Input
              aria-label="Template name"
              placeholder="Template name (e.g. CW clients v2)"
              value={templateName}
              onChange={(e) => setTemplateName(e.target.value)}
              className="w-64"
            />
            <Button variant="secondary" onClick={() => void saveTemplate()} disabled={busy || templateName.trim() === ""}>
              Save template
            </Button>
          </div>
        </Card>
      ) : null}

      {step >= 3 && parsed !== null ? (
        <Card>
          <h2 className="p-4 pb-2 text-sm font-semibold">5 · Commit</h2>
          <p className="px-4 pb-2 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Creates the batch, stages {Math.ceil(parsed.rows.length / CHUNK_ROWS)} chunk
            {Math.ceil(parsed.rows.length / CHUNK_ROWS) === 1 ? "" : "s"} of up to {CHUNK_ROWS} rows, and queues the
            worker to commit them (idempotent per row, same write path as dry-run).
          </p>
          <div className="px-4 pb-4">
            <Button onClick={() => void createAndRun()} disabled={busy}>
              {busy ? <Spinner /> : null} Create batch &amp; run
            </Button>
          </div>
        </Card>
      ) : null}

      <Card>
        <h2 className="p-4 pb-2 text-sm font-semibold">Run history</h2>
        {batches === null ? (
          <div className="p-4"><Spinner /></div>
        ) : batches.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No import runs yet. Dry-run probes are recorded here too, marked as probes." />
          </div>
        ) : (
          <div className="max-h-96 overflow-auto px-4 pb-4">
            <Table>
              <thead>
                <tr>
                  <Th>Label</Th>
                  <Th>Entity</Th>
                  <Th>Kind</Th>
                  <Th className="text-right">Rows</Th>
                  <Th className="text-right">Created</Th>
                  <Th className="text-right">Updated</Th>
                  <Th className="text-right">Skipped</Th>
                  <Th className="text-right">Failed</Th>
                  <Th>Finished</Th>
                </tr>
              </thead>
              <tbody>
                {batches.map((b) => {
                  const committed =
                    b.rows_created + b.rows_updated + b.rows_skipped + b.rows_failed;
                  const pct =
                    b.dry_run || b.rows_total <= 0
                      ? null
                      : Math.min(100, Math.round((committed / b.rows_total) * 100));
                  return (
                    <Tr key={b.id}>
                      <Td>
                        <div className="flex items-center gap-2">
                          <span className="font-medium">{b.label}</span>
                          {b.dry_run ? (
                            <Tag tone="neutral">probe</Tag>
                          ) : b.queued_at != null && b.finished_at == null ? (
                            <Tag tone="warning">queued</Tag>
                          ) : (
                            <Tag tone={b.rows_failed > 0 ? "danger" : "success"}>
                              {b.rows_failed > 0 ? `${b.rows_failed} failed` : "done"}
                            </Tag>
                          )}
                        </div>
                        {pct !== null ? (
                          <div
                            className="mt-1 h-1.5 w-40 overflow-hidden rounded-full"
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
                      </Td>
                      <Td>{b.entity_kind}</Td>
                      <Td>{b.conflict_policy}</Td>
                      <Td className="text-right tabular-nums">{b.rows_total}</Td>
                      <Td className="text-right tabular-nums">{b.rows_created}</Td>
                      <Td className="text-right tabular-nums">{b.rows_updated}</Td>
                      <Td className="text-right tabular-nums">{b.rows_skipped}</Td>
                      <Td className="text-right tabular-nums">{b.rows_failed}</Td>
                      <Td>
                        {b.finished_at != null ? new Date(b.finished_at).toLocaleString() : "—"}
                      </Td>
                    </Tr>
                  );
                })}
              </tbody>
            </Table>
          </div>
        )}
      </Card>
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
