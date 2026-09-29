// /portal/quotes — the client's quotes (Portal v2, Wave 34). Non-draft
// documents only, newest first. Sent quotes carry their validity window
// honestly; decided quotes show their outcome. Approval happens on the
// quote's own page.
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import { Card, CardContent, Spinner, StatusTag } from "@marula-core/ui";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";
import { portalApi, useRequirePortalAuth, type PortalQuote } from "../../../lib/portal-api";

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

const STATUS_LABEL: Record<PortalQuote["status"], string> = {
  sent: "Awaiting your decision",
  accepted: "Approved",
  declined: "Declined",
  expired: "Expired",
};

const STATUS_TONE: Record<PortalQuote["status"], "neutral" | "success" | "danger" | "warning"> = {
  sent: "warning",
  accepted: "success",
  declined: "danger",
  expired: "neutral",
};

export default function PortalQuotesPage() {
  useRequirePortalAuth();
  const router = useRouter();
  const [rows, setRows] = useState<PortalQuote[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    portalApi
      .listQuotes()
      .then((quotes) => {
        if (!cancelled) setRows(quotes);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(friendlyMutationText(err, "Failed to load quotes"));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const awaiting = useMemo(
    () => (rows ?? []).filter((r) => r.status === "sent" && daysUntil(r.valid_until) >= 0).length,
    [rows],
  );

  if (error !== null) {
    return <p className="text-sm" style={{ color: "var(--m-status-danger)" }}>{error}</p>;
  }
  if (rows === null) {
    return (
      <div className="flex justify-center py-16">
        <Spinner aria-label="Loading quotes" />
      </div>
    );
  }

  return (
    <section className="m-rise">
      <div className="mb-5">
        <h1 className="text-xl font-semibold tracking-tight">Quotes</h1>
        <p className="mt-0.5 text-sm" style={{ color: "var(--m-text-muted)" }}>
          Proposals from your service provider. Approve a quote and your
          provider turns it into the agreed work; every quote shows the
          last day it can be accepted.
        </p>
      </div>

      {awaiting > 0 && (
        <div className="mb-4" data-testid="quotes-awaiting-strip">
          <span
            className="rounded-[0.5rem] px-2.5 py-1 text-xs"
            style={{ background: "var(--m-status-warning-bg)", color: "var(--m-status-warning-fg)" }}
          >
            <span className="font-semibold tabular-nums">{awaiting}</span>{" "}
            {awaiting === 1 ? "quote awaits" : "quotes await"} your decision
          </span>
        </div>
      )}

      {rows.length === 0 ? (
        <Card>
          <CardContent>
            <p className="text-sm" style={{ color: "var(--m-text-muted)" }}>
              No quotes yet. When your provider sends one, it appears here
              for your review.
            </p>
          </CardContent>
        </Card>
      ) : (
        <ul className="space-y-2" data-testid="quotes-list">
          {rows.map((q, i) => {
            const lapsed = q.status === "sent" && daysUntil(q.valid_until) < 0;
            return (
              <li
                key={q.id}
                className="m-rise"
                style={{ animationDelay: `${Math.min(i * 30, 180)}ms` }}
              >
                <button
                  type="button"
                  onClick={() => router.push(`/portal/quotes/${q.id}`)}
                  className="block w-full rounded-[0.75rem] text-left transition-colors duration-100"
                  style={{ background: "var(--m-surface)", border: "1px solid var(--m-border)" }}
                  data-testid={`quote-row-${q.number}`}
                >
                  <CardContent>
                    <div className="flex flex-wrap items-center justify-between gap-3 py-1">
                      <div className="min-w-0">
                        <p className="text-sm font-semibold tabular-nums">{q.number}</p>
                        <p className="mt-0.5 text-xs tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                          Issued {dateOnly(q.issued_on)} · Valid until {dateOnly(q.valid_until)}
                        </p>
                      </div>
                      <div className="flex items-center gap-3">
                        <span className="font-mono text-sm font-semibold tabular-nums">
                          {money(q.total_minor, q.currency)}
                        </span>
                        {lapsed ? (
                          <StatusTag tone="neutral">Valid until passed</StatusTag>
                        ) : (
                          <StatusTag tone={STATUS_TONE[q.status]}>{STATUS_LABEL[q.status]}</StatusTag>
                        )}
                      </div>
                    </div>
                  </CardContent>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
