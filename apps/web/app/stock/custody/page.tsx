// /stock/custody — where every physical unit is.
//
// Purpose: the class each unit sits in (sale stock, the fleet, the rental pool,
// client custody, e-waste, retired), the counts per class, and the append-only
// ledger that moved units between them. One place answers "where is it" for
// both identities the product keeps: a stock serial and a device asset.
//
// Non-purpose: no moves (the movement, loan, receipt and e-waste flows own the
// writes), no valuation (the item and asset pages carry money).

"use client";

import {
  Badge,
  Button,
  Card,
  EmptyState,
  FieldError,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  SurfaceHeader,
  Table,
  Td,
  Th,
  Tr,
  formatDayKey,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useRef, useState } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { showsEmptyRegister } from "../../../lib/list-view";
import { LocalTime } from "../../../components/local-time";

type CustodyClass = components["schemas"]["CustodyClassView"];
type ClassMovement = components["schemas"]["ClassMovementView"];

const CLASS_LABELS: Record<string, string> = {
  sale_stock: "Sale stock",
  internal_fleet: "Internal fleet",
  rental_in: "Rented in",
  rental_out: "Rented out",
  client_custody: "Client custody",
  ewaste_custody: "E-waste",
  tools: "Tools",
  sold: "Sold",
  recycled: "Recycled",
  destroyed: "Destroyed",
  returned: "Returned",
};

function classLabel(value: string): string {
  return CLASS_LABELS[value] ?? value.replaceAll("_", " ");
}

function grams(value: number): string {
  return value.toLocaleString();
}

