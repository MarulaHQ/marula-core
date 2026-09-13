// /portal/invoices — the client's invoice list (Portal v2).
//
// Purpose: every issued or paid invoice of the signed-in client's
// organization, newest first. Void and draft documents are staff-internal
// and never appear. Rows show number, status, due date, and outstanding
// amount; the detail view owns lines, receipts, and payment.
// Non-purpose: no dunning internals (staff-only), no dispute flows.
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import { Button, EmptyState, Spinner, StatusTag, type StatusTone } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { portalApi, usePortalAuth, useRequirePortalAuth, type PortalInvoice } from "../../../lib/portal-api";

function statusTone(status: PortalInvoice["status"]): StatusTone {
  return status === "paid" ? "success" : "neutral";
}

function money(minor: number, currency: string): string {
  return new Intl.NumberFormat(undefined, { style: "currency", currency }).format(minor / 100);
}

function dateOnly(iso: string): string {
  return new Date(`${iso}T00:00:00`).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export default function PortalInvoicesPage() {
  useRequirePortalAuth();
  const router = useRouter();
  const [rows, setRows] = useState<PortalInvoice[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    portalApi
      .listInvoices()
      .then((invoices) => {
        if (!cancelled) setRows(invoices);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(friendlyMutationText(err, "Failed to load invoices"));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (error !== null) {
    return <p className="text-sm" style={{ color: "var(--m-status-danger)" }}>{error}</p>;
  }
  if (rows === null) {
    return (
      <div className="flex justify-center py-16">
        <Spinner aria-label="Loading invoices" />
      </div>
    );
  }

  return (
    <section>
      <div className="mb-4">
        <h1 className="text-base font-semibold">Invoices</h1>
        <p className="mt-0.5 text-xs" style={{ color: "var(--m-text-muted)" }}>
          Statements from your service provider. Payments are recorded on
          your account — no card is charged through this portal.
        </p>
      </div>

      {rows.length === 0 ? (
        <EmptyState message="No invoices yet — statements appear here when your provider bills your account." />
      ) : (
        <ul className="divide-y rounded-md border" style={{ borderColor: "var(--m-border)" }}>
          {rows.map((inv) => (
            <li key={inv.id}>
              <button
                type="button"
                onClick={() => router.push(`/portal/invoices/${inv.id}`)}
                className="flex w-full items-center justify-between gap-3 px-3 py-2.5 text-left transition-colors duration-100 hover:bg-[var(--m-surface-hover)]"
              >
                <div className="min-w-0">
                  <div className="text-sm font-medium">{inv.number}</div>
                  <div className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                    Issued {dateOnly(inv.issued_at)} · Due {dateOnly(inv.due_on)}
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-3">
                  <span className="text-sm tabular-nums">
                    {inv.status === "paid"
                      ? money(inv.total_minor, inv.currency)
                      : `${money(inv.outstanding_minor, inv.currency)} due`}
                  </span>
                  <StatusTag tone={statusTone(inv.status)}>
                    {inv.status === "paid" ? "Paid" : "Open"}
                  </StatusTag>
                </div>
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="mt-4">
        <Button variant="ghost" size="sm" onClick={() => router.push("/portal/tickets")}>
          Back to requests
        </Button>
      </div>
    </section>
  );
}
