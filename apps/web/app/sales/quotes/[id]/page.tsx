// /sales/quotes/[id] — the quote document and its editor.
//
// Derivation: the pack's quoting surface, state Q13 (`DESIGN-NOTES-quoting.md`),
// the quote editor built by hand. This app has no AI grant on quotes, so the
// editor draws exactly what Q13 draws and nothing the granted editor adds:
// no provenance dots, no assumed-value highlighting, no source tabs, no slim
// change bar, no needs-a-look list. The summary column is the summary. Add
// item and From the catalogue are the manual controls and are always there.
//
// Why this page exists at all: canonical Q01 ends its ledger at STATUS and
// draws no per-row actions. The lifecycle legs (send, accept, decline, expire,
// convert) belong to the document, not the row, so they moved here and the
// ledger lost its Actions column.
//
// Non-purpose: no AI describe-it panel, no clause library, no contract editor.
// Editing is limited to the lines and terms a draft may carry — the API
// refuses a lifecycle leg on anything but a draft with a 409, which the page
// reports in words rather than swallowing.
"use client";

import { rejectionText } from "../../../../lib/error-copy";
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
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../../../lib/api";
import { runEnvelopeMutation } from "../../../../lib/envelope-write";
import { money } from "../../../../lib/format";
import { formatQtyMilli } from "../../../../lib/stock-format";
import { daysUntil } from "../../../../lib/money-summary";
import { quotePill } from "../../../../lib/money-ui";
import { MoneyCard, MoneyHeader, MoneyKv, MoneyPillTag } from "../../../../components/money";

type Quote = components["schemas"]["Quote"];
type QuoteStatus = components["schemas"]["QuoteStatus"];
type QuoteLineInput = components["schemas"]["QuoteLineInput"];
type CatalogItem = components["schemas"]["CatalogItemView"];
type Client = components["schemas"]["Client"];

type QuoteAction = "send" | "accept" | "decline" | "expire";

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

function lineAmount(line: QuoteLineInput): number {
  return Math.round(((line.quantity_milli ?? 0) / 1000) * (line.unit_amount_minor ?? 0));
}

