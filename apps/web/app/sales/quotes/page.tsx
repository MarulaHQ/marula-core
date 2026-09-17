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
  StatusTag,
  Tag,
  Table,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { money } from "../../../lib/format";
import { daysUntil, QUOTE_STATUS_TONE } from "../../../lib/money-summary";

type Quote = components["schemas"]["Quote"];
type QuoteStatus = components["schemas"]["QuoteStatus"];
type QuoteLineInput = components["schemas"]["QuoteLineInput"];
type CatalogItem = components["schemas"]["CatalogItemView"];
type Client = components["schemas"]["Client"];

type QuoteAction = "send" | "accept" | "decline" | "expire" | "convert-to-invoice";

const STATUSES: readonly (QuoteStatus | "all")[] = [
  "all",
  "draft",
  "sent",
  "accepted",
  "declined",
  "expired",
];

const PENDING_STATUS: Record<"send" | "accept" | "decline" | "expire", QuoteStatus> = {
  send: "sent",
  accept: "accepted",
  decline: "declined",
  expire: "expired",
};

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
  const [statusFilter, setStatusFilter] = useState<QuoteStatus | "all">("all");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busyQuoteId, setBusyQuoteId] = useState<string | null>(null);

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
  }, [auth.status]);

  const filtered = useMemo(
    () => (quotes ?? []).filter((q) => statusFilter === "all" || q.status === statusFilter),
    [quotes, statusFilter],
  );

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

  async function refreshQuotes(): Promise<void> {
    const { data } = await api.api.GET("/api/v1/quotes");
    setQuotes(data ?? []);
  }

  async function runQuoteAction(
    quote: Quote,
    action: "send" | "accept" | "decline" | "expire",
  ): Promise<void> {
    setActionError(null);
    setBusyQuoteId(quote.id);
    const result = await runEnvelopeMutation({
      describe: `Quote ${action}`,
      entityId: quote.id,
      buildBody: (base) => ({ ...base }),
      optimistic: () => {
        setQuotes((cur) =>
          (cur ?? []).map((q) =>
            q.id === quote.id
              ? { ...q, status: PENDING_STATUS[action], updated_at: new Date().toISOString() }
              : q,
          ),
        );
      },
      revert: () => {
        setQuotes((cur) =>
          (cur ?? []).map((q) => (q.id === quote.id ? { ...q, status: quote.status } : q)),
        );
      },
      request: (client, body) =>
        client.POST(`/api/v1/quotes/{id}/${action}`, {
          params: { path: { id: quote.id } },
          body: body as components["schemas"]["QuoteLifecycleRequest"],
        }),
    });
    setBusyQuoteId(null);
    if (result.ok === false) {
      setActionError(
        `Could not ${action.replaceAll("-", " ")} quote ${quote.number}: ${rejectionText(result, "rejected.")}`,
      );
      return;
    }
    await refreshQuotes();
  }

  async function convertQuote(quote: Quote): Promise<void> {
    setActionError(null);
    setBusyQuoteId(quote.id);
    const result = await runEnvelopeMutation({
      describe: "Convert quote to invoice",
      entityId: quote.id,
      buildBody: (base) => ({ ...base }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.POST("/api/v1/quotes/{id}/convert-to-invoice", {
          params: { path: { id: quote.id } },
          body: body as components["schemas"]["QuoteLifecycleRequest"],
        }),
    });
    setBusyQuoteId(null);
    if (result.ok === false) {
      setActionError(`Could not convert quote ${quote.number}: ${rejectionText(result, "rejected.")}`);
      return;
    }
    await refreshQuotes();
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

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
      <header className="m-rise flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Quotes
          </h1>
          <p className="mt-0.5 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Build quotes, walk them through send → decision, convert accepted ones to invoices.
          </p>
        </div>
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
                    <div className="grid gap-2 sm:grid-cols-2">
                      <div className="grid gap-1">
                        <Label>From catalog</Label>
                        <Select
                          value={line.catalog_item_id ?? "none"}
                          onValueChange={(v) => pickCatalogItem(index, v)}
                        >
                          <SelectTrigger data-testid={`quote-line-item-${index}`}>
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
                        <Label>Description</Label>
                        <Input
                          value={line.description ?? ""}
                          onChange={(e) => setLine(index, { description: e.target.value })}
                        />
                      </div>
                    </div>
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
      </header>

      <div className="m-rise flex flex-wrap gap-1.5" role="group" aria-label="Filter by status">
        {STATUSES.map((s) => (
          <button
            key={s}
            type="button"
            className="rounded-full border px-2.5 py-1 text-[11px] font-semibold transition-opacity duration-100 hover:opacity-100"
            style={{
              borderColor: statusFilter === s ? "var(--m-accent)" : "var(--m-border)",
              color: statusFilter === s ? "var(--m-accent)" : "var(--m-text-muted)",
              background: statusFilter === s ? "var(--m-accent-soft, var(--m-bg-subtle))" : "transparent",
              opacity: statusFilter === s ? 1 : 0.85,
            }}
            data-testid={`quotes-filter-${s}`}
            aria-pressed={statusFilter === s}
            onClick={() => setStatusFilter(s)}
          >
            {s}
          </button>
        ))}
      </div>

      {actionError !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="quotes-action-error">
          {actionError}
        </p>
      )}

      <Card className="m-rise overflow-x-auto p-0">
        <Table data-testid="quotes-table">
          <thead>
            <tr>
              <Th>Number</Th>
              <Th>Client</Th>
              <Th>Status</Th>
              <Th className="text-right">Total</Th>
              <Th>Valid until</Th>
              <Th className="text-right">Actions</Th>
            </tr>
          </thead>
          <tbody>
            {filtered.map((q) => {
              const validDays = daysUntil(q.valid_until ?? "");
              return (
                <Tr key={q.id} className="row-hover" data-testid={`quotes-row-${q.id}`}>
                  <Td>
                    <span className="font-mono text-[13px] font-medium">{q.number}</span>
                  </Td>
                  <Td>
                    <Link
                      href={`/clients/${q.client_id}`}
                      className="underline decoration-[var(--m-border)] underline-offset-2"
                      style={{ color: "var(--m-accent)" }}
                    >
                      {(clients ?? []).find((c) => c.id === q.client_id)?.name ?? "Client"}
                    </Link>
                  </Td>
                  <Td>
                    <StatusTag tone={QUOTE_STATUS_TONE[q.status]}>{q.status}</StatusTag>
                    {q.converted_invoice_id != null ? (
                      <Link
                        href={`/billing/${q.converted_invoice_id}`}
                        className="ml-1.5 text-[11px] underline decoration-[var(--m-border)] underline-offset-2"
                        data-testid={`quote-invoice-link-${q.id}`}
                      >
                        invoice →
                      </Link>
                    ) : null}
                  </Td>
                  <Td className="text-right font-mono text-[13px] tabular-nums">{money(q.total_minor, q.currency)}</Td>
                  <Td>
                    <span className="tabular-nums">{q.valid_until}</span>
                    {validDays !== null && validDays < 0 && q.status === "sent" ? (
                      <Tag tone="danger" className="ml-1">past valid-until</Tag>
                    ) : validDays !== null && validDays <= 7 && q.status === "sent" ? (
                      <Tag tone="warning" className="ml-1">expires soon</Tag>
                    ) : null}
                  </Td>
                  <Td>
                    <div className="flex justify-end gap-1">
                      {q.status === "draft" && (
                        <Button size="sm" disabled={busyQuoteId === q.id} onClick={() => void runQuoteAction(q, "send")}>
                          Send
                        </Button>
                      )}
                      {q.status === "sent" && (
                        <>
                          <Button size="sm" disabled={busyQuoteId === q.id} onClick={() => void runQuoteAction(q, "accept")}>
                            Accept
                          </Button>
                          <Button size="sm" variant="secondary" disabled={busyQuoteId === q.id} onClick={() => void runQuoteAction(q, "decline")}>
                            Decline
                          </Button>
                          <Button size="sm" variant="ghost" disabled={busyQuoteId === q.id} onClick={() => void runQuoteAction(q, "expire")}>
                            Expire
                          </Button>
                        </>
                      )}
                      {q.status === "accepted" && (
                        <Button size="sm" disabled={busyQuoteId === q.id} onClick={() => void convertQuote(q)}>
                          Convert to invoice
                        </Button>
                      )}
                      {(q.status === "declined" || q.status === "expired") && (
                        <span style={{ color: "var(--m-text-muted)" }}>Decided</span>
                      )}
                    </div>
                  </Td>
                </Tr>
              );
            })}
            {filtered.length === 0 && (
              <Tr>
                <Td>
                  <EmptyState message="No quotes match this filter yet." />
                </Td>
              </Tr>
            )}
          </tbody>
        </Table>
      </Card>
    </div>
  );
}
