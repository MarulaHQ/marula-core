// /sales/quotes — quote ledger + the Describe it authoring surface.
//
// Purpose: quotes CRUD end-to-end: the org's quote list (filterable by
// status), and one builder that is used for every way a quote starts — blank,
// from a dictionary of catalogue items, or from words the agent read. The
// lifecycle legs (send, accept, decline, expire, convert) stay on the document
// page; the ledger carries no per-row actions.
//
// The Describe it panel (pack Q01–Q11, Q14) docks on the right at 440px, it
// composes typed, spoken and uploaded sources, and it says what it understood
// before it builds. A draft it produces fills this builder with provenance on
// every line: gold for the words it came from, blush for what it assumed, and a
// "things to check" list beside the total. Nothing is saved until the ordinary
// create route runs from the builder, which re-validates.
//
// Non-purpose: no invoice editing (convert hands the result to the billing
// ledger), and no AI trace at all without the grant — no door, no key, no mark,
// no gap (pack Q12).
"use client";

import { rejectionText } from "../../../lib/error-copy";
import {
  Button,
  Card,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  Dialog,
  DialogContent,
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
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { downloadRowsCsv } from "../../../lib/csv-download";
import { useDataSurfacePrefs, useSavedChoice } from "../../../lib/data-surface-prefs";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { money } from "../../../lib/format";
import { daysUntil, perCurrencySum } from "../../../lib/money-summary";
import { quotePill } from "../../../lib/money-ui";
import {
  proposeQuoteChange,
  type ChangeOperation,
  type MoneySource,
  type QuoteDraft,
  type Understanding,
} from "../../../lib/money-authoring";
import { MoneyCard, MoneyHeader, MoneyPillTag } from "../../../components/money";
import {
  DescribeMoney,
  DescribePanel,
  NewMoneySplit,
  useMoneyAuthoring,
} from "../../../components/money/describe-money";
import {
  ChangeProposalCard,
  QuoteContextColumn,
  QuoteItemsTable,
  QuoteNotesTab,
  QuoteSummary,
  type BuilderTab,
  type EditableLine,
} from "../../../components/money/quote-draft-builder";
import { proposedLineIndexes, quoteTotals } from "../../../components/money/draft-math";
import { lineNumbers } from "../../../components/money/draft-provenance";
import { formatQtyMilli } from "../../../lib/stock-format";

type Quote = components["schemas"]["Quote"];
type QuoteStatus = components["schemas"]["QuoteStatus"];
type QuoteLineInput = components["schemas"]["QuoteLineInput"];
type CatalogItem = components["schemas"]["CatalogItemView"];
type Client = components["schemas"]["Client"];
type StockItemView = components["schemas"]["StockItemView"];
type OnHandRow = components["schemas"]["OnHandRow"];

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

function clausesFromSaved(
  saved: { field: string; op: string; value?: string }[],
): FilterClause[] {
  const allowed = new Set(FILTER_FIELDS.map((f) => f.key));
  return saved
    .filter((f) => allowed.has(f.field))
    .map((f) => ({
      id: `${f.field}:${f.op}:${f.value ?? ""}`,
      field: f.field,
      op: f.op as FilterClause["op"],
      value: f.value,
    }));
}

const EMPTY_LINE: EditableLine = {
  description: "",
  quantity_milli: 1000,
  unit_price_minor: 0,
  unit: "each",
  catalog_item_id: null,
  taxable: true,
  optional: false,
  section: null,
  source: "assumed",
  source_span: null,
  quantity_assumed: false,
  price_assumed: false,
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
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const granted = useMoneyAuthoring("quote_authoring");

  // The ledger shell is the shared one, so the quotes ledger answers the pack's
  // money-cluster toolbar rule (views left, filters, Columns/Filter/Export
  // right) exactly as the invoice and credit-note ledgers do.
  const prefs = useDataSurfacePrefs();
  const saved = useMemo(() => prefs.view("quotes"), [prefs]);
  const [statusFilter, setStatusFilter] = useSavedChoice(prefs.ready, saved.view, STATUSES, "all");
  const [filterOpen, setFilterOpen] = useState(false);
  const [clauses, setClauses] = useState<FilterClause[]>([]);
  const savedColumns = saved.columns;
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

  const filtersRestored = useRef(false);
  useEffect(() => {
    if (!prefs.ready || filtersRestored.current) return;
    filtersRestored.current = true;
    const restored = clausesFromSaved(saved.filters);
    if (restored.length > 0) setClauses(restored);
  }, [prefs.ready, saved.filters]);

  // ---- builder state -----------------------------------------------------
  const [createOpen, setCreateOpen] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [createBusy, setCreateBusy] = useState(false);
  const [newClientId, setNewClientId] = useState("");
  const [newCurrency, setNewCurrency] = useState("USD");
  const [newValidUntil, setNewValidUntil] = useState(isoDay(30));
  const [newTaxRateBps, setNewTaxRateBps] = useState("0");
  const [newNotes, setNewNotes] = useState("");
  const [lines, setLines] = useState<EditableLine[]>([{ ...EMPTY_LINE }]);
  const [catalogPick, setCatalogPick] = useState("");

  // The draft's own memory: the words it came from, what to check, and the
  // check step's answers. All of it is in-memory only — nothing is stored.
  const [sources, setSources] = useState<MoneySource[]>([]);
  const [checks, setChecks] = useState<string[]>([]);
  const [understanding, setUnderstanding] = useState<Understanding | null>(null);
  const [budgetMinor, setBudgetMinor] = useState<number | null>(null);
  const [fromNotes, setFromNotes] = useState(false);
  const [builderTab, setBuilderTab] = useState<BuilderTab>("summary");

  // ---- changes by describing (Q11) --------------------------------------
  const [changeInstruction, setChangeInstruction] = useState("");
  const [changeBusy, setChangeBusy] = useState(false);
  const [changeError, setChangeError] = useState<string | null>(null);
  const [changeSentence, setChangeSentence] = useState<string | null>(null);
  const [changeOperations, setChangeOperations] = useState<ChangeOperation[]>([]);
  const [changeDraft, setChangeDraft] = useState<QuoteDraft | null>(null);
  const [changeTotal, setChangeTotal] = useState<number | null>(null);
  /** The draft the proposal was described against; a newer draft makes it stale. */
  const [changeBase, setChangeBase] = useState<string | null>(null);
  const changeRequest = useRef(0);
  const [proposed, setProposed] = useState<Set<number>>(new Set());

  // ---- describe panel ----------------------------------------------------
  const [describeOpen, setDescribeOpen] = useState(false);

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

  const totals = quoteTotals(lines, Math.max(0, Number(newTaxRateBps) || 0));
  const noteNumbers = lineNumbers(lines);

  function setLine(index: number, patch: Partial<EditableLine>): void {
    setLines((cur) =>
      cur.map((l, i) => (i === index ? { ...l, ...patch } : l)),
    );
  }

  function removeLine(index: number): void {
    setLines((cur) => (cur.length === 1 ? [{ ...EMPTY_LINE }] : cur.filter((_, i) => i !== index)));
    setProposed(new Set());
  }

  function appendCatalogLine(itemId: string): void {
    const item = (catalog ?? []).find((c) => c.id === itemId);
    if (item === undefined) return;
    setLines((cur) => [
      ...cur.filter((line) => line.description.trim() !== ""),
      {
        ...EMPTY_LINE,
        description: item.description ?? item.name,
        unit: item.unit,
        unit_price_minor: item.unit_price_minor,
        taxable: item.taxable,
        catalog_item_id: item.id,
        source: "catalogue",
      },
    ]);
  }

  function pickStockItem(itemId: string): void {
    const item = stockItems.find((s) => s.id === itemId);
    if (item === undefined) return;
    const linked = (catalog ?? []).find((c) => c.id === item.catalog_item_id);
    setLines((cur) => [
      ...cur.filter((line) => line.description.trim() !== ""),
      {
        ...EMPTY_LINE,
        description: item.name,
        unit: item.unit,
        unit_price_minor: linked?.unit_price_minor ?? 0,
        taxable: linked?.taxable ?? true,
        catalog_item_id: linked?.id ?? null,
        source: linked !== undefined ? "catalogue" : "assumed",
      },
    ]);
  }

  /** The draft the change endpoint takes: the builder's own lines, as they are. */
  function currentDraft(): QuoteDraft {
    return {
      title: "Quote",
      currency: newCurrency,
      // Calendar days, the same count `isoDay` applies, so a change round trip
      // cannot walk the selected date back a day.
      valid_for_days: Math.max(1, daysUntil(newValidUntil) ?? 30),
      notes: newNotes.trim() === "" ? null : newNotes,
      client_id: newClientId === "" ? null : newClientId,
      client_name: newClientId === "" ? null : clientName(newClientId),
      budget_minor: budgetMinor,
      lines: lines.map((line) => ({
        description: line.description,
        quantity_milli: line.quantity_milli,
        unit_price_minor: line.unit_price_minor,
        unit: line.unit ?? null,
        section: line.section ?? null,
        optional: line.optional === true,
        taxable: line.taxable ?? null,
        catalog_item_id: line.catalog_item_id ?? null,
        source: line.source ?? "assumed",
        source_span: line.source_span ?? null,
        quantity_assumed: line.quantity_assumed === true,
        price_assumed: line.price_assumed === true,
      })),
      assumptions: [],
      understanding,
      checks,
    };
  }

  /** A catalogue item's taxability, or the create payload's default. */
  function taxableOf(catalogItemId: string | null | undefined): boolean {
    if (catalogItemId === null || catalogItemId === undefined) return true;
    return (catalog ?? []).find((item) => item.id === catalogItemId)?.taxable ?? true;
  }

  function applyDraft(next: QuoteDraft): void {
    setNewCurrency(next.currency || "USD");
    setNewValidUntil(isoDay(next.valid_for_days ?? 30));
    setNewNotes(next.notes ?? "");
    // An unnamed client is a choice to make, not the last one still selected.
    setNewClientId(next.client_id ?? "");
    setBudgetMinor(next.budget_minor ?? null);
    setLines(
      next.lines.length === 0
        ? [{ ...EMPTY_LINE }]
        : next.lines.map((line) => ({
            description: line.description,
            quantity_milli: line.quantity_milli,
            unit_price_minor: line.unit_price_minor,
            unit: line.unit ?? "each",
            catalog_item_id: line.catalog_item_id ?? null,
            // A draft that states taxability wins; the catalogue is only the
            // fallback for a draft that left it unset, and the create payload's
            // default is the last resort.
            taxable: line.taxable ?? taxableOf(line.catalog_item_id),
            optional: line.optional === true,
            section: line.section ?? null,
            source: line.source ?? "assumed",
            source_span: line.source_span ?? null,
            quantity_assumed: line.quantity_assumed === true,
            price_assumed: line.price_assumed === true,
          })),
    );
    setProposed(new Set());
  }

  // A proposal is only good for the draft it was described against: the
  // fingerprint is that draft's content, so editing a price, a quantity or a
  // note after a proposal arrives makes the proposal stale instead of letting
  // Apply undo the edit.
  const draftKey = JSON.stringify(currentDraft());
  const changeStale = changeDraft !== null && changeBase !== null && changeBase !== draftKey;
  const draftKeyRef = useRef(draftKey);
  useEffect(() => {
    draftKeyRef.current = draftKey;
  }, [draftKey]);

  async function proposeChange(): Promise<void> {
    const requestId = ++changeRequest.current;
    const base = draftKey;
    setChangeBusy(true);
    setChangeError(null);
    const outcome = await proposeQuoteChange(currentDraft(), changeInstruction.trim());
    if (changeRequest.current !== requestId) return;
    setChangeBusy(false);
    if (!outcome.ok) {
      setChangeError(outcome.message);
      return;
    }
    if (!outcome.result.valid || outcome.result.draft === null || outcome.result.draft === undefined) {
      setChangeError(
        outcome.result.issues[0]?.message ?? "I could not read a change in that sentence.",
      );
      return;
    }
    if (draftKeyRef.current !== base) {
      setChangeError("The draft changed while I read that. Describe the change again.");
      return;
    }
    setChangeSentence(outcome.result.sentence);
    setChangeOperations(outcome.result.operations);
    setChangeDraft(outcome.result.draft);
    setChangeTotal(outcome.result.resulting_total_minor);
    setChangeBase(base);
    setProposed(proposedLineIndexes(outcome.result.operations.map((o) => o.path)));
  }

  /** Forget the proposal: its draft, its rows and the base it was read from. */
  function resetChange(): void {
    setChangeInstruction("");
    setChangeOperations([]);
    setChangeSentence(null);
    setChangeDraft(null);
    setChangeTotal(null);
    setChangeBase(null);
    setProposed(new Set());
  }

  function applyChange(): void {
    // The server returns the resulting draft, which is the authority for every
    // field a change may touch; the client never re-applies operations itself.
    // A proposal read against an older draft would replace newer edits, so it
    // is refused rather than applied.
    if (changeDraft === null || changeDraft === undefined || changeStale) return;
    applyDraft(changeDraft);
    resetChange();
  }

  // ---- create ------------------------------------------------------------
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
    // Optional lines are priced and shown beside the total, never inside it.
    // The create route carries the flag, so they persist as optional rows
    // instead of being dropped from the quote.
    const validLines: QuoteLineInput[] = lines
      .filter((l) => l.description.trim() !== "" && l.quantity_milli > 0)
      .map((line) => ({
        description: line.description.trim(),
        catalog_item_id: line.catalog_item_id ?? null,
        unit: line.unit ?? "each",
        quantity_milli: line.quantity_milli,
        unit_amount_minor: line.unit_price_minor,
        taxable: line.taxable ?? true,
        optional: line.optional === true,
      }));
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
            subtotal_minor: totals.subtotalMinor,
            tax_rate_bps: Math.max(0, Number(newTaxRateBps) || 0),
            tax_minor: totals.taxMinor,
            total_minor: totals.totalMinor,
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
    setLines([{ ...EMPTY_LINE }]);
    setNewNotes("");
    setNewTaxRateBps("0");
    setFromNotes(false);
    setChecks([]);
    setUnderstanding(null);
    setSources([]);
    resetChange();
    await refreshQuotes();
  }

  function openBlank(): void {
    setLines([{ ...EMPTY_LINE }]);
    setNewNotes("");
    setChecks([]);
    setUnderstanding(null);
    setBudgetMinor(null);
    setFromNotes(false);
    setSources([]);
    setCreateError(null);
    resetChange();
    setBuilderTab("summary");
    setCreateOpen(true);
  }

  // ---- keyboard: D opens the panel, or the change composer ---------------
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key.toLowerCase() !== "d") return;
      if (!granted) return;
      const target = event.target as HTMLElement | null;
      if (target !== null && /^(input|textarea|select)$/i.test(target.tagName)) return;
      if (createOpen) {
        setBuilderTab("describe");
        document.getElementById("change-composer")?.focus();
        return;
      }
      setDescribeOpen(true);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [createOpen, granted]);

  if (quotes === null && loadError === null) {
    return <Spinner />;
  }
  if (loadError !== null) {
    return <EmptyState message={loadError} />;
  }

  const ledgerTotal = perCurrencySum(
    filteredByClause.map((q) => ({ currency: q.currency as string, amount: q.total_minor })),
  );

  return (
    <div className="m-dsc-shell">
      <div className="m-dsc-main">
        <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
          <MoneyHeader
            title="Quotes"
            eyebrow="Billing"
            stats={
              <span className="m-money-count" data-testid="quotes-count">
                <span className="m-money-count-dot" aria-hidden="true" />
                {(quotes ?? []).length}
              </span>
            }
            actions={
              <>
                <DescribeMoney
                  kind="quote"
                  available={granted}
                  onOpen={() => setDescribeOpen(true)}
                  testId="quotes-describe"
                />
                <NewMoneySplit
                  label="New quote"
                  menuLabel="More ways to start a quote"
                  testId="quotes-new"
                  showDescribe={granted}
                  onBlank={openBlank}
                  onDescribe={() => setDescribeOpen(true)}
                />
              </>
            }
          />

          <DataToolbar
            label="Quote ledger controls"
            views={
              <ViewSwitch
                label="Quote lifecycle"
                activeId={statusFilter}
                onChange={(id) => {
                  setStatusFilter(id as QuoteStatus | "all");
                  void prefs.save("quotes", {
                    ...saved,
                    view: id,
                    columns: columns.map((c) => ({ key: c.key, visible: c.visible })),
                    filters: clauses,
                  });
                }}
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
            chips={
              <FilterChips
                fields={FILTER_FIELDS}
                clauses={clauses}
                onChange={(next) => {
                  setClauses(next);
                  void prefs.save("quotes", {
                    ...saved,
                    view: statusFilter,
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
                  void prefs.save("quotes", {
                    ...saved,
                    view: statusFilter,
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
                    return { key: n.key, label: def?.label ?? n.key, visible: n.visible, required: def?.required };
                  });
                  setColumns(merged);
                  void prefs.save("quotes", {
                    ...saved,
                    view: statusFilter,
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
      </div>

      {granted ? (
        <DescribePanel
          kind="quote"
          open={describeOpen}
          onClose={() => setDescribeOpen(false)}
          clients={(clients ?? []).map((client) => ({ id: client.id, name: client.name }))}
          onBuild={(draft, nextSources, nextKind) => {
            if (nextKind !== "quote" || !("lines" in draft)) return;
            applyDraft(draft);
            setSources(nextSources);
            setChecks(draft.checks ?? []);
            setUnderstanding(draft.understanding ?? null);
            setBudgetMinor(draft.budget_minor ?? null);
            setFromNotes(true);
            setBuilderTab("summary");
            setCreateOpen(true);
          }}
          onReady={() => setDescribeOpen(true)}
          onStartBlank={(nextSources) => {
            openBlank();
            setSources(nextSources);
            setFromNotes(true);
            setBuilderTab(nextSources.length > 0 ? "notes" : "summary");
          }}
          testId="quotes-describe-panel"
        />
      ) : null}

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent title="Quote builder">
          <Card className="grid max-h-[84vh] gap-3 overflow-auto p-4" data-testid="quote-builder">
            <div className="flex flex-wrap items-end gap-3">
              <div className="grid min-w-[180px] flex-1 gap-1">
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
              <div className="grid w-[92px] gap-1">
                <Label htmlFor="quote-currency">Currency</Label>
                <Input
                  id="quote-currency"
                  value={newCurrency}
                  maxLength={3}
                  onChange={(e) => setNewCurrency(e.target.value.toUpperCase())}
                />
              </div>
              <div className="grid w-[150px] gap-1">
                <Label htmlFor="quote-valid">Valid until</Label>
                <Input
                  id="quote-valid"
                  type="date"
                  value={newValidUntil}
                  onChange={(e) => setNewValidUntil(e.target.value)}
                />
              </div>
              <div className="grid w-[110px] gap-1">
                <Label htmlFor="quote-tax">Tax (bps)</Label>
                <Input
                  id="quote-tax"
                  inputMode="numeric"
                  value={newTaxRateBps}
                  onChange={(e) => setNewTaxRateBps(e.target.value)}
                />
                <FieldHint>800 = 8%.</FieldHint>
              </div>
            </div>

            {fromNotes ? (
              <div className="m-dsc-legend" data-testid="quote-from-notes">
                <span>
                  <span className="m-dsc-dot" data-source="notes" aria-hidden="true" /> From your notes
                </span>
                <span>
                  <span className="m-dsc-dot" data-source="assumed" aria-hidden="true" /> Assumed, please check
                </span>
                <span>
                  <span className="m-dsc-dot" data-source="catalogue" aria-hidden="true" /> From the catalogue
                </span>
                <span className="m-dsc-sp" />
                {checks.length > 0 ? (
                  <button
                    type="button"
                    className="m-dsc-option"
                    onClick={() => setBuilderTab("summary")}
                    data-testid="quote-checks-count"
                  >
                    {checks.length} to check
                  </button>
                ) : null}
              </div>
            ) : null}

            <div className="m-dsc-builder">
              <div className="m-dsc-builder-main">
                <div>
                  <Label htmlFor="quote-notes">Scope and notes</Label>
                  <textarea
                    id="quote-notes"
                    className="m-dsc-textarea"
                    style={{ minHeight: 88, border: "1px solid var(--m-input)", borderRadius: "var(--m-radius-control)" }}
                    value={newNotes}
                    onChange={(e) => setNewNotes(e.target.value)}
                    placeholder="What the customer will read on the quote."
                  />
                </div>

                <div className="m-dsc-panel-card">
                  <QuoteItemsTable
                    lines={lines}
                    currency={newCurrency}
                    onChange={(index, patch) => setLine(index, patch)}
                    showProvenance={fromNotes}
                    noteNumbers={noteNumbers}
                    proposed={proposed}
                    editable
                    onRemove={removeLine}
                    testIdPrefix="quote"
                  />
                  <div className="flex flex-wrap items-center gap-2 border-t px-3 py-2" style={{ borderColor: "var(--m-border)" }}>
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => setLines((cur) => [...cur.filter((l) => l.description.trim() !== ""), { ...EMPTY_LINE }])}
                      data-testid="quote-add-item"
                    >
                      Add item
                    </Button>
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => setLines((cur) => cur.filter((_, i) => i !== lines.length - 1))}
                      data-testid="quote-remove-item"
                    >
                      Remove last
                    </Button>
                    <span className="flex-1" />
                    <select
                      className="m-dsc-select"
                      style={{ width: 260 }}
                      value={catalogPick}
                      aria-label="From the catalogue"
                      data-testid="quote-from-catalogue"
                      onChange={(event) => {
                        setCatalogPick("");
                        if (event.target.value !== "") appendCatalogLine(event.target.value);
                      }}
                    >
                      <option value="">From the catalogue…</option>
                      {(catalog ?? []).map((item) => (
                        <option key={item.id} value={item.id}>
                          {item.name} · {money(item.unit_price_minor, item.currency)}
                        </option>
                      ))}
                    </select>
                    <select
                      className="m-dsc-select"
                      style={{ width: 220 }}
                      value=""
                      aria-label="From stock"
                      data-testid="quote-from-stock"
                      onChange={(event) => {
                        if (event.target.value !== "") pickStockItem(event.target.value);
                      }}
                    >
                      <option value="">From stock…</option>
                      {stockItems
                        .filter((item) => item.active)
                        .map((item) => {
                          const onHand = stockOnHand
                            .filter((row) => row.stock_item_id === item.id)
                            .reduce((sum, row) => sum + row.qty_milli, 0);
                          return (
                            <option key={item.id} value={item.id}>
                              {item.name} · {formatQtyMilli(onHand, item.unit)} on hand
                            </option>
                          );
                        })}
                    </select>
                  </div>
                </div>

                {fromNotes ? <ChangeBar onOpen={() => setBuilderTab("describe")} /> : null}
              </div>

              <QuoteContextColumn
                tab={fromNotes ? builderTab : "summary"}
                onTab={setBuilderTab}
                showTabs={fromNotes}
                summary={
                  <QuoteSummary
                    clientName={newClientId === "" ? null : clientName(newClientId)}
                    totalMinor={totals.totalMinor}
                    optionalMinor={totals.optionalMinor}
                    currency={newCurrency}
                    validForDays={daysUntil(newValidUntil)}
                    checks={checks}
                    budgetMinor={budgetMinor}
                    understanding={understanding}
                  />
                }
                notes={
                  <QuoteNotesTab
                    sources={sources}
                    lines={lines}
                    emptyHint="This quote was built by hand. There are no words behind its lines."
                  />
                }
                describe={
                  <ChangeProposalCard
                    instruction={changeInstruction}
                    sentence={changeSentence}
                    operations={changeOperations}
                    resultingTotalMinor={changeTotal}
                    currency={newCurrency}
                    busy={changeBusy}
                    stale={changeStale}
                    error={changeError}
                    onInstruction={setChangeInstruction}
                    onSubmit={() => void proposeChange()}
                    onApply={applyChange}
                    onDismiss={resetChange}
                  />
                }
              />
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
            <div className="flex items-center gap-2">
              <Button variant="secondary" onClick={() => setCreateOpen(false)}>
                Cancel
              </Button>
              <span className="flex-1" />
              <Button disabled={createBusy} onClick={() => void createQuote()} data-testid="quote-save">
                Create quote
              </Button>
            </div>
          </Card>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** The slim bar under the item table: changes stay one move away (key D). */
function ChangeBar({ onOpen }: { onOpen: () => void }): ReactNode {
  return (
    <div className="m-dsc-bar" data-testid="quote-change-bar">
      <span
        className="flex flex-1 items-center gap-2"
        style={{ color: "var(--m-text-secondary)", fontSize: 14 }}
      >
        <Kbd>D</Kbd>
        Change something, or add an item
      </span>
      <button type="button" className="m-dsc-option" aria-label="Describe a change" onClick={onOpen}>
        Describe a change
      </button>
    </div>
  );
}
