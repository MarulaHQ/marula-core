// TicketMoney — the ticket-detail Money tab (Wave 11 lane 10 close-out).
//
// Purpose: cost lines (parts / flat / other with qty × unit cost, billable
// flags) and expense entries, plus the honest billed lock: a line with
// `invoice_line_id` is read-only (the server refuses billed-line
// mutations with a typed 409) and renders with a lock affordance.
// Totals row sums billable value. Everything is gated behind
// `financial:read` — without it the tab renders a typed
// not-available state, never a dead surface. Edits ride the instant-first
// envelope (optimistic patch + authoritative reconcile).
// Non-purpose: no invoice-run authoring here — the billing side consumes
// GET /invoice-candidates read-only (Billing page).
"use client";

import { rejectionText } from "../lib/error-copy";
import { Icon as KitIcon } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import {
  Badge,
  Button,
  Card,
  EmptyState,
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
} from "@marula-core/ui";
import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/api";
import { runEnvelopeMutation } from "../lib/envelope-write";
import { LocalTime } from "./local-time";
import { money } from "../lib/format";
import { ticketPartDraft } from "../lib/scan/routing";
import { ScanBar } from "./scan/scan-bar";

type TicketCostLine = components["schemas"]["TicketCostLine"];
type TicketExpense = components["schemas"]["TicketExpense"];
type CostLineKind = components["schemas"]["CostLineKind"];

const KINDS: CostLineKind[] = ["parts", "flat", "other"];

function BilledLock() {
  return (
    <span title="Billed on an invoice — read-only" aria-label="Billed, read-only" style={{ color: "var(--m-text-muted)", display: "inline-flex" }}>
      <KitIcon name="lock" size={12} />
    </span>
  );
}

