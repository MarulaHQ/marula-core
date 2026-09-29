// /portal/invoices — the client's invoice list (Portal v2).
//
// Purpose: every issued or paid invoice of the signed-in client's
// organization, newest first. Void and draft documents are staff-internal
// and never appear. Rows show number, status, due date, and the honest
// outstanding amount (overdue/due-soon are derived client-side from the
// real due_on + issued status — no invented dunning data). The detail view
// owns lines, receipts, and payment.
// Non-purpose: no dunning internals (staff-only), no dispute flows, no
// payment-method management (the PaymentProvider seam is none-mode: the
// detail's copy says exactly that).
//
// Wave 17 lane 8 UX pass: honest stat strip (total due / overdue off real
// rows), due-soon signal, staggered row entrance, header-nav replaces the
// stray "back to requests" affordance.
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import { EmptyState, Spinner, StatusTag, type StatusTone } from "@marula-core/ui";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";
import { portalApi, useRequirePortalAuth, type PortalInvoice } from "../../../lib/portal-api";

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

function daysUntil(iso: string): number {
  const today = new Date().setHours(0, 0, 0, 0);
  const due = new Date(`${iso}T00:00:00`).setHours(0, 0, 0, 0);
  return Math.round((due - today) / 86_400_000);
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

  const stats = useMemo(() => {
    if (rows === null) return null;
    const issued = rows.filter((r) => r.status === "issued");
    // The "still due" sum is only honest in one currency; mixed-currency
    // portfolios drop the sum and keep the plain count.
    const currencies = new Set(issued.map((r) => r.currency));
    const dueMinor =
      currencies.size === 1 ? issued.reduce((acc, r) => acc + r.outstanding_minor, 0) : null;
    let overdue = 0;
    for (const r of issued) if (daysUntil(r.due_on) < 0) overdue += 1;
    return { dueMinor, currency: [...currencies][0] ?? "USD", overdue };
  }, [rows]);

  if (error !== null) {
    return <p className="text-sm" style={{ color: "var(--m-status-danger)" }}>{error}</p>;
  }
  if (rows === null || stats === null) {
    return (
      <div className="flex justify-center py-16">
        <Spinner aria-label="Loading invoices" />
      </div>
    );
  }

  return (
    <section className="m-rise">
      <div className="mb-5">
        <h1 className="text-xl font-semibold tracking-tight">Invoices</h1>
        <p className="mt-0.5 text-sm" style={{ color: "var(--m-text-muted)" }}>
          Statements from your service provider. Payments are recorded on
          your account — no card is charged through this portal.
        </p>
      </div>

      {rows.length > 0 && (stats.dueMinor !== null || stats.overdue > 0) && (
        <div className="mb-4 flex flex-wrap gap-2 text-xs" data-testid="invoices-stat-strip">
          {stats.dueMinor !== null && (
            <span
              className="rounded-[0.5rem] px-2.5 py-1"
              style={{ background: "var(--m-surface)", border: "1px solid var(--m-border)", color: "var(--m-text-secondary)" }}
            >
              <span className="font-semibold tabular-nums" style={{ color: "var(--m-text)" }}>
                {money(stats.dueMinor, stats.currency)}
              </span>{" "}
              still due
            </span>
          )}
          {stats.overdue > 0 && (
            <span
              className="rounded-[0.5rem] px-2.5 py-1"
              style={{ background: "var(--m-status-danger-bg)", color: "var(--m-status-danger-fg)" }}
            >
              <span className="font-semibold tabular-nums">{stats.overdue}</span>{" "}
              past due
            </span>
          )}
        </div>
      )}

      {rows.length === 0 ? (
        <EmptyState message="No invoices yet — statements appear here when your provider bills your account." />
      ) : (
        <ul
          className="m-rise overflow-hidden rounded-[0.75rem] border"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
        >
          {rows.map((inv, i) => {
            const overdue = inv.status === "issued" && daysUntil(inv.due_on) < 0;
            const soon =
              inv.status === "issued" && !overdue && daysUntil(inv.due_on) <= 7;
            return (
              <li key={inv.id} className="m-rise" style={{ animationDelay: `${Math.min(i * 30, 240)}ms` }}>
                <button
                  type="button"
                  onClick={() => router.push(`/portal/invoices/${inv.id}`)}
                  className="flex w-full items-center justify-between gap-3 border-b px-3 py-3 text-left transition-colors duration-100 last:border-b-0 hover:bg-[var(--m-surface-hover)]"
                  style={{ borderColor: "var(--m-border)" }}
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="font-mono text-sm tabular-nums">{inv.number}</span>
                      {overdue && <StatusTag tone="danger">Past due</StatusTag>}
                      {soon && <StatusTag tone="warning">Due soon</StatusTag>}
                    </div>
                    <div className="text-xs tabular-nums" style={{ color: "var(--m-text-muted)" }}>
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
            );
          })}
        </ul>
      )}
    </section>
  );
}
