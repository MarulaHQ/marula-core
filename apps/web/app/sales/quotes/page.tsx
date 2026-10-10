// /sales/quotes — quote ledger + builder (Wave 12 lane 2 backends, 0060).
//
// Purpose: quotes CRUD end-to-end: the org's quote list (filterable by
// status), the quote builder (client, currency, validity, flat tax rate,
// line editor backed by the product catalog), and the lifecycle legs —
// send, accept, decline, expire — plus convert-to-invoice for accepted
// quotes. Lifecycle mutations ride the IF1–IF3 envelope with idempotency
// keys; rejections surface inline as role="alert" and the optimistic
// status rolls back.
//
// Non-purpose: no invoice editing here (convert hands the result to the
// billing ledger; the invoice surfaces on /billing).
"use client";

import { rejectionText } from "../../../lib/error-copy";
import {
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
  StatusTag,
  Table,
  Tag,
  Td,
  Th,
  Tr,
  ViewSwitch,
  applyDensity,
  type FilterClause,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useEffect, useCallback, useMemo, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { downloadRowsCsv } from "../../../lib/csv-download";
import { useDataSurfacePrefs } from "../../../lib/data-surface-prefs";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { money } from "../../../lib/format";
import { formatQtyMilli } from "../../../lib/stock-format";
import { daysUntil, perCurrencySum } from "../../../lib/money-summary";
import { quotePill } from "../../../lib/money-ui";
import { MoneyCard, MoneyHeader, MoneyPillTag } from "../../../components/money";

type Quote = components["schemas"]["Quote"];
type QuoteStatus = components["schemas"]["QuoteStatus"];
type QuoteLineInput = components["schemas"]["QuoteLineInput"];
type CatalogItem = components["schemas"]["CatalogItemView"];
type Client = components["schemas"]["Client"];
type StockItemView = components["schemas"]["StockItemView"];
type OnHandRow = components["schemas"]["OnHandRow"];

type QuoteAction = "send" | "accept" | "decline" | "expire" | "convert-to-invoice";

const STATUSES: readonly (QuoteStatus | "all")[] = [
  "all",
  "draft",
  "sent",
  "accepted",
  "declined",
  "expired",
];

interface ColumnDef {
  key: string;
  label: string;
  required?: boolean;
}

/** The quote ledger's columns, in the quoting surface's order: the quote, its
 *  client, the value, the status, when it moves. The canonical ledger ends at
 *  STATUS and draws no per-row actions — the lifecycle legs live on
 *  `/sales/quotes/[id]` — so the Actions column is gone and the QUOTE cell is
 *  the way in. Actions are also outside the column set, so the chooser can
 *  never offer to hide them. */
const COLUMNS: readonly ColumnDef[] = [
  { key: "number", label: "Quote", required: true },
  { key: "client", label: "Client" },
  { key: "total", label: "Value" },
  { key: "status", label: "Status" },
  { key: "valid", label: "Valid until" },
];

const FILTER_FIELDS = [
  { key: "number", label: "Quote number" },
  { key: "client", label: "Client" },
  { key: "status", label: "Status" },
  { key: "valid_until", label: "Valid until" },
];

const EMPTY_LINE: QuoteLineInput = {
  description: null,
  catalog_item_id: null,
  unit: "each",
  quantity_milli: 1000,
  unit_amount_minor: 0,
  taxable: true,
};

function isoDay(offsetDays: number): string {
  const d = new Date();
  d.setUTCHours(0, 0, 0, 0);
  d.setUTCDate(d.getUTCDate() + offsetDays);
  return d.toISOString().slice(0, 10);
}

export default function QuotesPage(): ReactNode {
  const auth = useRequireAuth();
  const [quotes, setQuotes] = useState<Quote[] | null>(null);
  const [catalog, setCatalog] = useState<CatalogItem[] | null>(null);
  const [clients, setClients] = useState<Client[] | null>(null);
  const [stockItems, setStockItems] = useState<StockItemView[]>([]);
  const [stockOnHand, setStockOnHand] = useState<OnHandRow[]>([]);
  const [stockPick, setStockPick] = useState<Record<number, string>>({});
  const [statusFilter, setStatusFilter] = useState<QuoteStatus | "all">("all");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  // The ledger shell is the shared one, so the quotes ledger answers the pack's
  // money-cluster toolbar rule (views left, filters, Columns/Filter/Export
  // right) exactly as the invoice and credit-note ledgers do.
  const prefs = useDataSurfacePrefs();
  const [clauses, setClauses] = useState<FilterClause[]>([]);
  const savedColumns = useMemo(() => prefs.view("quotes").columns, [prefs]);
  const [columns, setColumns] = useState(() =>
    COLUMNS.map((c) => {
      const stored = savedColumns.find((s) => s.key === c.key);
      return { key: c.key, label: c.label, visible: stored?.visible ?? true, required: c.required };
    }),
  );
  const [order, setOrder] = useState<string[]>(() => {
    const known = savedColumns.map((c) => c.key).filter((k) => COLUMNS.some((c) => c.key === k));
    return [...known, ...COLUMNS.map((c) => c.key).filter((k) => !known.includes(k))];
  });
  useEffect(() => {
    if (!prefs.ready) return;
    setColumns(
      COLUMNS.map((c) => {
        const stored = savedColumns.find((s) => s.key === c.key);
        return { key: c.key, label: c.label, visible: stored?.visible ?? true, required: c.required };
      }),
    );
    const known = savedColumns.map((c) => c.key).filter((k) => COLUMNS.some((c) => c.key === k));
    setOrder([...known, ...COLUMNS.map((c) => c.key).filter((k) => !known.includes(k))]);
  }, [prefs.ready, savedColumns]);

  // Builder state
  const [createOpen, setCreateOpen] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [createBusy, setCreateBusy] = useState(false);
  const [newClientId, setNewClientId] = useState("");
  const [newCurrency, setNewCurrency] = useState("USD");
  const [newValidUntil, setNewValidUntil] = useState(isoDay(30));
  const [newTaxRateBps, setNewTaxRateBps] = useState("0");
  const [newNotes, setNewNotes] = useState("");
  const [lines, setLines] = useState<QuoteLineInput[]>([{ ...EMPTY_LINE, description: "" }]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const [quoteRes, catalogRes, clientRes] = await Promise.all([
        api.api.GET("/api/v1/quotes"),
        api.api.GET("/api/v1/catalog-items"),
        api.api.GET("/api/v1/clients"),
      ]);
      if (quoteRes.data === undefined) {
        setLoadError("Could not load quotes — try reloading.");
        return;
      }
      setQuotes(quoteRes.data ?? []);
      setCatalog(catalogRes.data ?? []);
      setClients(clientRes.data ?? []);
    })();
    void (async () => {
      const [stockRes, onHandRes] = await Promise.all([
        api.api.GET("/api/v1/stock-items", { params: { query: { limit: 500 } } }),
        api.api.GET("/api/v1/stock/on-hand", { params: { query: { limit: 1000 } } }),
      ]);
      setStockItems(stockRes.data ?? []);
      setStockOnHand(onHandRes.data ?? []);
    })();
  }, [auth.status]);

  const clientName = useCallback(
    (id: string) => (clients ?? []).find((c) => c.id === id)?.name ?? "Client",
    [clients],
  );

  /** The status view narrows first; the filter clauses narrow what is left. */
  const filtered = useMemo(
    () => (quotes ?? []).filter((q) => statusFilter === "all" || q.status === statusFilter),
    [quotes, statusFilter],
  );

  const filteredByClause = useMemo(() => {
    if (clauses.length === 0) return filtered;
    return filtered.filter((q) => {
      const raw = (key: string): string => {
        switch (key) {
          case "number": return q.number ?? "";
          case "client": return clientName(q.client_id);
          case "status": return q.status ?? "";
          case "valid_until": return q.valid_until ?? "";
          default: return "";
        }
      };
      return clauses.every((clause) => {
        const hay = raw(clause.field).toLowerCase();
        const needle = String(clause.value ?? "").toLowerCase();
        if (clause.op === "is_set") return hay !== "";
        if (clause.op === "is_not_set") return hay === "";
        if (clause.op === "ne") return hay !== needle;
        return hay.includes(needle);
      });
    });
  }, [filtered, clauses, clientName]);

  const visibleColumns = useMemo(
    () =>
      order
        .map((key) => columns.find((c) => c.key === key))
        .filter((c): c is (typeof columns)[number] => c !== undefined && c.visible),
    [order, columns],
  );

  const exportQuotes = useCallback(() => {
    const header = visibleColumns.map((c) => c.label);
    const body = filteredByClause.map((q) => {
      const values: Record<string, string> = {
        number: q.number ?? "",
        client: clientName(q.client_id),
        total: money(q.total_minor, q.currency),
        status: q.status ?? "",
        valid_until: q.valid_until ?? "",
      };
      return visibleColumns.map((c) => values[c.key] ?? "");
    });
    downloadRowsCsv("quotes.csv", header, body);
  }, [visibleColumns, filteredByClause, clientName]);

  const cell = (key: string, q: Quote) => {
    const validDays = daysUntil(q.valid_until ?? "");
    switch (key) {
      case "number":
        return (
          <Link
            href={`/sales/quotes/${q.id}`}
            className="underline decoration-[var(--m-border)] underline-offset-2"
            style={{ color: "var(--m-accent-text)", textDecoration: "none" }}
            data-testid={`quote-link-${q.id}`}
          >
            <span className="text-[13px]">{q.number}</span>
          </Link>
        );
      case "client":
        return (
          <Link
            href={`/clients/${q.client_id}`}
            className="underline decoration-[var(--m-border)] underline-offset-2"
            style={{ color: "var(--m-accent)" }}
          >
            {clientName(q.client_id)}
          </Link>
        );
      case "total":
        return <span className="block text-right text-[13px] tabular-nums">{money(q.total_minor, q.currency)}</span>;
      case "status":
        return (
          <>
            <MoneyPillTag pill={quotePill(q.status)} testId={`quote-status-${q.id}`} />
            {q.converted_invoice_id != null ? (
              <Link
                href={`/billing/${q.converted_invoice_id}`}
                className="ml-1.5 text-[11px] underline decoration-[var(--m-border)] underline-offset-2"
                data-testid={`quote-invoice-link-${q.id}`}
              >
                invoice →
              </Link>
            ) : null}
          </>
        );
      case "valid":
        return (
          <>
            <span className="tabular-nums">{q.valid_until}</span>
            {/* A quote past its date is a fact about money, not a breach:
                blush, and with the days in words. */}
            {validDays !== null && validDays < 0 && q.status === "sent" ? (
              <span className="m-money-pill m-money-pill-blush" style={{ marginLeft: 6 }}>
                past valid-until
              </span>
            ) : validDays !== null && validDays <= 7 && q.status === "sent" ? (
              <span className="m-money-pill m-money-pill-blush" style={{ marginLeft: 6 }}>
                expires in {validDays}d
              </span>
            ) : null}
          </>
        );
      default:
        return null;
    }
  };

  function quoteTotals(): { subtotalMinor: number; taxMinor: number; totalMinor: number } {
    let subtotalMinor = 0;
    let taxableMinor = 0;
    for (const line of lines) {
      const amount = Math.round((line.quantity_milli / 1000) * (line.unit_amount_minor ?? 0));
      subtotalMinor += amount;
      if (line.taxable === true) taxableMinor += amount;
    }
    const rate = Math.max(0, Number(newTaxRateBps) || 0);
    const taxMinor = Math.round((taxableMinor * rate) / 10000);
    return { subtotalMinor, taxMinor, totalMinor: subtotalMinor + taxMinor };
  }

  function setLine(index: number, patch: Partial<QuoteLineInput>): void {
    setLines((cur) => cur.map((l, i) => (i === index ? { ...l, ...patch } : l)));
  }

  function pickCatalogItem(index: number, itemId: string): void {
    const item = (catalog ?? []).find((c) => c.id === itemId);
    if (item === undefined) {
      setLine(index, { catalog_item_id: null });
      return;
    }
    setLine(index, {
      catalog_item_id: item.id,
      description: item.description ?? item.name,
      unit: item.unit,
      unit_amount_minor: item.unit_price_minor,
      taxable: item.taxable,
    });
  }

  /**
   * Pulls a line from stock: the item's own identity fills the line, and a
   * soft catalog link (when the stock item has one) brings the sell price.
   * Availability is shown, never reserved — a quote is not a hold.
   */
  function pickStockItem(index: number, itemId: string): void {
    const item = stockItems.find((s) => s.id === itemId);
    if (item === undefined) return;
    const linked = (catalog ?? []).find((c) => c.id === item.catalog_item_id);
    setLine(index, {
      catalog_item_id: linked?.id ?? null,
      description: item.name,
      unit: item.unit,
      unit_amount_minor: linked?.unit_price_minor ?? 0,
      taxable: linked?.taxable ?? true,
    });
  }

  function stockAvailability(itemId: string | null): { totalMilli: number; low: boolean } | null {
    if (itemId === null) return null;
    const rows = stockOnHand.filter((row) => row.stock_item_id === itemId);
    if (rows.length === 0) return { totalMilli: 0, low: false };
    return {
      totalMilli: rows.reduce((sum, row) => sum + row.qty_milli, 0),
      low: rows.some((row) => row.low),
    };
  }

  async function refreshQuotes(): Promise<void> {
    const { data } = await api.api.GET("/api/v1/quotes");
    setQuotes(data ?? []);
  }

  async function createQuote(): Promise<void> {
    setCreateError(null);
    if (newClientId === "") {
      setCreateError("Choose a client for the quote.");
      return;
    }
    const validLines = lines.filter(
      (l) => (l.description ?? "").trim() !== "" && (l.quantity_milli ?? 0) > 0,
    );
    if (validLines.length === 0) {
      setCreateError("At least one line with a description and quantity is required.");
      return;
    }
    setCreateBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Create quote",
      buildBody: (base) => ({
        client_id: newClientId,
        currency: newCurrency,
        valid_until: newValidUntil,
        lines: validLines,
        tax_rate_bps: Math.max(0, Number(newTaxRateBps) || 0),
        notes: newNotes.trim() === "" ? null : newNotes,
        ...base,
      }),
      optimistic: () => {
        setQuotes((cur) => [
          ...(cur ?? []),
          {
            id: `pending-${Date.now()}`,
            org_id: "",
            number: "…",
            client_id: newClientId,
            status: "draft" as QuoteStatus,
            currency: newCurrency,
            issued_on: isoDay(0),
            valid_until: newValidUntil,
            subtotal_minor: quoteTotals().subtotalMinor,
            tax_rate_bps: Math.max(0, Number(newTaxRateBps) || 0),
            tax_minor: quoteTotals().taxMinor,
            total_minor: quoteTotals().totalMinor,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
            sent_at: null,
            decided_at: null,
            converted_at: null,
            converted_invoice_id: null,
            converted_contract_id: null,
            notes: newNotes.trim() === "" ? null : newNotes,
            terms: null,
            created_by: null,
          },
        ]);
      },
      revert: () => {
        setQuotes((cur) => (cur ?? []).filter((q) => !q.id.startsWith("pending-")));
      },
      request: (client, body) =>
        client.POST("/api/v1/quotes", {
          body: body as components["schemas"]["CreateQuoteRequest"],
        }),
    });
    setCreateBusy(false);
    if (result.ok === false) {
      setCreateError(rejectionText(result, "Could not create the quote."));
      return;
    }
    setCreateOpen(false);
    setLines([{ ...EMPTY_LINE, description: "" }]);
    setNewNotes("");
    setNewTaxRateBps("0");
    await refreshQuotes();
  }

  if (quotes === null && loadError === null) {
    return <Spinner />;
  }
  if (loadError !== null) {
    return <EmptyState message={loadError} />;
  }

  const totals = quoteTotals();
  const ledgerTotal = perCurrencySum(
    filteredByClause.map((q) => ({ currency: q.currency as string, amount: q.total_minor })),
  );

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
      <MoneyHeader
        title="Quotes"
        eyebrow="Billing"
        stats={
          /* The quoting surface pairs its count with the title; the cluster's
             own shortcut pills still read from the unfiltered ledger. */
          <span className="m-money-count" data-testid="quotes-count">
            <span className="m-money-count-dot" aria-hidden="true" />
            {(quotes ?? []).length}
          </span>
        }
        actions={
        <Dialog open={createOpen} onOpenChange={setCreateOpen}>
          <DialogTrigger asChild>
            <Button data-testid="quotes-new">New quote</Button>
          </DialogTrigger>
          <DialogContent title="Quote builder">
            <Card className="grid max-h-[80vh] gap-3 overflow-auto p-4">
              <h2 className="text-sm font-semibold">Quote builder</h2>
              <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
                <div className="grid gap-1">
                  <Label htmlFor="quote-client">Client</Label>
                  <Select value={newClientId} onValueChange={(v) => setNewClientId(v)}>
                    <SelectTrigger id="quote-client" data-testid="quote-client">
                      <SelectValue placeholder="Choose client" />
                    </SelectTrigger>
                    <SelectContent>
                      {(clients ?? []).map((c) => (
                        <SelectItem key={c.id} value={c.id}>
                          {c.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="grid gap-1">
                  <Label htmlFor="quote-currency">Currency</Label>
                  <Input
                    id="quote-currency"
                    value={newCurrency}
                    maxLength={3}
                    onChange={(e) => setNewCurrency(e.target.value.toUpperCase())}
                  />
                </div>
                <div className="grid gap-1">
                  <Label htmlFor="quote-valid">Valid until</Label>
                  <Input
                    id="quote-valid"
                    type="date"
                    value={newValidUntil}
                    onChange={(e) => setNewValidUntil(e.target.value)}
                  />
                </div>
              </div>

              <div className="grid gap-2">
                <Label>Lines</Label>
                {lines.map((line, index) => (
                  <div
                    key={index}
                    className="grid gap-2 rounded-[var(--m-radius-control)] border p-2.5"
                    style={{ borderColor: "var(--m-border)" }}
                    data-testid={`quote-line-${index}`}
                  >
                    <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)_minmax(0,1.3fr)]">
                      <div className="grid gap-1">
                        <Label>From catalog</Label>
                        <Select
                          value={line.catalog_item_id ?? "none"}
                          onValueChange={(v) => pickCatalogItem(index, v)}
                        >
                          <SelectTrigger data-testid={`quote-line-item-${index}`} className="w-full [&>span]:truncate">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="none">— free-text line —</SelectItem>
                            {(catalog ?? []).map((item) => (
                              <SelectItem key={item.id} value={item.id}>
                                {item.name}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="grid gap-1">
                        <Label>From stock</Label>
                        <Select
                          value={stockPick[index] ?? "none"}
                          onValueChange={(v) => {
                            setStockPick((cur) => ({ ...cur, [index]: v }));
                            if (v !== "none") pickStockItem(index, v);
                          }}
                        >
                          <SelectTrigger data-testid={`quote-line-stock-${index}`} className="w-full [&>span]:truncate">
                            <SelectValue placeholder="— none —" />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="none">— none —</SelectItem>
                            {stockItems
                              .filter((item) => item.active)
                              .map((item) => (
                                <SelectItem key={item.id} value={item.id}>
                                  {item.name}
                                </SelectItem>
                              ))}
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="grid gap-1">
                        <Label>Description</Label>
                        <Input
                          value={line.description ?? ""}
                          onChange={(e) => setLine(index, { description: e.target.value })}
                        />
                      </div>
                    </div>
                    {(() => {
                      const stockItem = stockItems.find((s) => s.id === stockPick[index]);
                      const availability = stockAvailability(stockItem?.id ?? null);
                      if (stockItem === undefined || availability === null) return null;
                      return (
                        <p
                          className="text-[12px]"
                          style={{
                            color:
                              availability.totalMilli > 0
                                ? "var(--m-text-secondary)"
                                : "var(--m-status-warning-fg)",
                          }}
                          data-testid={`quote-line-availability-${index}`}
                        >
                          {formatQtyMilli(availability.totalMilli, stockItem.unit)} on hand
                          {availability.low ? " · low stock" : ""}
                          {availability.totalMilli === 0 ? " — the line is not reserved" : ""}
                        </p>
                      );
                    })()}
                    <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                      <div className="grid gap-1">
                        <Label>Quantity</Label>
                        <Input
                          inputMode="decimal"
                          value={String((line.quantity_milli ?? 0) / 1000)}
                          onChange={(e) => {
                            const v = Number(e.target.value);
                            setLine(index, {
                              quantity_milli: Number.isNaN(v) ? 0 : Math.round(v * 1000),
                            });
                          }}
                        />
                      </div>
                      <div className="grid gap-1">
                        <Label>Unit</Label>
                        <Input
                          value={line.unit ?? "each"}
                          onChange={(e) => setLine(index, { unit: e.target.value || "each" })}
                        />
                      </div>
                      <div className="grid gap-1">
                        <Label>Unit price (major)</Label>
                        <Input
                          inputMode="decimal"
                          value={String(((line.unit_amount_minor ?? 0) / 100).toFixed(2))}
                          onChange={(e) => {
                            const v = Number(e.target.value);
                            setLine(index, {
                              unit_amount_minor: Number.isNaN(v) ? 0 : Math.round(v * 100),
                            });
                          }}
                          data-testid={`quote-line-price-${index}`}
                        />
                      </div>
                      <div className="grid content-end gap-1">
                        <label className="flex items-center gap-1 text-xs" style={{ color: "var(--m-text-secondary)" }}>
                          <input
                            type="checkbox"
                            checked={line.taxable === true}
                            onChange={(e) => setLine(index, { taxable: e.target.checked })}
                          />
                          Taxable
                        </label>
                        <span style={{ color: "var(--m-text-muted)" }}>
                          {money(
                            Math.round((line.quantity_milli / 1000) * (line.unit_amount_minor ?? 0)),
                            newCurrency,
                          )}
                        </span>
                      </div>
                    </div>
                    {lines.length > 1 && (
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => setLines((cur) => cur.filter((_, i) => i !== index))}
                      >
                        Remove line
                      </Button>
                    )}
                  </div>
                ))}
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => setLines((cur) => [...cur, { ...EMPTY_LINE, description: "" }])}
                  data-testid="quote-add-line"
                >
                  Add line
                </Button>
              </div>

              <div className="grid gap-1">
                <Label htmlFor="quote-tax">Tax rate (bps, applied to taxable subtotal)</Label>
                <Input
                  id="quote-tax"
                  inputMode="numeric"
                  value={newTaxRateBps}
                  onChange={(e) => setNewTaxRateBps(e.target.value)}
                />
                <FieldHint>800 = 8%.</FieldHint>
              </div>
              <div className="grid gap-1">
                <Label htmlFor="quote-notes">Notes</Label>
                <Input id="quote-notes" value={newNotes} onChange={(e) => setNewNotes(e.target.value)} />
              </div>

              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-0.5 text-xs">
                <dt style={{ color: "var(--m-text-muted)" }}>Subtotal</dt>
                <dd style={{ color: "var(--m-text)" }}>{money(totals.subtotalMinor, newCurrency)}</dd>
                <dt style={{ color: "var(--m-text-muted)" }}>Tax</dt>
                <dd style={{ color: "var(--m-text)" }}>{money(totals.taxMinor, newCurrency)}</dd>
                <dt style={{ color: "var(--m-text-muted)" }}>Total</dt>
                <dd className="font-semibold" style={{ color: "var(--m-text)" }}>
                  {money(totals.totalMinor, newCurrency)}
                </dd>
              </dl>

              {createError !== null && (
                <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="quote-create-error">
                  {createError}
                </p>
              )}
              <DialogActions>
                <Button variant="secondary" onClick={() => setCreateOpen(false)}>
                  Cancel
                </Button>
                <Button disabled={createBusy} onClick={() => void createQuote()} data-testid="quote-save">
                  Create quote
                </Button>
              </DialogActions>
            </Card>
          </DialogContent>
        </Dialog>
        }
      />

      <DataToolbar
        label="Quote ledger controls"
        views={
          <ViewSwitch
            label="Quote lifecycle"
            activeId={statusFilter}
            onChange={(id) => setStatusFilter(id as QuoteStatus | "all")}
            views={STATUSES.map((st) => ({
              id: st,
              label: st === "all" ? "All quotes" : st,
              hint:
                st === "all"
                  ? String((quotes ?? []).length)
                  : String((quotes ?? []).filter((q) => q.status === st).length),
            }))}
          />
        }
        filters={
          <FilterBar
            fields={FILTER_FIELDS}
            clauses={clauses}
            onChange={(next) => {
              setClauses(next);
              void prefs.save("quotes", {
                columns: columns.map((c) => ({ key: c.key, visible: c.visible })),
                filters: next,
              });
            }}
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
                return { key: n.key, label: def?.label ?? n.key, visible: n.visible, required: def?.required };
              });
              setColumns(merged);
              void prefs.save("quotes", {
                columns: merged.map((c) => ({ key: c.key, visible: c.visible })),
                filters: clauses,
              });
            }}
          />
        }
        export={
          <ExportControl
            columns={visibleColumns.map((c) => c.label)}
            rowCount={filteredByClause.length}
            filters={clauses}
            onExport={exportQuotes}
            formats={["csv"]}
          />
        }
      />

      {actionError !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="quotes-action-error">
          {actionError}
        </p>
      )}

      <MoneyCard testId="quotes-ledger">
        <div className="m-money-bar">
          <span>
            <b>{filteredByClause.length}</b> {statusFilter === "all" ? "quotes" : statusFilter} ·{" "}
            {/* Only ever a single-currency roll-up: a mixed set says so
                rather than inventing an FX total. */}
            <b>{ledgerTotal.value}</b> in total
          </span>
        </div>
        <div style={{ overflowX: "auto" }}>
        <Table data-testid="quotes-table">
          <thead>
            <tr>
              {visibleColumns.map((column) => (
                <Th
                  key={column.key}
                  columnKey={column.key}
                  className={column.key === "total" ? "text-right" : undefined}
                >
                  {column.label}
                </Th>
              ))}
            </tr>
          </thead>
          <tbody>
            {filteredByClause.map((q) => (
              <Tr key={q.id} className="row-hover" data-testid={`quotes-row-${q.id}`}>
                {visibleColumns.map((column) => (
                  <Td key={column.key}>{cell(column.key, q)}</Td>
                ))}
              </Tr>
            ))}
            {filteredByClause.length === 0 && (
              <Tr>
                <Td>
                  <EmptyState message="No quotes match this filter yet." />
                </Td>
              </Tr>
            )}
          </tbody>
        </Table>
        </div>
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
      </footer>
    </div>
  );
}
