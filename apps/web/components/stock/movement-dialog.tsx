// MovementDialog — the one stock-movement form (Wave 32 lane 4).
//
// Purpose: every physical-goods action (receive, consume, adjust, transfer,
// ship-out) runs through this dialog, so the same form serves the stock
// board, item detail, a purchase receipt's follow-up, and a ticket's
// "Parts used" flow without a surface switch. Availability at the chosen
// location is read before the write so an over-consumption is visible
// before it is rejected.
//
// Wave 32 lane 6 (`p32-stock-flows-context`) makes it context-embedding:
// an optional ScanBar fills the form, a filter tames long item lists, and a
// ticket picker lets a stockroom operator attach a movement to a ticket
// without opening that ticket. None of it submits — scans and picks fill,
// the human records.
//
// Non-purpose: no purchase-order receiving (routes/purchasing owns that
// dialog), no kit checkout (the tools surface owns that), no offline queue.
"use client";

import {
  Badge,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  FieldError,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Textarea,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api } from "../../lib/api";
import { rejectionText } from "../../lib/error-copy";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { formatQtyMilli, parseQtyToMilli } from "../../lib/stock-format";
import { ScanBar, type ScanEvent } from "../scan/scan-bar";

type StockItemView = components["schemas"]["StockItemView"];
type StockLocationView = components["schemas"]["StockLocationView"];
type StockMovementView = components["schemas"]["StockMovementView"];
type StockSerialView = components["schemas"]["StockSerialView"];

export type MovementKind = "receive" | "consume" | "adjust" | "transfer" | "ship-out";

const KIND_LABELS: { kind: MovementKind; label: string }[] = [
  { kind: "receive", label: "Receive" },
  { kind: "consume", label: "Use" },
  { kind: "adjust", label: "Adjust" },
  { kind: "transfer", label: "Transfer" },
  { kind: "ship-out", label: "Ship out" },
];

const ONE_UNIT_MILLI = 1000;

export interface MovementDialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  items: StockItemView[];
  locations: StockLocationView[];
  clients?: { id: string; name: string }[];
  initialKind?: MovementKind;
  initialItemId?: string;
  /** Ticket anchor for the consume leg (a job consumed these parts). */
  ticketId?: string;
  /**
   * Human label for the fixed anchor (e.g. "DEMO-1078 · Loud fan"). Falls
   * back to a shortened ticket id when the caller has no label.
   */
  ticketLabel?: string;
  /** Hide the kind switcher (e.g. the ticket parts-used flow). */
  lockKind?: boolean;
  title?: string;
  /** The recorded movement plus which leg it was (receipt copy). */
  onRecorded(movement: StockMovementView, kind?: MovementKind): void;
  /**
   * Render a ScanBar inside the dialog (the surface key decides what it
   * accepts). A scan fills the item / serial / location fields; it never
   * records a movement.
   */
  scanSurface?: string;
  /** Filter field above the item select — for long item lists. */
  searchableItems?: boolean;
  /** Pre-fill the quantity field. */
  initialQty?: string;
  /** Pre-select the ship-out client (e.g. the ticket's client). */
  initialClientId?: string;
  /**
   * Offer a ticket picker (consume / ship-out) so the movement can be
   * attached to a chosen ticket from a stock or receiving context without
   * opening that ticket. Not rendered when `ticketId` is fixed.
   */
  ticketOptions?: { id: string; label: string }[];
  /** Pre-select a ticket in the picker. */
  initialTicketId?: string;
}

const NO_TICKET = "__none__";

interface ItemOption {
  id: string;
  name: string;
  unit: string;
  kind: string;
  search: string;
}

