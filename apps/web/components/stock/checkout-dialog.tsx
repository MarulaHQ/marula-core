// CheckoutDialog — the tool checkout form (Wave 32 lane 4).
//
// Purpose: sends tools out either as a whole kit or as an item list. The kit
// path reads `/tool-kits/{id}/availability` first and shows per-member
// required vs available units, so a not-ready kit is visible before the
// server rejects it with a 409. The item path picks a tool plus an optional
// serial; an empty serial lets the server take the oldest in-stock unit.
//
// Non-purpose: no client loans (the loans register owns that flow), no
// purchase receiving, no returned-tool inspection (CheckInDialog does that).
"use client";

import {
  Button,
  Card,
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
  StatusTag,
  Tag,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useEffect, useState, type ReactNode } from "react";

import { api } from "../../lib/api";
import { rejectionText } from "../../lib/error-copy";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { dateInputValue, endOfDayIso } from "../../lib/stock-format";

type StockItemView = components["schemas"]["StockItemView"];
type StockLocationView = components["schemas"]["StockLocationView"];
type StockSerialView = components["schemas"]["StockSerialView"];
type ToolKitView = components["schemas"]["ToolKitView"];
type KitAvailabilityResponse = components["schemas"]["KitAvailabilityResponse"];
type CreateCheckoutRequest = components["schemas"]["CreateCheckoutRequest"];

export interface HolderOption {
  id: string;
  label: string;
}

interface ItemRow {
  key: number;
  itemId: string;
  serialId: string;
}

export interface CheckoutDialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  items: StockItemView[];
  locations: StockLocationView[];
  kits: ToolKitView[];
  holders: HolderOption[];
  /** Pre-picked holder (the caller); the select still lists the others. */
  defaultHolderId: string;
  onCheckedOut(): void;
  /** Seed the first row (a scan or an item-detail dispatch). */
  initialItemId?: string;
  initialSerialId?: string;
  /** Display label for a seeded serial that is not in the loaded options. */
  initialSerialLabel?: string;
  /** Seed the kit path (the day pack's one-tap kit checkout). */
  initialKitId?: string;
  /** Seed the location the tools leave from (van vs shelf). */
  initialLocationId?: string;
}

