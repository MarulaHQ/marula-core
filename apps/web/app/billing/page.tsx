// /billing — staff invoicing (Wave 9 lane 1 close-out; Wave 17 lane 5 UX pass).
//
// Purpose: the invoice ledger for the org: glanceable recovery posture
// (stat tiles over stored counts + single-currency sums only), a
// filterable invoice list (search / status / dunning), invoice-run
// history, and the run composer — org-wide billing preview over a window
// (GET /billing/preview) followed by the gated run create
// (POST /billing/invoice-runs) whose IF2 envelope reconciles the
// optimistic run row and issued invoices.
//
// Non-purpose: no invoice editing (documents are issued artifacts); per-
// invoice actions live on /billing/[id].
"use client";

import { rejectionText } from "../../lib/error-copy";
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
  Tag,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { money } from "../../lib/format";
import { dueBucket } from "../../lib/format";
import {
  dunningTone,
  invoiceStatusTone,
  perCurrencySum,
  type Tone,
} from "../../lib/money-summary";

type Invoice = components["schemas"]["Invoice"];
type InvoiceRun = components["schemas"]["InvoiceRun"];
type OrgBillingPreview = components["schemas"]["OrgBillingPreview"];
type InvoiceStatus = components["schemas"]["InvoiceStatus"];
type DunningState = components["schemas"]["DunningState"];
type InvoiceRunResponse = components["schemas"]["InvoiceRunResponse"];

const STATUSES: (InvoiceStatus | "all")[] = [
  "all",
  "draft",
  "issued",
  "paid",
  "void",
];
const DUNNING: (DunningState | "all")[] = [
  "all",
  "none",
  "reminder_sent",
  "overdue",
  "final_notice",
  "collections",
];

function isoDay(offsetDays: number): string {
  const d = new Date();
  d.setUTCHours(0, 0, 0, 0);
  d.setUTCDate(d.getUTCDate() + offsetDays);
  return d.toISOString().slice(0, 10);
}

// Dunning escalation tones (theme.md §3 semantic pairs) share one mapping
// with the detail page via lib/money-summary; paid is success-green and
// dunning escalation walks neutral → warning → danger.

