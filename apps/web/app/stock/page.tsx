// /stock — the physical-goods surface (Wave 32 lane 4).
//
// Purpose: the stock-on-hand read model made operable: every item with its
// total on hand, a low-stock banner fed by the same read model the
// automations key on, per-location quantities, and the one movement dialog
// for receive / use / adjust / transfer / ship-out. Tools carry their
// checkout state on /stock/tools; this board stays the inventory truth.
//
// Wave 32 lane 6 (`p32-stock-flows-context`) adds the stockroom dispatch
// workflow: scan or search an item, attach the movement to a chosen ticket
// — the operator never opens that ticket.
//
// Non-purpose: no catalog pricing edits (sales/catalog), no configuration
// items (that is /inventory), no purchase receiving (a PO detail flow).
"use client";

import {
  Button,
  Card,
  Checkbox,
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
  StatusTag,
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { MovementDialog, type MovementKind } from "../../components/stock/movement-dialog";
import { ScanBar, type ScanEvent } from "../../components/scan/scan-bar";
import { api, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { rejectionText } from "../../lib/error-copy";
import { formatQtyMilli } from "../../lib/stock-format";

type StockItemView = components["schemas"]["StockItemView"];
type StockLocationView = components["schemas"]["StockLocationView"];
type OnHandRow = components["schemas"]["OnHandRow"];
type CatalogItemView = components["schemas"]["CatalogItemView"];
type TicketRow = components["schemas"]["TicketWithFields"];

interface ItemForm {
  name: string;
  sku: string;
  partNumber: string;
  unit: string;
  currency: string;
  kind: "consumable" | "serialized";
  isTool: boolean;
  catalogItemId: string;
}

const EMPTY_FORM: ItemForm = {
  name: "",
  sku: "",
  partNumber: "",
  unit: "each",
  currency: "USD",
  kind: "consumable",
  isTool: false,
  catalogItemId: "",
};

export default function StockPage(): ReactNode {
  const auth = useRequireAuth();
  const [items, setItems] = useState<StockItemView[] | null>(null);
  const [onHand, setOnHand] = useState<OnHandRow[]>([]);
  const [locations, setLocations] = useState<StockLocationView[]>([]);
  const [toolOut, setToolOut] = useState<Map<string, "checked out" | "on loan">>(new Map());
  const [catalogItems, setCatalogItems] = useState<CatalogItemView[]>([]);
  const [clients, setClients] = useState<{ id: string; name: string }[]>([]);
  const [ticketOptions, setTicketOptions] = useState<{ id: string; label: string }[]>([]);
  const [scanNotice, setScanNotice] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [kindFilter, setKindFilter] = useState<"all" | "consumable" | "serialized">("all");
  const [toolsOnly, setToolsOnly] = useState(false);
  const [lowOnly, setLowOnly] = useState(false);
  const [showArchived, setShowArchived] = useState(false);

  const [dialogOpen, setDialogOpen] = useState(false);
  const [dialogKind, setDialogKind] = useState<MovementKind>("receive");
  const [dialogItemId, setDialogItemId] = useState<string | undefined>(undefined);

  const [createOpen, setCreateOpen] = useState(false);
  const [form, setForm] = useState<ItemForm>(EMPTY_FORM);
  const [formError, setFormError] = useState<string | null>(null);
  const [formBusy, setFormBusy] = useState(false);
  const [lastMovement, setLastMovement] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  async function refresh(): Promise<void> {
    const [itemsRes, onHandRes, locationsRes, serialsRes] = await Promise.all([
      api.api.GET("/api/v1/stock-items", { params: { query: { limit: 500 } } }),
      api.api.GET("/api/v1/stock/on-hand", { params: { query: { limit: 1000 } } }),
      api.api.GET("/api/v1/stock-locations", { params: { query: {} } }),
      api.api.GET("/api/v1/stock-serials", { params: { query: { tool_only: true, limit: 500 } } }),
    ]);
    if (itemsRes.data === undefined) {
      setLoadError("Could not load stock — try reloading.");
      return;
    }
    setItems(itemsRes.data ?? []);
    setOnHand(onHandRes.data ?? []);
    setLocations(locationsRes.data ?? []);
    // A tool whose units are all out should not read "ok" on the board.
    const counts = new Map<string, { out: number; loaned: number }>();
    for (const unit of serialsRes.data ?? []) {
      if (unit.status !== "checked_out" && unit.status !== "loaned") continue;
      const current = counts.get(unit.stock_item_id) ?? { out: 0, loaned: 0 };
      if (unit.status === "checked_out") current.out += 1;
      else current.loaned += 1;
      counts.set(unit.stock_item_id, current);
    }
    const outMap = new Map<string, "checked out" | "on loan">();
    for (const [itemId, c] of counts) outMap.set(itemId, c.loaned > c.out ? "on loan" : "checked out");
    setToolOut(outMap);
  }

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
    void (async () => {
      const [catalogRes, clientsRes, ticketsRes] = await Promise.all([
        api.api.GET("/api/v1/catalog-items"),
        api.api.GET("/api/v1/clients"),
        api.api.GET("/api/v1/tickets", { params: { query: { limit: 200 } } }),
      ]);
      setCatalogItems(catalogRes.data ?? []);
      setClients((clientsRes.data ?? []).map((c) => ({ id: c.id, name: c.name })));
      const openTickets = ((ticketsRes.data ?? []) as TicketRow[]).filter(
        (ticket) => ticket.category !== "closed",
      );
      setTicketOptions(
        openTickets.map((ticket) => ({
          id: ticket.id,
          label: `${ticket.number} · ${ticket.title}`,
        })),
      );
    })();
  }, [auth.status]);

  const lowRows = useMemo(() => onHand.filter((row) => row.low), [onHand]);

  const totalsByItem = useMemo(() => {
    const map = new Map<string, { qty: number; locations: number; low: boolean }>();
    for (const row of onHand) {
      const current = map.get(row.stock_item_id) ?? { qty: 0, locations: 0, low: false };
      current.qty += row.qty_milli;
      current.locations += 1;
      current.low = current.low || row.low;
      map.set(row.stock_item_id, current);
    }
    return map;
  }, [onHand]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (items ?? []).filter((item) => {
      if (!showArchived && !item.active) return false;
      if (toolsOnly && !item.is_tool) return false;
      if (kindFilter !== "all" && item.kind !== kindFilter) return false;
      if (lowOnly && (totalsByItem.get(item.id)?.low ?? false) === false) return false;
      if (q === "") return true;
      return (
        item.name.toLowerCase().includes(q) ||
        (item.sku ?? "").toLowerCase().includes(q) ||
        (item.part_number ?? "").toLowerCase().includes(q)
      );
    });
  }, [items, query, kindFilter, toolsOnly, lowOnly, showArchived, totalsByItem]);

  function openMovement(kind: MovementKind, itemId?: string): void {
    setDialogKind(kind);
    setDialogItemId(itemId);
    setDialogOpen(true);
  }

  /**
   * The stockroom dispatch scan: an item or unit opens the movement form
   * with that item staged and the ticket picker ready. A location does not
   * start a dispatch (the form's own scan sets the source bin).
   */
  function dispatchScan(event: ScanEvent): void {
    const resolution = event.resolution;
    const itemId = resolution.stock_item_id ?? resolution.entity_id ?? "";
    if (itemId === "") {
      setScanNotice("That code did not match a stock item — nothing to dispatch.");
      return;
    }
    setScanNotice(null);
    openMovement("consume", itemId);
  }

  async function createItem(): Promise<void> {
    setFormError(null);
    const name = form.name.trim();
    if (name === "") {
      setFormError("Name is required.");
      return;
    }
    const kind = form.isTool ? "serialized" : form.kind;
    setFormBusy(true);
    const tempId = `pending-${Date.now()}`;
    const result = await runEnvelopeMutation({
      describe: "Create stock item",
      buildBody: (base) => ({
        name,
        kind,
        unit: form.unit.trim() || "each",
        currency: form.currency.trim().toUpperCase() || "USD",
        is_tool: form.isTool,
        sku: form.sku.trim() === "" ? null : form.sku,
        part_number: form.partNumber.trim() === "" ? null : form.partNumber,
        catalog_item_id: form.catalogItemId === "" ? null : form.catalogItemId,
        ...base,
      }),
      optimistic: () => {
        setItems((cur) => [
          ...(cur ?? []),
          {
            id: tempId,
            org_id: "",
            name,
            kind,
            unit: form.unit.trim() || "each",
            currency: form.currency.trim().toUpperCase() || "USD",
            is_tool: form.isTool,
            sku: form.sku.trim() === "" ? null : form.sku,
            part_number: form.partNumber.trim() === "" ? null : form.partNumber,
            catalog_item_id: form.catalogItemId === "" ? null : form.catalogItemId,
            active: true,
            last_unit_cost_minor: null,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
            created_by: null,
          },
        ]);
      },
      revert: () => setItems((cur) => (cur ?? []).filter((i) => i.id !== tempId)),
      request: (client, body) =>
        client.POST("/api/v1/stock-items", {
          body: body as components["schemas"]["CreateStockItemRequest"],
        }),
    });
    setFormBusy(false);
    if (result.ok === false) {
      setFormError(rejectionText(result, "Could not create the stock item."));
      return;
    }
    setCreateOpen(false);
    setForm(EMPTY_FORM);
    await refresh();
  }

  async function archiveItem(item: StockItemView): Promise<void> {
    setActionError(null);
    setBusyId(item.id);
    const result = await runEnvelopeMutation({
      describe: "Archive stock item",
      entityId: item.id,
      buildBody: (base) => ({ ...base }),
      optimistic: () =>
        setItems((cur) => (cur ?? []).map((i) => (i.id === item.id ? { ...i, active: false } : i))),
      revert: () =>
        setItems((cur) => (cur ?? []).map((i) => (i.id === item.id ? { ...i, active: item.active } : i))),
      request: (client, body) =>
        client.POST("/api/v1/stock-items/{id}/archive", {
          params: { path: { id: item.id } },
          body: body as components["schemas"]["ArchiveStockItemRequest"],
        }),
    });
    setBusyId(null);
    if (result.ok === false) {
      setActionError(`Could not archive ${item.name}: ${rejectionText(result, "refused.")}`);
    }
  }

  if (items === null && loadError === null) return <Spinner />;
  if (loadError !== null) return <EmptyState message={loadError} />;

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
      <header className="m-rise flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Stock
          </h1>
          <p className="mt-0.5 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Physical parts and tools: quantities per location, low-stock alerts, and the movement ledger.
          </p>
        </div>
        <div className="flex flex-wrap gap-1">
          <Button variant="secondary" onClick={() => openMovement("receive")} data-testid="stock-receive">
            Receive stock
          </Button>
          <Button variant="secondary" onClick={() => openMovement("consume")} data-testid="stock-dispatch">
            Dispatch to ticket
          </Button>
          <Button onClick={() => setCreateOpen(true)} data-testid="stock-new-item">
            New item
          </Button>
        </div>
      </header>

      {lowRows.length > 0 && (
        <Card
          className="m-rise flex flex-wrap items-center justify-between gap-3 p-4"
          style={{ borderColor: "var(--m-status-warning-fg)" }}
          data-testid="stock-low-banner"
        >
          <div>
            <p className="text-[13px] font-semibold" style={{ color: "var(--m-status-warning-fg)" }}>
              {lowRows.length} item{lowRows.length === 1 ? "" : "s"} at or below the reorder point
            </p>
            <p className="mt-0.5 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              {lowRows
                .slice(0, 3)
                .map((row) => `${row.item_name} at ${row.location_name} (${formatQtyMilli(row.qty_milli, row.unit)})`)
                .join(" · ")}
              {lowRows.length > 3 ? ` · +${lowRows.length - 3} more` : ""}
            </p>
          </div>
          <Button size="sm" variant="secondary" onClick={() => setLowOnly((v) => !v)} data-testid="stock-low-toggle">
            {lowOnly ? "Show all" : "Show low only"}
          </Button>
        </Card>
      )}

      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {(
          [
            { label: "Items", value: (items ?? []).filter((i) => i.active).length, hint: `${(items ?? []).filter((i) => !i.active).length} archived`, testId: "stock-stat-items" },
            { label: "Low stock", value: lowRows.length, hint: "at or below reorder point", testId: "stock-stat-low" },
            { label: "Tools", value: (items ?? []).filter((i) => i.is_tool && i.active).length, hint: "checkout-eligible", testId: "stock-stat-tools" },
            { label: "Locations", value: locations.filter((l) => l.active).length, hint: "warehouse · van · truck", testId: "stock-stat-locations" },
          ] as const
        ).map((tile, i) => (
          <Card key={tile.label} className="m-rise p-4" style={{ animationDelay: `${Math.min(i, 6) * 40}ms` }}>
            <p className="text-[11px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              {tile.label}
            </p>
            <p className="mt-1 font-mono text-xl font-semibold tabular-nums" style={{ color: "var(--m-text)" }} data-testid={tile.testId}>
              {tile.value}
            </p>
            <p className="mt-0.5 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              {tile.hint}
            </p>
          </Card>
        ))}
      </div>

      <div className="max-w-xl" data-testid="stock-dispatch-scan">
        <ScanBar
          surface="dispatch"
          onScan={dispatchScan}
          hint="Scan a part or unit to dispatch it to a ticket — the ticket picker opens next."
        />
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search name, SKU, part number…"
          aria-label="Search stock"
          data-testid="stock-search"
          style={{ maxWidth: 300 }}
        />        <Select value={kindFilter} onValueChange={(value) => setKindFilter(value as typeof kindFilter)}>
          <SelectTrigger aria-label="Filter by kind" data-testid="stock-kind-filter" style={{ width: 160 }}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All kinds</SelectItem>
            <SelectItem value="consumable">Consumables</SelectItem>
            <SelectItem value="serialized">Serialized</SelectItem>
          </SelectContent>
        </Select>
        <label className="flex items-center gap-1.5 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
          <Checkbox checked={toolsOnly} onCheckedChange={(v) => setToolsOnly(v === true)} data-testid="stock-tools-only" />
          Tools only
        </label>
        <label className="flex items-center gap-1.5 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
          <Checkbox checked={showArchived} onCheckedChange={(v) => setShowArchived(v === true)} data-testid="stock-show-archived" />
          Show archived
        </label>
      </div>

      {actionError !== null && (
        <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="stock-action-error">
          {actionError}
        </p>
      )}
      {lastMovement !== null && (
        <p className="text-xs" role="status" style={{ color: "var(--m-status-success-fg)" }} data-testid="stock-movement-recorded">
          {lastMovement}
        </p>
      )}
      {scanNotice !== null && (
        <p className="text-xs" role="status" style={{ color: "var(--m-status-warning-fg)" }} data-testid="stock-scan-notice">
          {scanNotice}
        </p>
      )}

      <Card className="m-rise overflow-x-auto p-0">
        <Table data-testid="stock-table">
          <thead>
            <tr>
              <Th>Item</Th>
              <Th>Kind</Th>
              <Th className="text-right">On hand</Th>
              <Th className="text-right">Locations</Th>
              <Th>State</Th>
              <Th className="text-right">Actions</Th>
            </tr>
          </thead>
          <tbody>
            {filtered.map((item) => {
              const total = totalsByItem.get(item.id);
              return (
                <Tr key={item.id} className="row-hover" data-testid={`stock-row-${item.id}`}>
                  <Td>
                    <Link href={`/stock/${item.id}`} className="font-medium" style={{ color: "var(--m-text)" }}>
                      {item.name}
                    </Link>
                    {item.is_tool && (
                      <Tag tone="outline" className="ml-1.5 text-[10px]">
                        tool
                      </Tag>
                    )}
                    <span className="ml-1.5 font-mono text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                      {item.sku ?? item.part_number ?? ""}
                    </span>
                  </Td>
                  <Td className="text-[12px]">{item.kind}</Td>
                  <Td className="text-right font-mono text-[13px] tabular-nums">
                    {formatQtyMilli(total?.qty ?? 0, item.unit)}
                  </Td>
                  <Td className="text-right font-mono text-[13px] tabular-nums">{total?.locations ?? 0}</Td>
                  <Td>
                    <div className="flex flex-wrap items-center gap-1">
                      {!item.active && <StatusTag tone="neutral">archived</StatusTag>}
                      {item.active && total?.low === true && <StatusTag tone="warning">low</StatusTag>}
                      {item.active && total?.low !== true && (total?.qty ?? 0) === 0 && toolOut.has(item.id) && (
                        <StatusTag tone="warning" data-testid={`stock-state-out-${item.id}`}>
                          {toolOut.get(item.id) ?? "out"}
                        </StatusTag>
                      )}
                      {item.active &&
                        total?.low !== true &&
                        ((total?.qty ?? 0) > 0 || !toolOut.has(item.id)) && (
                          <StatusTag tone="success">ok</StatusTag>
                        )}
                    </div>
                  </Td>
                  <Td>
                    <div className="flex justify-end gap-1">
                      <Button size="sm" variant="secondary" onClick={() => openMovement("receive", item.id)} data-testid={`stock-receive-${item.id}`}>
                        Receive
                      </Button>
                      {!item.is_tool && (
                        <Button size="sm" variant="ghost" onClick={() => openMovement("consume", item.id)} data-testid={`stock-use-${item.id}`}>
                          Use
                        </Button>
                      )}
                      <Button size="sm" variant="ghost" onClick={() => openMovement("transfer", item.id)} data-testid={`stock-transfer-${item.id}`}>
                        Move
                      </Button>
                      {item.active ? (
                        <Button size="sm" variant="ghost" disabled={busyId === item.id} onClick={() => void archiveItem(item)}>
                          Archive
                        </Button>
                      ) : (
                        <Button
                          size="sm"
                          variant="ghost"
                          disabled={busyId === item.id}
                          onClick={() => {
                            setBusyId(item.id);
                            void runEnvelopeMutation({
                              describe: "Reactivate stock item",
                              entityId: item.id,
                              buildBody: (base) => ({ active: true, ...base }),
                              optimistic: () =>
                                setItems((cur) => (cur ?? []).map((i) => (i.id === item.id ? { ...i, active: true } : i))),
                              revert: () =>
                                setItems((cur) => (cur ?? []).map((i) => (i.id === item.id ? { ...i, active: false } : i))),
                              request: (client, body) =>
                                client.PATCH("/api/v1/stock-items/{id}", {
                                  params: { path: { id: item.id } },
                                  body: body as components["schemas"]["UpdateStockItemRequest"],
                                }),
                            }).then(() => setBusyId(null));
                          }}
                        >
                          Reactivate
                        </Button>
                      )}
                    </div>
                  </Td>
                </Tr>
              );
            })}
            {filtered.length === 0 && (
              <Tr>
                <Td>
                  <EmptyState message="No stock items match. Receive stock or create an item to start the ledger." />
                </Td>
              </Tr>
            )}
          </tbody>
        </Table>
      </Card>

      <MovementDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        items={items ?? []}
        locations={locations}
        clients={clients}
        initialKind={dialogKind}
        initialItemId={dialogItemId}
        ticketOptions={ticketOptions}
        scanSurface="stock-locations"
        searchableItems
        onRecorded={() => {
          setLastMovement("Movement recorded.");
          window.setTimeout(() => setLastMovement(null), 6000);
          void refresh();
        }}
      />

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent title="New stock item">
          <div className="grid max-h-[80vh] gap-3 overflow-auto p-1">
            <div className="grid gap-1">
              <Label htmlFor="stock-name">Name</Label>
              <Input id="stock-name" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} data-testid="stock-name" />
            </div>
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
              <div className="grid gap-1">
                <Label htmlFor="stock-sku">SKU</Label>
                <Input id="stock-sku" value={form.sku} onChange={(e) => setForm({ ...form, sku: e.target.value })} />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="stock-part">Part number</Label>
                <Input id="stock-part" value={form.partNumber} onChange={(e) => setForm({ ...form, partNumber: e.target.value })} />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="stock-unit">Unit</Label>
                <Select value={form.unit} onValueChange={(value) => setForm({ ...form, unit: value })}>
                  <SelectTrigger id="stock-unit" data-testid="stock-unit">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="each">each</SelectItem>
                    <SelectItem value="box">box</SelectItem>
                    <SelectItem value="meter">meter</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-1">
                <Label htmlFor="stock-currency">Currency</Label>
                <Input
                  id="stock-currency"
                  value={form.currency}
                  maxLength={3}
                  onChange={(e) => setForm({ ...form, currency: e.target.value.toUpperCase() })}
                />
              </div>
            </div>
            <div className="grid gap-2 sm:grid-cols-2">
              <div className="grid gap-1">
                <Label htmlFor="stock-kind">Tracking</Label>
                <Select
                  value={form.isTool ? "serialized" : form.kind}
                  onValueChange={(value) => setForm({ ...form, kind: value as ItemForm["kind"] })}
                >
                  <SelectTrigger id="stock-kind" data-testid="stock-kind">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="consumable">Consumable (counted in bulk)</SelectItem>
                    <SelectItem value="serialized">Serialized (one row per unit)</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-1">
                <Label htmlFor="stock-catalog">Catalog link (optional)</Label>
                <Select value={form.catalogItemId} onValueChange={(value) => setForm({ ...form, catalogItemId: value })}>
                  <SelectTrigger id="stock-catalog" data-testid="stock-catalog">
                    <SelectValue placeholder="No catalog item" />
                  </SelectTrigger>
                  <SelectContent>
                    {catalogItems.map((c) => (
                      <SelectItem key={c.id} value={c.id}>
                        {c.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FieldHint>A soft link for pricing context; the item stands alone.</FieldHint>
              </div>
            </div>
            <label className="flex items-center gap-2 text-xs" style={{ color: "var(--m-text-secondary)" }}>
              <Checkbox
                checked={form.isTool}
                onCheckedChange={(v) => setForm({ ...form, isTool: v === true, kind: v === true ? "serialized" : form.kind })}
                data-testid="stock-is-tool"
              />
              This is a tool (serialized, checkout-eligible)
            </label>
            {formError !== null && (
              <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="stock-form-error">
                {formError}
              </p>
            )}
            <DialogActions>
              <Button variant="secondary" onClick={() => setCreateOpen(false)}>
                Cancel
              </Button>
              <Button disabled={formBusy} onClick={() => void createItem()} data-testid="stock-create">
                Create item
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
