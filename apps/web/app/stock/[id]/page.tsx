// /stock/{id} — one stock item: quantities per location, serials
// where-they-are-now, the movement ledger, and the tool checkout state
// (Wave 32 lane 4).
//
// Purpose: the traceability surface for a single item. Every number here
// comes from a read model (on-hand, serials, movements, checkouts); the
// writes are the same envelope mutations the board uses — a movement through
// MovementDialog, thresholds per location, an edit, an archive, and a tool
// check-in.
//
// Non-purpose: no purchase receiving (PO detail owns that), no kit
// management or checkout creation (the tools board owns those).
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
import { useParams } from "next/navigation";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { CheckInDialog } from "../../../components/stock/check-in-dialog";
import { MovementDialog, type MovementKind } from "../../../components/stock/movement-dialog";
import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { rejectionText } from "../../../lib/error-copy";
import { money } from "../../../lib/format";
import {
  duePhrase,
  formatQtyMilli,
  formatSignedQty,
  movementLabel,
  parseQtyToMilli,
  serialStatusLabel,
  serialStatusTone,
} from "../../../lib/stock-format";

type StockItemView = components["schemas"]["StockItemView"];
type StockLocationView = components["schemas"]["StockLocationView"];
type StockMovementView = components["schemas"]["StockMovementView"];
type StockSerialView = components["schemas"]["StockSerialView"];
type OnHandRow = components["schemas"]["OnHandRow"];
type ToolSerialWhereaboutsView = components["schemas"]["ToolSerialWhereaboutsView"];
type ToolCheckoutDetailResponse = components["schemas"]["ToolCheckoutDetailResponse"];

interface EditForm {
  name: string;
  sku: string;
  partNumber: string;
  unit: string;
  isTool: boolean;
  active: boolean;
}

function idPrefix(id: string): string {
  return `${id.slice(0, 8)}…`;
}

function parseThresholdValue(text: string): number | null | "invalid" {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const parsed = parseQtyToMilli(trimmed);
  return parsed === null ? "invalid" : parsed;
}