export default function BillingPage() {
  const auth = useRequireAuth();
  const [invoices, setInvoices] = useState<Invoice[] | null>(null);
  const [runs, setRuns] = useState<InvoiceRun[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  const [statusFilter, setStatusFilter] = useState<InvoiceStatus | "all">("all");
  const [dunningFilter, setDunningFilter] = useState<DunningState | "all">("all");

  const [runOpen, setRunOpen] = useState(false);
  const [runFrom, setRunFrom] = useState(isoDay(-30));
  const [runTo, setRunTo] = useState(isoDay(0));
  const [dueInDays, setDueInDays] = useState("30");
  const [preview, setPreview] = useState<OrgBillingPreview | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [running, setRunning] = useState(false);
  const [runError, setRunError] = useState<string | null>(null);
  const [runNote, setRunNote] = useState<string | null>(null);
  const [candidates, setCandidates] = useState<components["schemas"]["InvoiceCandidateView"][]>([]);

  const refresh = useCallback(async () => {
    const [inv, ir, cand] = await Promise.all([
      api.api.GET("/api/v1/invoices"),
      api.api.GET("/api/v1/billing/invoice-runs"),
      api.api.GET("/api/v1/invoice-candidates"),
    ]);
    if (inv.error !== undefined || ir.error !== undefined) {
      setLoadError("Failed to load billing data.");
      return;
    }
    setInvoices(inv.data ?? []);
    setRuns(ir.data ?? []);
    if (cand.error === undefined && cand.data !== undefined) setCandidates(cand.data);
    setLoadError(null);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const filtered = useMemo(() => {
    if (invoices === null) return null;
    const q = query.trim().toLowerCase();
    return invoices.filter(
      (i) =>
        (q === "" || i.number.toLowerCase().includes(q)) &&
        (statusFilter === "all" || i.status === statusFilter) &&
        (dunningFilter === "all" || i.dunning_state === dunningFilter),
    );
  }, [invoices, statusFilter, dunningFilter, query]);

  // Posture tiles — counts over the real list; the outstanding sum only
  // materializes when the selection is single-currency (never a made-up FX sum).
  const posture = useMemo(() => {
    const list = invoices ?? [];
    const open = list.filter((i) => i.status === "issued" || i.status === "draft");
    const overdue = open.filter(
      (i) => i.dunning_state === "overdue" || i.dunning_state === "final_notice" || i.dunning_state === "collections",
    );
    const paid = list.filter((i) => i.status === "paid");
    const outstanding = perCurrencySum(
      open.filter((i) => i.status === "issued").map((i) => ({ currency: i.currency, amount: i.total_minor - i.total_paid_minor })),
    );
    return { openCount: open.length, overdueCount: overdue.length, paidCount: paid.length, outstanding };
  }, [invoices]);

  const loadPreview = async () => {
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
  };

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

  const tiles: { label: string; value: React.ReactNode; hint?: string; dataTestId?: string }[] = [
    {
      label: "Open",
      value: posture.openCount,
      hint: "draft + issued invoices",
      dataTestId: "billing-stat-open",
    },
    {
      label: "Overdue",
      value: posture.overdueCount,
      hint: "in collections posture",
      dataTestId: "billing-stat-overdue",
    },
    {
      label: "Outstanding",
      value: posture.outstanding.value,
      hint: posture.outstanding.currency ?? "issued invoices only",
      dataTestId: "billing-stat-outstanding",
    },
    {
      label: "Paid",
      value: posture.paidCount,
      hint: "settled invoices",
      dataTestId: "billing-stat-paid",
    },
  ];

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="m-rise">
          <h1 className="text-lg font-semibold">Billing</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Invoices, payment receipts, and collection posture.
          </p>
        </div>
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
            <Button className="m-rise">New invoice run</Button>
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
                  className="no-underline transition-opacity duration-100 hover:opacity-80"
                  style={{ color: "var(--m-accent)" }}
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
                    {preview.evaluations.length === 1 ? "" : "s"},{" "}
                    {money(preview.total_minor, preview.evaluations[0]?.currency ?? "USD")} total
                    {preview.evaluations.length === 0 ? " — nothing billable in this window." : "."}
                  </p>
                  <ul className="mt-2 grid gap-1">
                    {preview.evaluations.map((e) => (
                      <li key={e.contract_id} className="flex justify-between gap-4 tabular-nums">
                        <span className="font-mono">{e.contract_number}</span>
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
      </div>

      {runNote !== null ? (
        <p role="status" className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {runNote}
        </p>
      ) : null}

      {/* Recovery-posture strip — counts + single-currency sums over the real list. */}
      <div data-testid="billing-stat-strip" className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {tiles.map((t, i) => (
          <Card
            key={t.label}
            className="m-rise p-4"
            style={{ animationDelay: `${Math.min(i, 6) * 40}ms` }}
          >
            <p className="text-[11px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              {t.label}
            </p>
            <p
              className="mt-1 font-mono text-xl font-semibold tabular-nums"
              style={{ color: "var(--m-text)" }}
              data-testid={t.dataTestId}
            >
              {t.value}
            </p>
            <p className="mt-0.5 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              {t.hint}
            </p>
          </Card>
        ))}
      </div>

      <Card className="m-rise overflow-x-auto">
        <div className="flex flex-wrap items-center justify-between gap-3 p-4 pb-3">
          <h2 className="text-sm font-semibold">Invoices</h2>
          <div className="flex shrink-0 flex-wrap items-center gap-2">
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search by number…"
              aria-label="Search invoices by number"
              data-testid="billing-search"
              style={{ width: 180 }}
            />
            <Select value={statusFilter} onValueChange={(v) => setStatusFilter(v as InvoiceStatus | "all")}>
              <SelectTrigger aria-label="Filter by status" style={{ width: 150 }}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {STATUSES.map((s) => (
                  <SelectItem key={s} value={s}>
                    {s === "all" ? "All statuses" : s}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Select
              value={dunningFilter}
              onValueChange={(v) => setDunningFilter(v as DunningState | "all")}
            >
              <SelectTrigger aria-label="Filter by dunning state" style={{ width: 180 }}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {DUNNING.map((d) => (
                  <SelectItem key={d} value={d}>
                    {d === "all" ? "All dunning" : d}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
        {filtered === null ? (
          <div className="p-4">
            {loadError !== null ? (
              <p role="alert" className="text-[13px]">{loadError}</p>
            ) : (
              <Spinner />
            )}
          </div>
        ) : filtered.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No invoices match. Run an invoice over a billing window to issue the first batch." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Number</Th>
                <Th>Status</Th>
                <Th>Dunning</Th>
                <Th>Issued</Th>
                <Th>Due</Th>
                <Th className="text-right">Total</Th>
                <Th className="text-right">Paid</Th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((inv, i) => {
                const bucket = dueBucket(inv.due_on ?? null);
                const unpaid = inv.total_minor - inv.total_paid_minor;
                // Overdue is a balance posture: only issued invoices with an
                // unpaid remainder carry the due-date chip.
                const chipVisible = inv.status === "issued" && unpaid > 0;
                return (
                  <Tr key={inv.id} className="row-hover">
                    <Td>
                      <Link
                        href={`/billing/${inv.id}`}
                        className="font-mono text-[13px] underline decoration-[var(--m-border)] underline-offset-2"
                      >
                        {inv.number}
                      </Link>
                    </Td>
                    <Td>
                      <Tag tone={invoiceStatusTone(inv.status)}>{inv.status}</Tag>
                    </Td>
                    <Td>
                      {inv.dunning_state !== "none" ? (
                        <Tag tone={dunningTone(inv.dunning_state) as "warning" | "danger" | "neutral"}>{inv.dunning_state}</Tag>
                      ) : (
                        <span style={{ color: "var(--m-text-muted)" }}>—</span>
                      )}
                    </Td>
                    <Td className="text-[12px]">{inv.issued_at ?? "—"}</Td>
                    <Td>
                      {inv.due_on ?? "—"}
                      {chipVisible && bucket === "overdue" ? (
                        <Tag tone="danger" className="ml-1">
                          overdue
                        </Tag>
                      ) : chipVisible && bucket === "today" ? (
                        <Tag tone="warning" className="ml-1">
                          due today
                        </Tag>
                      ) : null}
                    </Td>
                    <Td className="text-right font-mono text-[12px] tabular-nums">{money(inv.total_minor, inv.currency)}</Td>
                    <Td className="text-right font-mono text-[12px] tabular-nums">
                      {inv.status === "issued" && unpaid > 0 ? (
                        <span style={{ color: "var(--m-text-secondary)" }}>
                          {money(inv.total_paid_minor, inv.currency)}
                          <span style={{ color: "var(--m-text-muted)" }}> · {money(unpaid, inv.currency)} left</span>
                        </span>
                      ) : (
                        money(inv.total_paid_minor, inv.currency)
                      )}
                    </Td>
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}
      </Card>

      <Card className="m-rise overflow-x-auto">
        <h2 className="p-4 pb-0 text-sm font-semibold">Invoice runs</h2>
        {runs === null ? (
          <div className="p-4">
            <Spinner />
          </div>
        ) : runs.length === 0 ? (
          <div className="p-4">
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
                  <Td className="font-mono text-[13px]">{r.number}</Td>
                  <Td className="tabular-nums">
                    {r.period_from} → {r.period_to}
                  </Td>
                  <Td className="text-right tabular-nums">{r.invoice_count}</Td>
                  <Td className="text-right font-mono text-[12px] tabular-nums">
                    {r.currency != null ? money(r.total_minor, r.currency) : "…"}
                  </Td>
                  <Td className="text-[12px]">{r.created_at !== undefined ? new Date(r.created_at).toLocaleString() : "—"}</Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>
      <Card className="m-rise overflow-x-auto">
        <h2 className="p-4 pb-0 text-sm font-semibold">Unbilled ticket candidates</h2>
        <p className="px-4 pt-1 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
          Billable cost lines and expenses waiting to be picked up by an invoice run (read-only — consumption is marked through <code className="font-mono text-[11px]">invoice_line_id</code> when a run lands).
        </p>
        {candidates.length === 0 ? (
          <div className="p-4 pt-2">
            <EmptyState message="No unbilled candidates. Ticket cost lines and expenses marked billable appear here." />
          </div>
        ) : (
          <Table>
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
                    <Tag tone={c.source === "cost" ? "neutral" : "success"}>{c.source}</Tag>
                  </Td>
                  <Td>{c.description}</Td>
                  <Td>
                    <Link href={`/tickets/${c.ticket_id}`} className="underline decoration-[var(--m-border)] underline-offset-2">
                      {c.ticket_id.slice(0, 8)}…
                    </Link>
                  </Td>
                  <Td className="text-right font-mono text-[12px] tabular-nums">{money(c.amount_minor, c.currency)}</Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>
    </div>
  );
}
