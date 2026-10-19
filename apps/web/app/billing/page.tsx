// /billing — the invoice ledger (Wave 9 lane 1 close-out; Wave 17 lane 5 UX
// pass; Wave 41 lane 8 rebuilt to the Marula surface pack's approvals
// reference, §9 "the rest of the money cluster derives").
//
// Purpose: every invoice the org has issued, as a data surface: the view
// switch, the filter chips and Columns / Filter / Export on one toolbar row,
// per-user columns that persist, an ageing pill on anything still owed, and the
// run that issues a batch — because a ledger whose numbers cannot be traced
// back to the window that produced them is not a ledger anyone trusts.
//
// The status colours mean one thing (pack §7): Sent is a gold tint, Paid is
// green with a check, and an invoice past its date is BLUSH with the days in
// words. Red stays with a breached SLA.
//
// Non-purpose: no editing of an issued document (the run is the only author);
// the run history and the unbilled candidates sit under the ledger because they
// are about producing the next one, not about reading this one.
"use client";

import { rejectionText } from "../../lib/error-copy";
import {
  Badge,
  Button,
  Card,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  ExportControl,
  FieldHint,
  FilterChips,
  FilterTrigger,
  Input,
  Kbd,
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
  ViewSwitch,
  applyDensity,
  type FilterClause,
} from "@marula-core/ui";
import { LocalTime } from "../../components/local-time";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";

import { api, useRequireAuth } from "../../lib/api";
import { downloadRowsCsv } from "../../lib/csv-download";
import { useDataSurfacePrefs } from "../../lib/data-surface-prefs";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { money } from "../../lib/format";
import { perCurrencySum } from "../../lib/money-summary";
import { ageingBucket, ageingLabel, invoicePill } from "../../lib/money-ui";
import {
  AgeingPill,
  MoneyCard,
  MoneyHeader,
  MoneyKv,
  MoneyPillTag,
  MoneyStatPill,
} from "../../components/money";

type Invoice = components["schemas"]["Invoice"];
type InvoiceRun = components["schemas"]["InvoiceRun"];
type InvoiceStatus = components["schemas"]["InvoiceStatus"];
type InvoiceCandidate = components["schemas"]["InvoiceCandidateView"];
type Client = components["schemas"]["Client"];
type OrgBillingPreview = components["schemas"]["OrgBillingPreview"];
type InvoiceRunResponse = components["schemas"]["InvoiceRunResponse"];

const STATUSES: readonly InvoiceStatus[] = ["draft", "issued", "paid", "void"];

interface ColumnDef {
  key: string;
  label: string;
  required?: boolean;
}

/** The ledger's columns, in the pack's order: invoice, client, status, due,
 *  then the money. Ageing rides the due column, because that is the date it is
 *  a fact about. */
const COLUMNS: readonly ColumnDef[] = [
  { key: "invoice", label: "Invoice", required: true },
  { key: "client", label: "Client" },
  { key: "status", label: "Status" },
  { key: "due", label: "Due" },
  { key: "amount", label: "Amount" },
  { key: "paid", label: "Paid" },
  { key: "balance", label: "Balance" },
];

const FILTER_FIELDS = [
  { key: "number", label: "Invoice number" },
  { key: "client", label: "Client" },
  { key: "status", label: "Status" },
  { key: "due", label: "Due date" },
];

type View = "all" | "open" | "overdue" | "paid";

