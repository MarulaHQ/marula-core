// /portal/tickets — the client's ticket list (compact rows).
//
// Purpose: every ticket of the signed-in client's organization across
// boards the MSP exposed, newest activity first. Rows are deliberately
// compact — number, title, status tone, updated time. Server-side scoping
// (client + exposure) means nothing hidden can even be listed.
// Non-purpose: no staff triage columns (assignee/SLA/board internals), no
// filtering beyond what portal v1 exposes.
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import { EmptyState, Spinner, StatusTag, type StatusTone } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { portalApi, usePortalAuth, useRequirePortalAuth } from "../../../lib/portal-api";

interface Row {
  id: string;
  number: string;
  title: string;
  category: "open" | "in_progress" | "waiting" | "closed";
  updated_at: string;
}

/** Industry-standard PSA status semantics via Workbench tokens only:
 * amber for waiting, green for closed; open/in-progress stay neutral. */
function categoryTone(category: Row["category"]): StatusTone {
  switch (category) {
    case "waiting":
      return "warning";
    case "closed":
      return "success";
    default:
      return "neutral";
  }
}

const CATEGORY_LABEL: Record<Row["category"], string> = {
  open: "Open",
  in_progress: "In progress",
  waiting: "Waiting",
  closed: "Closed",
};

function relative(iso: string): string {
  const deltaMs = Date.now() - new Date(iso).getTime();
  const minutes = Math.round(deltaMs / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  return `${days}d ago`;
}

export default function PortalTicketsPage() {
  useRequirePortalAuth();
  const { me } = usePortalAuth();
  const router = useRouter();
  const [rows, setRows] = useState<Row[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    portalApi
      .listTickets()
      .then((tickets) => {
        if (!cancelled)
          setRows(
            tickets.map((t) => ({
              id: t.id,
              number: t.number,
              title: t.title,
              category: t.category,
              updated_at: t.updated_at,
            })),
          );
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(friendlyMutationText(err, "Failed to load tickets"));
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
        <Spinner aria-label="Loading tickets" />
      </div>
    );
  }

  return (
    <section>
      <div className="mb-4">
        <h1 className="text-base font-semibold">Your requests</h1>
        <p className="mt-0.5 text-xs" style={{ color: "var(--m-text-muted)" }}>
          {me !== null && me.role === "it_manager"
            ? "IT-manager view: includes your vendor's in-house updates on each request."
            : "Requests your organization has open with the service desk."}
        </p>
      </div>

      {rows.length === 0 ? (
        <EmptyState
          message="No requests yet — when your service desk publishes a board to you, its requests appear here."
        />
      ) : (
        <ul className="divide-y rounded-md border" style={{ borderColor: "var(--m-border)" }}>
          {rows.map((t) => (
            <li key={t.id}>
              <button
                type="button"
                onClick={() => router.push(`/portal/tickets/${t.id}`)}
                className="flex w-full items-center gap-3 px-3 py-2.5 text-left hover:bg-[var(--m-surface-hover)]"
              >
                <span
                  className="shrink-0 font-mono text-xs"
                  style={{ color: "var(--m-text-muted)" }}
                >
                  {t.number}
                </span>
                <span className="min-w-0 flex-1 truncate text-sm">{t.title}</span>
                <StatusTag tone={categoryTone(t.category)}>{CATEGORY_LABEL[t.category]}</StatusTag>
                <span className="w-16 shrink-0 text-right text-xs" style={{ color: "var(--m-text-muted)" }}>
                  {relative(t.updated_at)}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
