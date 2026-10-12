// /billing/credit-notes — credit-note ledger (Wave 11 lane 10 close-out of
// the p11-backend-gaps credit-note REST, migration 0058).
//
// Purpose: issue / apply / refund / void credit notes against invoices,
// with the honest lifecycle: only `issued` and `void` are stored states —
// applied / partially applied / fully applied are DERIVED from the
// counters (applied_minor, refunded_minor, remaining). The detail drawer
// shows the application ledger (GET /{id}). Actions are owner/admin
// commercial actions (`financial:read` gate); mutations ride the
// instant-first envelope and reconcile from the typed responses.
// Rejections (insufficient remaining, not payable, has allocations,
// version conflicts) render inline with their typed reason.
// Non-purpose: no external payment-provider refund — the `none` provider
// keeps every receipt a ledger entry.
"use client";

import { rejectionText } from "../../../lib/error-copy";
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
  EmptyState,
  ExportControl,
  FilterBar,
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
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { money } from "../../../lib/format";
import { perCurrencySum } from "../../../lib/money-summary";
import { downloadRowsCsv } from "../../../lib/csv-download";
import { useDataSurfacePrefs } from "../../../lib/data-surface-prefs";
import {
  MoneyCard,
  MoneyHeader,
  MoneyPillTag,
  MoneyStatPill,
} from "../../../components/money";

type CreditNote = components["schemas"]["CreditNote"];
type CreditNoteDetail = components["schemas"]["CreditNoteDetail"];
type Client = components["schemas"]["Client"];
type Invoice = components["schemas"]["Invoice"];

const VIEW_FINANCIALS = "financial:read";

/** Lifecycle is derived, never stored (only issued | void exist). */
function lifecycleOf(note: CreditNote): { label: string; tone: "success" | "warning" | "danger" | "neutral" } {
  if (note.status === "void") return { label: "void", tone: "danger" };
  const remaining = note.total_minor - note.applied_minor - note.refunded_minor;
  if (remaining <= 0) return { label: "fully used", tone: "neutral" };
  if (note.applied_minor > 0 || note.refunded_minor > 0) return { label: "partially used", tone: "warning" };
  return { label: "open credit", tone: "success" };
}

interface CreditColumnDef {
  key: string;
  label: string;
  required?: boolean;
}

/** The credit ledger's columns, in the pack's order (pack §9: a credit note is
 *  "a ledger, and a detail that links back to the invoice it credits"). */
const CREDIT_COLUMNS: readonly CreditColumnDef[] = [
  { key: "number", label: "Number", required: true },
  { key: "client", label: "Client" },
  { key: "lifecycle", label: "Lifecycle" },
  { key: "invoice", label: "Against invoice" },
  { key: "total", label: "Total" },
  { key: "usage", label: "Usage" },
  { key: "remaining", label: "Remaining" },
];

const CREDIT_FILTER_FIELDS = [
  { key: "number", label: "Number" },
  { key: "client", label: "Client" },
  { key: "invoice", label: "Against invoice" },
];

export default function CreditNotesPage() {
  return (
    <Suspense fallback={null}>
      <CreditNotesInner />
    </Suspense>
  );
}