export default function CustodyPage(): React.ReactNode {
  const auth = useRequireAuth();
  const [classes, setClasses] = useState<CustodyClass[]>([]);
  const [movements, setMovements] = useState<ClassMovement[]>([]);
  const [unitKind, setUnitKind] = useState("all");
  // Which filter the rows on screen actually belong to; a failed refresh keeps
  // the old rows, so the label must not claim the new filter.
  const [movementFilter, setMovementFilter] = useState("all");
  // True only once the current filter's read has failed; while it is running,
  // the retained rows are simply old, not a failure.
  const [ledgerStale, setLedgerStale] = useState(false);
  // `loading` owns the first paint; `reading` is any read in flight. A manual
  // retry sets `reading` only, so a stalled request cannot disable the button
  // that would start a fresh one, and the sequence guard discards the old
  // answer.
  const [loading, setLoading] = useState(true);
  const [reading, setReading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const requestSeq = useRef(0);

  const refresh = useCallback(async (): Promise<void> => {
    const seq = ++requestSeq.current;
    setReading(true);
    setLoadError(null);
    setLedgerStale(false);
    try {
      const [classRes, ledgerRes] = await Promise.all([
        api.api.GET("/api/v1/inventory/custody"),
        api.api.GET("/api/v1/inventory/class-movements", {
          params: { query: { unit_kind: unitKind === "all" ? null : unitKind, limit: 100 } },
        }),
      ]);
      if (seq !== requestSeq.current) return;
      if (classRes.data === undefined) {
        setLoadError("Could not load the custody classes — try reloading.");
        return;
      }
      setClasses(classRes.data);
      if (ledgerRes.data === undefined) {
        // Keep the ledger that is on screen: a failed refresh is not an empty
        // ledger, and saying "no movements yet" would be a false answer.
        setLedgerStale(true);
        setLoadError("The class ledger could not be loaded — the rows below may be stale.");
        return;
      }
      setMovements(ledgerRes.data);
      setMovementFilter(unitKind);
    } catch {
      if (seq === requestSeq.current) {
        setLedgerStale(true);
        setLoadError("Could not load the custody view — try reloading.");
      }
    } finally {
      if (seq === requestSeq.current) {
        setReading(false);
        setLoading(false);
      }
    }
  }, [unitKind]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    setLoading(true);
    void refresh();
  }, [auth.status, refresh]);

  const totalSerials = classes.reduce((sum, row) => sum + row.serials, 0);
  const totalAssets = classes.reduce((sum, row) => sum + row.assets, 0);

  if (auth.status === "loading") return <Spinner />;

  return (
    <div className="grid gap-4">
      <SurfaceHeader
        title="Custody"
        description={`One class per physical unit: ${grams(totalSerials)} stock serials and ${grams(totalAssets)} device assets across the classes below.`}
        actions={
          <Button variant="secondary" onClick={() => void refresh()} disabled={loading}>
            Refresh
          </Button>
        }
      />

      {loadError !== null ? <FieldError>{loadError}</FieldError> : null}

      <Card>
        {loading ? (
          <div className="p-6">
            <Spinner />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Class</Th>
                <Th className="text-right">Stock serials</Th>
                <Th className="text-right">Device assets</Th>
                <Th className="text-right">Total</Th>
              </tr>
            </thead>
            <tbody>
              {classes.map((row) => (
                <Tr key={row.custody_class}>
                  <Td>
                    <Badge tone={row.total === 0 ? "neutral" : "success"}>
                      {classLabel(row.custody_class)}
                    </Badge>
                  </Td>
                  <Td className="text-right tabular-nums text-sm">{grams(row.serials)}</Td>
                  <Td className="text-right tabular-nums text-sm">{grams(row.assets)}</Td>
                  <Td className="text-right tabular-nums text-sm">{grams(row.total)}</Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>

      <div className="flex items-end justify-between gap-3">
        <div>
          <h2 className="text-base font-semibold" style={{ color: "var(--m-text)" }}>
            Class ledger
          </h2>
          <p className="text-sm" style={{ color: "var(--m-text-secondary)" }}>
            Newest first. The ledger is append-only: each move between classes is written once
            and never edited.
          </p>
        </div>
        <Select value={unitKind} onValueChange={setUnitKind}>
          <SelectTrigger className="w-40" aria-label="Unit kind" data-testid="custody-unit-kind">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">Both kinds</SelectItem>
            <SelectItem value="serial">Stock serials</SelectItem>
            <SelectItem value="asset">Device assets</SelectItem>
          </SelectContent>
        </Select>
      </div>

      {movements.length >= 100 ? (
        <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          Showing the newest 100 moves. Older moves are in the ledger and not on this page.
        </p>
      ) : null}
      {ledgerStale && movementFilter !== unitKind && movements.length > 0 ? (
        <p className="text-xs" style={{ color: "var(--m-status-warning-fg)" }}>
          Showing {movementFilter === "all" ? "both kinds" : movementFilter === "serial" ? "stock serials" : "device assets"} from the last successful read; the current filter did not load.
        </p>
      ) : null}

      <Card>
        {loading || (reading && movements.length === 0) ? (
          <div className="p-6">
            <Spinner />
          </div>
        ) : movements.length === 0 ? (
          // A failed read is not an empty ledger: the error above says what
          // happened, and an "empty" claim under it would contradict it.
          showsEmptyRegister(movements.length, loadError) ? (
            <EmptyState message="No class movements yet. A receipt, a loan, a rental or an e-waste disposition writes the first one." />
          ) : null
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>When</Th>
                <Th>Unit</Th>
                <Th>From</Th>
                <Th>To</Th>
                <Th>Why</Th>
              </tr>
            </thead>
            <tbody>
              {movements.map((movement) => (
                <Tr key={movement.id}>
                  <Td className="text-sm">
                    <LocalTime value={movement.created_at} kind="datetime" />
                  </Td>
                  <Td className="text-sm">{movement.unit_label}</Td>
                  <Td className="text-sm">{classLabel(movement.from_class)}</Td>
                  <Td className="text-sm">
                    <Badge tone="neutral">{classLabel(movement.to_class)}</Badge>
                  </Td>
                  <Td className="text-sm">{movement.reason}</Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>
    </div>
  );
}
