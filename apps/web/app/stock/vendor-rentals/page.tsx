// /stock/vendor-rentals — the gear the organisation rents IN.
//
// Purpose: record a vendor rental (the asset, the cycle, the cost, the
// deposit), read the register, and return a unit with its condition, damage or
// loss. An inbound rental is custody, never owned stock: the cost is an
// expense and the unit never enters inventory value.
//
// Non-purpose: no purchase-order flow (the receipt path lives on purchasing),
// no lease billing (client leases are /stock/loans), no vendor directory
// administration.

"use client";

import {
  Badge,
  Button,
  Card,
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
  SurfaceHeader,
  formatDayKey,
  Table,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { rejectionText } from "../../../lib/error-copy";
import { showsEmptyRegister } from "../../../lib/list-view";

import { money } from "../../../lib/format";

type VendorRental = components["schemas"]["VendorRentalView"] & {
  // Present once the re-cut SDK lands; optional so the page compiles before it.
  asset_tag?: string | null;
  asset_name?: string | null;
};

const CYCLES = [
  { value: "monthly", label: "Monthly" },
  { value: "quarterly", label: "Quarterly" },
  { value: "annual", label: "Annual" },
] as const;

const CONDITIONS = [
  { value: "good", label: "Good" },
  { value: "fair", label: "Fair" },
  { value: "damaged", label: "Damaged" },
] as const;

function idPrefix(id: string): string {
  return id.slice(0, 8);
}

/** The reader's calendar day, not UTC's: a date input is a calendar, not an instant. */
function todayIso(): string {
  const now = new Date();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${now.getFullYear()}-${month}-${day}`;
}

export default function VendorRentalsPage(): React.ReactNode {
  const auth = useRequireAuth();
  const [rentals, setRentals] = useState<VendorRental[]>([]);
  const [statusFilter, setStatusFilter] = useState("open");
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const [createOpen, setCreateOpen] = useState(false);
  const [vendorName, setVendorName] = useState("");
  const [assetTag, setAssetTag] = useState("");
  const [assetName, setAssetName] = useState("");
  const [cycle, setCycle] = useState("monthly");
  const [cost, setCost] = useState("");
  const [currency, setCurrency] = useState("USD");
  const [deposit, setDeposit] = useState("");
  const [startOn, setStartOn] = useState(todayIso());
  const [endOn, setEndOn] = useState("");
  const [notes, setNotes] = useState("");
  const [createError, setCreateError] = useState<string | null>(null);
  const [createBusy, setCreateBusy] = useState(false);
  const createSession = useRef(0);

  const [returning, setReturning] = useState<VendorRental | null>(null);
  const [returnCondition, setReturnCondition] = useState("good");
  const [lost, setLost] = useState(false);
  const [damage, setDamage] = useState("");
  const [loss, setLoss] = useState("");
  const [returnNotes, setReturnNotes] = useState("");
  const [returnError, setReturnError] = useState<string | null>(null);
  const [returnBusy, setReturnBusy] = useState(false);
  const returnSession = useRef(0);

  const requestSeq = useRef(0);
  const filterRef = useRef(statusFilter);
  filterRef.current = statusFilter;

  const refresh = useCallback(async (): Promise<void> => {
    const seq = ++requestSeq.current;
    setLoadError(null);
    try {
      // Read the filter at request time: a mutation that completes after the
      // operator changed the filter must refresh what they are looking at.
      const status = filterRef.current;
      const res = await api.api.GET("/api/v1/vendor-rentals", {
        params: { query: { status: status === "all" ? null : status } },
      });
      if (seq !== requestSeq.current) return; // a newer filter owns the screen
      if (res.data === undefined) {
        setLoadError("Could not load the vendor rentals — try reloading.");
        return;
      }
      setRentals(res.data);
    } catch {
      if (seq === requestSeq.current) {
        setLoadError("Could not load the vendor rentals — try reloading.");
      }
    } finally {
      if (seq === requestSeq.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    setLoading(true);
    void refresh();
  }, [auth.status, refresh, statusFilter]);

  const openCount = useMemo(() => rentals.filter((r) => r.status === "open").length, [rentals]);

  const submitCreate = async (): Promise<void> => {
    const session = createSession.current;
    setCreateError(null);
    const costMinor = Math.round(Number(cost) * 100);
    if (!Number.isFinite(costMinor) || costMinor <= 0) {
      setCreateError("The cycle cost must be greater than zero.");
      return;
    }
    if (!/^[A-Z]{3}$/.test(currency)) {
      setCreateError("The currency is a 3-letter code.");
      return;
    }
    if (assetTag.trim() === "" || assetName.trim() === "") {
      setCreateError("The rented unit needs a tag and a name.");
      return;
    }
    const depositMinor =
      deposit.trim() === "" ? null : Math.round(Number(deposit) * 100);
    if (depositMinor !== null && (!Number.isFinite(depositMinor) || depositMinor < 0)) {
      setCreateError("A deposit cannot be negative.");
      return;
    }
    setCreateBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Record a vendor rental",
      optimistic: () => {},
      revert: () => {},
      buildBody: (base) => ({
        vendor_name: vendorName.trim() === "" ? null : vendorName.trim(),
        asset: { tag: assetTag.trim(), name: assetName.trim(), kind: "device" },
        cycle,
        cost_minor: costMinor,
        currency,
        deposit_minor: depositMinor,
        deposit_currency: depositMinor === null ? null : currency,
        start_on: startOn,
        end_on: endOn.trim() === "" ? null : endOn,
        notes: notes.trim() === "" ? null : notes.trim(),
        ...base,
      }),
      request: (client, body) =>
        client.POST("/api/v1/vendor-rentals", {
          body: body as components["schemas"]["CreateVendorRentalRequest"],
        }),
    });
    if (!result.ok) {
      setCreateBusy(false);
      if (session === createSession.current) {
        setCreateError(rejectionText(result, "The vendor rental was refused."));
      }
      return;
    }
    // Busy until the refresh and the cleanup finish: a second submission in
    // the same dialog must not race the first one's close.
    await refresh();
    if (session !== createSession.current) {
      setCreateBusy(false);
      return;
    }
    setCreateOpen(false);
    setVendorName("");
    setAssetTag("");
    setAssetName("");
    setCost("");
    setDeposit("");
    setEndOn("");
    setNotes("");
    setCreateBusy(false);
    setNotice("Vendor rental recorded.");
  };

  const openReturn = (rental: VendorRental): void => {
    returnSession.current += 1;
    setReturning(rental);
    setReturnCondition("good");
    setLost(false);
    setDamage("");
    setLoss("");
    setReturnNotes("");
    setReturnError(null);
  };

  const submitReturn = async (): Promise<void> => {
    const rental = returning;
    if (rental === null) return;
    const session = returnSession.current;
    setReturnError(null);
    const damageMinor = damage.trim() === "" ? null : Math.round(Number(damage) * 100);
    const lossMinor = loss.trim() === "" ? null : Math.round(Number(loss) * 100);
    if (damageMinor !== null && damageMinor < 0) {
      setReturnError("A damage charge cannot be negative.");
      return;
    }
    if (lossMinor !== null && lossMinor < 0) {
      setReturnError("A loss charge cannot be negative.");
      return;
    }
    if (lost && lossMinor === null) {
      setReturnError("A lost unit needs its loss charge.");
      return;
    }
    setReturnBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Return a vendor rental",
      entityId: rental.id,
      optimistic: () => {},
      revert: () => {},
      buildBody: (base) => ({
        return_condition: lost ? null : returnCondition,
        lost,
        damage_minor: damageMinor,
        loss_minor: lossMinor,
        notes: returnNotes.trim() === "" ? null : returnNotes.trim(),
        ...base,
      }),
      request: (client, body) =>
        client.POST("/api/v1/vendor-rentals/{id}/return", {
          params: { path: { id: rental.id } },
          body: body as components["schemas"]["VendorRentalReturnRequest"],
        }),
      versions: (data) => [[data.rental.id, data.mutation.version]],
    });
    if (!result.ok) {
      setReturnBusy(false);
      if (session === returnSession.current) {
        setReturnError(rejectionText(result, "The return was refused."));
      }
      return;
    }
    await refresh();
    if (session !== returnSession.current) {
      setReturnBusy(false);
      return;
    }
    setReturning(null);
    setReturnBusy(false);
    setNotice("Vendor rental returned.");
  };

  if (auth.status === "loading") return <Spinner />;

  return (
    <div className="grid gap-4">
      <SurfaceHeader
        title="Vendor rentals"
        description={`Gear rented in from a supplier: ${rentals.length} in view, ${openCount} open here. Not owned stock, never in inventory value.`}
        actions={
          <div className="flex items-center gap-2">
            <Select value={statusFilter} onValueChange={setStatusFilter}>
              <SelectTrigger className="w-36" aria-label="Vendor rental status">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="open">Open</SelectItem>
                <SelectItem value="returned">Returned</SelectItem>
                <SelectItem value="all">All</SelectItem>
              </SelectContent>
            </Select>
            <Button
            onClick={() => {
              createSession.current += 1;
              setCreateOpen(true);
            }}
            data-testid="vendor-rentals-new"
          >
              Record a rental
            </Button>
          </div>
        }
      />

      {notice !== null ? (
        <p role="status" className="text-sm" style={{ color: "var(--m-status-success-fg)" }}>
          {notice}
        </p>
      ) : null}
      {loadError !== null ? <FieldError>{loadError}</FieldError> : null}
      {rentals.length >= 500 ? (
        <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          Showing the 500 most recent. The register holds older rentals too; switch the status
          filter to narrow this view.
        </p>
      ) : null}

      <Card>
        {loading ? (
          <div className="p-6">
            <Spinner />
          </div>
        ) : rentals.length === 0 ? (
          // A failed read is not an empty register: the error above says what
          // happened, and an "empty" claim under it would contradict it.
          showsEmptyRegister(rentals.length, loadError) ? (
            <EmptyState message="No vendor rentals in this view. Record a unit the organisation rents in, with its cycle cost and deposit." />
          ) : null
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Unit</Th>
                <Th>Vendor</Th>
                <Th>Cycle</Th>
                <Th className="text-right">Cost</Th>
                <Th className="text-right">Deposit</Th>
                <Th>Started</Th>
                <Th>Ends</Th>
                <Th>Status</Th>
                <Th />
              </tr>
            </thead>
            <tbody>
              {rentals.map((rental) => (
                <Tr key={rental.id}>
                  <Td>
                    <span className="font-medium">
                      {rental.asset_tag ?? `Unit ${idPrefix(rental.device_asset_id)}`}
                      {rental.asset_name ? ` · ${rental.asset_name}` : ""}
                    </span>
                    {rental.agreement_ref ? (
                      <span className="ml-1.5 text-xs" style={{ color: "var(--m-text-muted)" }}>
                        {rental.agreement_ref}
                      </span>
                    ) : null}
                  </Td>
                  <Td className="text-sm">{rental.vendor_name ?? "—"}</Td>
                  <Td className="text-sm capitalize">{rental.cycle}</Td>
                  <Td className="text-right tabular-nums text-sm">
                    {money(rental.cost_minor, rental.currency)}
                  </Td>
                  <Td className="text-right tabular-nums text-sm">
                    {rental.deposit_minor == null
                      ? "—"
                      : money(rental.deposit_minor, rental.deposit_currency ?? rental.currency)}
                  </Td>
                  <Td className="text-sm">{formatDayKey(rental.start_on, "day")}</Td>
                  <Td className="text-sm">
                    {rental.end_on == null ? "Open" : formatDayKey(rental.end_on, "day")}
                  </Td>
                  <Td>
                    {rental.lost ? (
                      <Badge tone="danger">lost</Badge>
                    ) : rental.status === "returned" ? (
                      <Badge tone="neutral">returned</Badge>
                    ) : (
                      <Badge tone="success">open</Badge>
                    )}
                  </Td>
                  <Td className="text-right">
                    {rental.status === "open" ? (
                      <Button variant="secondary" onClick={() => openReturn(rental)}>
                        Return
                      </Button>
                    ) : null}
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>

      <Dialog
        open={createOpen}
        onOpenChange={(open) => {
          if (open) createSession.current += 1;
          setCreateOpen(open);
          if (!open) setCreateError(null);
        }}
      >
        <DialogContent
          title="Record a vendor rental"
          description="A unit rented in from a supplier. The cost is an expense; it never enters inventory value."
        >
          <div className="grid gap-3 p-1">
            <div>
              <Label htmlFor="vr-vendor">Vendor (optional)</Label>
              <Input
                id="vr-vendor"
                value={vendorName}
                onChange={(e) => setVendorName(e.target.value)}
                placeholder="Supplier name"
                data-testid="vendor-rentals-vendor"
              />
            </div>
            <div className="flex gap-2">
              <div className="w-40">
                <Label htmlFor="vr-tag">Asset tag</Label>
                <Input
                  id="vr-tag"
                  value={assetTag}
                  onChange={(e) => setAssetTag(e.target.value)}
                  placeholder="VR-001"
                  data-testid="vendor-rentals-tag"
                />
              </div>
              <div className="flex-1">
                <Label htmlFor="vr-name">Asset name</Label>
                <Input
                  id="vr-name"
                  value={assetName}
                  onChange={(e) => setAssetName(e.target.value)}
                  placeholder="Loaned switch"
                  data-testid="vendor-rentals-name"
                />
              </div>
            </div>
            <div className="flex gap-2">
              <div className="flex-1">
                <Label htmlFor="vr-cycle">Cycle</Label>
                <Select value={cycle} onValueChange={setCycle}>
                  <SelectTrigger id="vr-cycle" className="w-full" data-testid="vendor-rentals-cycle">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {CYCLES.map((c) => (
                      <SelectItem key={c.value} value={c.value}>
                        {c.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="w-32">
                <Label htmlFor="vr-cost">Cost</Label>
                <Input
                  id="vr-cost"
                  type="number"
                  min="0"
                  step="0.01"
                  value={cost}
                  onChange={(e) => setCost(e.target.value)}
                  data-testid="vendor-rentals-cost"
                />
              </div>
              <div className="w-20">
                <Label htmlFor="vr-currency">Currency</Label>
                <Input
                  id="vr-currency"
                  maxLength={3}
                  value={currency}
                  onChange={(e) => setCurrency(e.target.value.toUpperCase())}
                  data-testid="vendor-rentals-currency"
                />
              </div>
            </div>
            <div className="flex gap-2">
              <div className="w-32">
                <Label htmlFor="vr-deposit">Deposit</Label>
                <Input
                  id="vr-deposit"
                  type="number"
                  min="0"
                  step="0.01"
                  value={deposit}
                  onChange={(e) => setDeposit(e.target.value)}
                  data-testid="vendor-rentals-deposit"
                />
              </div>
              <div className="flex-1">
                <Label htmlFor="vr-start">Started</Label>
                <Input
                  id="vr-start"
                  type="date"
                  value={startOn}
                  onChange={(e) => setStartOn(e.target.value)}
                  data-testid="vendor-rentals-start"
                />
              </div>
              <div className="flex-1">
                <Label htmlFor="vr-end">Ends (optional)</Label>
                <Input
                  id="vr-end"
                  type="date"
                  value={endOn}
                  onChange={(e) => setEndOn(e.target.value)}
                  data-testid="vendor-rentals-end"
                />
              </div>
            </div>
            <div>
              <Label htmlFor="vr-notes">Notes</Label>
              <Input
                id="vr-notes"
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
                data-testid="vendor-rentals-notes"
              />
            </div>
            <FieldHint>
              The unit is created as a rental-in asset, so it is custody rather than owned stock.
            </FieldHint>
            {createError !== null ? <FieldError>{createError}</FieldError> : null}
            <DialogActions>
              <Button variant="secondary" onClick={() => setCreateOpen(false)}>
                Cancel
              </Button>
              <Button disabled={createBusy} onClick={() => void submitCreate()} data-testid="vendor-rentals-save">
                {createBusy ? "Recording…" : "Record rental"}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog
        open={returning !== null}
        onOpenChange={(open) => {
          if (!open) setReturning(null);
        }}
      >
        <DialogContent title="Return a vendor rental" data-testid="vendor-rentals-return">
          <div className="grid gap-3 p-1">
            <p className="text-sm" style={{ color: "var(--m-text-secondary)" }}>
              {returning?.vendor_name ?? "Vendor"} ·{" "}
              {returning === null
                ? ""
                : (returning.asset_tag ?? `unit ${idPrefix(returning.device_asset_id)}`)}
            </p>
            <div>
              <Label>Was it lost?</Label>
              <Select value={lost ? "lost" : "returned"} onValueChange={(v) => setLost(v === "lost")}>
                <SelectTrigger className="w-full" aria-label="Lost or returned" data-testid="vendor-rentals-lost">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="returned">Returned</SelectItem>
                  <SelectItem value="lost">Lost</SelectItem>
                </SelectContent>
              </Select>
            </div>
            {!lost ? (
              <div>
                <Label>Condition</Label>
                <Select value={returnCondition} onValueChange={setReturnCondition}>
                  <SelectTrigger className="w-full" aria-label="Return condition" data-testid="vendor-rentals-condition">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {CONDITIONS.map((c) => (
                      <SelectItem key={c.value} value={c.value}>
                        {c.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            ) : null}
            <div className="flex gap-2">
              <div className="flex-1">
                <Label htmlFor="vr-damage">
                  Damage{returning?.currency ? ` (${returning.currency})` : ""}
                </Label>
                <Input
                  id="vr-damage"
                  type="number"
                  min="0"
                  step="0.01"
                  value={damage}
                  onChange={(e) => setDamage(e.target.value)}
                  data-testid="vendor-rentals-damage"
                />
              </div>
              <div className="flex-1">
                <Label htmlFor="vr-loss">
                  Loss{returning?.currency ? ` (${returning.currency})` : ""}
                </Label>
                <Input
                  id="vr-loss"
                  type="number"
                  min="0"
                  step="0.01"
                  value={loss}
                  onChange={(e) => setLoss(e.target.value)}
                  data-testid="vendor-rentals-loss"
                />
              </div>
            </div>
            <div>
              <Label htmlFor="vr-return-notes">Notes</Label>
              <Input
                id="vr-return-notes"
                value={returnNotes}
                onChange={(e) => setReturnNotes(e.target.value)}
                data-testid="vendor-rentals-return-notes"
              />
            </div>
            {returnError !== null ? <FieldError>{returnError}</FieldError> : null}
            <DialogActions>
              <Button variant="secondary" onClick={() => setReturning(null)}>
                Cancel
              </Button>
              <Button disabled={returnBusy} onClick={() => void submitReturn()} data-testid="vendor-rentals-return-save">
                {returnBusy ? "Returning…" : "Return"}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
