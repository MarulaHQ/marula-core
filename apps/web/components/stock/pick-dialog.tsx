// PickDialog — pin a stock item to a ticket's day pack (Wave 32 lane 8).
//
// A pick is INTENT, not a movement: it says "this job needs X", and the
// movement ledger stays untouched. The day pack reads picks back as grab
// lines. The ticket page and a checklist reminder both open this dialog.
"use client";

import {
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
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useEffect, useMemo, useState } from "react";

import { pinItem } from "../../lib/day-pack";

type StockItemView = components["schemas"]["StockItemView"];

export interface PickDialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  ticketId: string;
  /** Human anchor copy ("DEMO-1078 · Loud fan"). */
  ticketLabel?: string;
  items: StockItemView[];
  onPinned(): void;
  /** Seed the item select (a checklist reminder's matched item). */
  initialItemId?: string;
}

export function PickDialog({
  open,
  onOpenChange,
  ticketId,
  ticketLabel,
  items,
  onPinned,
  initialItemId,
}: PickDialogProps) {
  const [itemId, setItemId] = useState("");
  const [qty, setQty] = useState("1");
  const [note, setNote] = useState("");
  const [filter, setFilter] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setItemId(initialItemId ?? "");
    setQty("1");
    setNote("");
    setFilter("");
    setBusy(false);
    setError(null);
  }, [open, initialItemId]);

  const active = useMemo(() => items.filter((item) => item.active), [items]);
  const filtered = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    if (needle === "") return active;
    return active.filter((item) =>
      [item.name, item.sku ?? "", item.part_number ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(needle),
    );
  }, [active, filter]);

  const selected = active.find((item) => item.id === itemId);
  const qtyNumber = Number.parseFloat(qty);
  const valid = itemId !== "" && Number.isFinite(qtyNumber) && qtyNumber > 0;

  async function submit(): Promise<void> {
    if (!valid) {
      setError("Pick an item and a positive quantity.");
      return;
    }
    setBusy(true);
    setError(null);
    const failure = await pinItem({
      ticket_id: ticketId,
      stock_item_id: itemId,
      qty_milli: Math.round(qtyNumber * 1000),
      note: note.trim() === "" ? null : note.trim(),
    });
    setBusy(false);
    if (failure !== null) {
      setError(failure);
      return;
    }
    onPinned();
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title="Pin to the day pack" data-testid="day-pack-pick-dialog">
        <div className="grid gap-3 p-1">
          <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
            {ticketLabel !== undefined && ticketLabel !== ""
              ? `Pinning for ${ticketLabel}.`
              : "Pinning for this ticket."}{" "}
            The ledger does not move until you check the item out or use it.
          </p>
          <div className="grid gap-1">
            <Label htmlFor="pick-filter">Find an item</Label>
            <Input
              id="pick-filter"
              value={filter}
              placeholder="Search by name, SKU, or part number"
              onChange={(e) => setFilter(e.target.value)}
            />
          </div>
          <div className="grid gap-1">
            <Label htmlFor="pick-item">Item</Label>
            <Select value={itemId} onValueChange={setItemId}>
              <SelectTrigger id="pick-item" data-testid="pick-item">
                <SelectValue placeholder="Pick a stock item" />
              </SelectTrigger>
              <SelectContent>
                {filtered.map((item) => (
                  <SelectItem key={item.id} value={item.id}>
                    {item.name}
                    {item.sku !== null && item.sku !== undefined ? ` · ${item.sku}` : ""}
                    {item.is_tool ? " · tool" : ""}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {filtered.length === 0 && (
              <FieldHint>No active stock items match that search.</FieldHint>
            )}
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1">
              <Label htmlFor="pick-qty">
                Quantity{selected !== undefined ? ` (${selected.unit})` : ""}
              </Label>
              <Input
                id="pick-qty"
                type="number"
                min={1}
                step="1"
                value={qty}
                onChange={(e) => setQty(e.target.value)}
              />
            </div>
            <div className="grid gap-1">
              <Label htmlFor="pick-note">Note</Label>
              <Input
                id="pick-note"
                value={note}
                placeholder="Why this is needed"
                onChange={(e) => setNote(e.target.value)}
              />
            </div>
          </div>
          {error !== null && <FieldError>{error}</FieldError>}
        </div>
        <DialogActions>
          <Button variant="ghost" size="sm" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button size="sm" disabled={busy || !valid} onClick={() => void submit()}>
            {busy ? "Pinning…" : "Pin to day pack"}
          </Button>
        </DialogActions>
      </DialogContent>
    </Dialog>
  );
}