function CreditNotesInner() {
  const auth = useRequireAuth();
  const params = useSearchParams();
  const prefillClient = params.get("client") ?? "";
  const prefillInvoice = params.get("invoice") ?? "";
  const canSee = (auth.me?.permissions ?? []).includes(VIEW_FINANCIALS) ?? false;

  const [notes, setNotes] = useState<CreditNote[] | null>(null);
  const [clients, setClients] = useState<Client[]>([]);
  const [invoices, setInvoices] = useState<Invoice[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [clientFilter, setClientFilter] = useState<string>("all");
  const [lifecycle, setLifecycle] = useState<"all" | "open" | "used" | "void">("all");
  const [clauses, setClauses] = useState<FilterClause[]>([]);
  const prefs = useDataSurfacePrefs();
  const savedColumns = useMemo(() => prefs.view("credit-notes").columns, [prefs]);
  const [columns, setColumns] = useState(() =>
    CREDIT_COLUMNS.map((c) => {
      const stored = savedColumns.find((x) => x.key === c.key);
      return {
        key: c.key,
        label: c.label,
        visible: stored?.visible ?? true,
        required: c.required,
      };
    }),
  );
  useEffect(() => {
    if (!prefs.ready) return;
    setColumns(
      CREDIT_COLUMNS.map((c) => {
        const stored = savedColumns.find((x) => x.key === c.key);
        return {
          key: c.key,
          label: c.label,
          visible: stored?.visible ?? true,
          required: c.required,
        };
      }),
    );
  }, [prefs.ready, savedColumns]);
  const shownColumns = columns.filter((c) => c.visible);

  // Issue dialog
  const [issueOpen, setIssueOpen] = useState(false);
  const [issueClientId, setIssueClientId] = useState("");
  const [issueInvoiceId, setIssueInvoiceId] = useState<string>("none");
  const [issueTotal, setIssueTotal] = useState("");
  const [issueCurrency, setIssueCurrency] = useState("USD");
  const [issueReason, setIssueReason] = useState("");
  const [issueBusy, setIssueBusy] = useState(false);
  const [issueError, setIssueError] = useState<string | null>(null);

  // Money action dialog (apply / refund)
  const [action, setAction] = useState<{ kind: "apply" | "refund"; note: CreditNote } | null>(null);
  const [actionAmount, setActionAmount] = useState("");
  const [actionInvoice, setActionInvoice] = useState("");
  const [actionNotes, setActionNotes] = useState("");
  const [actionBusy, setActionBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  // Detail drawer
  const [detail, setDetail] = useState<CreditNoteDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [rowNote, setRowNote] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const [n, c, i] = await Promise.all([
      api.api.GET("/api/v1/credit-notes"),
      api.api.GET("/api/v1/clients"),
      api.api.GET("/api/v1/invoices"),
    ]);
    if (n.error !== undefined || n.data === undefined) {
      setLoadError("Failed to load credit notes.");
      return;
    }
    setNotes(n.data);
    if (c.data !== undefined) setClients(c.data);
    if (i.data !== undefined) setInvoices(i.data);
    setLoadError(null);
  }, []);

  useEffect(() => {
    if (auth.status === "authed" && canSee) void refresh();
  }, [auth.status, canSee, refresh]);

  // Deep-link prefill from the invoice detail ("Issue credit note"): scope
  // the ledger to the client and open the issue dialog with the invoice
  // preselected once the client list has loaded.
  const [prefilled, setPrefilled] = useState(false);
  useEffect(() => {
    if (prefilled || clients.length === 0 || prefillClient === "") return;
    if (!clients.some((c) => c.id === prefillClient)) return;
    setPrefilled(true);
    setClientFilter(prefillClient);
    setIssueClientId(prefillClient);
    if (prefillInvoice !== "") {
      setIssueInvoiceId(prefillInvoice);
      setIssueOpen(true);
    }
  }, [clients, prefillClient, prefillInvoice, prefilled]);

  const clientName = useCallback(
    (id: string | null | undefined) =>
      id == null ? "—" : (clients.find((c) => c.id === id)?.name ?? id.slice(0, 8)),
    [clients],
  );

  const clientInvoices = useMemo(
    () =>
      invoices.filter(
        (i) =>
          i.client_id === (action?.note.client_id ?? issueClientId) &&
          i.status !== "void" &&
          (issueInvoiceId === "none" ? true : i.currency === (action?.note.currency ?? issueCurrency)),
      ),
    [invoices, action, issueClientId, issueInvoiceId, issueCurrency],
  );

  const issue = async () => {
    setIssueBusy(true);
    setIssueError(null);
    const total = Math.round(Number(issueTotal) * 100);
    if (!Number.isFinite(total) || total <= 0) {
      setIssueError("Credit value must be greater than zero.");
      setIssueBusy(false);
      return;
    }
    if (issueInvoiceId === "none" && !/^[A-Z]{3}$/.test(issueCurrency)) {
      setIssueError("Free-standing credits need a 3-letter currency code.");
      setIssueBusy(false);
      return;
    }
    const result = await runEnvelopeMutation<
      components["schemas"]["IssueCreditNoteRequest"],
      components["schemas"]["CreditNoteMutationResponse"]
    >({
      describe: "Issue credit note",
      optimistic: () => undefined,
      revert: () => undefined,
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        client_id: issueClientId,
        invoice_id: issueInvoiceId === "none" ? null : issueInvoiceId,
        currency: issueInvoiceId === "none" ? issueCurrency.toUpperCase() : null,
        total_minor: total,
        reason: issueReason.trim() === "" ? null : issueReason.trim(),
      }),
      request: (client, body) => client.POST("/api/v1/credit-notes", { body }),
    });
    setIssueBusy(false);
    if (!result.ok) {
      setIssueError(
        result.message?.includes("already in org") || result.reason === "forbidden"
          ? (rejectionText(result, "The credit note was rejected."))
          : (rejectionText(result, "The credit note was rejected.")),
      );
      return;
    }
    setIssueOpen(false);
    setIssueTotal("");
    setIssueReason("");
    void refresh();
  };

  const openAction = (kind: "apply" | "refund", note: CreditNote) => {
    setAction({ kind, note });
    setActionAmount("");
    setActionInvoice("");
    setActionNotes("");
    setActionError(null);
  };

  // Money-correctness: a voided note carries no usable credit — the raw
  // counter difference must never re-materialize as spendable balance.
  const remainingOf = (note: CreditNote) =>
    note.status === "void"
      ? 0
      : note.total_minor - note.applied_minor - note.refunded_minor;

  const runAction = async () => {
    if (action === null) return;
    const amount = Math.round(Number(actionAmount) * 100);
    const remaining = remainingOf(action.note);
    if (!Number.isFinite(amount) || amount <= 0 || amount > remaining) {
      setActionError(`Amount must be between 0 and ${money(remaining, action.note.currency)}.`);
      return;
    }
    if (action.kind === "apply" && actionInvoice === "") {
      setActionError("Pick the invoice to apply the credit to.");
      return;
    }
    setActionBusy(true);
    setActionError(null);
    const before = notes ?? [];
    const result = await runEnvelopeMutation<
      components["schemas"]["ApplyCreditNoteRequest"] | components["schemas"]["RefundCreditNoteRequest"],
      components["schemas"]["CreditNoteAllocationResponse"]
    >({
      describe: action.kind === "apply" ? "Apply credit" : "Refund credit",
      entityId: action.note.id,
      optimistic: () =>
        setNotes((prev) =>
          (prev ?? []).map((n) =>
            n.id === action.note.id
              ? {
                  ...n,
                  applied_minor: action.kind === "apply" ? n.applied_minor + amount : n.applied_minor,
                  refunded_minor: action.kind === "refund" ? n.refunded_minor + amount : n.refunded_minor,
                }
              : n,
          ),
        ),
      revert: () => setNotes(before),
      buildBody: (base) =>
        action.kind === "apply"
          ? { idempotency_key: base.idempotency_key, amount_minor: amount, invoice_id: actionInvoice }
          : { idempotency_key: base.idempotency_key, amount_minor: amount, notes: actionNotes.trim() === "" ? null : actionNotes.trim() },
      request: (client, body) =>
        action.kind === "apply"
          ? client.POST("/api/v1/credit-notes/{id}/apply", {
              params: { path: { id: action.note.id } },
              body: body as components["schemas"]["ApplyCreditNoteRequest"],
            })
          : client.POST("/api/v1/credit-notes/{id}/refund", {
              params: { path: { id: action.note.id } },
              body: body as components["schemas"]["RefundCreditNoteRequest"],
            }),
    });
    setActionBusy(false);
    if (!result.ok) {
      setActionError(rejectionText(result, "The action was rejected."));
      return;
    }
    setAction(null);
    setRowNote(action.kind === "apply" ? "Credit applied — the invoice gained a paired payment receipt." : "Credit refunded back out.");
    void refresh();
  };

  const voidNote = async (note: CreditNote) => {
    const before = notes ?? [];
    const result = await runEnvelopeMutation<
      components["schemas"]["VoidCreditNoteRequest"],
      components["schemas"]["CreditNoteMutationResponse"]
    >({
      describe: "Void credit note",
      entityId: note.id,
      optimistic: () => setNotes((prev) => (prev ?? []).map((n) => (n.id === note.id ? { ...n, status: "void" as const } : n))),
      revert: () => setNotes(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key }),
      request: (client, body) =>
        client.POST("/api/v1/credit-notes/{id}/void", { params: { path: { id: note.id } }, body }),
    });
    if (!result.ok) {
      setRowNote(
        result.message?.includes("has allocations")
          ? "This credit has allocations — only a fully unallocated credit can be voided."
          : (rejectionText(result, "The void was rejected.")),
      );
      return;
    }
    setRowNote("Credit note voided.");
    void refresh();
  };

  const openDetail = async (note: CreditNote) => {
    setDetailLoading(true);
    const { data } = await api.api.GET("/api/v1/credit-notes/{id}", { params: { path: { id: note.id } } });
    setDetailLoading(false);
    if (data !== undefined) setDetail(data);
  };

  if (auth.status !== "authed") return null;

  if (!canSee) {
    return (
      <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
        <h1 className="text-lg font-semibold">Credit notes</h1>
        <Card>
          <EmptyState message="Credit notes are a financial surface — available to roles with financial visibility." />
        </Card>
      </div>
    );
  }

  const openCredits = (notes ?? []).filter((n) => n.status === "issued");
  const remainingTotal = perCurrencySum(
    openCredits.map((n) => ({
      currency: n.currency as string,
      amount: n.total_minor - n.applied_minor - n.refunded_minor,
    })),
  );
  const usedUp = (notes ?? []).filter(
    (n) => n.status === "issued" && remainingOf(n) <= 0,
  );

  const filteredNotes = (notes ?? []).filter((n) => {
    if (clientFilter !== "all" && n.client_id !== clientFilter) return false;
    if (lifecycle === "open" && !(n.status === "issued" && remainingOf(n) > 0)) return false;
    if (lifecycle === "used" && !(n.status === "issued" && remainingOf(n) <= 0)) return false;
    if (lifecycle === "void" && n.status !== "void") return false;
    for (const clause of clauses) {
      const raw =
        clause.field === "number"
          ? n.number
          : clause.field === "client"
            ? clientName(n.client_id)
            : (n.issued_against_invoice_id ?? "free-standing");
      const has = raw.toLowerCase().includes((clause.value ?? "").toLowerCase());
      if (clause.op === "eq" && !has) return false;
      if (clause.op === "is_set" && raw.trim() === "") return false;
      if (clause.op === "is_not_set" && raw.trim() !== "") return false;
    }
    return true;
  });

  const exportCredits = (): void => {
    downloadRowsCsv(
      "credit-notes.csv",
      shownColumns.map((c) => c.label),
      filteredNotes.map((n) =>
        shownColumns.map((c) => {
          switch (c.key) {
            case "number":
              return n.number;
            case "client":
              return clientName(n.client_id);
            case "lifecycle":
              return lifecycleOf(n).label;
            case "invoice":
              return n.issued_against_invoice_id ?? "free-standing";
            case "total":
              return money(n.total_minor, n.currency);
            case "usage":
              return `${money(n.applied_minor, n.currency)} applied · ${money(n.refunded_minor, n.currency)} refunded`;
            case "remaining":
              return money(remainingOf(n), n.currency);
            default:
              return "";
          }
        }),
      ),
    );
  };

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
      <MoneyHeader
        title="Credit notes"
        stats={
          <div
            style={{ display: "flex", gap: 8, alignItems: "center" }}
            data-testid="credit-stat-strip"
          >
            <MoneyStatPill
              label="Open credits"
              count={openCredits.length}
              tone="gold"
              pressed={lifecycle === "open"}
              onClick={() => setLifecycle(lifecycle === "open" ? "all" : "open")}
              testId="credit-stat-open"
            />
            <MoneyStatPill
              label="Used up"
              count={usedUp.length}
              tone="bark"
              pressed={lifecycle === "used"}
              onClick={() => setLifecycle(lifecycle === "used" ? "all" : "used")}
              testId="credit-stat-used"
            />
          </div>
        }
        actions={
          <div className="flex items-center gap-2">
          <Button
            onClick={() => {
              setIssueOpen(true);
              setIssueError(null);
            }}
            data-testid="issue-credit-note"
          >
            Issue credit note
          </Button>
          </div>
        }
      />

      <DataToolbar
        label="Credit note ledger controls"
        views={
          <ViewSwitch
            label="Views"
            activeId={lifecycle}
            onChange={(id) => setLifecycle(id as "all" | "open" | "used" | "void")}
            views={[
              { id: "all", label: "All", hint: String((notes ?? []).length) },
              { id: "open", label: "Open", hint: String(openCredits.length) },
              { id: "used", label: "Used up", hint: String(usedUp.length) },
              { id: "void", label: "Void", hint: String((notes ?? []).filter((n) => n.status === "void").length) },
            ]}
          />
        }
        filters={
          <div className="flex flex-wrap items-center gap-2">
            {/* The client pick is a filter, so it lives on the filter row. In
                the header's action slot it pushed the primary action onto its
                own line, four pixels past the row's width. */}
            <Select value={clientFilter} onValueChange={setClientFilter}>
              <SelectTrigger style={{ width: 192 }} aria-label="Filter by client">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All clients</SelectItem>
                {clients.map((c) => (
                  <SelectItem key={c.id} value={c.id}>
                    {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <FilterBar
              fields={CREDIT_FILTER_FIELDS}
              clauses={clauses}
              onChange={(next) => {
                setClauses(next);
                void prefs.save("credit-notes", {
                  columns: columns.map((c) => ({ key: c.key, visible: c.visible })),
                  filters: next,
                });
              }}
            />
          </div>
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
                const def = CREDIT_COLUMNS.find((d) => d.key === n.key);
                return { key: n.key, label: def?.label ?? n.key, visible: n.visible, required: def?.required };
              });
              setColumns(merged);
              void prefs.save("credit-notes", {
                columns: merged.map((c) => ({ key: c.key, visible: c.visible })),
                filters: clauses,
              });
            }}
          />
        }
        export={
          <ExportControl
            columns={shownColumns.map((c) => c.label)}
            rowCount={filteredNotes.length}
            filters={clauses}
            onExport={exportCredits}
            formats={["csv"]}
          />
        }
      />

      {rowNote !== null ? (
        <p role="status" className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {rowNote}
        </p>
      ) : null}

      {loadError !== null ? (
        <p role="alert" className="text-[13px]">
          {loadError}
        </p>
      ) : notes === null ? (
        <Card>
          <div className="flex justify-center p-6">
            <Spinner />
          </div>
        </Card>
      ) : filteredNotes.length === 0 ? (
        <Card>
          <EmptyState message="No credit notes yet. Issue one against an invoice or as free-standing credit for a client." />
        </Card>
      ) : (
        <MoneyCard testId="credit-ledger">
          <div className="m-money-bar">
            <span>
              <b>{filteredNotes.length}</b> {filteredNotes.length === 1 ? "note" : "notes"} ·{" "}
              <b>{remainingTotal.value}</b> remaining{" "}
              <span style={{ color: "var(--m-text-muted)" }}>
                {remainingTotal.currency ?? "single-currency roll-up only"}
              </span>
            </span>
          </div>
          <div style={{ overflowX: "auto" }}>
          <Table>
            <thead>
              <tr>
                {shownColumns.map((column) => (
                  <Th
                    key={column.key}
                    columnKey={column.key}
                    className={
                      column.key === "total" || column.key === "remaining" ? "text-right" : undefined
                    }
                  >
                    {column.label}
                  </Th>
                ))}
                <Th className="text-right">Actions</Th>
              </tr>
            </thead>
            <tbody>
              {filteredNotes.map((note) => {
                const life = lifecycleOf(note);
                const remaining = remainingOf(note);
                const total = note.total_minor;
                const appliedPct = total > 0 ? Math.min(100, Math.round((note.applied_minor / total) * 100)) : 0;
                const refundedPct = total > 0 ? Math.min(100 - appliedPct, Math.round((note.refunded_minor / total) * 100)) : 0;
                return (
                  <Tr key={note.id} className="row-hover">
                    {shownColumns.map((column) => {
                      switch (column.key) {
                        case "number":
                          return (
 <Td key={column.key} className="text-[13px] font-medium tabular-nums">
                              <button
                                type="button"
                                style={{
                                  cursor: "pointer",
                                  border: "none",
                                  background: "transparent",
                                  padding: 0,
                                  textDecoration: "underline",
                                  textUnderlineOffset: 2,
                                }}
                                onClick={() => void openDetail(note)}
                              >
                                {note.number}
                              </button>
                            </Td>
                          );
                        case "client":
                          return (
                            <Td key={column.key} className="text-[13px]">
                              {clientName(note.client_id)}
                            </Td>
                          );
                        case "lifecycle":
                          return (
                            <Td key={column.key}>
                              <MoneyPillTag
                                pill={{
                                  label: life.label,
                                  family:
                                    life.tone === "success"
                                      ? "green"
                                      : life.tone === "warning"
                                        ? "gold"
                                        : "neutral",
                                }}
                                testId={`credit-status-${note.number}`}
                              />
                            </Td>
                          );
                        case "invoice":
                          return (
                            <Td key={column.key} className="text-[12px]">
                              {note.issued_against_invoice_id != null ? (
                                <Link
                                  href={`/billing/${note.issued_against_invoice_id}`}
                                  style={{ color: "var(--m-accent-text)" }}
                                >
                                  invoice →
                                </Link>
                              ) : (
                                <span style={{ color: "var(--m-text-muted)" }}>free-standing</span>
                              )}
                            </Td>
                          );
                        case "total":
                          return (
 <Td key={column.key} className="text-right text-[12px] tabular-nums">
                              {money(note.total_minor, note.currency)}
                            </Td>
                          );
                        case "usage":
                          return (
                            <Td key={column.key}>
                              {/* Applied is canopy, refunded is a hairline of
                                  neutral: both spend credit, neither is an alarm. */}
                              <div
                                aria-hidden="true"
                                className="flex h-1.5 w-24 overflow-hidden rounded-full"
                                style={{ background: "var(--m-border)" }}
                              >
                                <span
                                  style={{ width: `${appliedPct}%`, background: "var(--m-status-success-fg)" }}
                                />
                                <span
                                  style={{ width: `${refundedPct}%`, background: "var(--m-bark)" }}
                                />
                              </div>
                              <span
 className="text-[11px] tabular-nums"
                                style={{ color: "var(--m-text-muted)" }}
                              >
                                applied {money(note.applied_minor, note.currency)} · refunded{" "}
                                {money(note.refunded_minor, note.currency)}
                              </span>
                            </Td>
                          );
                        case "remaining":
                          return (
                            <Td
                              key={column.key}
 className="text-right text-[12px] font-medium tabular-nums"
                            >
                              {money(remaining, note.currency)}
                            </Td>
                          );
                        default:
                          return <Td key={column.key} />;
                      }
                    })}
                    <Td className="text-right">
                      <div className="flex justify-end gap-1">
                        <Button size="sm" variant="ghost" disabled={remaining <= 0} onClick={() => openAction("apply", note)}>
                          Apply
                        </Button>
                        <Button size="sm" variant="ghost" disabled={remaining <= 0} onClick={() => openAction("refund", note)}>
                          Refund
                        </Button>
                        {note.status === "issued" ? (
                          <Button size="sm" variant="ghost" onClick={() => void voidNote(note)}>
                            Void
                          </Button>
                        ) : null}
                      </div>
                    </Td>
                  </Tr>
                );
              })}
            </tbody>
          </Table>
          </div>
        </MoneyCard>
      )}

      <footer className="m-money-foot">
        <span className="m-money-foot-sp" />
        <DensitySwitch
          value={prefs.density}
          onChange={(step) => {
            applyDensity(step);
            void prefs.setDensity(step);
          }}
        />
        <Kbd>C</Kbd>
        <span>issue a credit note</span>
      </footer>

      {/* Issue dialog */}
      <Dialog open={issueOpen} onOpenChange={(o) => { setIssueOpen(o); if (!o) setIssueError(null); }}>
        <DialogContent
          title="Issue credit note"
          description="Against an invoice (inherits its currency) or as free-standing client credit."
        >
          <div className="grid gap-3 p-4">
            <div>
              <Label>Client</Label>
              <Select value={issueClientId} onValueChange={(v) => { setIssueClientId(v); setIssueInvoiceId("none"); }}>
                <SelectTrigger className="w-full" aria-label="Credit note client">
                  <SelectValue placeholder="Pick a client" />
                </SelectTrigger>
                <SelectContent>
                  {clients.map((c) => (
                    <SelectItem key={c.id} value={c.id}>
                      {c.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label>Against invoice (optional)</Label>
              <Select value={issueInvoiceId} onValueChange={setIssueInvoiceId}>
                <SelectTrigger className="w-full" aria-label="Invoice to credit against">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">Free-standing credit</SelectItem>
                  {invoices
                    .filter((i) => i.client_id === issueClientId && i.status !== "void")
                    .map((i) => (
                      <SelectItem key={i.id} value={i.id}>
                        {i.number} · {i.status} · {money(i.total_minor - i.total_paid_minor, i.currency)} due
                      </SelectItem>
                    ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex gap-2">
              <div className="flex-1">
                <Label htmlFor="cn-total">Credit value</Label>
                <Input id="cn-total" type="number" min="0" step="0.01" value={issueTotal} onChange={(e) => setIssueTotal(e.target.value)} />
              </div>
              <div className="w-20">
                <Label htmlFor="cn-currency">Currency</Label>
                <Input
                  id="cn-currency"
                  maxLength={3}
                  value={issueInvoiceId === "none" ? issueCurrency : "…"}
                  disabled={issueInvoiceId !== "none"}
                  onChange={(e) => setIssueCurrency(e.target.value.toUpperCase())}
                />
              </div>
            </div>
            <div>
              <Label htmlFor="cn-reason">Reason (optional)</Label>
              <Input id="cn-reason" value={issueReason} onChange={(e) => setIssueReason(e.target.value)} placeholder="Goodwill credit — missed SLA" />
            </div>
            {issueError !== null ? (
              <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {issueError}
              </p>
            ) : null}
            <DialogActions>
              <Button variant="ghost" onClick={() => setIssueOpen(false)}>
                Cancel
              </Button>
              <Button variant="primary" onClick={() => void issue()} disabled={issueBusy || issueClientId === ""}>
                {issueBusy ? <Spinner /> : null} Issue
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      {/* Apply / refund dialog */}
      <Dialog open={action !== null} onOpenChange={(o) => { if (!o) { setAction(null); setActionError(null); } }}>
        <DialogContent
          title={action?.kind === "apply" ? "Apply credit to an invoice" : "Refund credit back out"}
          description={
            action?.kind === "apply"
              ? "The move pairs an invoice payment receipt (provider credit_note) with the credit-side ledger entry."
              : "Money moves back out; the credit's remaining balance shrinks."
          }
        >
          <div className="grid gap-3 p-4">
            {action !== null ? (
              <>
                <p className="text-[13px]">
                  {action.note.number} · remaining {money(remainingOf(action.note), action.note.currency)}
                </p>
                {action.kind === "apply" ? (
                  <div>
                    <Label>Invoice (same client + currency)</Label>
                    <Select value={actionInvoice} onValueChange={setActionInvoice}>
                      <SelectTrigger className="w-full" aria-label="Invoice to apply credit to">
                        <SelectValue placeholder="Pick an invoice" />
                      </SelectTrigger>
                      <SelectContent>
                        {clientInvoices
                          .filter((i) => i.currency === action.note.currency)
                          .map((i) => (
                            <SelectItem key={i.id} value={i.id}>
                              {i.number} · {i.status} · {money(i.total_minor - i.total_paid_minor, i.currency)} due
                            </SelectItem>
                          ))}
                      </SelectContent>
                    </Select>
                  </div>
                ) : (
                  <div>
                    <Label htmlFor="cn-refund-notes">Notes (optional)</Label>
                    <Input id="cn-refund-notes" value={actionNotes} onChange={(e) => setActionNotes(e.target.value)} />
                  </div>
                )}
                <div>
                  <Label htmlFor="cn-amount">Amount ({action.note.currency})</Label>
                  <Input id="cn-amount" type="number" min="0" step="0.01" value={actionAmount} onChange={(e) => setActionAmount(e.target.value)} />
                </div>
              </>
            ) : null}
            {actionError !== null ? (
              <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {actionError}
              </p>
            ) : null}
            <DialogActions>
              <Button variant="ghost" onClick={() => setAction(null)}>
                Cancel
              </Button>
              <Button variant="primary" onClick={() => void runAction()} disabled={actionBusy}>
                {actionBusy ? <Spinner /> : null} {action?.kind === "apply" ? "Apply" : "Refund"}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      {/* Detail drawer */}
      <Dialog open={detail !== null} onOpenChange={(o) => { if (!o) setDetail(null); }}>
        <DialogContent title={detail !== null ? `Credit note ${detail.number}` : "Credit note"}>
          <div className="grid gap-3 p-4">
            {detailLoading || detail === null ? (
              <Spinner />
            ) : (
              <>
                <div className="flex flex-wrap gap-2 text-[13px]">
                  <Badge tone={lifecycleOf(detail).tone}>{lifecycleOf(detail).label}</Badge>
                  <Badge>{money(detail.total_minor, detail.currency)}</Badge>
                  <Badge tone="neutral">applied {money(detail.applied_minor, detail.currency)}</Badge>
                  <Badge tone="neutral">refunded {money(detail.refunded_minor, detail.currency)}</Badge>
                  <Badge tone="neutral">remaining {money(detail.remaining_minor, detail.currency)}</Badge>
                </div>
                {detail.reason != null ? (
                  <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                    {detail.reason}
                  </p>
                ) : null}
                <div>
                  <Label>Application ledger</Label>
                  {detail.applications.length === 0 ? (
                    <p className="mt-1 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                      No applications yet.
                    </p>
                  ) : (
                    <Table>
                      <thead>
                        <tr>
                          <Th>When</Th>
                          <Th>Invoice</Th>
                          <Th className="text-right">Amount</Th>
                        </tr>
                      </thead>
                      <tbody>
                        {detail.applications.map((app) => (
                          <Tr key={app.id}>
                            <Td className="text-[12px]">{new Date(app.created_at).toLocaleString()}</Td>
                            <Td className="text-[12px]">
                              <Link href={`/billing/${app.invoice_id}`} className="underline decoration-[var(--m-border)] underline-offset-2">
                                invoice
                              </Link>
                            </Td>
                            <Td className="text-right tabular-nums text-[12px]">{money(app.amount_minor, detail.currency)}</Td>
                          </Tr>
                        ))}
                      </tbody>
                    </Table>
                  )}
                </div>
              </>
            )}
            <DialogActions>
              <Button variant="ghost" onClick={() => setDetail(null)}>
                Close
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