export function MovementDialog({
  open,
  onOpenChange,
  items,
  locations,
  clients = [],
  initialKind = "receive",
  initialItemId,
  ticketId,
  ticketLabel,
  lockKind = false,
  title,
  onRecorded,
  scanSurface,
  searchableItems = false,
  initialQty,
  initialClientId,
  ticketOptions,
  initialTicketId,
}: MovementDialogProps): ReactNode {
  const [kind, setKind] = useState<MovementKind>(initialKind);
  const [itemId, setItemId] = useState<string>("");
  const [itemFilter, setItemFilter] = useState("");
  const [scannedItem, setScannedItem] = useState<ItemOption | null>(null);
  const [fromLocationId, setFromLocationId] = useState<string>("");
  const [toLocationId, setToLocationId] = useState<string>("");
  const [qtyText, setQtyText] = useState("1");
  const [unitCostText, setUnitCostText] = useState("");
  const [serialNumbersText, setSerialNumbersText] = useState("");
  const [serialId, setSerialId] = useState<string>("");
  const [scannedSerial, setScannedSerial] = useState<{ id: string; label: string } | null>(null);
  const [serialOptions, setSerialOptions] = useState<StockSerialView[] | null>(null);
  const [clientId, setClientId] = useState<string>("");
  const [carrier, setCarrier] = useState("");
  const [trackingRef, setTrackingRef] = useState("");
  const [note, setNote] = useState("");
  const [ticketIdState, setTicketIdState] = useState<string>("");
  const [ticketFilter, setTicketFilter] = useState("");
  const [availableMilli, setAvailableMilli] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const itemOptions = useMemo<ItemOption[]>(() => {
    const base: ItemOption[] = items.map((i) => ({
      id: i.id,
      name: i.name,
      unit: i.unit,
      kind: i.kind,
      search: `${i.name} ${i.sku ?? ""} ${i.part_number ?? ""}`.toLowerCase(),
    }));
    if (scannedItem !== null && !base.some((option) => option.id === scannedItem.id)) {
      base.unshift(scannedItem);
    }
    const needle = itemFilter.trim().toLowerCase();
    if (!searchableItems || needle === "") return base;
    return base.filter((option) => option.search.includes(needle));
  }, [items, itemFilter, scannedItem, searchableItems]);

  const item = useMemo(
    () => itemOptions.find((option) => option.id === itemId) ?? null,
    [itemOptions, itemId],
  );
  const serialized = item?.kind === "serialized";
  const outbound = kind === "consume" || kind === "transfer" || kind === "ship-out";
  const fromLocation =
    kind === "transfer" ? fromLocationId : kind === "receive" ? toLocationId : fromLocationId;

  /** The ticket the movement will carry: fixed anchor, picked one, or none. */
  const anchoredTicketId =
    ticketId !== undefined
      ? ticketId
      : ticketIdState === "" || ticketIdState === NO_TICKET
        ? null
        : ticketIdState;

  const chosenTicket =
    ticketOptions?.find((candidate) => candidate.id === anchoredTicketId) ?? null;

  const visibleTickets = useMemo(() => {
    if (ticketOptions === undefined) return [];
    const needle = ticketFilter.trim().toLowerCase();
    if (needle === "") return ticketOptions;
    return ticketOptions.filter((ticket) => ticket.label.toLowerCase().includes(needle));
  }, [ticketOptions, ticketFilter]);

  /** Serial choices = in-stock units at the location + any scanned one. */
  const serialChoices = useMemo(() => {
    const base = (serialOptions ?? []).map((serial) => ({
      id: serial.id,
      label: serial.serial_number,
    }));
    if (scannedSerial !== null && !base.some((serial) => serial.id === scannedSerial.id)) {
      base.unshift(scannedSerial);
    }
    return base;
  }, [serialOptions, scannedSerial]);

  useEffect(() => {
    if (!open) return;
    setKind(initialKind);
    setItemId(initialItemId ?? "");
    setItemFilter("");
    setScannedItem(null);
    setFromLocationId("");
    setToLocationId("");
    setQtyText(initialQty ?? "1");
    setUnitCostText("");
    setSerialNumbersText("");
    setSerialId("");
    setScannedSerial(null);
    setSerialOptions(null);
    setClientId(initialClientId ?? "");
    setCarrier("");
    setTrackingRef("");
    setNote("");
    setTicketIdState(initialTicketId ?? "");
    setTicketFilter("");
    setAvailableMilli(null);
    setError(null);
    setBusy(false);
  }, [open, initialKind, initialItemId, initialQty, initialClientId, initialTicketId]);

  /**
   * A scan fills the form — the shared focus law applies (ScanBar swallows
   * the burst Enter and never submits). Items/serials pick the subject; a
   * location scan sets the source (or the receive destination).
   */
  function handleScan({ resolution }: ScanEvent): void {
    if (resolution.kind === "stock_location") {
      const locationId = resolution.location_id ?? "";
      if (kind === "receive") setToLocationId(locationId);
      else setFromLocationId(locationId);
      return;
    }
    const resolvedItemId = resolution.stock_item_id ?? resolution.entity_id;
    if (resolvedItemId === null || resolvedItemId === undefined || resolvedItemId === "") {
      return;
    }
    setItemId(resolvedItemId);
    setScannedItem({
      id: resolvedItemId,
      name: resolution.display_name !== "" ? resolution.display_name : "Scanned item",
      unit: "",
      kind: resolution.item_kind === "serialized" ? "serialized" : "consumable",
      search: `${resolution.display_name} ${resolution.secondary ?? ""}`.toLowerCase(),
    });
    if (resolution.kind === "serialized_unit" && resolution.serial_id != null) {
      setSerialId(resolution.serial_id);
      setScannedSerial({
        id: resolution.serial_id,
        label: resolution.secondary ?? resolution.display_name,
      });
    } else {
      setSerialId("");
      setScannedSerial(null);
    }
  }

  // Availability read at the chosen location — the pre-write check.
  useEffect(() => {
    if (!open || !outbound || itemId === "" || fromLocation === "") {
      setAvailableMilli(null);
      return;
    }
    let cancelled = false;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/stock/on-hand", {
        params: { query: { item_id: itemId, location_id: fromLocation, limit: 5 } },
      });
      if (cancelled) return;
      const row = data?.[0];
      setAvailableMilli(row?.qty_milli ?? 0);
    })();
    return () => {
      cancelled = true;
    };
  }, [open, outbound, itemId, fromLocation]);

  // Serialized OUT legs pick the physical unit explicitly.
  useEffect(() => {
    if (!open || !(outbound && serialized) || itemId === "" || fromLocation === "") {
      setSerialOptions(null);
      setSerialId("");
      return;
    }
    let cancelled = false;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/stock-items/{id}/serials", {
        params: { path: { id: itemId }, query: { status: "in_stock", location_id: fromLocation, limit: 100 } },
      });
      if (cancelled) return;
      setSerialOptions(data ?? []);
    })();
    return () => {
      cancelled = true;
    };
  }, [open, outbound, serialized, itemId, fromLocation]);

  const serialLines = serialNumbersText
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "");
  const qtyMilli = serialized
    ? serialLines.length * ONE_UNIT_MILLI
    : kind === "adjust"
      ? qtyMilliSigned()
      : (parseQtyToMilli(qtyText) ?? 0);

  const overAvailable =
    outbound && availableMilli !== null && !serialized && qtyMilli > availableMilli;

  async function submit(): Promise<void> {
    setError(null);
    if (itemId === "") {
      setError("Pick an item.");
      return;
    }
    if (kind !== "receive" && fromLocationId === "" && kind !== "ship-out") {
      setError("Pick the source location.");
      return;
    }
    if (kind === "ship-out" && fromLocationId === "") {
      setError("Pick the source location.");
      return;
    }
    if ((kind === "receive" || kind === "transfer") && toLocationId === "") {
      setError("Pick the destination location.");
      return;
    }
    if (kind === "transfer" && fromLocationId === toLocationId) {
      setError("Transfer needs two different locations.");
      return;
    }
    if (serialized) {
      if (kind === "receive") {
        if (serialLines.length === 0) {
          setError("A serialized receive needs one serial number per unit, one per line.");
          return;
        }
      } else if (outbound && serialId === "") {
        setError("Pick the serial unit moving out.");
        return;
      }
    } else if (kind === "adjust" ? qtyMilli === 0 : qtyMilli <= 0) {
      setError(
        kind === "adjust"
          ? "Adjust needs a signed quantity (negative shrinks the count)."
          : "Quantity must be greater than zero.",
      );
      return;
    }
    if (overAvailable) {
      setError(`Only ${formatQtyMilli(availableMilli ?? 0, item?.unit ?? "")} is at that location.`);
      return;
    }

    setBusy(true);
    const unitCostMinor =
      unitCostText.trim() === "" ? null : Math.round(Number(unitCostText) * 100) || null;
    let recorded: StockMovementView | null = null;

    const result = await runEnvelopeMutation({
      describe: `Record ${kind}`,
      buildBody: (base) => {
        const common = { item_id: itemId, note: note.trim() === "" ? null : note, ...base };
        switch (kind) {
          case "receive":
            return {
              ...common,
              location_id: toLocationId,
              qty_milli: qtyMilli,
              unit_cost_minor: unitCostMinor,
              serial_numbers: serialized ? serialLines : null,
              ticket_id: anchoredTicketId,
            };
          case "consume":
            return {
              ...common,
              location_id: fromLocationId,
              qty_milli: qtyMilli,
              serial_id: serialized ? serialId : null,
              ticket_id: anchoredTicketId,
            };
          case "adjust":
            return { ...common, location_id: fromLocationId, qty_milli: qtyMilliSigned() };
          case "transfer":
            return {
              ...common,
              from_location_id: fromLocationId,
              to_location_id: toLocationId,
              qty_milli: qtyMilli,
              serial_id: serialized ? serialId : null,
            };
          case "ship-out":
            return {
              ...common,
              location_id: fromLocationId,
              qty_milli: qtyMilli,
              serial_id: serialized ? serialId : null,
              client_id: clientId === "" ? null : clientId,
              carrier: carrier.trim() === "" ? null : carrier,
              tracking_ref: trackingRef.trim() === "" ? null : trackingRef,
              ticket_id: anchoredTicketId,
            };
        }
      },
      optimistic: () => {},
      revert: () => {},
      request: async (client, body) => {
        const response =
          kind === "receive"
            ? await client.POST("/api/v1/stock/receive", {
                body: body as components["schemas"]["ReceiveRequest"],
              })
            : kind === "consume"
              ? await client.POST("/api/v1/stock/consume", {
                  body: body as components["schemas"]["ConsumeRequest"],
                })
              : kind === "adjust"
                ? await client.POST("/api/v1/stock/adjust", {
                    body: body as components["schemas"]["AdjustRequest"],
                  })
                : kind === "transfer"
                  ? await client.POST("/api/v1/stock/transfer", {
                      body: body as components["schemas"]["TransferRequest"],
                    })
                  : await client.POST("/api/v1/stock/ship-out", {
                      body: body as components["schemas"]["ShipOutRequest"],
                    });
        if (response.data !== undefined) recorded = response.data.movement;
        return response;
      },
      versions: (data) => (data.mutation === undefined ? [] : [[data.movement.id, data.mutation.version]]),
    });
    setBusy(false);
    if (result.ok === false) {
      setError(rejectionText(result, "The movement was refused."));
      return;
    }
    onRecorded(recorded ?? ({} as StockMovementView), kind);
    onOpenChange(false);
  }

  function qtyMilliSigned(): number {
    const trimmed = qtyText.trim();
    const parsed = parseQtyToMilli(trimmed.startsWith("-") ? trimmed.slice(1) : trimmed);
    if (parsed === null) return 0;
    return trimmed.startsWith("-") ? -parsed : parsed;
  }

  const unitHint = item === null || item.unit === "" ? undefined : item.unit;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={title ?? (ticketId !== undefined ? "Parts used" : "Record movement")}>
        <div className="grid max-h-[80vh] gap-3 overflow-auto p-1">
          {!lockKind && (
            <div className="flex flex-wrap gap-1" role="tablist" aria-label="Movement kind">
              {KIND_LABELS.map((entry) => (
                <Button
                  key={entry.kind}
                  size="sm"
                  variant={kind === entry.kind ? "primary" : "secondary"}
                  role="tab"
                  aria-selected={kind === entry.kind}
                  onClick={() => setKind(entry.kind)}
                  data-testid={`movement-kind-${entry.kind}`}
                >
                  {entry.label}
                </Button>
              ))}
            </div>
          )}

          {scanSurface !== undefined && (
            <div data-testid="movement-scan">
              <ScanBar
                surface={scanSurface}
                onScan={handleScan}
                hint="Scan an item, a serial unit, or a bin — it fills the form below, never records on its own."
              />
            </div>
          )}

          <div className="grid gap-1">
            <Label htmlFor="movement-item">Item</Label>
            {searchableItems && (
              <Input
                value={itemFilter}
                onChange={(e) => setItemFilter(e.target.value)}
                placeholder="Search items by name, SKU, or part number"
                aria-label="Filter stock items"
                data-testid="movement-item-filter"
              />
            )}
            <Select
              value={itemId}
              onValueChange={(value) => {
                setItemId(value);
                setSerialId("");
                setScannedSerial(null);
              }}
            >
              <SelectTrigger id="movement-item" data-testid="movement-item">
                <SelectValue placeholder="Pick a stock item" />
              </SelectTrigger>
              <SelectContent>
                {itemOptions.map((i) => (
                  <SelectItem key={i.id} value={i.id}>
                    {i.name}
                    {i.kind === "serialized" ? " (serialized)" : ""}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            {kind === "receive" ? (
              <div className="grid gap-1">
                <Label htmlFor="movement-to">Receive into</Label>
                <Select value={toLocationId} onValueChange={setToLocationId}>
                  <SelectTrigger id="movement-to" data-testid="movement-to-location">
                    <SelectValue placeholder="Pick a location" />
                  </SelectTrigger>
                  <SelectContent>
                    {locations.map((l) => (
                      <SelectItem key={l.id} value={l.id}>
                        {l.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {anchoredTicketId !== null && (
                  <FieldHint>
                    Receiving with a ticket anchor records a return or restock on that ticket.
                  </FieldHint>
                )}
              </div>
            ) : (
              <div className="grid gap-1">
                <Label htmlFor="movement-from">
                  {kind === "ship-out" ? "Ship from" : kind === "consume" ? "Use from" : "Move from"}
                </Label>
                <Select value={fromLocationId} onValueChange={setFromLocationId}>
                  <SelectTrigger id="movement-from" data-testid="movement-from-location">
                    <SelectValue placeholder="Pick a location" />
                  </SelectTrigger>
                  <SelectContent>
                    {locations.map((l) => (
                      <SelectItem key={l.id} value={l.id}>
                        {l.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}
            {kind === "transfer" && (
              <div className="grid gap-1">
                <Label htmlFor="movement-transfer-to">Move to</Label>
                <Select value={toLocationId} onValueChange={setToLocationId}>
                  <SelectTrigger id="movement-transfer-to" data-testid="movement-transfer-to">
                    <SelectValue placeholder="Pick a location" />
                  </SelectTrigger>
                  <SelectContent>
                    {locations.map((l) => (
                      <SelectItem key={l.id} value={l.id}>
                        {l.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}
          </div>

          {ticketOptions !== undefined && ticketId === undefined && (kind === "consume" || kind === "ship-out") && (
            <div className="grid gap-1">
              <Label htmlFor="movement-ticket">Attach to ticket (optional)</Label>
              {ticketOptions.length > 8 && (
                <Input
                  value={ticketFilter}
                  onChange={(e) => setTicketFilter(e.target.value)}
                  placeholder="Filter tickets by number or title"
                  aria-label="Filter tickets"
                  data-testid="movement-ticket-filter"
                />
              )}
              <Select
                value={ticketIdState === "" ? NO_TICKET : ticketIdState}
                onValueChange={(value) => setTicketIdState(value === NO_TICKET ? "" : value)}
              >
                <SelectTrigger id="movement-ticket" data-testid="movement-ticket">
                  <SelectValue placeholder="No ticket — stock only" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={NO_TICKET}>No ticket — stock only</SelectItem>
                  {visibleTickets.map((ticket) => (
                    <SelectItem key={ticket.id} value={ticket.id}>
                      {ticket.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <FieldHint>The movement lands on that ticket&apos;s timeline — no ticket visit needed.</FieldHint>
            </div>
          )}

          {outbound && availableMilli !== null && !serialized && (
            <p className="text-xs" style={{ color: "var(--m-text-secondary)" }} data-testid="movement-available">
              On hand at that location:{" "}
 <span className="tabular-nums">
                {formatQtyMilli(availableMilli, item?.unit ?? "")}
              </span>
            </p>
          )}

          {serialized && kind === "receive" ? (
            <div className="grid gap-1">
              <Label htmlFor="movement-serials">Serial numbers (one per unit, one per line)</Label>
              <Textarea
                id="movement-serials"
                rows={3}
                value={serialNumbersText}
                onChange={(e) => setSerialNumbersText(e.target.value)}
                placeholder={"SN-001\nSN-002"}
                data-testid="movement-serials"
              />
              <FieldHint>
                {serialLines.length === 0
                  ? "A serialized receive moves one physical unit per line."
                  : `${serialLines.length} unit${serialLines.length === 1 ? "" : "s"} will be received.`}
              </FieldHint>
            </div>
          ) : serialized ? (
            <div className="grid gap-1">
              <Label htmlFor="movement-serial">Serial unit</Label>
              <Select value={serialId} onValueChange={setSerialId}>
                <SelectTrigger id="movement-serial" data-testid="movement-serial">
                  <SelectValue placeholder={serialChoices.length > 0 ? "Pick a unit" : "No in-stock units here"} />
                </SelectTrigger>
                <SelectContent>
                  {serialChoices.map((s) => (
                    <SelectItem key={s.id} value={s.id}>
                      {s.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {scannedSerial !== null && serialId === scannedSerial.id &&
                !(serialOptions ?? []).some((serial) => serial.id === scannedSerial.id) && (
                  <FieldHint>
                    The scanned unit is not listed as in stock at this location — the record will
                    confirm it.
                  </FieldHint>
                )}
            </div>
          ) : (
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <div className="grid gap-1">
                <Label htmlFor="movement-qty">Quantity{unitHint === undefined ? "" : ` (${unitHint})`}</Label>
                <Input
                  id="movement-qty"
                  inputMode="decimal"
                  value={qtyText}
                  onChange={(e) => setQtyText(e.target.value)}
                  data-testid="movement-qty"
                />
                {kind === "adjust" && <FieldHint>A negative number shrinks the count.</FieldHint>}
              </div>
              {kind === "receive" && (
                <div className="grid gap-1">
                  <Label htmlFor="movement-cost">Unit cost (major, optional)</Label>
                  <Input
                    id="movement-cost"
                    inputMode="decimal"
                    value={unitCostText}
                    onChange={(e) => setUnitCostText(e.target.value)}
                    data-testid="movement-cost"
                  />
                  <FieldHint>The receipt locks this cost against the item currency.</FieldHint>
                </div>
              )}
            </div>
          )}

          {kind === "ship-out" && (
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
              <div className="grid gap-1">
                <Label htmlFor="movement-client">Client (optional)</Label>
                <Select value={clientId} onValueChange={setClientId}>
                  <SelectTrigger id="movement-client" data-testid="movement-client">
                    <SelectValue placeholder="No client" />
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
              <div className="grid gap-1">
                <Label htmlFor="movement-carrier">Carrier</Label>
                <Input
                  id="movement-carrier"
                  value={carrier}
                  onChange={(e) => setCarrier(e.target.value)}
                  data-testid="movement-carrier"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="movement-tracking">Tracking ref</Label>
                <Input
                  id="movement-tracking"
                  value={trackingRef}
                  onChange={(e) => setTrackingRef(e.target.value)}
                  data-testid="movement-tracking"
                />
              </div>
            </div>
          )}

          <div className="grid gap-1">
            <Label htmlFor="movement-note">Note</Label>
            <Input id="movement-note" value={note} onChange={(e) => setNote(e.target.value)} data-testid="movement-note" />
          </div>

          {anchoredTicketId !== null && (
            <Badge tone="neutral" className="w-fit" data-testid="movement-ticket-anchor">
              {chosenTicket !== null
                ? `Charges ${chosenTicket.label}`
                : ticketLabel !== undefined
                  ? `Charges ${ticketLabel}`
                  : `Charges ticket ${anchoredTicketId.slice(0, 8)}…`}
            </Badge>
          )}

          {error !== null && <FieldError data-testid="movement-error">{error}</FieldError>}

          <DialogActions>
            <Button variant="secondary" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button disabled={busy} onClick={() => void submit()} data-testid="movement-save">
              {busy ? "Recording…" : "Record movement"}
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}