function shortWhen(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export default function StockItemDetailPage(): ReactNode {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const itemId = params.id;

  const [item, setItem] = useState<StockItemView | null>(null);
  const [onHand, setOnHand] = useState<OnHandRow[]>([]);
  const [serials, setSerials] = useState<StockSerialView[]>([]);
  const [whereabouts, setWhereabouts] = useState<ToolSerialWhereaboutsView[]>([]);
  const [movements, setMovements] = useState<StockMovementView[]>([]);
  const [locations, setLocations] = useState<StockLocationView[]>([]);
  const [clients, setClients] = useState<{ id: string; name: string }[]>([]);
  const [checkouts, setCheckouts] = useState<ToolCheckoutDetailResponse[]>([]);
  const [actorNames, setActorNames] = useState<Map<string, string>>(new Map());
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [movementOpen, setMovementOpen] = useState(false);
  const [movementKind, setMovementKind] = useState<MovementKind>("receive");

  const [editOpen, setEditOpen] = useState(false);
  const [editBefore, setEditBefore] = useState<StockItemView | null>(null);
  const [form, setForm] = useState<EditForm>({ name: "", sku: "", partNumber: "", unit: "", isTool: false, active: true });
  const [formError, setFormError] = useState<string | null>(null);
  const [formBusy, setFormBusy] = useState(false);

  const [thresholdRow, setThresholdRow] = useState<OnHandRow | null>(null);
  const [thresholdMin, setThresholdMin] = useState("");
  const [thresholdMax, setThresholdMax] = useState("");
  const [thresholdReorder, setThresholdReorder] = useState("");
  const [thresholdError, setThresholdError] = useState<string | null>(null);
  const [thresholdBusy, setThresholdBusy] = useState(false);

  const [checkinId, setCheckinId] = useState<string | null>(null);

  async function refresh(): Promise<void> {
    const itemRes = await api.api.GET("/api/v1/stock-items/{id}", { params: { path: { id: itemId } } });
    if (itemRes.data === undefined) {
      setLoadError("Could not load this stock item — try reloading.");
      return;
    }
    const loaded = itemRes.data;
    setItem(loaded);
    const [onHandRes, locationsRes, clientsRes, movementsRes] = await Promise.all([
      api.api.GET("/api/v1/stock/on-hand", { params: { query: { item_id: itemId, limit: 200 } } }),
      api.api.GET("/api/v1/stock-locations", { params: { query: {} } }),
      api.api.GET("/api/v1/clients"),
      api.api.GET("/api/v1/stock-items/{id}/movements", {
        params: { path: { id: itemId }, query: { limit: 200 } },
      }),
    ]);
    setOnHand(onHandRes.data ?? []);
    setLocations(locationsRes.data ?? []);
    setClients((clientsRes.data ?? []).map((c) => ({ id: c.id, name: c.name })));
    setMovements(movementsRes.data ?? []);
    // Ledger "Who" reads as a person, not a uuid: the staff directory when
    // the viewer may read it, the caller's own name otherwise. A 403 only
    // means the id-prefix fallback stays.
    void (async () => {
      const names = new Map<string, string>();
      const me = await api.api.GET("/api/v1/me");
      if (me.data !== undefined) names.set(me.data.user_id, me.data.display_name);
      const dir = await api.api.GET("/api/v1/org/users", {});
      if (dir.data !== undefined) {
        for (const member of dir.data.users) names.set(member.id, member.display_name);
      }
      if (names.size > 0) setActorNames(names);
    })();
    if (loaded.kind === "serialized") {
      const [serialsRes, whereRes] = await Promise.all([
        api.api.GET("/api/v1/stock-items/{id}/serials", {
          params: { path: { id: itemId }, query: { limit: 200 } },
        }),
        api.api.GET("/api/v1/stock-serials", {
          params: { query: { item_id: itemId, limit: 200 } },
        }),
      ]);
      setSerials(serialsRes.data ?? []);
      setWhereabouts(whereRes.data ?? []);
    } else {
      setSerials([]);
      setWhereabouts([]);
    }
    if (loaded.is_tool) {
      const checkoutsRes = await api.api.GET("/api/v1/tool-checkouts", {
        params: { query: { status: "open", limit: 200 } },
      });
      setCheckouts(
        (checkoutsRes.data ?? []).filter((row) =>
          row.lines.some((line) => line.stock_item_id === loaded.id),
        ),
      );
    } else {
      setCheckouts([]);
    }
  }

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
  }, [auth.status, itemId]);

  const locationName = useMemo(() => {
    const map = new Map(locations.map((location) => [location.id, location.name]));
    return (id: string | null | undefined): string => {
      if (id == null) return "—";
      return map.get(id) ?? idPrefix(id);
    };
  }, [locations]);

  const clientName = useMemo(() => {
    const map = new Map(clients.map((client) => [client.id, client.name]));
    return (id: string | null | undefined): string => {
      if (id == null) return "—";
      return map.get(id) ?? idPrefix(id);
    };
  }, [clients]);

  const serialWhere = useMemo(
    () => new Map(whereabouts.map((row) => [row.id, row])),
    [whereabouts],
  );

  const serialLabels = useMemo(() => {
    const map: Record<string, string> = {};
    for (const serial of serials) map[serial.id] = serial.serial_number;
    return map;
  }, [serials]);

  const itemLabels = useMemo(() => {
    const map: Record<string, string> = {};
    if (item !== null) map[item.id] = item.name;
    return map;
  }, [item]);

  function openEdit(): void {
    if (item === null) return;
    setEditBefore(item);
    setForm({
      name: item.name,
      sku: item.sku ?? "",
      partNumber: item.part_number ?? "",
      unit: item.unit,
      isTool: item.is_tool,
      active: item.active,
    });
    setFormError(null);
    setEditOpen(true);
  }

  async function saveEdit(): Promise<void> {
    const before = editBefore;
    if (before === null) return;
    setFormError(null);
    const name = form.name.trim();
    if (name === "") {
      setFormError("Name is required.");
      return;
    }
    const sku = form.sku.trim() === "" ? null : form.sku;
    const partNumber = form.partNumber.trim() === "" ? null : form.partNumber;
    const unit = form.unit.trim() === "" ? before.unit : form.unit.trim();
    setFormBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Update stock item",
      entityId: before.id,
      buildBody: (base) => ({
        name,
        sku,
        part_number: partNumber,
        unit,
        active: form.active,
        ...(before.kind === "serialized" ? { is_tool: form.isTool } : {}),
        ...base,
      }),
      optimistic: () =>
        setItem((cur) =>
          cur === null
            ? cur
            : {
                ...cur,
                name,
                sku,
                part_number: partNumber,
                unit,
                active: form.active,
                is_tool: before.kind === "serialized" ? form.isTool : cur.is_tool,
                updated_at: new Date().toISOString(),
              },
        ),
      revert: () => setItem(before),
      request: (client, body) =>
        client.PATCH("/api/v1/stock-items/{id}", {
          params: { path: { id: before.id } },
          body: body as components["schemas"]["UpdateStockItemRequest"],
        }),
      versions: (data) => [[data.id, data.mutation.version]],
    });
    setFormBusy(false);
    if (result.ok === false) {
      setFormError(rejectionText(result, "Could not save the item."));
      return;
    }
    setEditOpen(false);
    setNotice("Item saved.");
    await refresh();
  }

  async function setActive(active: boolean): Promise<void> {
    const current = item;
    if (current === null) return;
    setActionError(null);
    setBusy(true);
    const result = await runEnvelopeMutation({
      describe: active ? "Reactivate stock item" : "Archive stock item",
      entityId: current.id,
      buildBody: (base) => ({ active, ...base }),
      optimistic: () => setItem((cur) => (cur === null ? cur : { ...cur, active })),
      revert: () => setItem((cur) => (cur === null ? cur : { ...cur, active: current.active })),
      request: (client, body) =>
        client.PATCH("/api/v1/stock-items/{id}", {
          params: { path: { id: current.id } },
          body: body as components["schemas"]["UpdateStockItemRequest"],
        }),
      versions: (data) => [[data.id, data.mutation.version]],
    });
    setBusy(false);
    if (result.ok === false) {
      setActionError(
        `Could not ${active ? "reactivate" : "archive"} ${current.name}: ${rejectionText(result, "refused.")}`,
      );
      return;
    }
    setNotice(active ? "Item reactivated." : "Item archived.");
    await refresh();
  }

  function openThresholds(row: OnHandRow): void {
    setThresholdRow(row);
    setThresholdMin(row.min_qty_milli == null ? "" : formatQtyMilli(row.min_qty_milli, ""));
    setThresholdMax(row.max_qty_milli == null ? "" : formatQtyMilli(row.max_qty_milli, ""));
    setThresholdReorder(row.reorder_point_milli == null ? "" : formatQtyMilli(row.reorder_point_milli, ""));
    setThresholdError(null);
  }

  async function saveThresholds(): Promise<void> {
    const row = thresholdRow;
    if (row === null) return;
    setThresholdError(null);
    const min = parseThresholdValue(thresholdMin);
    const max = parseThresholdValue(thresholdMax);
    const reorder = parseThresholdValue(thresholdReorder);
    if (min === "invalid" || max === "invalid" || reorder === "invalid") {
      setThresholdError("Quantities must be numbers, or empty to clear a value.");
      return;
    }
    setThresholdBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Set stock thresholds",
      buildBody: (base) => ({
        item_id: row.stock_item_id,
        location_id: row.stock_location_id,
        min_qty_milli: min,
        max_qty_milli: max,
        reorder_point_milli: reorder,
        ...base,
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.POST("/api/v1/stock/levels/thresholds", {
          body: body as components["schemas"]["ThresholdsRequest"],
        }),
    });
    setThresholdBusy(false);
    if (result.ok === false) {
      setThresholdError(rejectionText(result, "Could not save the thresholds."));
      return;
    }
    setThresholdRow(null);
    setNotice("Thresholds saved.");
    await refresh();
  }

  function serialWhereParts(serial: StockSerialView): ReactNode[] {
    const where = serialWhere.get(serial.id);
    const holder = where?.holder_user_id ?? null;
    const clientId = where?.client_id ?? serial.client_id ?? null;
    const locationId = serial.location_id ?? null;
    const location = where?.location_name ?? (locationId === null ? null : locationName(locationId));
    const parts: ReactNode[] = [];
    if (location !== null) parts.push(<span key="location">{location}</span>);
    if (holder !== null) parts.push(<span key="holder">holder {idPrefix(holder)}</span>);
    if (clientId !== null) {
      parts.push(
        <span key="client">
          client{" "}
          <Link href={`/clients/${clientId}`} style={{ color: "var(--m-status-info-fg)" }}>
            {clientName(clientId)}
          </Link>
        </span>,
      );
    }
    return parts;
  }

  async function recordCheckin(checkoutId: string): Promise<void> {
    setNotice(`Checkout ${idPrefix(checkoutId)} checked in.`);
    await refresh();
  }

  if (item === null && loadError === null) return <Spinner />;
  if (loadError !== null) return <EmptyState message={loadError} />;

  const itemValue = item as StockItemView;
  const serialized = itemValue.kind === "serialized";

  return (
    <div className="mx-auto grid w-full max-w-350 gap-6 p-4 md:p-6">
      <header className="m-rise flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }} data-testid="stock-detail-name">
              {itemValue.name}
            </h1>
            <Tag tone="outline">{itemValue.kind}</Tag>
            <Tag tone="outline">{itemValue.unit}</Tag>
            {itemValue.is_tool && <Tag tone="outline">tool</Tag>}
            <StatusTag tone={itemValue.active ? "success" : "neutral"}>
              {itemValue.active ? "active" : "archived"}
            </StatusTag>
          </div>
          <p className="mt-0.5 font-mono text-xs" style={{ color: "var(--m-text-muted)" }}>
            {itemValue.sku ?? "no SKU"} · {itemValue.part_number ?? "no part number"} ·{" "}
            {itemValue.last_unit_cost_minor == null
              ? "last unit cost —"
              : `last unit cost ${money(itemValue.last_unit_cost_minor, itemValue.currency)}`}
          </p>
          <p className="mt-0.5 text-sm" style={{ color: "var(--m-text-secondary)" }}>
            <Link href="/stock" style={{ color: "var(--m-status-info-fg)" }}>
              Stock
            </Link>{" "}
            / item detail
          </p>
        </div>
        <div className="flex flex-wrap gap-1">
          <Button
            variant="secondary"
            onClick={() => {
              setMovementKind("receive");
              setMovementOpen(true);
            }}
            data-testid="stock-record-movement"
          >
            Record movement
          </Button>
          <Button variant="secondary" onClick={openEdit} data-testid="stock-edit">
            Edit
          </Button>
          {itemValue.active ? (
            <Button variant="ghost" disabled={busy} onClick={() => void setActive(false)} data-testid="stock-archive">
              Archive
            </Button>
          ) : (
            <Button variant="ghost" disabled={busy} onClick={() => void setActive(true)} data-testid="stock-reactivate">
              Reactivate
            </Button>
          )}
        </div>
      </header>

      {actionError !== null && (
        <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="stock-detail-error">
          {actionError}
        </p>
      )}
      {notice !== null && (
        <p className="text-xs" role="status" style={{ color: "var(--m-status-success-fg)" }} data-testid="stock-detail-notice">
          {notice}
        </p>
      )}

      <Card className="m-rise p-0" data-testid="stock-onhand-card">
        <div className="flex flex-wrap items-center justify-between gap-2 p-4 pb-2">
          <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            Quantities per location
          </h2>
          <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
            min / reorder per row · low rows carry the warning tag
          </span>
        </div>
        <div className="overflow-x-auto">
          <Table data-testid="stock-onhand-table">
            <thead>
              <tr>
                <Th>Location</Th>
                <Th className="text-right">On hand</Th>
                <Th className="text-right">Min</Th>
                <Th className="text-right">Reorder</Th>
                <Th className="text-right">Max</Th>
                <Th>State</Th>
                <Th className="text-right">Actions</Th>
              </tr>
            </thead>
            <tbody>
              {onHand.map((row) => (
                <Tr key={row.stock_location_id} className="row-hover" data-testid={`stock-onhand-row-${row.stock_location_id}`}>
                  <Td>
                    <span style={{ color: "var(--m-text)" }}>{row.location_name}</span>
                    <span className="ml-1.5 text-xs" style={{ color: "var(--m-text-muted)" }}>
                      {row.location_kind}
                    </span>
                  </Td>
                  <Td className="text-right font-mono text-sm tabular-nums" data-testid={`stock-onhand-qty-${row.stock_location_id}`}>
                    {formatQtyMilli(row.qty_milli, row.unit)}
                  </Td>
                  <Td className="text-right font-mono text-sm tabular-nums" data-testid={`stock-onhand-min-${row.stock_location_id}`}>
                    {row.min_qty_milli == null ? "—" : formatQtyMilli(row.min_qty_milli, "")}
                  </Td>
                  <Td className="text-right font-mono text-sm tabular-nums" data-testid={`stock-onhand-reorder-${row.stock_location_id}`}>
                    {row.reorder_point_milli == null ? "—" : formatQtyMilli(row.reorder_point_milli, "")}
                  </Td>
                  <Td className="text-right font-mono text-sm tabular-nums">
                    {row.max_qty_milli == null ? "—" : formatQtyMilli(row.max_qty_milli, "")}
                  </Td>
                  <Td>
                    {row.low ? (
                      <StatusTag tone="warning">low</StatusTag>
                    ) : (
                      <StatusTag tone="success">ok</StatusTag>
                    )}
                  </Td>
                  <Td>
                    <div className="flex justify-end">
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => openThresholds(row)}
                        data-testid={`stock-thresholds-${row.stock_location_id}`}
                      >
                        Thresholds
                      </Button>
                    </div>
                  </Td>
                </Tr>
              ))}
              {onHand.length === 0 && (
                <Tr>
                  <Td>
                    <EmptyState message="No quantity rows yet. Record a receive movement to place this item somewhere." />
                  </Td>
                </Tr>
              )}
            </tbody>
          </Table>
        </div>
      </Card>

      {serialized && (
        <Card className="m-rise overflow-x-auto p-0" data-testid="stock-serials-card">
          <div className="p-4 pb-2">
            <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
              Serials — where each unit is now
            </h2>
          </div>
          <Table data-testid="stock-serials-table">
            <thead>
              <tr>
                <Th>Serial</Th>
                <Th>Status</Th>
                <Th>Where</Th>
                <Th>Due</Th>
              </tr>
            </thead>
            <tbody>
              {serials.map((serial) => {
                const where = serialWhere.get(serial.id);
                const due = where?.due_at ?? where?.expected_return_at ?? null;
                const parts = serialWhereParts(serial);
                return (
                  <Tr key={serial.id} className="row-hover" data-testid={`stock-serial-row-${serial.id}`}>
                    <Td className="font-mono text-xs">{serial.serial_number}</Td>
                    <Td>
                      <StatusTag tone={serialStatusTone(serial.status)}>{serialStatusLabel(serial.status)}</StatusTag>
                    </Td>
                    <Td className="text-xs">
                      {parts.length === 0 ? (
                        <span style={{ color: "var(--m-text-muted)" }}>—</span>
                      ) : (
                        parts.map((part, index) => (
                          <span key={index}>
                            {index > 0 ? " · " : ""}
                            {part}
                          </span>
                        ))
                      )}
                    </Td>
                    <Td className="text-xs">
                      {due === null ? (
                        <span style={{ color: "var(--m-text-muted)" }}>—</span>
                      ) : (
                        <span className="flex items-center gap-1">
                          {duePhrase(due)}
                          {where?.overdue === true && <StatusTag tone="danger">overdue</StatusTag>}
                        </span>
                      )}
                    </Td>
                  </Tr>
                );
              })}
              {serials.length === 0 && (
                <Tr>
                  <Td>
                    <EmptyState message="No serial units recorded for this item yet." />
                  </Td>
                </Tr>
              )}
            </tbody>
          </Table>
        </Card>
      )}

      {itemValue.is_tool && (
        <Card className="m-rise overflow-x-auto p-0" data-testid="stock-tool-checkouts-card">
          <div className="p-4 pb-2">
            <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
              Open checkouts
            </h2>
          </div>
          <Table data-testid="stock-tool-checkouts-table">
            <thead>
              <tr>
                <Th>Holder</Th>
                <Th className="text-right">Units out</Th>
                <Th>Due</Th>
                <Th className="text-right">Actions</Th>
              </tr>
            </thead>
            <tbody>
              {checkouts.map((row) => {
                const mine = row.lines.filter((line) => line.stock_item_id === itemValue.id);
                return (
                  <Tr key={row.checkout.id} className="row-hover" data-testid={`stock-checkout-${row.checkout.id}`}>
                    <Td className="font-mono text-xs">{idPrefix(row.checkout.holder_user_id)}</Td>
                    <Td className="text-right font-mono text-sm tabular-nums">{mine.length}</Td>
                    <Td className="text-xs">
                      <span className="flex items-center gap-1">
                        {duePhrase(row.checkout.due_at)}
                        {row.checkout.overdue && <StatusTag tone="danger">overdue</StatusTag>}
                      </span>
                    </Td>
                    <Td>
                      <div className="flex justify-end">
                        <Button
                          size="sm"
                          variant="secondary"
                          onClick={() => setCheckinId(row.checkout.id)}
                          data-testid={`stock-checkin-${row.checkout.id}`}
                        >
                          Check in
                        </Button>
                      </div>
                    </Td>
                  </Tr>
                );
              })}
              {checkouts.length === 0 && (
                <Tr>
                  <Td>
                    <EmptyState message="No open checkouts for this tool." />
                  </Td>
                </Tr>
              )}
            </tbody>
          </Table>
        </Card>
      )}

      <Card className="m-rise overflow-x-auto p-0" data-testid="stock-ledger-card">
        <div className="p-4 pb-2">
          <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            Movement ledger ({movements.length} rows, newest first)
          </h2>
        </div>
        <Table data-testid="stock-ledger-table">
          <thead>
            <tr>
              <Th>When</Th>
              <Th>Kind</Th>
              <Th className="text-right">Qty</Th>
              <Th>From → to</Th>
              <Th>Who</Th>
              <Th>Carrier / tracking</Th>
              <Th>Links</Th>
              <Th className="text-right">Cost</Th>
            </tr>
          </thead>
          <tbody>
            {movements.map((movement) => (
              <Tr key={movement.id} className="row-hover" data-testid={`stock-ledger-row-${movement.id}`}>
                <Td className="whitespace-nowrap text-xs">{shortWhen(movement.created_at)}</Td>
                <Td className="text-xs">{movementLabel(movement.kind)}</Td>
                <Td className="text-right font-mono text-sm tabular-nums">
                  {formatSignedQty(movement.qty_milli, itemValue.unit)}
                </Td>
                <Td className="text-xs">
                  {locationName(movement.from_location_id)}
                  {" → "}
                  {locationName(movement.to_location_id)}
                </Td>
                <Td className="text-xs" title={movement.created_by ?? undefined}>
                  {movement.created_by == null
                    ? "—"
                    : (actorNames.get(movement.created_by) ?? idPrefix(movement.created_by))}
                </Td>
                <Td className="text-xs">
                  {movement.carrier == null && movement.tracking_ref == null ? (
                    <span style={{ color: "var(--m-text-muted)" }}>—</span>
                  ) : (
                    [movement.carrier, movement.tracking_ref].filter((part) => part != null).join(" · ")
                  )}
                </Td>
                <Td className="text-xs">
                  <span className="flex flex-wrap gap-1.5">
                    {movement.ticket_id != null && (
                      <Link
                        href={`/tickets/${movement.ticket_id}`}
                        style={{ color: "var(--m-status-info-fg)" }}
                        data-testid={`stock-ledger-ticket-${movement.id}`}
                      >
                        ticket
                      </Link>
                    )}
                    {movement.client_id != null && (
                      <Link
                        href={`/clients/${movement.client_id}`}
                        style={{ color: "var(--m-status-info-fg)" }}
                        data-testid={`stock-ledger-client-${movement.id}`}
                      >
                        client
                      </Link>
                    )}
                    {movement.ticket_id == null && movement.client_id == null && (
                      <span style={{ color: "var(--m-text-muted)" }}>—</span>
                    )}
                  </span>
                </Td>
                <Td className="text-right font-mono text-xs tabular-nums">
                  {movement.cost_minor == null ? (
                    <span style={{ color: "var(--m-text-muted)" }}>—</span>
                  ) : (
                    money(movement.cost_minor, movement.currency ?? itemValue.currency)
                  )}
                </Td>
              </Tr>
            ))}
            {movements.length === 0 && (
              <Tr>
                <Td>
                  <EmptyState message="No movements yet. Record a receive to start the ledger." />
                </Td>
              </Tr>
            )}
          </tbody>
        </Table>
      </Card>

      <MovementDialog
        open={movementOpen}
        onOpenChange={setMovementOpen}
        items={[itemValue]}
        locations={locations}
        clients={clients}
        initialKind={movementKind}
        initialItemId={itemValue.id}
        onRecorded={() => {
          setNotice("Movement recorded.");
          void refresh();
        }}
      />

      <CheckInDialog
        open={checkinId !== null}
        onOpenChange={(open) => {
          if (!open) setCheckinId(null);
        }}
        checkoutId={checkinId}
        locations={locations}
        serialLabels={serialLabels}
        itemLabels={itemLabels}
        onCheckedIn={(checkoutId) => void recordCheckin(checkoutId)}
      />

      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent title={`Edit ${itemValue.name}`} data-testid="stock-edit-dialog">
          <div className="grid gap-3 overflow-auto p-1" style={{ maxHeight: "80vh" }}>
            <div className="grid gap-1">
              <Label htmlFor="stock-edit-name">Name</Label>
              <Input
                id="stock-edit-name"
                value={form.name}
                onChange={(e) => setForm({ ...form, name: e.target.value })}
                data-testid="stock-edit-name"
              />
            </div>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
              <div className="grid gap-1">
                <Label htmlFor="stock-edit-sku">SKU</Label>
                <Input
                  id="stock-edit-sku"
                  value={form.sku}
                  onChange={(e) => setForm({ ...form, sku: e.target.value })}
                  data-testid="stock-edit-sku"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="stock-edit-part">Part number</Label>
                <Input
                  id="stock-edit-part"
                  value={form.partNumber}
                  onChange={(e) => setForm({ ...form, partNumber: e.target.value })}
                  data-testid="stock-edit-part"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="stock-edit-unit">Unit</Label>
                <Input
                  id="stock-edit-unit"
                  value={form.unit}
                  onChange={(e) => setForm({ ...form, unit: e.target.value })}
                  data-testid="stock-edit-unit"
                />
              </div>
            </div>
            <label className="flex items-center gap-2 text-xs" style={{ color: "var(--m-text-secondary)" }}>
              <Checkbox
                checked={form.active}
                onCheckedChange={(value) => setForm({ ...form, active: value === true })}
                data-testid="stock-edit-active"
              />
              Active
            </label>
            {serialized && (
              <label className="flex items-center gap-2 text-xs" style={{ color: "var(--m-text-secondary)" }}>
                <Checkbox
                  checked={form.isTool}
                  onCheckedChange={(value) => setForm({ ...form, isTool: value === true })}
                  data-testid="stock-edit-is-tool"
                />
                Checkout-eligible tool
              </label>
            )}
            <FieldHint>{serialized ? "Flagging adds the item to the tools board. It must already be serialized." : "Kind stays as recorded."}</FieldHint>
            {formError !== null && <FieldError data-testid="stock-edit-error">{formError}</FieldError>}
            <DialogActions>
              <Button variant="secondary" onClick={() => setEditOpen(false)}>
                Cancel
              </Button>
              <Button disabled={formBusy} onClick={() => void saveEdit()} data-testid="stock-edit-save">
                {formBusy ? "Saving…" : "Save"}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog
        open={thresholdRow !== null}
        onOpenChange={(open) => {
          if (!open) setThresholdRow(null);
        }}
      >
        <DialogContent title="Location thresholds" data-testid="stock-thresholds-dialog">
          <div className="grid gap-3 overflow-auto p-1" style={{ maxHeight: "80vh" }}>
            {thresholdRow !== null && (
              <>
                <p className="text-sm" style={{ color: "var(--m-text-secondary)" }}>
                  {thresholdRow.location_name} · on hand {formatQtyMilli(thresholdRow.qty_milli, thresholdRow.unit)}.
                  Leave a field empty to clear that threshold.
                </p>
                <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
                  <div className="grid gap-1">
                    <Label htmlFor="threshold-min">Minimum</Label>
                    <Input
                      id="threshold-min"
                      inputMode="decimal"
                      value={thresholdMin}
                      onChange={(e) => setThresholdMin(e.target.value)}
                      data-testid="stock-threshold-min"
                    />
                  </div>
                  <div className="grid gap-1">
                    <Label htmlFor="threshold-reorder">Reorder point</Label>
                    <Input
                      id="threshold-reorder"
                      inputMode="decimal"
                      value={thresholdReorder}
                      onChange={(e) => setThresholdReorder(e.target.value)}
                      data-testid="stock-threshold-reorder"
                    />
                  </div>
                  <div className="grid gap-1">
                    <Label htmlFor="threshold-max">Maximum</Label>
                    <Input
                      id="threshold-max"
                      inputMode="decimal"
                      value={thresholdMax}
                      onChange={(e) => setThresholdMax(e.target.value)}
                      data-testid="stock-threshold-max"
                    />
                  </div>
                </div>
                <FieldHint>
                  The reorder point drives the low tag and the low-stock banner. The minimum sits below it.
                </FieldHint>
                {thresholdError !== null && <FieldError data-testid="stock-threshold-error">{thresholdError}</FieldError>}
                <DialogActions>
                  <Button variant="secondary" onClick={() => setThresholdRow(null)}>
                    Cancel
                  </Button>
                  <Button disabled={thresholdBusy} onClick={() => void saveThresholds()} data-testid="stock-threshold-save">
                    {thresholdBusy ? "Saving…" : "Save thresholds"}
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