export default function BillingPage() {
  const auth = useRequireAuth();
  const prefs = useDataSurfacePrefs();

  const [invoices, setInvoices] = useState<Invoice[] | null>(null);
  const [runs, setRuns] = useState<InvoiceRun[] | null>(null);
  const [candidates, setCandidates] = useState<InvoiceCandidate[]>([]);
  const [clients, setClients] = useState<Client[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [view, setView] = useState<View>("all");
  const [filterOpen, setFilterOpen] = useState(false);
  const [clauses, setClauses] = useState<FilterClause[]>([]);

  const [runOpen, setRunOpen] = useState(false);
  const [runFrom, setRunFrom] = useState("");
  const [runTo, setRunTo] = useState("");
  const [dueInDays, setDueInDays] = useState("30");
  const [preview, setPreview] = useState<OrgBillingPreview | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [running, setRunning] = useState(false);
  const [runNote, setRunNote] = useState<string | null>(null);
  const [runError, setRunError] = useState<string | null>(null);

  const savedColumns = useMemo(() => prefs.view("billing").columns, [prefs]);
  const [columns, setColumns] = useState(() =>
    COLUMNS.map((c) => {
      const stored = savedColumns.find((s) => s.key === c.key);
      return {
        key: c.key,
        label: c.label,
        visible: stored?.visible ?? true,
        required: c.required,
      };
    }),
  );
  const [order, setOrder] = useState<string[]>(() => {
    const known = savedColumns
      .map((c) => c.key)
      .filter((k) => COLUMNS.some((c) => c.key === k));
    return [...known, ...COLUMNS.map((c) => c.key).filter((k) => !known.includes(k))];
  });
  useEffect(() => {
    if (!prefs.ready) return;
    setColumns(
      COLUMNS.map((c) => {
        const stored = savedColumns.find((s) => s.key === c.key);
        return {
          key: c.key,
          label: c.label,
          visible: stored?.visible ?? true,
          required: c.required,
        };
      }),
    );
    const known = savedColumns
      .map((c) => c.key)
      .filter((k) => COLUMNS.some((c) => c.key === k));
    setOrder([...known, ...COLUMNS.map((c) => c.key).filter((k) => !known.includes(k))]);
  }, [prefs.ready, savedColumns]);

  const refresh = useCallback(async () => {
    const today = new Date();
    const monthStart = new Date(Date.UTC(today.getUTCFullYear(), today.getUTCMonth(), 1))
      .toISOString()
      .slice(0, 10);
    if (runFrom === "") setRunFrom(monthStart);
    if (runTo === "") setRunFrom(monthStart);
    setLoadError(null);
    try {
      const [invoiceRes, runRes, candidateRes, clientRes] = await Promise.all([
        api.api.GET("/api/v1/invoices"),
        api.api.GET("/api/v1/billing/invoice-runs"),
        api.api.GET("/api/v1/invoice-candidates"),
        api.api.GET("/api/v1/clients"),
      ]);
      if (invoiceRes.data === undefined) {
        setLoadError("Could not load the invoices — try reloading.");
        return;
      }
      setInvoices(invoiceRes.data ?? []);
      setRuns(runRes.data ?? []);
      setCandidates(candidateRes.data ?? []);
      setClients(clientRes.data ?? []);
    } catch {
      setLoadError("Could not load the invoices — try reloading.");
    }
  }, [runFrom, runTo]);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const clientName = useCallback(
    (id: string): string => clients.find((c) => c.id === id)?.name ?? "—",
    [clients],
  );

  const withAging = useMemo(
    () =>
      (invoices ?? []).map((inv) => {
        const unpaid = inv.total_minor - inv.total_paid_minor;
        const bucket = ageingBucket({
          status: inv.status,
          dueOn: inv.due_on,
          unpaidMinor: unpaid,
        });
        return { inv, unpaid, bucket, late: ageingLabel(inv.due_on, bucket) };
      }),
    [invoices],
  );

  const posture = useMemo(() => {
    const open = withAging.filter((row) => row.inv.status === "issued" && row.unpaid > 0);
    return {
      open: open.length,
      overdue: open.filter((row) => row.bucket === "overdue").length,
      paid: withAging.filter((row) => row.inv.status === "paid").length,
      outstanding: perCurrencySum(
        open.map((row) => ({ currency: row.inv.currency, amount: row.unpaid })),
      ),
    };
  }, [withAging]);

  const viewRows = useMemo(() => {
    switch (view) {
      case "open":
        return withAging.filter((row) => row.inv.status !== "paid" && row.inv.status !== "void");
      case "overdue":
        return withAging.filter((row) => row.bucket === "overdue");
      case "paid":
        return withAging.filter((row) => row.inv.status === "paid");
      default:
        return withAging;
    }
  }, [view, withAging]);

  const filtered = useMemo(() => {
    if (clauses.length === 0) return viewRows;
    return viewRows.filter(({ inv }) => {
      const fields: Record<string, string> = {
        number: inv.number,
        client: clientName(inv.client_id),
        status: inv.status,
        due: inv.due_on ?? "",
      };
      for (const clause of clauses) {
        const raw = (fields[clause.field] ?? "").toLowerCase();
        const has = raw.includes((clause.value ?? "").toLowerCase());
        if (clause.op === "eq" && !has) return false;
        if (clause.op === "is_set" && raw === "") return false;
        if (clause.op === "is_not_set" && raw !== "") return false;
      }
      return true;
    });
  }, [viewRows, clauses, clientName]);

  const visibleColumns = useMemo(
    () =>
      order
        .map((key) => columns.find((c) => c.key === key))
        .filter((c): c is (typeof columns)[number] => c !== undefined && c.visible),
    [order, columns],
  );

  const cell = (key: string, row: (typeof withAging)[number]) => {
    const { inv, unpaid, bucket, late } = row;
    switch (key) {
      case "invoice":
        return (
          <Link
            href={`/billing/${inv.id}`}
            className="text-[13px]"
            style={{ textDecoration: "none" }}
          >
            {inv.number}
          </Link>
        );
      case "client":
        return (
          <Link
            href={`/clients/${inv.client_id}`}
            className="text-[13px]"
            style={{ color: "var(--m-accent-text)", textDecoration: "none" }}
          >
            {clientName(inv.client_id)}
          </Link>
        );
      case "status":
        return (
          <MoneyPillTag
            pill={invoicePill(inv.status, inv.total_paid_minor, inv.total_minor)}
            testId={`billing-status-${inv.number}`}
          />
        );
      case "due":
        return (
          <span style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
            <span className="tabular-nums text-[12px]">{inv.due_on}</span>
            <AgeingPill bucket={bucket} label={late} testId={`billing-ageing-${inv.number}`} />
          </span>
        );
      case "amount":
        return (
 <span className="text-right text-[13px] tabular-nums" style={{ display: "block" }}>
            {money(inv.total_minor, inv.currency)}
          </span>
        );
      case "paid":
        return (
 <span className="text-right text-[13px] tabular-nums" style={{ display: "block" }}>
            {money(inv.total_paid_minor, inv.currency)}
          </span>
        );
      case "balance":
        return (
          <span
 className="text-right text-[13px] font-semibold tabular-nums"
            style={{
              display: "block",
              color: bucket === "overdue" ? "var(--m-status-due-fg)" : "var(--m-text)",
            }}
          >
            {money(unpaid, inv.currency)}
          </span>
        );
      default:
        return null;
    }
  };

  const exportRows = useCallback(() => {
    const header = visibleColumns.map((c) => c.label);
    const body = filtered.map((row) => {
      const { inv, unpaid } = row;
      const values: Record<string, string> = {
        invoice: inv.number,
        client: clientName(inv.client_id),
        status: inv.status,
        due: inv.due_on ?? "",
        amount: money(inv.total_minor, inv.currency),
        paid: money(inv.total_paid_minor, inv.currency),
        balance: money(unpaid, inv.currency),
      };
      return visibleColumns.map((c) => values[c.key] ?? "");
    });
    downloadRowsCsv("invoices.csv", header, body);
  }, [visibleColumns, filtered, clientName]);

  const loadPreview = useCallback(async () => {
    setPreviewing(true);
    setRunError(null);
    const { data, error } = await api.api.GET("/api/v1/billing/preview", {
      params: { query: { from: runFrom, to: runTo } },
    });
    setPreviewing(false);
    if (error !== undefined || data === undefined) {
      setPreview(null);
      setRunError("Preview failed — check the window and try again.");
      return;
    }
    setPreview(data);
  }, [runFrom, runTo]);

  const createRun = async () => {
    if (preview === null) return;
    setRunning(true);
    setRunError(null);
    const tempId = `pending-run:${runFrom}:${runTo}`;
    // Optimistic run row (client-side only until the envelope lands).
    const optimisticRun: InvoiceRun = {
      id: tempId,
      org_id: "",
      number: "…",
      period_from: runFrom,
      period_to: runTo,
      due_in_days: Number(dueInDays) || 30,
      invoice_count: preview.evaluations.length,
      total_minor: preview.total_minor,
      currency: null,
      created_at: new Date().toISOString(),
      created_by: null,
    };
    setRuns((prev) => [optimisticRun, ...(prev ?? [])]);
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateInvoiceRunRequest"],
      InvoiceRunResponse
    >({
      describe: "Create invoice run",
      entityId: tempId,
      optimistic: () => {},
      revert: () => setRuns((prev) => (prev ?? []).filter((r) => r.id !== tempId)),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        from: runFrom,
        to: runTo,
        due_in_days: Number(dueInDays) || null,
      }),
      request: (client, body) =>
        client.POST("/api/v1/billing/invoice-runs", { body }),
      versions: (response) => [
        [response.run.id, response.mutation.version],
        ...response.invoices.map((i) => [i.invoice.id, response.mutation.version] as [string, number]),
      ],
    });
    setRunning(false);
    if (!result.ok) {
      setRunError(rejectionText(result, "The run was rejected. Nothing was issued."));
      return;
    }
    setRunOpen(false);
    setPreview(null);
    setRunNote("Run created. Invoices are in the list below.");
    void refresh();
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="m-money-page mx-auto w-full max-w-[1400px] p-4 md:p-6">
      <MoneyHeader
        title="Invoices"
        stats={
          <>
            <MoneyStatPill
              label="Open"
              count={posture.open}
              tone="gold"
              pressed={view === "open"}
              onClick={() => setView(view === "open" ? "all" : "open")}
              testId="billing-stat-open"
            />
            <MoneyStatPill
              label="Overdue"
              count={posture.overdue}
              tone="blush"
              pressed={view === "overdue"}
              onClick={() => setView(view === "overdue" ? "all" : "overdue")}
              testId="billing-stat-overdue"
            />
            <MoneyStatPill
              label="Paid"
              count={posture.paid}
              tone="bark"
              pressed={view === "paid"}
              onClick={() => setView(view === "paid" ? "all" : "paid")}
              testId="billing-stat-paid"
            />
          </>
        }
        actions={
          <Dialog
            open={runOpen}
            onOpenChange={(open) => {
              setRunOpen(open);
              if (!open) {
                setPreview(null);
                setRunError(null);
              }
            }}
          >
            <DialogTrigger asChild>
              <Button>New invoice run</Button>
            </DialogTrigger>
            <DialogContent
              title="New invoice run"
              description="Preview every billable contract over the window, then issue the invoices."
            >
              <div className="grid gap-3 p-4">
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <Label htmlFor="run-from">From</Label>
                    <Input
                      id="run-from"
                      type="date"
                      value={runFrom}
                      onChange={(e) => setRunFrom(e.target.value)}
                    />
                  </div>
                  <div>
                    <Label htmlFor="run-to">To</Label>
                    <Input
                      id="run-to"
                      type="date"
                      value={runTo}
                      onChange={(e) => setRunTo(e.target.value)}
                    />
                  </div>
                </div>
                <div>
                  <Label htmlFor="run-due">Payment terms (days)</Label>
                  <Input
                    id="run-due"
                    type="number"
                    min={0}
                    value={dueInDays}
                    onChange={(e) => setDueInDays(e.target.value)}
                  />
                  <FieldHint>Due dates on the issued invoices.</FieldHint>
                </div>
                {/* Wave 39 lane 2: the ritual this composer used to skip.
                    The verification workspace is where a period gets read,
                    dispositioned, and signed off before anyone gets here.
                    It reads as guidance here, ABOVE the action row: tucked
                    under a full-width Preview button it looked like that
                    button's caption, so nobody read it. */}
                <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                  <Link
                    href={`/billing/verification?from=${runFrom}&to=${runTo}`}
                    className="m-money-linkbtn"
                    data-testid="run-open-verification"
                  >
                    Verify this window first
                  </Link>{" "}
                  — every entry, its disposition, and the close sign-off live there.
                </p>
                <Button variant="secondary" onClick={() => void loadPreview()} disabled={previewing}>
                  {previewing ? <Spinner /> : null} Preview
                </Button>
                {preview !== null ? (
                  <div
                    className="rounded-[var(--m-radius-control)] border p-3 text-[13px]"
                    style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)" }}
                  >
                    <p>
                      {preview.evaluations.length} billable contract
                      {preview.evaluations.length === 1 ? "" : "s"}{" "}
                      {money(preview.total_minor, preview.evaluations[0]?.currency ?? "USD")} total
                      {preview.evaluations.length === 0 ? " — nothing billable in this window." : "."}
                    </p>
                    <ul className="mt-2 grid gap-1">
                      {preview.evaluations.map((e) => (
                        <li key={e.contract_id} className="flex justify-between gap-4 tabular-nums">
                          <span className="">{e.contract_number}</span>
                          <span style={{ color: "var(--m-text-secondary)" }}>
                            {money(e.total_minor, e.currency)}
                          </span>
                        </li>
                      ))}
                    </ul>
                  </div>
                ) : null}
                {runError !== null ? (
                  <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                    {runError}
                  </p>
                ) : null}
                <DialogActions>
                  <Button variant="ghost" onClick={() => setRunOpen(false)}>
                    Cancel
                  </Button>
                  <Button
                    onClick={() => void createRun()}
                    disabled={running || preview === null || preview.evaluations.length === 0}
                  >
                    {running ? <Spinner /> : null} Issue invoices
                  </Button>
                </DialogActions>
              </div>
            </DialogContent>
          </Dialog>
        }
      />

      <DataToolbar
        label="Invoice ledger controls"
        views={
          <ViewSwitch
            label="Views"
            activeId={view}
            onChange={(id) => setView(id as View)}
            views={[
              { id: "all", label: "All", hint: String(withAging.length) },
              { id: "open", label: "Open", hint: String(posture.open) },
              { id: "overdue", label: "Overdue", hint: String(posture.overdue) },
              { id: "paid", label: "Paid", hint: String(posture.paid) },
            ]}
          />
        }
        chips={
          <FilterChips
            fields={FILTER_FIELDS}
            clauses={clauses}
            onChange={(next) => {
              setClauses(next);
              void prefs.save("billing", {
                columns: columns.map((c) => ({ key: c.key, visible: c.visible })),
                filters: next,
              });
            }}
            onAdd={() => setFilterOpen(true)}
          />
        }
        filter={
          <FilterTrigger
            fields={FILTER_FIELDS}
            clauses={clauses}
            onChange={(next) => {
              setClauses(next);
              void prefs.save("billing", {
                columns: columns.map((c) => ({ key: c.key, visible: c.visible })),
                filters: next,
              });
            }}
            open={filterOpen}
            onOpenChange={setFilterOpen}
          />
        }
        columns={
          <ColumnChooser
            columns={columns.map((c) => ({
              key: c.key,
              label: c.label,
              visible: c.visible,
              required: c.required,
            }))}
            onChange={(next) => {
              const merged = next.map((n) => {
                const def = COLUMNS.find((d) => d.key === n.key);
                return {
                  key: n.key,
                  label: def?.label ?? n.key,
                  visible: n.visible,
                  required: def?.required,
                };
              });
              setColumns(merged);
              setOrder(merged.map((c) => c.key));
              void prefs.save("billing", {
                columns: merged.map((c) => ({ key: c.key, visible: c.visible })),
                filters: clauses,
              });
            }}
          />
        }
        export={
          <ExportControl
            columns={visibleColumns.map((c) => c.label)}
            rowCount={filtered.length}
            filters={clauses}
            onExport={exportRows}
            formats={["csv"]}
          />
        }
        trailing={
          <span style={{ fontSize: 13, color: "var(--m-text-secondary)" }}>
            Outstanding {posture.outstanding.value}{" "}
            <span style={{ color: "var(--m-text-muted)" }}>
              {posture.outstanding.currency ?? "issued invoices only"}
            </span>
          </span>
        }
      />

      {runNote !== null ? (
        <p role="status" className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {runNote}
        </p>
      ) : null}
      {loadError !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {loadError}
        </p>
      ) : null}

      <MoneyCard testId="billing-ledger">
        <div className="m-money-bar">
          <span>
            <b>{filtered.length}</b> {view === "all" ? "invoices" : "in this view"} ·{" "}
            <b>{posture.outstanding.value}</b> outstanding
          </span>
        </div>
        {invoices === null ? (
          <div className="p-4">
            <Spinner />
          </div>
        ) : filtered.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No invoices match. Run an invoice over a billing window to issue the first batch." />
          </div>
        ) : (
          <div style={{ overflowX: "auto" }}>
            <Table>
              <thead>
                <tr>
                  {visibleColumns.map((column) => (
                    <Th
                      key={column.key}
                      columnKey={column.key}
                      className={
                        column.key === "amount" || column.key === "paid" || column.key === "balance"
                          ? "text-right"
                          : undefined
                      }
                    >
                      {column.label}
                    </Th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {filtered.map((row) => (
                  <Tr key={row.inv.id} className="row-hover">
                    {visibleColumns.map((column) => (
                      <Td
                        key={column.key}
                        className={
                          column.key === "amount" || column.key === "paid" || column.key === "balance"
                            ? "text-right"
                            : undefined
                        }
                      >
                        {cell(column.key, row)}
                      </Td>
                    ))}
                  </Tr>
                ))}
              </tbody>
            </Table>
          </div>
        )}
      </MoneyCard>

      <footer className="m-money-foot">
        <span className="m-money-foot-sp" />
        <DensitySwitch
          value={prefs.density}
          onChange={(step) => {
            applyDensity(step);
            void prefs.setDensity(step);
          }}
        />
        <Kbd>N</Kbd>
        <span>new run</span>
      </footer>

      <div className="m-money-inv" style={{ marginTop: 4 }}>
        <MoneyCard title="Invoice runs" testId="billing-runs">
          <div style={{ padding: "10px 18px 14px" }}>
            <MoneyKv label="Runs">{runs === null ? "…" : runs.length}</MoneyKv>
          </div>
          {runs === null ? (
            <div className="p-4">
              <Spinner />
            </div>
          ) : runs.length === 0 ? (
            <div className="p-4 pt-2">
              <EmptyState message="No runs yet. An invoice run evaluates every billable contract over one window and issues the invoices in a single batch." />
            </div>
          ) : (
            <Table>
              <thead>
                <tr>
                  <Th>Run</Th>
                  <Th>Window</Th>
                  <Th className="text-right">Invoices</Th>
                  <Th className="text-right">Total</Th>
                  <Th>Created</Th>
                </tr>
              </thead>
              <tbody>
                {runs.map((r) => (
                  <Tr key={r.id} className="row-hover">
                    <Td className="text-[13px]">{r.number}</Td>
                    <Td className="tabular-nums">
                      {r.period_from} → {r.period_to}
                    </Td>
                    <Td className="text-right tabular-nums">{r.invoice_count}</Td>
 <Td className="text-right text-[12px] tabular-nums">
                      {r.currency != null ? money(r.total_minor, r.currency) : "…"}
                    </Td>
                    <Td className="text-[12px]">
                      {r.created_at !== undefined ? <LocalTime value={r.created_at} kind="datetime" /> : "—"}
                    </Td>
                  </Tr>
                ))}
              </tbody>
            </Table>
          )}
        </MoneyCard>

        <MoneyCard title="Unbilled candidates" testId="billing-candidates">
          <div style={{ padding: "10px 18px 0" }}>
            <p className="text-[12px]" style={{ color: "var(--m-text-secondary)", margin: 0 }}>
              Billable cost lines and expenses waiting to be picked up by a run (read-only —
              consumption is marked when a run lands).
            </p>
          </div>
          {candidates.length === 0 ? (
            <div className="p-4 pt-2">
              <EmptyState message="No unbilled candidates. Ticket cost lines and expenses marked billable appear here." />
            </div>
          ) : (
            /* The ticket title is one nowrap line with an ellipsis, so an auto
               table sizes that column to the whole title and the card's
               two-column grid (1.7fr / 1fr) cannot make it fit: the measured
               137px and 78px overhangs were this table, not the ledger. Fixed
               layout with stated shares lets the ellipsis do its job. */
            <div className="m-money-scroll">
              <Table>
                <colgroup>
                  <col style={{ width: "20%" }} />
                  <col style={{ width: "32%" }} />
                  <col style={{ width: "27%" }} />
                  <col style={{ width: "21%" }} />
                </colgroup>
                <thead>
                  <tr>
                    <Th>Source</Th>
                    <Th>Description</Th>
                    <Th>Ticket</Th>
                    <Th className="text-right">Amount</Th>
                  </tr>
                </thead>
                <tbody>
                  {candidates.map((c) => (
                    <Tr key={c.line_id} className="row-hover">
                      <Td>
                        <Badge tone={c.source === "cost" ? "neutral" : "success"}>{c.source}</Badge>
                      </Td>
                      <Td>{c.description}</Td>
                      <Td>
                        {/* The ticket named for a human reader: `#18402 Cable
                            drop`, not an id prefix. */}
                        <Link
                          href={`/tickets/${c.ticket_id}`}
                          title={c.ticket_title}
                          style={{
                            color: "var(--m-accent-text)",
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            whiteSpace: "nowrap",
                            display: "block",
                          }}
                        >
                          #{c.ticket_number} {c.ticket_title}
                        </Link>
                      </Td>
                      <Td className="text-right text-[12px] tabular-nums">
                        {money(c.amount_minor, c.currency)}
                      </Td>
                    </Tr>
                  ))}
                </tbody>
              </Table>
            </div>
          )}
        </MoneyCard>
      </div>
    </div>
  );
}