export function CheckoutDialog({
  open,
  onOpenChange,
  items,
  locations,
  kits,
  holders,
  defaultHolderId,
  onCheckedOut,
  initialItemId,
  initialSerialId,
  initialSerialLabel,
  initialKitId,
  initialLocationId,
}: CheckoutDialogProps): ReactNode {
  const toolItems = items.filter((item) => item.is_tool && item.active);
  const [holderId, setHolderId] = useState("");
  const [fromLocationId, setFromLocationId] = useState("");
  const [dueDate, setDueDate] = useState(dateInputValue(1));
  const [kitId, setKitId] = useState("");
  const [rows, setRows] = useState<ItemRow[]>([{ key: 1, itemId: "", serialId: "" }]);
  const [nextKey, setNextKey] = useState(2);
  const [serialOptions, setSerialOptions] = useState<Record<number, StockSerialView[]>>({});
  const [availability, setAvailability] = useState<KitAvailabilityResponse | null>(null);
  const [availabilityFailed, setAvailabilityFailed] = useState(false);
  const [notes, setNotes] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    const seeded = initialItemId !== undefined && initialItemId !== "";
    setHolderId(defaultHolderId);
    setFromLocationId(initialLocationId ?? "");
    setDueDate(dateInputValue(1));
    setKitId(initialKitId ?? "");
    setRows([
      {
        key: 1,
        itemId: seeded ? (initialItemId ?? "") : "",
        serialId: seeded ? (initialSerialId ?? "") : "",
      },
    ]);
    setNextKey(2);
    setSerialOptions({});
    setAvailability(null);
    setAvailabilityFailed(false);
    setNotes("");
    setError(null);
    setBusy(false);
    if (seeded) {
      void (async () => {
        const { data } = await api.api.GET("/api/v1/stock-items/{id}/serials", {
          params: {
            path: { id: initialItemId ?? "" },
            query: { status: "in_stock", location_id: null, limit: 100 },
          },
        });
        const options = data ?? [];
        const scanned =
          initialSerialId === undefined || initialSerialId === ""
            ? null
            : ({
                id: initialSerialId,
                serial_number: initialSerialLabel ?? initialSerialId,
              } as StockSerialView);
        const merged =
          scanned !== null && !options.some((option) => option.id === scanned.id)
            ? [scanned, ...options]
            : options;
        setSerialOptions({ 1: merged });
      })();
    }
  }, [
    open,
    defaultHolderId,
    initialItemId,
    initialSerialId,
    initialSerialLabel,
    initialKitId,
    initialLocationId,
  ]);

  // Kit readiness at the chosen location — the pre-write check (F).
  useEffect(() => {
    if (!open || kitId === "") {
      setAvailability(null);
      setAvailabilityFailed(false);
      return;
    }
    let cancelled = false;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/tool-kits/{id}/availability", {
        params: { path: { id: kitId }, query: { location_id: fromLocationId === "" ? null : fromLocationId } },
      });
      if (cancelled) return;
      if (data === undefined) {
        setAvailability(null);
        setAvailabilityFailed(true);
        return;
      }
      setAvailability(data);
      setAvailabilityFailed(false);
    })();
    return () => {
      cancelled = true;
    };
  }, [open, kitId, fromLocationId]);

  function itemFor(row: ItemRow): StockItemView | undefined {
    return toolItems.find((item) => item.id === row.itemId);
  }

  function loadSerials(row: ItemRow): void {
    if (row.itemId === "") return;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/stock-items/{id}/serials", {
        params: {
          path: { id: row.itemId },
          query: {
            status: "in_stock",
            location_id: fromLocationId === "" ? null : fromLocationId,
            limit: 100,
          },
        },
      });
      setSerialOptions((cur) => ({ ...cur, [row.key]: data ?? [] }));
    })();
  }

  function setRow(key: number, patch: Partial<ItemRow>): void {
    setRows((cur) => cur.map((row) => (row.key === key ? { ...row, ...patch } : row)));
  }

  async function submit(): Promise<void> {
    setError(null);
    if (holderId === "") {
      setError("Pick a holder.");
      return;
    }
    if (fromLocationId === "") {
      setError("Pick the location the tools leave from.");
      return;
    }
    if (dueDate === "") {
      setError("Pick a due date.");
      return;
    }
    const chosenRows = rows.filter((row) => row.itemId !== "");
    if (kitId === "" && chosenRows.length === 0) {
      setError("Add at least one tool, or pick a kit.");
      return;
    }
    setBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Check out tools",
      buildBody: (base) => ({
        holder_user_id: holderId,
        from_location_id: fromLocationId,
        due_at: endOfDayIso(dueDate),
        kit_id: kitId === "" ? null : kitId,
        items:
          kitId === ""
            ? chosenRows.map((row) => ({
                stock_item_id: row.itemId,
                serial_id: row.serialId === "" ? null : row.serialId,
              }))
            : [],
        notes: notes.trim() === "" ? null : notes,
        ...base,
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.POST("/api/v1/tool-checkouts", { body: body as CreateCheckoutRequest }),
      versions: (data) => [[data.checkout.id, data.mutation.version]],
    });
    setBusy(false);
    if (result.ok === false) {
      setError(rejectionText(result, "The checkout was refused."));
      return;
    }
    onCheckedOut();
    onOpenChange(false);
  }

  const kitSelected = kitId !== "";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title="Check out tools" data-testid="tools-checkout-dialog">
        <div className="grid gap-3 overflow-auto p-1" style={{ maxHeight: "80vh" }}>
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
            <div className="grid gap-1">
              <Label htmlFor="checkout-holder">Holder</Label>
              <Select value={holderId} onValueChange={setHolderId}>
                <SelectTrigger id="checkout-holder" data-testid="tools-checkout-holder">
                  <SelectValue placeholder="Pick a holder" />
                </SelectTrigger>
                <SelectContent>
                  {holders.map((holder) => (
                    <SelectItem key={holder.id} value={holder.id}>
                      {holder.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="grid gap-1">
              <Label htmlFor="checkout-from">From location</Label>
              <Select value={fromLocationId} onValueChange={setFromLocationId}>
                <SelectTrigger id="checkout-from" data-testid="tools-checkout-from">
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
            </div>
            <div className="grid gap-1">
              <Label htmlFor="checkout-due">Due back</Label>
              <Input
                id="checkout-due"
                type="date"
                value={dueDate}
                onChange={(e) => setDueDate(e.target.value)}
                data-testid="tools-checkout-due"
              />
            </div>
          </div>

          <div className="grid gap-1">
            <Label htmlFor="checkout-kit">Kit (or pick tools below)</Label>
            <Select
              value={kitId}
              onValueChange={(value) => {
                setKitId(value === "__none" ? "" : value);
              }}
            >
              <SelectTrigger id="checkout-kit" data-testid="tools-checkout-kit">
                <SelectValue placeholder="No kit — pick tools" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none">No kit — pick tools</SelectItem>
                {kits
                  .filter((kit) => kit.active)
                  .map((kit) => (
                    <SelectItem key={kit.id} value={kit.id}>
                      {kit.name}
                    </SelectItem>
                  ))}
              </SelectContent>
            </Select>
          </div>

          {kitSelected && (
            <Card className="grid gap-2 p-3" data-testid="tools-checkout-kit-readiness">
              {availabilityFailed && (
                <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>
                  Could not read kit availability — the server still checks on submit.
                </p>
              )}
              {availability === null && !availabilityFailed && (
                <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                  Reading kit availability…
                </p>
              )}
              {availability !== null && (
                <>
                  <div className="flex items-center gap-2">
                    <StatusTag tone={availability.ready ? "success" : "danger"}>
                      {availability.ready ? "ready" : "not ready"}
                    </StatusTag>
                    <p className="text-sm" style={{ color: "var(--m-text)" }}>
                      {availability.ready
                        ? "Every member has enough in-stock units here."
                        : "Some members are short at this location."}
                    </p>
                  </div>
                  <div className="grid gap-1">
                    {availability.members.map((member) => (
                      <div
                        key={member.stock_item_id}
                        className="flex flex-wrap items-center justify-between gap-2 text-xs"
                        style={{ color: "var(--m-text-secondary)" }}
                        data-testid={`tools-checkout-kit-member-${member.stock_item_id}`}
                      >
                        <span>{member.item_name}</span>
                        <span className="font-mono tabular-nums">
                          {member.available_units} of {member.required_units}
                        </span>
                      </div>
                    ))}
                    {availability.members.length === 0 && (
                      <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                        This kit has no members yet.
                      </p>
                    )}
                  </div>
                </>
              )}
            </Card>
          )}

          {!kitSelected && (
            <div className="grid gap-2">
              {rows.map((row, index) => {
                const item = itemFor(row);
                const serialized = item?.kind === "serialized";
                return (
                  <div
                    key={row.key}
                    className="grid gap-2 border p-2"
                    style={{ borderColor: "var(--m-border)" }}
                    data-testid={`tools-checkout-item-row-${row.key}`}
                  >
                    <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                      <div className="grid gap-1">
                        <Label htmlFor={`checkout-item-${row.key}`}>Tool {index + 1}</Label>
                        <Select
                          value={row.itemId}
                          onValueChange={(value) => {
                            setRow(row.key, { itemId: value, serialId: "" });
                            loadSerials({ ...row, itemId: value });
                          }}
                        >
                          <SelectTrigger id={`checkout-item-${row.key}`} data-testid={`tools-checkout-item-${row.key}`}>
                            <SelectValue placeholder="Pick a tool" />
                          </SelectTrigger>
                          <SelectContent>
                            {toolItems.map((tool) => (
                              <SelectItem key={tool.id} value={tool.id}>
                                {tool.name}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                      {serialized && (
                        <div className="grid gap-1">
                          <Label htmlFor={`checkout-serial-${row.key}`}>Serial (optional)</Label>
                          <Select value={row.serialId} onValueChange={(value) => setRow(row.key, { serialId: value })}>
                            <SelectTrigger id={`checkout-serial-${row.key}`} data-testid={`tools-checkout-serial-${row.key}`}>
                              <SelectValue placeholder="Auto-pick the oldest unit" />
                            </SelectTrigger>
                            <SelectContent>
                              {(serialOptions[row.key] ?? []).map((serial) => (
                                <SelectItem key={serial.id} value={serial.id}>
                                  {serial.serial_number}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        </div>
                      )}
                    </div>
                    <div className="flex items-center justify-between gap-2">
                      {serialized ? (
                        <Tag tone="outline" className="text-xs">
                          serialized — one unit per row
                        </Tag>
                      ) : (
                        <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                          {item === undefined ? "Pick a tool for this row." : "Counted tool."}
                        </span>
                      )}
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={rows.length === 1}
                        onClick={() => setRows((cur) => cur.filter((r) => r.key !== row.key))}
                        data-testid={`tools-checkout-remove-${row.key}`}
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
                  setRows((cur) => [...cur, { key: nextKey, itemId: "", serialId: "" }]);
                  setNextKey((key) => key + 1);
                }}
                data-testid="tools-checkout-add-item"
              >
                Add tool
              </Button>
            </div>
          )}

          <div className="grid gap-1">
            <Label htmlFor="checkout-notes">Notes</Label>
            <Input
              id="checkout-notes"
              value={notes}
              onChange={(e) => setNotes(e.target.value)}
              data-testid="tools-checkout-notes"
            />
          </div>

          {error !== null && <FieldError data-testid="tools-checkout-error">{error}</FieldError>}
          <FieldHint>
            {kitSelected
              ? "A kit checkout sends every member unit; items are ignored."
              : "An empty serial lets the server take the oldest in-stock unit."}
          </FieldHint>

          <DialogActions>
            <Button variant="secondary" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button disabled={busy} onClick={() => void submit()} data-testid="tools-checkout-save">
              {busy ? "Checking out…" : "Check out"}
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}
