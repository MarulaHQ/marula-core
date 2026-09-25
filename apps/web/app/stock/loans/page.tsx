// /stock/loans — the client loan register (Wave 32 lane 4).
//
// Purpose: devices and tools that left stock on a client agreement, with the
// two writes of that flow: issue a loan from stock (a serialized item picks
// the physical unit), and return it with a per-line condition and a
// destination location for the stock-sourced units.
//
// Non-purpose: no tool checkouts (the tools board owns holders, not
// clients), no invoicing or lease billing (the contract/billing surfaces
// own money), no fleet asset editing.
"use client";

import {
  Button,
  Card,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  FieldError,
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
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { rejectionText } from "../../../lib/error-copy";
import { duePhrase, dateInputValue, loanStatusTone } from "../../../lib/stock-format";

type StockItemView = components["schemas"]["StockItemView"];
type StockLocationView = components["schemas"]["StockLocationView"];
type StockSerialView = components["schemas"]["StockSerialView"];
type ClientLoanDetailResponse = components["schemas"]["ClientLoanDetailResponse"];
type ClientLoanLineView = components["schemas"]["ClientLoanLineView"];

const CONDITIONS = [
  { value: "good", label: "Good" },
  { value: "fair", label: "Fair" },
  { value: "damaged", label: "Damaged" },
] as const;

interface LoanItemRow {
  key: number;
  itemId: string;
  serialId: string;
}

function idPrefix(id: string): string {
  return `${id.slice(0, 8)}…`;
}

export default function StockLoansPage(): ReactNode {
  const auth = useRequireAuth();

  const [loans, setLoans] = useState<ClientLoanDetailResponse[] | null>(null);
  const [items, setItems] = useState<StockItemView[]>([]);
  const [locations, setLocations] = useState<StockLocationView[]>([]);
  const [clients, setClients] = useState<{ id: string; name: string }[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const [clientFilter, setClientFilter] = useState("");
  const [statusFilter, setStatusFilter] = useState<"all" | "open" | "returned">("all");
  const [overdueOnly, setOverdueOnly] = useState(false);

  const [issueOpen, setIssueOpen] = useState(false);
  const [issueClientId, setIssueClientId] = useState("");
  const [issueExpected, setIssueExpected] = useState(dateInputValue(7));
  const [issueRows, setIssueRows] = useState<LoanItemRow[]>([{ key: 1, itemId: "", serialId: "" }]);
  const [issueNextKey, setIssueNextKey] = useState(2);
  const [issueSerialOptions, setIssueSerialOptions] = useState<Record<number, StockSerialView[]>>({});
  const [issueAgreement, setIssueAgreement] = useState("");
  const [isLease, setIsLease] = useState(false);
  const [leaseCycle, setLeaseCycle] = useState("monthly");
  const [leaseCost, setLeaseCost] = useState("");
  const [leaseCurrency, setLeaseCurrency] = useState("USD");
  const [issueNotes, setIssueNotes] = useState("");
  const [issueError, setIssueError] = useState<string | null>(null);
  const [issueBusy, setIssueBusy] = useState(false);

  const [returnLoan, setReturnLoan] = useState<ClientLoanDetailResponse | null>(null);
  const [returnConditions, setReturnConditions] = useState<Record<string, string>>({});
  const [returnNotes, setReturnNotes] = useState("");
  const [returnLocationId, setReturnLocationId] = useState("");
  const [returnError, setReturnError] = useState<string | null>(null);
  const [returnBusy, setReturnBusy] = useState(false);

  const refresh = useCallback(async (): Promise<void> => {
    const loansRes = await api.api.GET("/api/v1/client-loans", {
      params: {
        query: {
          status: statusFilter === "all" ? null : statusFilter,
          client_id: clientFilter === "" ? null : clientFilter,
          overdue_only: overdueOnly ? true : null,
          limit: 500,
        },
      },
    });
    if (loansRes.data === undefined) {
      setLoadError("Could not load the loan register — try reloading.");
      return;
    }
    setLoans(loansRes.data);
    const [itemsRes, locationsRes, clientsRes] = await Promise.all([
      api.api.GET("/api/v1/stock-items", { params: { query: { limit: 500 } } }),
      api.api.GET("/api/v1/stock-locations", { params: { query: {} } }),
      api.api.GET("/api/v1/clients"),
    ]);
    setItems(itemsRes.data ?? []);
    setLocations(locationsRes.data ?? []);
    setClients((clientsRes.data ?? []).map((client) => ({ id: client.id, name: client.name })));
  }, [clientFilter, statusFilter, overdueOnly]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
  }, [auth.status, refresh]);

  const clientName = useMemo(() => {
    const map = new Map(clients.map((client) => [client.id, client.name]));
    return (id: string | null | undefined): string => (id == null ? "—" : (map.get(id) ?? idPrefix(id)));
  }, [clients]);

  const itemName = useMemo(() => {
    const map = new Map(items.map((item) => [item.id, item.name]));
    return (id: string | null | undefined): string => (id == null ? "—" : (map.get(id) ?? idPrefix(id)));
  }, [items]);

  const openLines = useMemo(
    () => (returnLoan === null ? [] : returnLoan.lines.filter((line) => line.returned_at == null)),
    [returnLoan],
  );
  const returnNeedsLocation = openLines.some((line) => line.from_stock);

  function summaryOf(detail: ClientLoanDetailResponse): string {
    const count = detail.lines.length;
    const names: string[] = [];
    for (const line of detail.lines) {
      if (line.stock_item_id != null) {
        const name = itemName(line.stock_item_id);
        if (!names.includes(name)) names.push(name);
      }
    }
    const noun = count === 1 ? "item" : "items";
    if (names.length === 0) return `${count} ${noun}`;
    return `${count} ${noun} — ${names.slice(0, 2).join(", ")}${names.length > 2 ? ` +${names.length - 2}` : ""}`;
  }

  function openIssue(): void {
    setIssueClientId(clientFilter);
    setIssueExpected(dateInputValue(7));
    setIssueRows([{ key: 1, itemId: "", serialId: "" }]);
    setIssueNextKey(2);
    setIssueSerialOptions({});
    setIssueAgreement("");
    setIsLease(false);
    setLeaseCycle("monthly");
    setLeaseCost("");
    setLeaseCurrency("USD");
    setIssueNotes("");
    setIssueError(null);
    setIssueOpen(true);
  }

  function loadIssueSerials(row: LoanItemRow, itemId: string): void {
    if (itemId === "") return;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/stock-items/{id}/serials", {
        params: { path: { id: itemId }, query: { status: "in_stock", limit: 100 } },
      });
      setIssueSerialOptions((cur) => ({ ...cur, [row.key]: data ?? [] }));
    })();
  }

  function setIssueRow(key: number, patch: Partial<LoanItemRow>): void {
    setIssueRows((cur) => cur.map((row) => (row.key === key ? { ...row, ...patch } : row)));
  }

  async function submitIssue(): Promise<void> {
    setIssueError(null);
    if (issueClientId === "") {
      setIssueError("Pick a client.");
      return;
    }
    if (issueExpected === "") {
      setIssueError("Pick an expected return date.");
      return;
    }
    const chosen = issueRows.filter((row) => row.itemId !== "");
    if (chosen.length === 0) {
      setIssueError("Add at least one stock item.");
      return;
    }
    const missingSerial = chosen.find((row) => {
      const item = items.find((candidate) => candidate.id === row.itemId);
      return item?.kind === "serialized" && row.serialId === "";
    });
    if (missingSerial !== undefined) {
      setIssueError(`Pick the serial unit for ${itemName(missingSerial.itemId)}.`);
      return;
    }
    const leaseCostMinor =
      isLease && leaseCost.trim() !== "" ? Math.round(Number(leaseCost) * 100) : null;
    if (isLease && leaseCost.trim() !== "" && (leaseCostMinor === null || Number.isNaN(leaseCostMinor))) {
      setIssueError("Lease cost must be a number.");
      return;
    }
    setIssueBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Issue client loan",
      buildBody: (base) => ({
        client_id: issueClientId,
        expected_return_at: issueExpected,
        items: chosen.map((row) => ({
          stock_item_id: row.itemId,
          serial_id: row.serialId === "" ? null : row.serialId,
        })),
        agreement_ref: issueAgreement.trim() === "" ? null : issueAgreement.trim(),
        is_lease: isLease,
        lease_cycle: isLease ? leaseCycle : null,
        lease_cost_minor: leaseCostMinor,
        lease_currency: isLease ? leaseCurrency : null,
        notes: issueNotes.trim() === "" ? null : issueNotes,
        ...base,
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.POST("/api/v1/client-loans", { body: body as components["schemas"]["IssueLoanRequest"] }),
      versions: (data) => [[data.loan.id, data.mutation.version]],
    });
    setIssueBusy(false);
    if (result.ok === false) {
      setIssueError(rejectionText(result, "The loan was refused."));
      return;
    }
    setIssueOpen(false);
    setNotice("Loan issued.");
    await refresh();
  }

  function openReturn(detail: ClientLoanDetailResponse): void {
    setReturnLoan(detail);
    const conditions: Record<string, string> = {};
    for (const line of detail.lines) {
      if (line.returned_at == null) conditions[line.id] = "good";
    }
    setReturnConditions(conditions);
    setReturnNotes("");
    setReturnLocationId("");
    setReturnError(null);
  }

  function lineLabel(line: ClientLoanLineView): string {
    const item = line.stock_item_id == null ? "stock item" : itemName(line.stock_item_id);
    if (line.serial_id == null) return item;
    return `${item} · serial ${idPrefix(line.serial_id)}`;
  }

  async function submitReturn(): Promise<void> {
    const detail = returnLoan;
    if (detail === null) return;
    setReturnError(null);
    if (openLines.length === 0) {
      setReturnError("Every device on this loan is already back.");
      return;
    }
    if (returnNeedsLocation && returnLocationId === "") {
      setReturnError("Pick where the stock-sourced units land.");
      return;
    }
    setReturnBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Return client loan",
      entityId: detail.loan.id,
      buildBody: (base) => ({
        lines: openLines.map((line) => ({
          line_id: line.id,
          return_condition: returnConditions[line.id] ?? null,
        })),
        notes: returnNotes.trim() === "" ? null : returnNotes,
        to_location_id: returnLocationId === "" ? null : returnLocationId,
        ...base,
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.POST("/api/v1/client-loans/{id}/return", {
          params: { path: { id: detail.loan.id } },
          body: body as components["schemas"]["ReturnLoanRequest"],
        }),
      versions: (data) => [[data.loan.id, data.mutation.version]],
    });
    setReturnBusy(false);
    if (result.ok === false) {
      setReturnError(rejectionText(result, "The return was refused."));
      return;
    }
    setReturnLoan(null);
    setNotice("Loan returned.");
    await refresh();
  }

  if (loans === null && loadError === null) return <Spinner />;
  if (loadError !== null) return <EmptyState message={loadError} />;

  const list = loans ?? [];
  const clientLoans = clientFilter === "" ? [] : list.filter((detail) => detail.loan.client_id === clientFilter);
  const clientOpen = clientLoans.filter((detail) => detail.loan.status === "open");
  const clientItemsOut = clientOpen.reduce(
    (sum, detail) => sum + detail.lines.filter((line) => line.returned_at == null).length,
    0,
  );

  return (
    <div className="mx-auto grid w-full max-w-350 gap-6 p-4 md:p-6">
      <header className="m-rise flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Client loans
          </h1>
          <p className="mt-0.5 text-sm" style={{ color: "var(--m-text-secondary)" }}>
            Devices and tools that left stock on a client agreement. Returns land back in a chosen location.
          </p>
        </div>
        <Button onClick={openIssue} data-testid="loans-issue-open">
          Issue loan
        </Button>
      </header>

      <div className="flex flex-wrap items-center gap-2">
        <Select
          value={clientFilter === "" ? "__all" : clientFilter}
          onValueChange={(value) => setClientFilter(value === "__all" ? "" : value)}
        >
          <SelectTrigger aria-label="Filter by client" data-testid="loans-client-filter" style={{ width: 220 }}>
            <SelectValue placeholder="All clients" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__all">All clients</SelectItem>
            {clients.map((client) => (
              <SelectItem key={client.id} value={client.id}>
                {client.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={statusFilter} onValueChange={(value) => setStatusFilter(value as typeof statusFilter)}>
          <SelectTrigger aria-label="Filter by status" data-testid="loans-status-filter" style={{ width: 160 }}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All statuses</SelectItem>
            <SelectItem value="open">Open</SelectItem>
            <SelectItem value="returned">Returned</SelectItem>
          </SelectContent>
        </Select>
        <label className="flex items-center gap-1.5 text-xs" style={{ color: "var(--m-text-secondary)" }}>
          <Checkbox
            checked={overdueOnly}
            onCheckedChange={(value) => setOverdueOnly(value === true)}
            data-testid="loans-overdue-only"
          />
          Overdue only
        </label>
      </div>

      {clientFilter !== "" && (
        <Card className="m-rise flex flex-wrap items-center gap-4 p-4" data-testid="loans-client-banner">
          <div>
            <p className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
              {clientName(clientFilter)}
            </p>
            <p className="mt-0.5 text-xs" style={{ color: "var(--m-text-secondary)" }}>
              {clientOpen.length} open loan{clientOpen.length === 1 ? "" : "s"} · {clientItemsOut} item
              {clientItemsOut === 1 ? "" : "s"} out
            </p>
          </div>
          <Link
            href={`/clients/${clientFilter}`}
            className="text-xs"
            style={{ color: "var(--m-status-info-fg)" }}
          >
            Open client record
          </Link>
        </Card>
      )}

      {actionError !== null && (
        <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="loans-error">
          {actionError}
        </p>
      )}
      {notice !== null && (
        <p className="text-xs" role="status" style={{ color: "var(--m-status-success-fg)" }} data-testid="loans-notice">
          {notice}
        </p>
      )}

      <Card className="m-rise overflow-x-auto p-0">
        <Table data-testid="loans-table">
          <thead>
            <tr>
              <Th>Client</Th>
              <Th>Items</Th>
              <Th>Out</Th>
              <Th>Expected return</Th>
              <Th>State</Th>
              <Th className="text-right">Actions</Th>
            </tr>
          </thead>
          <tbody>
            {list.map((detail) => (
              <Tr key={detail.loan.id} className="row-hover" data-testid={`loans-row-${detail.loan.id}`}>
                <Td>
                  <Link href={`/clients/${detail.loan.client_id}`} style={{ color: "var(--m-text)" }}>
                    {clientName(detail.loan.client_id)}
                  </Link>
                  {detail.loan.is_lease && (
                    <Tag tone="outline" className="ml-1.5 text-xs">
                      lease
                    </Tag>
                  )}
                </Td>
                <Td className="text-xs">{summaryOf(detail)}</Td>
                <Td className="whitespace-nowrap font-mono text-xs">{detail.loan.out_at.slice(0, 10)}</Td>
                <Td className="text-xs">
                  <span className="flex items-center gap-1">
                    {duePhrase(detail.loan.expected_return_at)}
                    {detail.loan.overdue && <StatusTag tone="danger">overdue</StatusTag>}
                  </span>
                </Td>
                <Td>
                  <StatusTag tone={loanStatusTone(detail.loan.status, detail.loan.overdue)}>
                    {detail.loan.status}
                  </StatusTag>
                </Td>
                <Td>
                  <div className="flex justify-end">
                    {detail.loan.status === "open" && (
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => openReturn(detail)}
                        data-testid={`loans-return-${detail.loan.id}`}
                      >
                        Return
                      </Button>
                    )}
                  </div>
                </Td>
              </Tr>
            ))}
            {list.length === 0 && (
              <Tr>
                <Td>
                  <EmptyState message="No loans match these filters. Issue a loan to send stock to a client." />
                </Td>
              </Tr>
            )}
          </tbody>
        </Table>
      </Card>

      <Dialog open={issueOpen} onOpenChange={setIssueOpen}>
        <DialogContent title="Issue loan" data-testid="loans-issue-dialog">
          <div className="grid gap-3 overflow-auto p-1" style={{ maxHeight: "80vh" }}>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <div className="grid gap-1">
                <Label htmlFor="loan-client">Client</Label>
                <Select value={issueClientId} onValueChange={setIssueClientId}>
                  <SelectTrigger id="loan-client" data-testid="loans-issue-client">
                    <SelectValue placeholder="Pick a client" />
                  </SelectTrigger>
                  <SelectContent>
                    {clients.map((client) => (
                      <SelectItem key={client.id} value={client.id}>
                        {client.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-1">
                <Label htmlFor="loan-expected">Expected return</Label>
                <Input
                  id="loan-expected"
                  type="date"
                  value={issueExpected}
                  onChange={(e) => setIssueExpected(e.target.value)}
                  data-testid="loans-issue-expected"
                />
              </div>
            </div>

            <div className="grid gap-2">
              {issueRows.map((row, index) => {
                const item = items.find((candidate) => candidate.id === row.itemId);
                const serialized = item?.kind === "serialized";
                return (
                  <div
                    key={row.key}
                    className="grid gap-2 border p-2"
                    style={{ borderColor: "var(--m-border)" }}
                    data-testid={`loans-issue-item-row-${row.key}`}
                  >
                    <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                      <div className="grid gap-1">
                        <Label htmlFor={`loan-item-${row.key}`}>Stock item {index + 1}</Label>
                        <Select
                          value={row.itemId}
                          onValueChange={(value) => {
                            setIssueRow(row.key, { itemId: value, serialId: "" });
                            loadIssueSerials(row, value);
                          }}
                        >
                          <SelectTrigger id={`loan-item-${row.key}`} data-testid={`loans-issue-item-${row.key}`}>
                            <SelectValue placeholder="Pick a stock item" />
                          </SelectTrigger>
                          <SelectContent>
                            {items.map((candidate) => (
                              <SelectItem key={candidate.id} value={candidate.id}>
                                {candidate.name}
                                {candidate.kind === "serialized" ? " (serialized)" : ""}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                      {serialized && (
                        <div className="grid gap-1">
                          <Label htmlFor={`loan-serial-${row.key}`}>Serial unit</Label>
                          <Select value={row.serialId} onValueChange={(value) => setIssueRow(row.key, { serialId: value })}>
                            <SelectTrigger id={`loan-serial-${row.key}`} data-testid={`loans-issue-serial-${row.key}`}>
                              <SelectValue placeholder="Pick an in-stock unit" />
                            </SelectTrigger>
                            <SelectContent>
                              {(issueSerialOptions[row.key] ?? []).map((serial) => (
                                <SelectItem key={serial.id} value={serial.id}>
                                  {serial.serial_number}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        </div>
                      )}
                    </div>
                    <div className="flex justify-end">
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={issueRows.length === 1}
                        onClick={() => setIssueRows((cur) => cur.filter((candidate) => candidate.key !== row.key))}
                        data-testid={`loans-issue-remove-${row.key}`}
                      >
                        Remove
                      </Button>
                    </div>
                  </div>
                );
              })}
              <Button
                size="sm"
                variant="secondary"
                className="w-fit"
                onClick={() => {
                  setIssueRows((cur) => [...cur, { key: issueNextKey, itemId: "", serialId: "" }]);
                  setIssueNextKey((key) => key + 1);
                }}
                data-testid="loans-issue-add-item"
              >
                Add item
              </Button>
            </div>

            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <div className="grid gap-1">
                <Label htmlFor="loan-agreement">Agreement ref (optional)</Label>
                <Input
                  id="loan-agreement"
                  value={issueAgreement}
                  onChange={(e) => setIssueAgreement(e.target.value)}
                  data-testid="loans-issue-agreement"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="loan-notes">Notes</Label>
                <Input
                  id="loan-notes"
                  value={issueNotes}
                  onChange={(e) => setIssueNotes(e.target.value)}
                  data-testid="loans-issue-notes"
                />
              </div>
            </div>

            <label className="flex items-center gap-2 text-xs" style={{ color: "var(--m-text-secondary)" }}>
              <Checkbox
                checked={isLease}
                onCheckedChange={(value) => setIsLease(value === true)}
                data-testid="loans-issue-lease"
              />
              Lease terms
            </label>
            {isLease && (
              <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
                <div className="grid gap-1">
                  <Label htmlFor="loan-cycle">Cycle</Label>
                  <Select value={leaseCycle} onValueChange={setLeaseCycle}>
                    <SelectTrigger id="loan-cycle" data-testid="loans-issue-lease-cycle">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="monthly">Monthly</SelectItem>
                      <SelectItem value="quarterly">Quarterly</SelectItem>
                      <SelectItem value="annual">Annual</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="grid gap-1">
                  <Label htmlFor="loan-cost">Cost (major)</Label>
                  <Input
                    id="loan-cost"
                    inputMode="decimal"
                    value={leaseCost}
                    onChange={(e) => setLeaseCost(e.target.value)}
                    data-testid="loans-issue-lease-cost"
                  />
                </div>
                <div className="grid gap-1">
                  <Label htmlFor="loan-currency">Currency</Label>
                  <Input
                    id="loan-currency"
                    value={leaseCurrency}
                    maxLength={3}
                    onChange={(e) => setLeaseCurrency(e.target.value.toUpperCase())}
                    data-testid="loans-issue-lease-currency"
                  />
                </div>
              </div>
            )}
            <FieldHint>A serialized stock item needs its physical unit picked before the loan can go out.</FieldHint>

            {issueError !== null && <FieldError data-testid="loans-issue-error">{issueError}</FieldError>}
            <DialogActions>
              <Button variant="secondary" onClick={() => setIssueOpen(false)}>
                Cancel
              </Button>
              <Button disabled={issueBusy} onClick={() => void submitIssue()} data-testid="loans-issue-save">
                {issueBusy ? "Issuing…" : "Issue loan"}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog
        open={returnLoan !== null}
        onOpenChange={(open) => {
          if (!open) setReturnLoan(null);
        }}
      >
        <DialogContent title="Return loan" data-testid="loans-return-dialog">
          <div className="grid gap-3 overflow-auto p-1" style={{ maxHeight: "80vh" }}>
            {returnLoan !== null && (
              <>
                <p className="text-sm" style={{ color: "var(--m-text-secondary)" }}>
                  {clientName(returnLoan.loan.client_id)} · {openLines.length} unit
                  {openLines.length === 1 ? "" : "s"} still out
                </p>
                <div className="grid gap-2">
                  {openLines.map((line) => (
                    <div
                      key={line.id}
                      className="grid gap-1 border p-2"
                      style={{ borderColor: "var(--m-border)" }}
                      data-testid={`loans-return-line-${line.id}`}
                    >
                      <p className="text-sm font-medium" style={{ color: "var(--m-text)" }}>
                        {lineLabel(line)}
                        {line.from_stock && (
                          <Tag tone="outline" className="ml-1.5 text-xs">
                            from stock
                          </Tag>
                        )}
                      </p>
                      <div className="grid gap-1">
                        <Label htmlFor={`return-condition-${line.id}`}>Condition</Label>
                        <Select
                          value={returnConditions[line.id] ?? "good"}
                          onValueChange={(value) =>
                            setReturnConditions((cur) => ({ ...cur, [line.id]: value }))
                          }
                        >
                          <SelectTrigger
                            id={`return-condition-${line.id}`}
                            aria-label={`Condition for ${lineLabel(line)}`}
                            data-testid={`loans-return-condition-${line.id}`}
                          >
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {CONDITIONS.map((condition) => (
                              <SelectItem key={condition.value} value={condition.value}>
                                {condition.label}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                    </div>
                  ))}
                </div>
                <div className="grid gap-1">
                  <Label htmlFor="return-location">
                    Return to location{returnNeedsLocation ? "" : " (optional)"}
                  </Label>
                  <Select value={returnLocationId} onValueChange={setReturnLocationId}>
                    <SelectTrigger id="return-location" data-testid="loans-return-location">
                      <SelectValue placeholder="Pick a location" />
                    </SelectTrigger>
                    <SelectContent>
                      {locations.map((location) => (
                        <SelectItem key={location.id} value={location.id}>
                          {location.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <FieldHint>
                    {returnNeedsLocation
                      ? "Stock-sourced units need a location to land in."
                      : "Only stock-sourced units ride the ledger. A location is optional here."}
                  </FieldHint>
                </div>
                <div className="grid gap-1">
                  <Label htmlFor="return-notes">Notes</Label>
                  <Input
                    id="return-notes"
                    value={returnNotes}
                    onChange={(e) => setReturnNotes(e.target.value)}
                    data-testid="loans-return-notes"
                  />
                </div>
                {returnError !== null && <FieldError data-testid="loans-return-error">{returnError}</FieldError>}
                <DialogActions>
                  <Button variant="secondary" onClick={() => setReturnLoan(null)}>
                    Cancel
                  </Button>
                  <Button disabled={returnBusy || openLines.length === 0} onClick={() => void submitReturn()} data-testid="loans-return-save">
                    {returnBusy ? "Returning…" : "Return loan"}
                  </Button>
                </DialogActions>
              </>
            )}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