export function TicketMoney({ ticketId }: { ticketId: string }) {
  const [lines, setLines] = useState<TicketCostLine[] | null>(null);
  const [expenses, setExpenses] = useState<TicketExpense[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  // add-cost-line form
  const [clKind, setClKind] = useState<CostLineKind>("parts");
  const [clDesc, setClDesc] = useState("");
  const [clQty, setClQty] = useState("1");
  const [clUnit, setClUnit] = useState("");
  const [clCurrency, setClCurrency] = useState("USD");
  const [clBillable, setClBillable] = useState(true);
  const [clBusy, setClBusy] = useState(false);
  const [clError, setClError] = useState<string | null>(null);
  const [scanNote, setScanNote] = useState<string | null>(null);

  // add-expense form
  const [exDesc, setExDesc] = useState("");
  const [exAmount, setExAmount] = useState("");
  const [exCurrency, setExCurrency] = useState("USD");
  const [exBillable, setExBillable] = useState(true);
  const [exBusy, setExBusy] = useState(false);
  const [exError, setExError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const [cl, ex] = await Promise.all([
      api.api.GET("/api/v1/tickets/{id}/cost-lines", { params: { path: { id: ticketId } } }),
      api.api.GET("/api/v1/tickets/{id}/expenses", { params: { path: { id: ticketId } } }),
    ]);
    if (cl.error === undefined && cl.data !== undefined) setLines(cl.data);
    if (ex.error === undefined && ex.data !== undefined) setExpenses(ex.data);
    if (cl.error !== undefined && ex.error !== undefined) setLoadError("Failed to load money lines.");
  }, [ticketId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const addCostLine = async () => {
    setClError(null);
    const unit = Math.round(Number(clUnit) * 100);
    const qty = Number(clQty);
    if (!Number.isFinite(qty) || qty <= 0) {
      setClError("Quantity must be greater than zero.");
      return;
    }
    if (!Number.isFinite(unit) || unit <= 0) {
      setClError("Unit cost must be greater than zero.");
      return;
    }
    setClBusy(true);
    const before = lines;
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateCostLineRequest"],
      components["schemas"]["CostLineMutationResponse"]
    >({
      describe: "Add cost line",
      entityId: ticketId,
      optimistic: () =>
        setLines((prev) => [
          ...(prev ?? []),
          {
            id: `pending-${Date.now()}`,
            org_id: "",
            ticket_id: ticketId,
            kind: clKind,
            description: clDesc.trim(),
            quantity: qty,
            unit_cost_minor: unit,
            currency: clCurrency.toUpperCase(),
            billable: clBillable,
            invoice_line_id: null,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
            created_by: null,
          } as TicketCostLine,
        ]),
      revert: () => setLines(before),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        kind: clKind,
        description: clDesc.trim(),
        quantity: qty,
        unit_cost_minor: unit,
        currency: clCurrency.toUpperCase(),
        billable: clBillable,
      }),
      request: (client, body) =>
        client.POST("/api/v1/tickets/{id}/cost-lines", { params: { path: { id: ticketId } }, body }),
    });
    setClBusy(false);
    if (!result.ok) {
      setClError(rejectionText(result, "The cost line was rejected."));
      return;
    }
    setClDesc("");
    setClUnit("");
    void refresh();
  };

  // Scan-to-parts (Wave 32 lane 5): a resolved stock item or serialized
  // unit FILLS the cost-line form — description, quantity 1, and the
  // item's last-known unit cost when the caller may read it. Nothing
  // submits: the human reviews and presses Add line.
  const onScanToPart = async (resolution: components["schemas"]["ScanResolutionView"]) => {
    const itemId = resolution.stock_item_id ?? resolution.entity_id;
    let item: components["schemas"]["StockItemView"] | null = null;
    if (itemId != null) {
      const { data } = await api.api.GET("/api/v1/stock-items/{id}", {
        params: { path: { id: itemId } },
      });
      if (data !== undefined) item = data as components["schemas"]["StockItemView"];
    }
    const draft = ticketPartDraft(resolution, item);
    setClKind("parts");
    setClDesc(draft.description);
    setClQty(String(draft.quantity));
    setClUnit(draft.unitCost);
    setClCurrency(draft.currency.toUpperCase());
    setScanNote(draft.note);
    setClError(null);
  };

  const addExpense = async () => {    setExError(null);
    const amount = Math.round(Number(exAmount) * 100);
    if (!Number.isFinite(amount) || amount <= 0) {
      setExError("Amount must be greater than zero.");
      return;
    }
    setExBusy(true);
    const before = expenses;
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateExpenseRequest"],
      components["schemas"]["ExpenseMutationResponse"]
    >({
      describe: "Add expense",
      entityId: ticketId,
      optimistic: () =>
        setExpenses((prev) => [
          ...(prev ?? []),
          {
            id: `pending-${Date.now()}`,
            org_id: "",
            ticket_id: ticketId,
            description: exDesc.trim(),
            amount_minor: amount,
            currency: exCurrency.toUpperCase(),
            billable: exBillable,
            invoice_line_id: null,
            incurred_at: new Date().toISOString(),
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
            incurred_by: null,
          } as TicketExpense,
        ]),
      revert: () => setExpenses(before),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        description: exDesc.trim(),
        amount_minor: amount,
        currency: exCurrency.toUpperCase(),
        billable: exBillable,
      }),
      request: (client, body) =>
        client.POST("/api/v1/tickets/{id}/expenses", { params: { path: { id: ticketId } }, body }),
    });
    setExBusy(false);
    if (!result.ok) {
      setExError(rejectionText(result, "The expense was rejected."));
      return;
    }
    setExDesc("");
    setExAmount("");
    void refresh();
  };

  const setLineBillable = async (line: TicketCostLine, billable: boolean) => {
    if (line.invoice_line_id != null) return;
    const before = lines;
    const result = await runEnvelopeMutation<
      components["schemas"]["UpdateCostLineRequest"],
      components["schemas"]["CostLineMutationResponse"]
    >({
      describe: "Update cost line",
      entityId: line.id,
      optimistic: () => setLines((prev) => (prev ?? []).map((l) => (l.id === line.id ? { ...l, billable } : l))),
      revert: () => setLines(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key, billable }),
      request: (client, body) =>
        client.PATCH("/api/v1/tickets/{id}/cost-lines/{line_id}", {
          params: { path: { id: ticketId, line_id: line.id } },
          body,
        }),
    });
    if (!result.ok) {
      setLoadError(
        result.message?.includes("already billed")
          ? "That line is already billed on an invoice and can no longer change."
          : (rejectionText(result, "The change was rejected.")),
      );
    }
  };

  const setExpenseBillable = async (expense: TicketExpense, billable: boolean) => {
    if (expense.invoice_line_id != null) return;
    const before = expenses;
    const result = await runEnvelopeMutation<
      components["schemas"]["UpdateExpenseRequest"],
      components["schemas"]["ExpenseMutationResponse"]
    >({
      describe: "Update expense",
      entityId: expense.id,
      optimistic: () => setExpenses((prev) => (prev ?? []).map((e) => (e.id === expense.id ? { ...e, billable } : e))),
      revert: () => setExpenses(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key, billable }),
      request: (client, body) =>
        client.PATCH("/api/v1/tickets/{id}/expenses/{expense_id}", {
          params: { path: { id: ticketId, expense_id: expense.id } },
          body,
        }),
    });
    if (!result.ok) {
      setLoadError(
        result.message?.includes("already billed")
          ? "That expense is already billed on an invoice and can no longer change."
          : (rejectionText(result, "The change was rejected.")),
      );
    }
  };

  const removeCostLine = async (line: TicketCostLine) => {
    if (line.invoice_line_id != null) return;
    const before = lines;
    const result = await runEnvelopeMutation<
      components["schemas"]["MutationParams"],
      unknown
    >({
      describe: "Remove cost line",
      entityId: line.id,
      optimistic: () => setLines((prev) => (prev ?? []).filter((l) => l.id !== line.id)),
      revert: () => setLines(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key }),
      request: (client, body) =>
        client.DELETE("/api/v1/tickets/{id}/cost-lines/{line_id}", {
          params: { path: { id: ticketId, line_id: line.id } },
        }) as Promise<{ data?: unknown; error?: unknown; response: Response }>,
    });
    if (!result.ok) {
      setLoadError(rejectionText(result, "The removal was rejected."));
    }
  };

  const removeExpense = async (expense: TicketExpense) => {
    if (expense.invoice_line_id != null) return;
    const before = expenses;
    const result = await runEnvelopeMutation<
      components["schemas"]["MutationParams"],
      unknown
    >({
      describe: "Remove expense",
      entityId: expense.id,
      optimistic: () => setExpenses((prev) => (prev ?? []).filter((e) => e.id !== expense.id)),
      revert: () => setExpenses(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key }),
      request: (client, body) =>
        client.DELETE("/api/v1/tickets/{id}/expenses/{expense_id}", {
          params: { path: { id: ticketId, expense_id: expense.id } },
        }) as Promise<{ data?: unknown; error?: unknown; response: Response }>,
    });
    if (!result.ok) {
      setLoadError(rejectionText(result, "The removal was rejected."));
    }
  };

  const billableTotal = (lines ?? [])
    .filter((l) => l.billable)
    .reduce((sum, l) => sum + Math.round(l.quantity * l.unit_cost_minor), 0) +
    (expenses ?? []).filter((e) => e.billable).reduce((sum, e) => sum + e.amount_minor, 0);
  const totalCurrency = lines?.[0]?.currency ?? expenses?.[0]?.currency ?? "USD";

  if (loadError !== null && lines === null && expenses === null) {
    return <p role="alert" className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>{loadError}</p>;
  }

  return (
    <div className="grid content-start gap-3" data-testid="money-panel">
      <Card>
        <h3 className="p-4 pb-2 text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
          Cost lines
        </h3>
        {lines === null ? (
          <div className="flex justify-center p-4">
            <Spinner />
          </div>
        ) : lines.length === 0 ? (
          <div className="px-4 pb-2">
            <EmptyState message="No cost lines. Parts, flat fees and other charges land here." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Kind</Th>
                <Th>Description</Th>
                <Th className="text-right">Qty</Th>
                <Th className="text-right">Unit</Th>
                <Th className="text-right">Total</Th>
                <Th>Billable</Th>
                <Th />
              </tr>
            </thead>
            <tbody>
              {lines.map((line) => {
                const billed = line.invoice_line_id != null;
                return (
                  <Tr key={line.id}>
                    <Td><Badge tone={line.kind === "parts" ? "neutral" : line.kind === "flat" ? "success" : "warning"}>{line.kind}</Badge></Td>
                    <Td className="text-xs">
                      <span className="inline-flex items-center gap-1">
                        {billed ? <BilledLock /> : null}
                        {line.description}
                      </span>
                    </Td>
                    <Td className="text-right text-xs tabular-nums">{line.quantity}</Td>
                    <Td className="text-right text-xs tabular-nums">{money(line.unit_cost_minor, line.currency)}</Td>
                    <Td className="text-right text-xs tabular-nums font-medium">{money(Math.round(line.quantity * line.unit_cost_minor), line.currency)}</Td>
                    <Td>
                      <input
                        type="checkbox"
                        aria-label={`Billable: ${line.description}`}
                        checked={line.billable}
                        disabled={billed}
                        onChange={(e) => void setLineBillable(line, e.target.checked)}
                      />
                    </Td>
                    <Td className="text-right">
                      {!billed ? (
                        <Button variant="ghost" size="sm" onClick={() => void removeCostLine(line)}>
                          Remove
                        </Button>
                      ) : null}
                    </Td>
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}
        <div className="px-4 pb-2">
          <ScanBar
            surface="ticket-parts"
            hint="Scan a part or a serialized unit to fill the line below — review it, then add it."
            onScan={({ resolution }) => void onScanToPart(resolution)}
            onRejected={(message) => setScanNote(message)}
          />
          {scanNote !== null ? (
            <p
              className="mt-1 text-[11px]"
              data-testid="ticket-parts-scan-note"
              style={{ color: "var(--m-text-secondary)" }}
            >
              {scanNote}
            </p>
          ) : null}
        </div>
        <form
          className="flex flex-wrap items-end gap-2 p-4"
          onSubmit={(e) => {
            e.preventDefault();
            void addCostLine();
          }}
        >
          <div className="w-24">
            <Label htmlFor="cl-kind">Kind</Label>
            <Select value={clKind} onValueChange={(v) => setClKind(v as CostLineKind)}>
              <SelectTrigger id="cl-kind" className="w-full"><SelectValue /></SelectTrigger>
              <SelectContent>
                {KINDS.map((k) => (
                  <SelectItem key={k} value={k}>{k}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="min-w-40 flex-1">
            <Label htmlFor="cl-desc">Description</Label>
            <Input id="cl-desc" value={clDesc} onChange={(e) => setClDesc(e.target.value)} placeholder="SSD 1TB replacement" />
          </div>
          <div className="w-16">
            <Label htmlFor="cl-qty">Qty</Label>
            <Input id="cl-qty" type="number" min="0" step="0.5" value={clQty} onChange={(e) => setClQty(e.target.value)} />
          </div>
          <div className="w-24">
            <Label htmlFor="cl-unit">Unit cost</Label>
            <Input id="cl-unit" type="number" min="0" step="0.01" value={clUnit} onChange={(e) => setClUnit(e.target.value)} />
          </div>
          <div className="w-20">
            <Label htmlFor="cl-cur">Cur.</Label>
            <Input id="cl-cur" maxLength={3} value={clCurrency} onChange={(e) => setClCurrency(e.target.value.toUpperCase())} />
          </div>
          <label className="flex items-center gap-1 pb-1 text-xs">
            <input type="checkbox" checked={clBillable} onChange={(e) => setClBillable(e.target.checked)} />
            Billable
          </label>
          <Button type="submit" size="sm" disabled={clBusy || clDesc.trim() === ""}>
            {clBusy ? <Spinner /> : null} Add line
          </Button>
          {clError !== null ? (
            <p role="alert" className="w-full text-xs" style={{ color: "var(--m-status-danger-fg)" }}>{clError}</p>
          ) : null}
        </form>
      </Card>

      <Card>
        <h3 className="p-4 pb-2 text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
          Expenses
        </h3>
        {expenses === null ? (
          <div className="flex justify-center p-4">
            <Spinner />
          </div>
        ) : expenses.length === 0 ? (
          <div className="px-4 pb-2">
            <EmptyState message="No expenses recorded." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Description</Th>
                <Th>Incurred</Th>
                <Th className="text-right">Amount</Th>
                <Th>Billable</Th>
                <Th />
              </tr>
            </thead>
            <tbody>
              {expenses.map((expense) => {
                const billed = expense.invoice_line_id != null;
                return (
                  <Tr key={expense.id}>
                    <Td className="text-xs">
                      <span className="inline-flex items-center gap-1">
                        {billed ? <BilledLock /> : null}
                        {expense.description}
                      </span>
                    </Td>
                    <Td className="text-xs"><LocalTime value={expense.incurred_at} kind="date" /></Td>
                    <Td className="text-right text-xs tabular-nums">{money(expense.amount_minor, expense.currency)}</Td>
                    <Td>
                      <input
                        type="checkbox"
                        aria-label={`Billable: ${expense.description}`}
                        checked={expense.billable}
                        disabled={billed}
                        onChange={(e) => void setExpenseBillable(expense, e.target.checked)}
                      />
                    </Td>
                    <Td className="text-right">
                      {!billed ? (
                        <Button variant="ghost" size="sm" onClick={() => void removeExpense(expense)}>
                          Remove
                        </Button>
                      ) : null}
                    </Td>
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}
        <form
          className="flex flex-wrap items-end gap-2 p-4"
          onSubmit={(e) => {
            e.preventDefault();
            void addExpense();
          }}
        >
          <div className="min-w-40 flex-1">
            <Label htmlFor="ex-desc">Description</Label>
            <Input id="ex-desc" value={exDesc} onChange={(e) => setExDesc(e.target.value)} placeholder="Parking at client site" />
          </div>
          <div className="w-24">
            <Label htmlFor="ex-amount">Amount</Label>
            <Input id="ex-amount" type="number" min="0" step="0.01" value={exAmount} onChange={(e) => setExAmount(e.target.value)} />
          </div>
          <div className="w-20">
            <Label htmlFor="ex-cur">Cur.</Label>
            <Input id="ex-cur" maxLength={3} value={exCurrency} onChange={(e) => setExCurrency(e.target.value.toUpperCase())} />
          </div>
          <label className="flex items-center gap-1 pb-1 text-xs">
            <input type="checkbox" checked={exBillable} onChange={(e) => setExBillable(e.target.checked)} />
            Billable
          </label>
          <Button type="submit" size="sm" disabled={exBusy || exDesc.trim() === ""}>
            {exBusy ? <Spinner /> : null} Add expense
          </Button>
          {exError !== null ? (
            <p role="alert" className="w-full text-xs" style={{ color: "var(--m-status-danger-fg)" }}>{exError}</p>
          ) : null}
        </form>
      </Card>

      <p className="text-xs" style={{ color: "var(--m-text-secondary)" }} data-testid="money-total">
        Billable total: <strong>{money(billableTotal, totalCurrency)}</strong>
        {lines !== null && lines.some((l) => l.invoice_line_id != null) ? (
          <span className="ml-2" style={{ color: "var(--m-text-muted)" }}>
            Billed lines are locked read-only.
          </span>
        ) : null}
      </p>
    </div>
  );
}