export default function QuoteDetailPage(): ReactNode {
  const params = useParams<{ id: string }>();
  const quoteId = params.id;
  const auth = useRequireAuth();

  const [quote, setQuote] = useState<components["schemas"]["QuoteDetail"] | null>(null);
  const [catalog, setCatalog] = useState<CatalogItem[] | null>(null);
  const [clients, setClients] = useState<Client[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [savedNote, setSavedNote] = useState<string | null>(null);

  /** The editable lines. A quote that has left the draft status keeps its
   *  stored lines and simply has nothing to add. */
  const [lines, setLines] = useState<QuoteLineInput[] | null>(null);
  const [validUntil, setValidUntil] = useState("");
  const [taxRateBps, setTaxRateBps] = useState("0");

  const refresh = useCallback(async () => {
    const res = await api.api.GET("/api/v1/quotes/{id}", { params: { path: { id: quoteId } } });
    if (res.data === undefined) {
      setLoadError("Could not load this quote — try reloading.");
      return;
    }
    setQuote(res.data);
    setValidUntil(res.data.valid_until ?? isoDay(30));
    setTaxRateBps(String(res.data.tax_rate_bps ?? 0));
    setLines(
      res.data.lines.map((l) => ({
        catalog_item_id: l.catalog_item_id ?? null,
        description: l.description,
        unit: l.unit,
        quantity_milli: l.quantity_milli,
        unit_amount_minor: l.unit_amount_minor,
        taxable: l.taxable,
      })),
    );
    setLoadError(null);
  }, [quoteId]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
    void (async () => {
      const [catalogRes, clientRes] = await Promise.all([
        api.api.GET("/api/v1/catalog-items"),
        api.api.GET("/api/v1/clients"),
      ]);
      setCatalog(catalogRes.data ?? []);
      setClients(clientRes.data ?? []);
    })();
  }, [auth.status, refresh]);

  const clientName = useCallback(
    (id: string) => clients.find((c) => c.id === id)?.name ?? "Client",
    [clients],
  );

  /** The app computes every figure. The API recomputes the stored ones; these
   *  are the ones on screen while a draft is being edited. */
  const totals = useMemo(() => {
    if (lines === null) return null;
    let subtotalMinor = 0;
    let taxableMinor = 0;
    for (const line of lines) {
      const amount = lineAmount(line);
      subtotalMinor += amount;
      if (line.taxable === true) taxableMinor += amount;
    }
    const rate = Math.max(0, Number(taxRateBps) || 0);
    const taxMinor = Math.round((taxableMinor * rate) / 10000);
    return { subtotalMinor, taxMinor, totalMinor: subtotalMinor + taxMinor };
  }, [lines, taxRateBps]);

  if (quote === null && loadError === null) return <Spinner />;
  if (loadError !== null || quote === null || lines === null) {
    return <EmptyState message={loadError ?? "Could not load this quote."} />;
  }

  const isDraft = quote.status === "draft";
  const validDays = daysUntil(quote.valid_until ?? "");

  /** A lifecycle leg. Conversion is its own call so the two response shapes
   *  never widen one generic. */
  async function runAction(action: QuoteAction): Promise<void> {
    if (quote === null) return;
    const doc = quote;
    setActionError(null);
    setSavedNote(null);
    setBusy(true);
    const result = await runEnvelopeMutation({
      describe: `Quote ${action}`,
      entityId: doc.id,
      buildBody: (base) => ({ ...base }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.POST(`/api/v1/quotes/{id}/${action}`, {
          params: { path: { id: doc.id } },
          body: body as components["schemas"]["QuoteLifecycleRequest"],
        }),
    });
    setBusy(false);
    if (result.ok === false) {
      setActionError(
        `Could not ${action.replaceAll("-", " ")} quote ${doc.number}: ${rejectionText(result, "rejected.")}`,
      );
      return;
    }
    await refresh();
  }

  async function convertQuote(): Promise<void> {
    if (quote === null) return;
    const doc = quote;
    setActionError(null);
    setSavedNote(null);
    setBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Convert quote to invoice",
      entityId: doc.id,
      buildBody: (base) => ({ ...base }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.POST("/api/v1/quotes/{id}/convert-to-invoice", {
          params: { path: { id: doc.id } },
          body: body as components["schemas"]["QuoteLifecycleRequest"],
        }),
    });
    setBusy(false);
    if (result.ok === false) {
      setActionError(`Could not convert quote ${doc.number}: ${rejectionText(result, "rejected.")}`);
      return;
    }
    setSavedNote(`Quote ${doc.number} converted to an invoice.`);
    await refresh();
  }

  async function saveDraft(): Promise<void> {
    if (quote === null || lines === null) return;
    const doc = quote;
    const current = lines;
    setActionError(null);
    setSavedNote(null);
    const validLines = current.filter(
      (l) => (l.description ?? "").trim() !== "" && (l.quantity_milli ?? 0) > 0,
    );
    if (validLines.length === 0) {
      setActionError("A quote needs at least one line with a description and a quantity.");
      return;
    }
    setBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Save quote lines",
      entityId: doc.id,
      buildBody: (base) => ({
        ...base,
        lines: validLines,
        valid_until: validUntil,
        tax_rate_bps: Math.max(0, Number(taxRateBps) || 0),
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.PATCH("/api/v1/quotes/{id}", {
          params: { path: { id: doc.id } },
          body: body as components["schemas"]["UpdateQuoteRequest"],
        }),
    });
    setBusy(false);
    if (result.ok === false) {
      setActionError(`Could not save ${doc.number}: ${rejectionText(result, "rejected.")}`);
      return;
    }
    setSavedNote("Saved.");
    await refresh();
  }

  return (
    <div className="m-money-page mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
      <nav className="m-money-crumbs" aria-label="Breadcrumb">
        <Link href="/sales/quotes">Quotes</Link>
        <span aria-hidden="true">›</span>
        <span>{quote.number}</span>
      </nav>

      <MoneyHeader
        title={quote.number}
        eyebrow="Billing"
        stats={
          <>
            <MoneyPillTag pill={quotePill(quote.status)} testId="quote-detail-status" />
            {/* A quote past its date is a fact about money, not a breach:
                blush, and with the days in words. */}
            {quote.status !== "declined" && quote.status !== "expired" && validDays !== null && validDays < 0 ? (
              <span className="m-money-pill m-money-pill-blush">past valid-until</span>
            ) : null}
          </>
        }
        actions={
          /* In order of risk, as on the invoice detail: a send touches
             nothing, a decline loses the deal, a payment or a conversion
             closes it. */
          <div className="flex flex-wrap items-center gap-2">
            {isDraft ? (
              <Button disabled={busy} onClick={() => void runAction("send")} data-testid="quote-send">
                Send
              </Button>
            ) : null}
            {quote.status === "sent" ? (
              <>
                <Button variant="secondary" disabled={busy} onClick={() => void runAction("decline")}>
                  Decline
                </Button>
                <Button variant="secondary" disabled={busy} onClick={() => void runAction("expire")}>
                  Expire
                </Button>
                <Button disabled={busy} onClick={() => void runAction("accept")} data-testid="quote-accept">
                  Accept
                </Button>
              </>
            ) : null}
            {quote.status === "accepted" ? (
              <Button disabled={busy} onClick={() => void convertQuote()} data-testid="quote-convert">
                Convert to invoice
              </Button>
            ) : null}
            {quote.status === "declined" || quote.status === "expired" ? (
              <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                Decided
              </span>
            ) : null}
          </div>
        }
      />

      {actionError !== null ? (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="quote-detail-error">
          {actionError}
        </p>
      ) : null}
      {savedNote !== null ? (
        <p className="text-[13px]" role="status" style={{ color: "var(--m-text-secondary)" }} data-testid="quote-detail-note">
          {savedNote}
        </p>
      ) : null}

      <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_320px]">
        <div className="grid gap-6">
          <MoneyCard testId="quote-lines">
            <div className="m-money-bar">
              <span>
                <b>{lines.length}</b> {lines.length === 1 ? "line" : "lines"}
              </span>
            </div>
            <div style={{ overflowX: "auto" }}>
              <table className="m-table" data-testid="quote-lines-table">
                <thead>
                  <tr>
                    <th>Description</th>
                    <th className="text-right">Qty</th>
                    <th>Unit</th>
                    <th className="text-right">Rate</th>
                    <th className="text-right">Amount</th>
                  </tr>
                </thead>
                <tbody>
                  {lines.map((line, index) => (
                    <tr key={index}>
                      <td>
                        <span className="text-[13px]">{line.description?.trim() || "—"}</span>
                      </td>
                      <td className="text-right tabular-nums">
                        {formatQtyMilli(line.quantity_milli ?? 0, line.unit ?? "each")}
                      </td>
                      <td>{line.unit ?? "each"}</td>
                      <td className="text-right tabular-nums">
                        {money(line.unit_amount_minor ?? 0, quote.currency)}
                      </td>
                      <td className="text-right tabular-nums">{money(lineAmount(line), quote.currency)}</td>
                    </tr>
                  ))}
                  {lines.length === 0 ? (
                    <tr>
                      <td colSpan={5}>
                        <EmptyState message="This quote has no lines yet. Add one below." />
                      </td>
                    </tr>
                  ) : null}
                </tbody>
              </table>
            </div>
          </MoneyCard>

          {isDraft ? (
            <Card className="grid gap-3 p-4">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <h2 className="text-sm font-semibold">Add to this quote</h2>
                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => setLines((cur) => [...(cur ?? []), { ...EMPTY_LINE, description: "" }])}
                    data-testid="quote-add-item"
                  >
                    Add item
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() =>
                      setLines((cur) => [...(cur ?? []), { ...EMPTY_LINE, description: "" } ])
                    }
                    data-testid="quote-from-catalogue"
                  >
                    From the catalogue
                  </Button>
                </div>
              </div>
              <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                From the catalogue adds a line and fills it from the catalogue below, so the price
                and taxability are the published ones rather than typed.
              </p>

              <div className="grid gap-3">
                {lines.map((line, index) => (
                  <div
                    key={index}
                    className="grid gap-2 rounded-[var(--m-radius-control)] border p-2.5"
                    style={{ borderColor: "var(--m-border)" }}
                    data-testid={`quote-edit-line-${index}`}
                  >
                    <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)_minmax(0,1.3fr)]">
                      <div className="grid gap-1">
                        <Label>From the catalogue</Label>
                        <Select
                          value={line.catalog_item_id ?? "none"}
                          onValueChange={(v) => {
                            const item = (catalog ?? []).find((c) => c.id === v);
                            setLines((cur) =>
                              (cur ?? []).map((l, i) =>
                                i === index
                                  ? {
                                      ...l,
                                      catalog_item_id: v === "none" ? null : v,
                                      description: item?.name ?? l.description,
                                      unit: item?.unit ?? l.unit,
                                      unit_price_minor: item?.unit_price_minor ?? l.unit_amount_minor,
                                      taxable: item?.taxable ?? l.taxable,
                                    }
                                  : l,
                              ),
                            );
                          }}
                        >
                          <SelectTrigger data-testid={`quote-edit-item-${index}`} className="w-full [&>span]:truncate">
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
                        <Label htmlFor={`quote-edit-desc-${index}`}>Description</Label>
                        <Input
                          id={`quote-edit-desc-${index}`}
                          value={line.description ?? ""}
                          onChange={(e) =>
                            setLines((cur) =>
                              (cur ?? []).map((l, i) =>
                                i === index ? { ...l, description: e.target.value } : l,
                              ),
                            )
                          }
                        />
                      </div>
                      <div className="grid gap-1">
                        <Label>Unit</Label>
                        <Input
                          value={line.unit ?? "each"}
                          onChange={(e) =>
                            setLines((cur) =>
                              (cur ?? []).map((l, i) =>
                                i === index ? { ...l, unit: e.target.value || "each" } : l,
                              ),
                            )
                          }
                        />
                      </div>
                    </div>
                    <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                      <div className="grid gap-1">
                        <Label htmlFor={`quote-edit-qty-${index}`}>Quantity</Label>
                        <Input
                          id={`quote-edit-qty-${index}`}
                          inputMode="decimal"
                          value={String((line.quantity_milli ?? 0) / 1000)}
                          onChange={(e) => {
                            const v = Number(e.target.value);
                            setLines((cur) =>
                              (cur ?? []).map((l, i) =>
                                i === index
                                  ? { ...l, quantity_milli: Number.isNaN(v) ? 0 : Math.round(v * 1000) }
                                  : l,
                              ),
                            );
                          }}
                        />
                      </div>
                      <div className="grid gap-1">
                        <Label htmlFor={`quote-edit-price-${index}`}>Unit price (major)</Label>
                        <Input
                          id={`quote-edit-price-${index}`}
                          inputMode="decimal"
                          value={String(((line.unit_amount_minor ?? 0) / 100).toFixed(2))}
                          onChange={(e) => {
                            const v = Number(e.target.value);
                            setLines((cur) =>
                              (cur ?? []).map((l, i) =>
                                i === index
                                  ? { ...l, unit_amount_minor: Number.isNaN(v) ? 0 : Math.round(v * 100) }
                                  : l,
                              ),
                            );
                          }}
                        />
                      </div>
                      <div className="grid gap-1">
                        <Label htmlFor={`quote-edit-tax-${index}`}>Taxable</Label>
                        <Select
                          value={line.taxable === true ? "yes" : "no"}
                          onValueChange={(v) =>
                            setLines((cur) =>
                              (cur ?? []).map((l, i) =>
                                i === index ? { ...l, taxable: v === "yes" } : l,
                              ),
                            )
                          }
                        >
                          <SelectTrigger id={`quote-edit-tax-${index}`}>
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="yes">Taxable</SelectItem>
                            <SelectItem value="no">Not taxable</SelectItem>
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="grid gap-1">
                        <Label>Remove</Label>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setLines((cur) => (cur ?? []).filter((_, i) => i !== index))}
                        >
                          Delete line
                        </Button>
                      </div>
                    </div>
                  </div>
                ))}
              </div>

              <div className="grid gap-3 sm:grid-cols-2">
                <div className="grid gap-1">
                  <Label htmlFor="quote-edit-valid">Valid until</Label>
                  <Input
                    id="quote-edit-valid"
                    type="date"
                    value={validUntil}
                    onChange={(e) => setValidUntil(e.target.value)}
                  />
                </div>
                <div className="grid gap-1">
                  <Label htmlFor="quote-edit-tax">Tax rate (basis points)</Label>
                  <Input
                    id="quote-edit-tax"
                    inputMode="numeric"
                    value={taxRateBps}
                    onChange={(e) => setTaxRateBps(e.target.value.replace(/\D/g, ""))}
                  />
                </div>
              </div>

              <div className="flex flex-wrap items-center gap-2">
                <Button disabled={busy} onClick={() => void saveDraft()} data-testid="quote-save">
                  Save lines
                </Button>
                <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                  Saving replaces the whole line list, in this order.
                </span>
              </div>
            </Card>
          ) : (
            <FieldHint>
              The lines are fixed once the quote leaves draft. Issue a revised quote rather than
              editing a {quote.status} one.
            </FieldHint>
          )}
        </div>

        {/* The summary column is just the summary — the one thing Q13 adds to
            the hand-built editor is nothing at all. */}
        <aside className="grid gap-4">
          <MoneyCard title="Summary">
            <MoneyKv label="Client">{clientName(quote.client_id)}</MoneyKv>
            <MoneyKv label="Currency">{quote.currency}</MoneyKv>
            <MoneyKv label="Subtotal">
              {money(totals?.subtotalMinor ?? quote.subtotal_minor, quote.currency)}
            </MoneyKv>
            <MoneyKv label="Tax">{money(totals?.taxMinor ?? quote.tax_minor, quote.currency)}</MoneyKv>
            <MoneyKv label="Total">
              {money(totals?.totalMinor ?? quote.total_minor, quote.currency)}
            </MoneyKv>
            <MoneyKv label="Valid until">{quote.valid_until}</MoneyKv>
            <MoneyKv label="Updated">
              {new Date(quote.updated_at).toISOString().slice(0, 10)}
            </MoneyKv>
            {quote.notes != null && quote.notes !== "" ? (
              <>
                <p className="m-money-lab" style={{ margin: "12px 0 4px" }}>
                  Notes
                </p>
                <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  {quote.notes}
                </p>
              </>
            ) : null}
          </MoneyCard>

          {quote.terms != null && quote.terms !== "" ? (
            <MoneyCard title="Terms">
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                {quote.terms}
              </p>
            </MoneyCard>
          ) : null}
        </aside>
      </div>
    </div>
  );
}
