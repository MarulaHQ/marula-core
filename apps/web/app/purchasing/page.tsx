// /purchasing — the purchase-order browser (Wave 32 lane 4).
//
// Purpose: every purchase order in one filterable list with the four numbers
// a buyer checks first — open, awaiting receipt, received this month, active
// vendors — and the one dialog that opens a new order with its lines, then
// lands on the new order's detail page.
//
// Non-purpose: no receiving, bill, deeplink, or line editing here (the order
// detail owns those) and no vendor records (that is /purchasing/vendors).
"use client";

import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
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
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { OrderStatusTag, orderStatusLabel } from "../../components/purchasing/status";
import { LedgerSurface, type LedgerColumn } from "../../components/assets-stock/ledger-surface";
import { LedgerRow } from "../../components/assets-stock/ledger-row";
import { csvFileName, downloadCsv } from "../../components/assets-stock/csv";
import { DueSoonMark, OverdueMark } from "../../components/assets-stock/ledger-marks";
import { api, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { rejectionText } from "../../lib/error-copy";
import { money } from "../../lib/format";
import { parseQtyToMilli } from "../../lib/stock-format";
import type { FilterClause } from "@marula-core/ui";

type PurchaseOrder = components["schemas"]["PurchaseOrder"];
type Vendor = components["schemas"]["Vendor"];
type Project = components["schemas"]["Project"];
type StockItemView = components["schemas"]["StockItemView"];
type OrderLineInput = components["schemas"]["OrderLineInput"];

const PURCHASING_VIEWS = [
  { id: "orders", label: "Orders" },
  { id: "vendors", label: "Vendors" },
];

const PURCHASING_COLUMNS: LedgerColumn[] = [
  { key: "po", label: "PO", required: true },
  { key: "vendor", label: "Vendor" },
  { key: "status", label: "Status" },
  { key: "value", label: "Value", align: "right" },
  { key: "expected", label: "Expected" },
  { key: "currency", label: "Currency", hiddenByDefault: true },
  { key: "reference", label: "Vendor reference", hiddenByDefault: true },
  { key: "actions", label: "Actions", align: "right" },
];

const PURCHASING_FILTERS = [
  { key: "po", label: "PO" },
  { key: "vendor", label: "Vendor" },
  { key: "status", label: "Status" },
  { key: "expected", label: "Expected" },
  { key: "reference", label: "Vendor reference" },
];

/** How many orders the list prices out. Past this the Value column reads "—". */
const VALUE_COLUMN_LIMIT = 25;

/** Receiving is legal while goods can still arrive: sent, not fully received. */
function canReceive(order: PurchaseOrder | undefined): boolean {
  if (order == null) return false;
  return order.status === "ordered" || order.status === "partially_received";
}

/**
 * The Expected column. An order still open past its expected date is a
 * genuine breach of what we told the vendor, so it takes red with the word
 * and an alert icon; an order due inside three days is blush. A closed order
 * is just a date.
 */
function OrderExpected({ order }: { order: PurchaseOrder }): ReactNode {
  if (order.expected_at == null || order.expected_at === "") {
    return <span style={{ color: "var(--m-text-muted)" }}>Not set</span>;
  }
  const closed = order.status === "received" || order.status === "closed" || order.status === "cancelled";
  if (closed) {
    return <span className="font-mono text-xs tabular-nums">{order.expected_at.slice(0, 10)}</span>;
  }
  const now = new Date();
  const at = new Date(`${order.expected_at.slice(0, 10)}T23:59:59`);
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const days = Number.isNaN(at.getTime())
    ? Number.POSITIVE_INFINITY
    : Math.round(
        (new Date(at.getFullYear(), at.getMonth(), at.getDate()).getTime() - startOfToday.getTime()) / 86_400_000,
      );
  if (days < 0) {
    return <OverdueMark label={`${Math.abs(days)}d past expected`} testId={`purchasing-late-${order.id}`} />;
  }
  if (days <= 3) {
    return <DueSoonMark label={days === 0 ? "Due today" : `In ${days}d`} testId={`purchasing-due-${order.id}`} />;
  }
  return <span className="font-mono text-xs tabular-nums">{order.expected_at.slice(0, 10)}</span>;
}

const STATUS_FILTERS = [
  "all",
  "draft",
  "ordered",
  "partially_received",
  "received",
  "closed",
  "cancelled",
] as const;

type StatusFilter = (typeof STATUS_FILTERS)[number];

const STATUS_FILTER_LABELS: Record<StatusFilter, string> = {
  all: "All",
  draft: "Draft",
  ordered: "Ordered",
  partially_received: "Partially received",
  received: "Received",
  closed: "Closed",
  cancelled: "Cancelled",
};

interface DraftLine {
  key: string;
  description: string;
  qty: string;
  unitCost: string;
  stockItemId: string;
}

interface OrderForm {
  vendorId: string;
  currency: string;
  projectId: string;
  expectedAt: string;
  vendorReference: string;
  notes: string;
  lines: DraftLine[];
}

function blankLine(): DraftLine {
  return {
    key: `line-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    description: "",
    qty: "1",
    unitCost: "0",
    stockItemId: "",
  };
}

function emptyForm(currency: string): OrderForm {
  return {
    vendorId: "",
    currency,
    projectId: "",
    expectedAt: "",
    vendorReference: "",
    notes: "",
    lines: [blankLine()],
  };
}

/** Major-unit cost text → minor units; null when it is not a number ≥ 0. */
function toMinor(major: string): number | null {
  const trimmed = major.trim();
  if (trimmed === "") return 0;
  const value = Number(trimmed);
  if (Number.isNaN(value) || value < 0) return null;
  return Math.round(value * 100);
}

export default function PurchasingPage(): ReactNode {
  const auth = useRequireAuth();
  const router = useRouter();
  const [orders, setOrders] = useState<PurchaseOrder[] | null>(null);
  const [totals, setTotals] = useState<Map<string, number>>(new Map());
  const [clauses, setClauses] = useState<FilterClause[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [awaitingOnly, setAwaitingOnly] = useState(false);
  const [pastExpectedOnly, setPastExpectedOnly] = useState(false);
  const [vendors, setVendors] = useState<Vendor[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [stockItems, setStockItems] = useState<StockItemView[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [vendorFilter, setVendorFilter] = useState("all");
  const [query, setQuery] = useState("");

  const [createOpen, setCreateOpen] = useState(false);
  const [form, setForm] = useState<OrderForm>(() => emptyForm("USD"));
  const [formError, setFormError] = useState<string | null>(null);
  const [formBusy, setFormBusy] = useState(false);

  async function refresh(): Promise<void> {
    const [ordersRes, vendorsRes] = await Promise.all([
      api.api.GET("/api/v1/purchase-orders", { params: { query: {} } }),
      api.api.GET("/api/v1/vendors", { params: { query: { include_inactive: true } } }),
    ]);
    if (ordersRes.data === undefined) {
      setLoadError("Could not load purchase orders — try reloading.");
      return;
    }
    const list = ordersRes.data ?? [];
    setOrders(list);
    setVendors(vendorsRes.data ?? []);
    // The pack's Value column needs a per-order total, and the list endpoint
    // carries none — the rollup is computed from an order's lines. So the
    // first page of orders is priced out in one bounded parallel batch and
    // the rest read "—" rather than the column inventing a figure.
    void priceFirstPage(list);
  }

  async function priceFirstPage(list: PurchaseOrder[]): Promise<void> {
    const page = list.slice(0, VALUE_COLUMN_LIMIT);
    const priced = await Promise.all(
      page.map(async (order) => {
        const { data } = await api.api.GET("/api/v1/purchase-orders/{id}", {
          params: { path: { id: order.id } },
        });
        return [order.id, data?.totals?.ordered_minor ?? null] as const;
      }),
    );
    setTotals((cur) => {
      const next = new Map(cur);
      for (const [id, value] of priced) {
        if (value != null) next.set(id, value);
      }
      return next;
    });
  }

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
    void (async () => {
      const [projectsRes, itemsRes] = await Promise.all([
        api.api.GET("/api/v1/projects", { params: { query: { limit: 200 } } }),
        api.api.GET("/api/v1/stock-items", { params: { query: { limit: 500 } } }),
      ]);
      setProjects(projectsRes.data ?? []);
      setStockItems(itemsRes.data ?? []);
    })();
  }, [auth.status]);

  const vendorById = useMemo(() => {
    const map = new Map<string, Vendor>();
    for (const vendor of vendors) map.set(vendor.id, vendor);
    return map;
  }, [vendors]);

  const projectNames = useMemo(() => {
    const map = new Map<string, string>();
    for (const project of projects) map.set(project.id, project.name);
    return map;
  }, [projects]);

  const stats = useMemo(() => {
    const list = orders ?? [];
    const now = new Date();
    const receivedThisMonth = list.filter((order) => {
      if (order.status !== "received" && order.status !== "closed") return false;
      const stamp = new Date(order.updated_at);
      return stamp.getFullYear() === now.getFullYear() && stamp.getMonth() === now.getMonth();
    }).length;
    return {
      open: list.filter(
        (o) => o.status === "draft" || o.status === "ordered" || o.status === "partially_received",
      ).length,
      awaiting: list.filter((o) => o.status === "ordered" || o.status === "partially_received").length,
      receivedThisMonth,
      activeVendors: vendors.filter((v) => v.active).length,
      inactiveVendors: vendors.filter((v) => !v.active).length,
    };
  }, [orders, vendors]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (orders ?? []).filter((order) => {
      if (statusFilter !== "all" && order.status !== statusFilter) return false;
      if (vendorFilter !== "all" && order.vendor_id !== vendorFilter) return false;
      if (q === "") return true;
      const vendor = vendorById.get(order.vendor_id)?.name.toLowerCase() ?? "";
      return (
        order.number.toLowerCase().includes(q) ||
        vendor.includes(q) ||
        (order.vendor_reference ?? "").toLowerCase().includes(q)
      );
    });
  }, [orders, statusFilter, vendorFilter, query, vendorById]);

  function openCreate(): void {
    const firstVendor = vendors.find((v) => v.active) ?? vendors[0];
    setForm(emptyForm(firstVendor?.default_currency ?? "USD"));
    setFormError(null);
    setCreateOpen(true);
  }

  function chooseVendor(vendorId: string): void {
    const vendor = vendorById.get(vendorId);
    setForm((cur) => ({
      ...cur,
      vendorId,
      currency: vendor?.default_currency ?? cur.currency,
    }));
  }

  function updateLine(key: string, patch: Partial<DraftLine>): void {
    setForm((cur) => ({
      ...cur,
      lines: cur.lines.map((line) => (line.key === key ? { ...line, ...patch } : line)),
    }));
  }

  async function createOrder(): Promise<void> {
    setFormError(null);
    if (form.vendorId === "") {
      setFormError("Choose a vendor.");
      return;
    }
    const lines: OrderLineInput[] = [];
    for (const [index, line] of form.lines.entries()) {
      const description = line.description.trim();
      if (description === "") {
        setFormError(`Line ${index + 1} needs a description.`);
        return;
      }
      const qty = parseQtyToMilli(line.qty);
      if (qty === null || qty <= 0) {
        setFormError(`Line ${index + 1} needs a quantity above zero.`);
        return;
      }
      const unitCost = toMinor(line.unitCost);
      if (unitCost === null) {
        setFormError(`Line ${index + 1} needs a unit cost of zero or more.`);
        return;
      }
      lines.push({
        description,
        qty_ordered_milli: qty,
        unit_cost_minor: unitCost,
        position: index,
        stock_item_id: line.stockItemId === "" ? null : line.stockItemId,
      });
    }
    if (lines.length === 0) {
      setFormError("Add at least one line.");
      return;
    }
    setFormBusy(true);
    // The create call navigates to the new order, so there is nothing to patch
    // in the list; the browser reconciles on the next visit.
    const created: { id: string | null } = { id: null };
    const result = await runEnvelopeMutation({
      describe: "Create purchase order",
      buildBody: (base) => ({
        vendor_id: form.vendorId,
        currency: form.currency.trim() === "" ? null : form.currency.trim().toUpperCase(),
        project_id: form.projectId === "" ? null : form.projectId,
        expected_at: form.expectedAt === "" ? null : form.expectedAt,
        vendor_reference:
          form.vendorReference.trim() === "" ? null : form.vendorReference.trim(),
        notes: form.notes.trim() === "" ? null : form.notes,
        lines,
        ...base,
      }),
      optimistic: () => {},
      revert: () => {},
      request: async (client, body) => {
        const response = await client.POST("/api/v1/purchase-orders", { body });
        if (response.data !== undefined) created.id = response.data.order.id;
        return response;
      },
      versions: (response) => [[response.order.id, response.mutation.version]],
    });
    setFormBusy(false);
    if (result.ok === false) {
      setFormError(rejectionText(result, "Could not create the purchase order."));
      return;
    }
    setCreateOpen(false);
    if (created.id !== null) {
      router.push(`/purchasing/${created.id}`);
      return;
    }
    await refresh();
  }

  if (orders === null && loadError === null) return <Spinner />;
  if (loadError !== null) return <EmptyState message={loadError} />;

  const pastExpected = (orders ?? []).filter((order) => {
    if (order.expected_at == null || order.expected_at === "") return false;
    if (order.status === "received" || order.status === "closed" || order.status === "cancelled") return false;
    return order.expected_at.slice(0, 10) < new Date().toISOString().slice(0, 10);
  }).length;

  const rows = (orders ?? []).filter((order) => {
    if (awaitingOnly && !canReceive(order)) return false;
    if (pastExpectedOnly) {
      const open = order.status !== "received" && order.status !== "closed" && order.status !== "cancelled";
      if (!open || order.expected_at == null || order.expected_at.slice(0, 10) >= new Date().toISOString().slice(0, 10)) {
        return false;
      }
    }
    for (const clause of clauses) {
      const value = (
        {
          po: order.number,
          vendor: vendorById.get(order.vendor_id)?.name ?? "",
          status: orderStatusLabel(order.status),
          expected: order.expected_at ?? "",
          reference: order.vendor_reference ?? "",
        }[clause.field] ?? ""
      ).toLowerCase();
      const want = (clause.value ?? "").toLowerCase();
      if (clause.op === "is_set" && value === "") return false;
      if (clause.op === "is_not_set" && value !== "") return false;
      if (clause.op === "contains" && !value.includes(want)) return false;
      if (clause.op === "eq" && value !== want) return false;
      if (clause.op === "ne" && value === want) return false;
    }
    return true;
  });

  return (
    <>
    <LedgerSurface
      surface="purchasing-orders"
      testId="purchasing-ledger"
      title="Purchase orders"
      subtitle="What the shop ordered, what suppliers still owe, and where each order stands."
      shortcuts={[
        {
          key: "awaiting",
          label: "Awaiting receive",
          count: stats.awaiting,
          tone: "blush",
          active: awaitingOnly,
          onClick: () => setAwaitingOnly((v) => !v),
        },
        {
          key: "overdue",
          label: "Past expected",
          count: pastExpected,
          tone: "red",
          active: pastExpectedOnly,
          onClick: () => setPastExpectedOnly((v) => !v),
        },
      ]}
      headerAction={
        <>
          <Button asChild variant="secondary">
            <Link href="/purchasing/vendors" data-testid="purchasing-vendors-link">
              Vendors
            </Link>
          </Button>
          <Button onClick={openCreate} data-testid="purchasing-new">
            New order
          </Button>
        </>
      }
      views={PURCHASING_VIEWS}
      activeView="orders"
      onViewChange={(id) => {
        if (id === "vendors") router.push("/purchasing/vendors");
      }}
      columns={PURCHASING_COLUMNS}
      filterFields={PURCHASING_FILTERS}
      filterClauses={clauses}
      onFilterChange={setClauses}
      rowCount={rows.length}
      rowsShown={rows.length}
      onExport={(request) => {
        const stamp = new Date().toISOString().slice(0, 10);
        downloadCsv(
          csvFileName("purchase-orders", stamp),
          request.columns,
          rows.map((order) => ({
            po: order.number,
            vendor: vendorById.get(order.vendor_id)?.name ?? "",
            status: orderStatusLabel(order.status),
            value: totals.get(order.id) == null ? "" : money(totals.get(order.id) ?? 0, order.currency),
            expected: order.expected_at ?? "",
            currency: order.currency,
            reference: order.vendor_reference ?? "",
          })),
        );
      }}
      selectedIds={selected}
      onToggleRow={(id, isSelected) =>
        setSelected((cur) => (isSelected ? [...cur, id] : cur.filter((row) => row !== id)))
      }
      onToggleAll={(isSelected) => setSelected(isSelected ? rows.map((o) => o.id) : [])}
      allSelected={rows.length > 0 && selected.length === rows.length}
      someSelected={selected.length > 0 && selected.length < rows.length}
      bulkActions={[
        {
          key: "receive",
          label: "Receive",
          disabled: selected.length !== 1 || !canReceive(rows.find((order) => order.id === selected[0])),
          title: "Receive goods against this order",
          onClick: () => {
            const only = rows.find((order) => order.id === selected[0]);
            if (only != null) router.push(`/purchasing/${only.id}?action=receive`);
          },
        },
      ]}
      onClearSelection={() => setSelected([])}
      emptyMessage="No purchase orders match. Raise one with New order."
      errorMessage={loadError}
      keyboardHints="Receive is a row action, not a separate flow"
    >
      {(columns) =>
        rows.map((order) => {
          const total = totals.get(order.id);
          return (
            <LedgerRow
              key={order.id}
              id={order.id}
              selected={selected.includes(order.id)}
              selectable
              onToggle={(rowId, isSelected) =>
                setSelected((cur) => (isSelected ? [...cur, rowId] : cur.filter((id) => id !== rowId)))
              }
              columns={columns}
              selectLabel={`Select order ${order.number}`}
              testId={`purchasing-row-${order.id}`}
              cells={{
                po: (
                  <Link
                    href={`/purchasing/${order.id}`}
                    className="font-mono text-xs font-medium"
                    style={{ color: "var(--m-text)" }}
                    data-testid={`purchasing-open-${order.id}`}
                  >
                    {order.number}
                  </Link>
                ),
                vendor: vendorById.get(order.vendor_id)?.name ?? "—",
                status: <OrderStatusTag status={order.status} />,
                value: (
                  <span className="font-mono tabular-nums">
                    {total == null ? "—" : money(total, order.currency)}
                  </span>
                ),
                expected: <OrderExpected order={order} />,
                currency: <span className="font-mono text-xs">{order.currency}</span>,
                reference: order.vendor_reference ?? "—",
                actions: (
                  <span className="flex justify-end gap-1">
                    {canReceive(order) ? (
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => router.push(`/purchasing/${order.id}?action=receive`)}
                        data-testid={`purchasing-receive-${order.id}`}
                      >
                        Receive
                      </Button>
                    ) : null}
                    <Button asChild size="sm" variant="ghost">
                      <Link href={`/purchasing/${order.id}`}>Open</Link>
                    </Button>
                  </span>
                ),
              }}
            />
          );
        })
      }
    </LedgerSurface>


      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent title="New purchase order">
          <div className="grid max-h-[80vh] gap-3 overflow-auto p-1">
            <div className="grid gap-2 sm:grid-cols-2">
              <div className="grid gap-1">
                <Label htmlFor="purchasing-order-vendor">Vendor</Label>
                <Select value={form.vendorId} onValueChange={chooseVendor}>
                  <SelectTrigger id="purchasing-order-vendor" data-testid="purchasing-order-vendor">
                    <SelectValue placeholder="Choose a vendor" />
                  </SelectTrigger>
                  <SelectContent>
                    {vendors
                      .filter((vendor) => vendor.active)
                      .map((vendor) => (
                        <SelectItem key={vendor.id} value={vendor.id}>
                          {vendor.name}
                        </SelectItem>
                      ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-order-currency">Currency</Label>
                <Input
                  id="purchasing-order-currency"
                  value={form.currency}
                  maxLength={3}
                  onChange={(e) => setForm({ ...form, currency: e.target.value.toUpperCase() })}
                  data-testid="purchasing-order-currency"
                />
                <FieldHint>Defaults to the vendor&apos;s currency.</FieldHint>
              </div>
            </div>

            <div className="grid gap-2 sm:grid-cols-2">
              <div className="grid gap-1">
                <Label htmlFor="purchasing-order-project">Project (optional)</Label>
                <Select
                  value={form.projectId === "" ? "none" : form.projectId}
                  onValueChange={(value) => setForm({ ...form, projectId: value === "none" ? "" : value })}
                >
                  <SelectTrigger id="purchasing-order-project" data-testid="purchasing-order-project">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="none">No project</SelectItem>
                    {projects.map((project) => (
                      <SelectItem key={project.id} value={project.id}>
                        {project.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-order-expected">Expected date</Label>
                <Input
                  id="purchasing-order-expected"
                  type="date"
                  value={form.expectedAt}
                  onChange={(e) => setForm({ ...form, expectedAt: e.target.value })}
                  data-testid="purchasing-order-expected"
                />
              </div>
            </div>

            <div className="grid gap-2 sm:grid-cols-2">
              <div className="grid gap-1">
                <Label htmlFor="purchasing-order-reference">Vendor reference</Label>
                <Input
                  id="purchasing-order-reference"
                  value={form.vendorReference}
                  onChange={(e) => setForm({ ...form, vendorReference: e.target.value })}
                  data-testid="purchasing-order-reference"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-order-notes">Notes</Label>
                <Input
                  id="purchasing-order-notes"
                  value={form.notes}
                  onChange={(e) => setForm({ ...form, notes: e.target.value })}
                  data-testid="purchasing-order-notes"
                />
              </div>
            </div>

            <div className="grid gap-2">
              <div className="flex items-center justify-between">
                <p className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                  Lines
                </p>
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => setForm((cur) => ({ ...cur, lines: [...cur.lines, blankLine()] }))}
                  data-testid="purchasing-add-line"
                >
                  Add line
                </Button>
              </div>
              {form.lines.map((line, index) => {
                const qty = parseQtyToMilli(line.qty);
                const unitCost = toMinor(line.unitCost);
                const lineTotal =
                  qty !== null && unitCost !== null
                    ? money(Math.round((qty / 1000) * unitCost), form.currency.trim() === "" ? "USD" : form.currency)
                    : null;
                return (
                  <div
                    key={line.key}
                    className="grid gap-2 border-t pt-2"
                    style={{ borderColor: "var(--m-border)" }}
                  >
                    <div className="grid gap-1">
                      <Label htmlFor={`purchasing-line-${index}-description`}>Description</Label>
                      <Input
                        id={`purchasing-line-${index}-description`}
                        value={line.description}
                        onChange={(e) => updateLine(line.key, { description: e.target.value })}
                        data-testid={`purchasing-line-${index}-description`}
                      />
                    </div>
                    <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                      <div className="grid gap-1">
                        <Label htmlFor={`purchasing-line-${index}-qty`}>Quantity</Label>
                        <Input
                          id={`purchasing-line-${index}-qty`}
                          inputMode="decimal"
                          value={line.qty}
                          onChange={(e) => updateLine(line.key, { qty: e.target.value })}
                          data-testid={`purchasing-line-${index}-qty`}
                        />
                      </div>
                      <div className="grid gap-1">
                        <Label htmlFor={`purchasing-line-${index}-cost`}>Unit cost</Label>
                        <Input
                          id={`purchasing-line-${index}-cost`}
                          inputMode="decimal"
                          value={line.unitCost}
                          onChange={(e) => updateLine(line.key, { unitCost: e.target.value })}
                          data-testid={`purchasing-line-${index}-cost`}
                        />
                      </div>
                      <div className="grid gap-1 sm:col-span-2">
                        <Label htmlFor={`purchasing-line-${index}-item`}>Stock item (optional)</Label>
                        <Select
                          value={line.stockItemId === "" ? "none" : line.stockItemId}
                          onValueChange={(value) =>
                            updateLine(line.key, { stockItemId: value === "none" ? "" : value })
                          }
                        >
                          <SelectTrigger
                            id={`purchasing-line-${index}-item`}
                            data-testid={`purchasing-line-${index}-item`}
                          >
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="none">No stock item</SelectItem>
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
                    </div>
                    <div className="flex items-center justify-between">
                      <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                        {lineTotal === null ? "Line total: —" : `Line total: ${lineTotal}`}
                      </span>
                      {form.lines.length > 1 && (
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() =>
                            setForm((cur) => ({
                              ...cur,
                              lines: cur.lines.filter((l) => l.key !== line.key),
                            }))
                          }
                          data-testid={`purchasing-line-${index}-remove`}
                        >
                          Remove
                        </Button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>

            {formError !== null && (
              <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="purchasing-order-form-error">
                {formError}
              </p>
            )}
            <DialogActions>
              <Button variant="secondary" onClick={() => setCreateOpen(false)}>
                Cancel
              </Button>
              <Button disabled={formBusy} onClick={() => void createOrder()} data-testid="purchasing-order-create">
                Create order
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
