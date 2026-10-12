// CheckInDialog — the one tool check-in form (Wave 32 lane 4).
//
// Purpose: returns outstanding tool units through the same form whether the
// caller starts from the tools board or from a tool item's detail page: one
// row per outstanding line with its condition, one destination location, and
// notes. The description of a line comes from the caller's label maps (serial
// number + item name) because the checkout read carries ids only.
//
// Non-purpose: no client-loan returns (the loans register owns that flow),
// no stock transfers, no partial-quantity math (a tool line is one unit).
"use client";

import {
  Badge,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  FieldError,
  FieldHint,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Textarea,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useEffect, useState, type ReactNode } from "react";

import { api } from "../../lib/api";
import { rejectionText } from "../../lib/error-copy";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { checkoutStatusTone, duePhrase } from "../../lib/stock-format";

type ToolCheckoutDetailResponse = components["schemas"]["ToolCheckoutDetailResponse"];
type StockLocationView = components["schemas"]["StockLocationView"];

const CONDITIONS = [
  { value: "good", label: "Good" },
  { value: "fair", label: "Fair" },
  { value: "damaged", label: "Damaged" },
] as const;

export interface CheckInDialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  /** The checkout being returned; null renders nothing while closed. */
  checkoutId: string | null;
  locations: StockLocationView[];
  /** serial_id → serial number labels for readable line rows. */
  serialLabels?: Record<string, string>;
  /** stock_item_id → item name labels for readable line rows. */
  itemLabels?: Record<string, string>;
  onCheckedIn(checkoutId: string): void;
}

export function CheckInDialog({
  open,
  onOpenChange,
  checkoutId,
  locations,
  serialLabels = {},
  itemLabels = {},
  onCheckedIn,
}: CheckInDialogProps): ReactNode {
  const [detail, setDetail] = useState<ToolCheckoutDetailResponse | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [conditions, setConditions] = useState<Record<string, string>>({});
  const [notes, setNotes] = useState("");
  const [toLocationId, setToLocationId] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setDetail(null);
    setLoadFailed(false);
    setConditions({});
    setNotes("");
    setToLocationId("");
    setError(null);
    if (!open || checkoutId === null) return;
    let cancelled = false;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/tool-checkouts/{id}", {
        params: { path: { id: checkoutId } },
      });
      if (cancelled) return;
      if (data === undefined) {
        setLoadFailed(true);
        return;
      }
      setDetail(data);
      const next: Record<string, string> = {};
      for (const line of data.lines) {
        if (line.returned_at == null) next[line.id] = "good";
      }
      setConditions(next);
    })();
    return () => {
      cancelled = true;
    };
  }, [open, checkoutId]);

  const openLines = (detail?.lines ?? []).filter((line) => line.returned_at == null);
  const checkout = detail?.checkout ?? null;
  const fromLocation = checkout === null
    ? undefined
    : locations.find((l) => l.id === checkout.from_location_id);

  function lineLabel(serialId: string, stockItemId: string): string {
    const serial = serialLabels[serialId] ?? `Serial ${serialId.slice(0, 8)}…`;
    const item = itemLabels[stockItemId] ?? `Item ${stockItemId.slice(0, 8)}…`;
    return `${item} · ${serial}`;
  }

  async function submit(): Promise<void> {
    const id = checkoutId;
    if (id === null || detail === null) return;
    setError(null);
    if (openLines.length === 0) {
      setError("Every unit on this checkout is already back.");
      return;
    }
    setBusy(true);
    const lines = openLines.map((line) => ({
      line_id: line.id,
      return_condition: conditions[line.id] ?? null,
    }));
    const result = await runEnvelopeMutation({
      describe: "Check in tools",
      entityId: id,
      buildBody: (base) => ({
        lines,
        notes: notes.trim() === "" ? null : notes,
        to_location_id: toLocationId === "" ? null : toLocationId,
        ...base,
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.POST("/api/v1/tool-checkouts/{id}/checkin", {
          params: { path: { id } },
          body: body as components["schemas"]["CheckinRequest"],
        }),
      versions: (data) => [[data.checkout.id, data.mutation.version]],
    });
    setBusy(false);
    if (result.ok === false) {
      setError(rejectionText(result, "The check-in was refused."));
      return;
    }
    onCheckedIn(id);
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title="Check in tools" data-testid="tools-checkin-dialog">
        <div className="grid gap-3 overflow-auto p-1" style={{ maxHeight: "80vh" }}>
          {detail === null && !loadFailed && <Spinner />}
          {loadFailed && <EmptyState message="Could not load that checkout — close and reopen it." />}
          {detail !== null && checkout !== null && (
            <>
              <div className="flex flex-wrap items-center gap-2 text-xs" style={{ color: "var(--m-text-secondary)" }}>
                <Badge tone={checkoutStatusTone(checkout.status, checkout.overdue)}>
                  {checkout.status === "returned" ? "returned" : "open"}
                </Badge>
                <span>Due {duePhrase(checkout.due_at)}</span>
                {fromLocation !== undefined && <span>· from {fromLocation.name}</span>}
              </div>

              {openLines.length === 0 ? (
                <EmptyState message="Everything on this checkout is already back." />
              ) : (
                <div className="grid gap-2">
                  {openLines.map((line) => (
                    <div
                      key={line.id}
                      className="grid gap-1 border p-2"
                      style={{ borderColor: "var(--m-border)" }}
                      data-testid={`tools-checkin-line-${line.id}`}
                    >
                      <p className="text-sm font-medium" style={{ color: "var(--m-text)" }}>
                        {lineLabel(line.serial_id, line.stock_item_id)}
                      </p>
                      <div className="grid gap-1">
                        <Label htmlFor={`checkin-condition-${line.id}`}>Condition</Label>
                        <Select
                          value={conditions[line.id] ?? "good"}
                          onValueChange={(value) => setConditions((cur) => ({ ...cur, [line.id]: value }))}
                        >
                          <SelectTrigger
                            id={`checkin-condition-${line.id}`}
                            aria-label={`Condition for ${lineLabel(line.serial_id, line.stock_item_id)}`}
                            data-testid={`tools-checkin-condition-${line.id}`}
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
              )}

              <div className="grid gap-1">
                <Label htmlFor="checkin-to">Return to location (optional)</Label>
                <Select value={toLocationId} onValueChange={setToLocationId}>
                  <SelectTrigger id="checkin-to" data-testid="tools-checkin-to-location">
                    <SelectValue placeholder={fromLocation === undefined ? "The checkout's location" : fromLocation.name} />
                  </SelectTrigger>
                  <SelectContent>
                    {locations.map((location) => (
                      <SelectItem key={location.id} value={location.id}>
                        {location.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FieldHint>Left empty, the tools land at the location they left from.</FieldHint>
              </div>

              <div className="grid gap-1">
                <Label htmlFor="checkin-notes">Notes</Label>
                <Textarea
                  id="checkin-notes"
                  rows={2}
                  value={notes}
                  onChange={(e) => setNotes(e.target.value)}
                  data-testid="tools-checkin-notes"
                />
              </div>

              {error !== null && <FieldError data-testid="tools-checkin-error">{error}</FieldError>}

              <DialogActions>
                <Button variant="secondary" onClick={() => onOpenChange(false)}>
                  Cancel
                </Button>
                <Button disabled={busy || openLines.length === 0} onClick={() => void submit()} data-testid="tools-checkin-save">
                  {busy ? "Checking in…" : "Check in"}
                </Button>
              </DialogActions>
            </>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
