// ReportDeliveryHistory — per-schedule delivery ledger (Wave 11 lane 10
// close-out of the p11-backend-gaps R4 read route).
//
// Purpose: the delivery-history table for ONE report schedule, consuming
// GET /report-schedules/{id}/deliveries (offset-paged, newest slot first).
// Rows show the claimed slot, the outcome status (delivered | partial |
// failed | skipped), and the per-recipient outcome detail — worker-written
// append-only data, rendered read-only with honest per-recipient counters.
// Non-purpose: no schedule editing here (the schedules table owns CRUD).
"use client";

import type { components } from "@marula-core/sdk";
import {
  Badge,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  Spinner,
  Table,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/api";

type Delivery = components["schemas"]["ReportDeliveryView"];

const PAGE = 50;

function statusTone(status: string): "success" | "warning" | "danger" | "neutral" {
  if (status === "delivered") return "success";
  if (status === "partial") return "warning";
  if (status === "failed") return "danger";
  return "neutral";
}

/** The detail payload is a JSON counter bag; render defensively. */
function detailSummary(detail: unknown): string {
  if (detail === null || typeof detail !== "object") return "—";
  const d = detail as { delivered?: number; disabled?: number; quiet_deferred?: number; failed?: unknown };
  const parts: string[] = [];
  if (typeof d.delivered === "number" && d.delivered > 0) parts.push(`${d.delivered} delivered`);
  if (typeof d.disabled === "number" && d.disabled > 0) parts.push(`${d.disabled} disabled`);
  if (typeof d.quiet_deferred === "number" && d.quiet_deferred > 0) parts.push(`${d.quiet_deferred} quiet-deferred`);
  if (Array.isArray(d.failed) && d.failed.length > 0) parts.push(`${d.failed.length} failed`);
  return parts.length > 0 ? parts.join(" · ") : "no recipients";
}

export function ReportDeliveryHistory({ scheduleId, scheduleName }: { scheduleId: string; scheduleName: string }) {
  const [open, setOpen] = useState(false);
  const [deliveries, setDeliveries] = useState<Delivery[] | null>(null);
  const [total, setTotal] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const fetchPage = useCallback(
    async (offset: number, replace: boolean) => {
      setError(null);
      const { data, error: apiError } = await api.api.GET("/api/v1/report-schedules/{id}/deliveries", {
        params: { path: { id: scheduleId }, query: { limit: PAGE, offset } },
      });
      if (apiError !== undefined || data === undefined) {
        setError("Failed to load the delivery history.");
        return;
      }
      setTotal(data.total);
      setDeliveries((prev) => (replace ? data.deliveries : [...(prev ?? []), ...data.deliveries]));
    },
    [scheduleId],
  );

  useEffect(() => {
    if (open) void fetchPage(0, true);
  }, [open, fetchPage]);

  return (
    <>
      <Button variant="ghost" size="sm" onClick={() => setOpen(true)} data-testid={`delivery-history-${scheduleId}`}>
        History
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent
          title={`Delivery history — ${scheduleName}`}
          description="Append-only ledger of every claimed slot and its delivery outcome, newest first."
        >
          <div className="grid max-h-[60vh] gap-3 overflow-auto p-4">
            {error !== null ? (
              <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {error}
              </p>
            ) : deliveries === null ? (
              <div className="flex justify-center p-4">
                <Spinner />
              </div>
            ) : deliveries.length === 0 ? (
              <EmptyState message="No deliveries yet. The worker records a ledger row for every claimed slot." />
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Slot</Th>
                    <Th>Status</Th>
                    <Th>Recipients</Th>
                    <Th>Recorded</Th>
                  </tr>
                </thead>
                <tbody>
                  {deliveries.map((d) => (
                    <Tr key={d.id}>
                      <Td className="text-[12px] tabular-nums">{new Date(d.slot_at).toLocaleString()}</Td>
                      <Td>
                        <Badge tone={statusTone(d.status)}>{d.status}</Badge>
                      </Td>
                      <Td className="text-[12px]">{detailSummary(d.detail)}</Td>
                      <Td className="text-[12px]">{new Date(d.recorded_at).toLocaleString()}</Td>
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
            {total !== null && deliveries !== null && deliveries.length < total ? (
              <div className="flex justify-center">
                <Button variant="secondary" size="sm" onClick={() => void fetchPage(deliveries.length, false)}>
                  Load older ({total - deliveries.length} remaining)
                </Button>
              </div>
            ) : null}
          </div>
          <DialogActions>
            <Button variant="ghost" onClick={() => setOpen(false)}>
              Close
            </Button>
          </DialogActions>
        </DialogContent>
      </Dialog>
    </>
  );
}
