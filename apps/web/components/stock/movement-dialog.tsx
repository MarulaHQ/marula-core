// MovementDialog — the one stock-movement form (Wave 32 lane 4).
//
// Purpose: every physical-goods action (receive, consume, adjust, transfer,
// ship-out) runs through this dialog, so the same form serves the stock
// board, item detail, a purchase receipt's follow-up, and a ticket's
// "Parts used" flow without a surface switch. Availability at the chosen
// location is read before the write so an over-consumption is visible
// before it is rejected.
//
// Non-purpose: no purchase-order receiving (routes/purchasing owns that
// dialog), no kit checkout (the tools surface owns that), no offline queue.
"use client";

import {
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  FieldHint,
  FieldError,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Tag,
  Textarea,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api } from "../../lib/api";
import { rejectionText } from "../../lib/error-copy";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { formatQtyMilli, parseQtyToMilli } from "../../lib/stock-format";

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
  /** Hide the kind switcher (e.g. the ticket parts-used flow). */
  lockKind?: boolean;
  title?: string;
  onRecorded(movement: StockMovementView): void;
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
  lockKind = false,
  title,
  onRecorded,
}: MovementDialogProps): ReactNode {
  const [kind, setKind] = useState<MovementKind>(initialKind);
  const [itemId, setItemId] = useState<string>("");
  const [fromLocationId, setFromLocationId] = useState<string>("");
  const [toLocationId, setToLocationId] = useState<string>("");
  const [qtyText, setQtyText] = useState("1");
  const [unitCostText, setUnitCostText] = useState("");
  const [serialNumbersText, setSerialNumbersText] = useState("");
  const [serialId, setSerialId] = useState<string>("");
  const [serialOptions, setSerialOptions] = useState<StockSerialView[] | null>(null);
  const [clientId, setClientId] = useState<string>("");
  const [carrier, setCarrier] = useState("");
  const [trackingRef, setTrackingRef] = useState("");
  const [note, setNote] = useState("");
  const [availableMilli, setAvailableMilli] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const item = useMemo(() => items.find((i) => i.id === itemId) ?? null, [items, itemId]);
  const serialized = item?.kind === "serialized";
  const outbound = kind === "consume" || kind === "transfer" || kind === "ship-out";
  const fromLocation =
    kind === "transfer" ? fromLocationId : kind === "receive" ? toLocationId : fromLocationId;

  useEffect(() => {
    if (!open) return;
    setKind(initialKind);
    setItemId(initialItemId ?? "");
    setFromLocationId("");
    setToLocationId("");
    setQtyText("1");
    setUnitCostText("");
    setSerialNumbersText("");
    setSerialId("");
    setSerialOptions(null);
    setClientId("");
    setCarrier("");
    setTrackingRef("");
    setNote("");
    setAvailableMilli(null);
    setError(null);
    setBusy(false);
  }, [open, initialKind, initialItemId]);

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
            };
          case "consume":
            return {
              ...common,
              location_id: fromLocationId,
              qty_milli: qtyMilli,
              serial_id: serialized ? serialId : null,
              ticket_id: ticketId ?? null,
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
              ticket_id: ticketId ?? null,
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
    onRecorded(recorded ?? ({} as StockMovementView));
    onOpenChange(false);
  }

  function qtyMilliSigned(): number {
    const trimmed = qtyText.trim();
    const parsed = parseQtyToMilli(trimmed.startsWith("-") ? trimmed.slice(1) : trimmed);
    if (parsed === null) return 0;
    return trimmed.startsWith("-") ? -parsed : parsed;
  }

  const unitHint = item === null ? undefined : item.unit;

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

          <div className="grid gap-1">
            <Label htmlFor="movement-item">Item</Label>
            <Select
              value={itemId}
              onValueChange={(value) => {
                setItemId(value);
                setSerialId("");
              }}
            >
              <SelectTrigger id="movement-item" data-testid="movement-item">
                <SelectValue placeholder="Pick a stock item" />
              </SelectTrigger>
              <SelectContent>
                {items.map((i) => (
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

          {outbound && availableMilli !== null && !serialized && (
            <p className="text-xs" style={{ color: "var(--m-text-secondary)" }} data-testid="movement-available">
              On hand at that location:{" "}
              <span className="font-mono tabular-nums">
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
                  <SelectValue placeholder={serialOptions?.length ? "Pick a unit" : "No in-stock units here"} />
                </SelectTrigger>
                <SelectContent>
                  {(serialOptions ?? []).map((s) => (
                    <SelectItem key={s.id} value={s.id}>
                      {s.serial_number}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
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
                <Input id="movement-carrier" value={carrier} onChange={(e) => setCarrier(e.target.value)} />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="movement-tracking">Tracking ref</Label>
                <Input id="movement-tracking" value={trackingRef} onChange={(e) => setTrackingRef(e.target.value)} />
              </div>
            </div>
          )}

          <div className="grid gap-1">
            <Label htmlFor="movement-note">Note</Label>
            <Input id="movement-note" value={note} onChange={(e) => setNote(e.target.value)} data-testid="movement-note" />
          </div>

          {ticketId !== undefined && (
            <Tag tone="outline" className="w-fit" data-testid="movement-ticket-anchor">
              Charges ticket {ticketId.slice(0, 8)}…
            </Tag>
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
