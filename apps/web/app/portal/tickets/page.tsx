// /portal/tickets — the client's ticket list (compact rows).
//
// Purpose: every ticket of the signed-in client's organization across
// boards the MSP exposed, newest activity first. Rows are deliberately
// compact — number, title, priority, status tone, updated time. A quiet
// client-side search and status filter sit on top (no server round-trip —
// the list is a snapshot read; filtering never re-asks for what the client
// already holds). Server-side scoping (client + exposure) means nothing
// hidden can even be listed.
// Non-purpose: no staff triage columns (assignee/SLA/board internals), no
// filtering beyond the portal surface exposes — no "new request" action
// exists in this portal surface, so none is shown (never a dead button).
//
// Wave 17 lane 8 UX pass: honest stat strip (open/waiting/closed live from
// the rows), staggered entrance (m-rise ≤120ms), row hover lift.
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import { EmptyState, Input, Spinner, StatusTag, Tag, type StatusTone } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";
import { portalApi, usePortalAuth, useRequirePortalAuth } from "../../../lib/portal-api";

interface Row {
  id: string;
  number: string;
  title: string;
  priority: "p1" | "p2" | "p3" | "p4";
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

const PRIORITY_LABEL: Record<Row["priority"], string> = {
  p1: "P1",
  p2: "P2",
  p3: "P3",
  p4: "P4",
};

const FILTERS = [
  { key: "all", label: "All" },
  { key: "open", label: "Open" },
  { key: "waiting", label: "Waiting" },
  { key: "closed", label: "Closed" },
] as const;

type FilterKey = (typeof FILTERS)[number]["key"];

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

function matchesFilter(row: Row, filter: FilterKey): boolean {
  switch (filter) {
    case "all":
      return true;
    case "closed":
      return row.category === "closed";
    case "waiting":
      return row.category === "waiting";
    case "open":
      return row.category === "open" || row.category === "in_progress";
    default:
      return true;
  }
}

export default function PortalTicketsPage() {
  useRequirePortalAuth();
  const { me } = usePortalAuth();
  const router = useRouter();
  const [rows, setRows] = useState<Row[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<FilterKey>("all");

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
              priority: t.priority,
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

  const counts = useMemo(() => {
    if (rows === null) return null;
    let open = 0;
    let waiting = 0;
    for (const r of rows) {
      if (r.category === "open" || r.category === "in_progress") open += 1;
      if (r.category === "waiting") waiting += 1;
    }
    return { open, waiting, total: rows.length };
  }, [rows]);

  const shown = useMemo(() => {
    if (rows === null) return null;
    const term = query.trim().toLowerCase();
    return rows.filter(
      (r) =>
        matchesFilter(r, filter) &&
        (term === "" || r.title.toLowerCase().includes(term) || r.number.toLowerCase().includes(term)),
    );
  }, [rows, query, filter]);

  if (error !== null) {
    return <p className="text-sm" style={{ color: "var(--m-status-danger)" }}>{error}</p>;
  }
  if (rows === null || shown === null) {
    return (
      <div className="flex justify-center py-16">
        <Spinner aria-label="Loading tickets" />
      </div>
    );
  }

  return (
    <section>
      <div className="mb-5">
        <h1 className="font-heading text-xl font-semibold tracking-tight">Your requests</h1>
        <p className="mt-0.5 text-sm" style={{ color: "var(--m-text-muted)" }}>
          {me !== null && me.role === "it_manager"
            ? "IT-manager view: includes your vendor's in-house updates on each request."
            : "Requests your organization has open with the service desk."}
        </p>
      </div>

      {counts !== null && counts.total > 0 && (
        <div className="mb-4 flex flex-wrap gap-2 text-xs" data-testid="tickets-stat-strip">
          <span
            className="rounded-[0.5rem] px-2.5 py-1"
            style={{ background: "var(--m-surface)", border: "1px solid var(--m-border)", color: "var(--m-text-secondary)" }}
          >
            <span className="font-semibold tabular-nums" style={{ color: "var(--m-text)" }}>{counts.open}</span>{" "}
            active
          </span>
          <span
            className="rounded-[0.5rem] px-2.5 py-1"
            style={{ background: "var(--m-surface)", border: "1px solid var(--m-border)", color: "var(--m-text-secondary)" }}
          >
            <span className="font-semibold tabular-nums" style={{ color: "var(--m-text)" }}>{counts.waiting}</span>{" "}
            waiting on you
          </span>
        </div>
      )}

      {rows.length > 0 && (
        <div className="mb-4 flex flex-wrap items-center gap-2">
          <Input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search your requests…"
            aria-label="Search your requests"
            className="w-full sm:max-w-xs"
          />
          <div className="flex gap-1" role="group" aria-label="Filter by status">
            {FILTERS.map((f) => (
              <button
                key={f.key}
                type="button"
                onClick={() => setFilter(f.key)}
                aria-pressed={filter === f.key}
                className="rounded-[999px] px-3 py-1 text-xs transition-colors duration-100"
                style={{
                  border: `1px solid ${filter === f.key ? "var(--m-accent)" : "var(--m-border)"}`,
                  color: filter === f.key ? "var(--m-accent)" : "var(--m-text-secondary)",
                  background: "var(--m-surface)",
                }}
              >
                {f.label}
              </button>
            ))}
          </div>
        </div>
      )}

      {rows.length === 0 ? (
        <EmptyState
          message="No requests yet — when your service desk publishes a board to you, its requests appear here."
        />
      ) : shown.length === 0 ? (
        <EmptyState message="Nothing matches that search — try different words or clear the filter." />
      ) : (
        <ul className="overflow-hidden rounded-[0.75rem] border" style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}>
          {shown.map((t, i) => (
            <li key={t.id} className="m-rise" style={{ animationDelay: `${Math.min(i * 30, 240)}ms` }}>
              <button
                type="button"
                onClick={() => router.push(`/portal/tickets/${t.id}`)}
                className="flex w-full items-center gap-3 border-b px-3 py-3 text-left transition-colors duration-100 last:border-b-0 hover:bg-[var(--m-surface-hover)]"
                style={{ borderColor: "var(--m-border)" }}
              >
                <span
                  className="shrink-0 font-mono text-xs tabular-nums"
                  style={{ color: "var(--m-text-muted)" }}
                >
                  {t.number}
                </span>
                <span className="min-w-0 flex-1 truncate text-sm">{t.title}</span>
                <Tag tone="outline">{PRIORITY_LABEL[t.priority]}</Tag>
                <StatusTag tone={categoryTone(t.category)}>{CATEGORY_LABEL[t.category]}</StatusTag>
                <span className="hidden w-16 shrink-0 text-right text-xs tabular-nums sm:block" style={{ color: "var(--m-text-muted)" }}>
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
