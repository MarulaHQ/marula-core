// /tickets — the standalone ticket list (Wave 15 lane 6).
//
// Purpose: the icon rail's Tickets target — a real, filterable list of
// tickets backed by the typed SDK (GET /api/v1/tickets, category tabs),
// resolving the QA finding that /tickets 404'd as a standalone route.
// Rows deep-link into the existing ticket workbench at /tickets/{id}; no
// route moves anywhere.
//
// Non-purpose: no board columns (boards keep that), no editing here — the
// workbench owns every write path.
"use client";

import { Button, Card, EmptyState, Spinner, StatusTag } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";

type TicketRow = components["schemas"]["TicketWithFields"];
type CategoryFilter = "open" | "in_progress" | "waiting" | "closed" | "all";

const TABS: { id: CategoryFilter; label: string }[] = [
  { id: "open", label: "Open" },
  { id: "in_progress", label: "In progress" },
  { id: "waiting", label: "Waiting" },
  { id: "closed", label: "Closed" },
  { id: "all", label: "All" },
];

const CATEGORY_LABEL: Record<string, string> = {
  open: "Open",
  in_progress: "In progress",
  waiting: "Waiting",
  closed: "Closed",
};

function priorityTone(priority: string): "danger" | "warning" | "neutral" {
  if (priority === "p1") return "danger";
  if (priority === "p2") return "warning";
  return "neutral";
}

function relativeTime(iso: string): string {
  const diffMs = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diffMs / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

export default function TicketsListPage() {
  const auth = useRequireAuth();
  const [tab, setTab] = useState<CategoryFilter>("open");
  const [tickets, setTickets] = useState<TicketRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async (filter: CategoryFilter) => {
    setTickets(null);
    setError(null);
    const query =
      filter === "all"
        ? { limit: 200 }
        : { category: filter, limit: 200 };
    const result = await api.api.GET("/api/v1/tickets", { params: { query } });
    if (result.error !== undefined || result.data === undefined) {
      setError("Failed to load tickets. Check your connection and retry.");
      setTickets([]);
      return;
    }
    setTickets([...result.data].sort((a, b) => b.updated_at.localeCompare(a.updated_at)));
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh(tab);
  }, [auth.status, tab, refresh]);

  if (auth.status !== "authed") return null;

  return (
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 p-4 md:p-6">
      <div className="min-w-0">
        <h1 className="text-lg font-semibold">Tickets</h1>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Every ticket you can reach, newest activity first. Open one to work it in the
          workbench.
        </p>
      </div>

      <div className="flex flex-wrap items-center gap-1" role="tablist" aria-label="Ticket filter">
        {TABS.map((t) => {
          const active = tab === t.id;
          return (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={active}
              onClick={() => setTab(t.id)}
              className="cursor-pointer rounded-lg border px-3 py-1 text-[13px]"
              style={{
                borderColor: active ? "var(--m-accent)" : "var(--m-border)",
                background: active ? "var(--m-accent-soft)" : "var(--m-surface)",
                color: active ? "var(--m-accent)" : "var(--m-text-secondary)",
                fontWeight: active ? 600 : 400,
              }}
            >
              {t.label}
            </button>
          );
        })}
      </div>

      {tickets === null ? (
        <Card>
          <div className="p-4">
            <Spinner />
          </div>
        </Card>
      ) : error !== null ? (
        <Card>
          <div className="flex items-center justify-between gap-3 p-4">
            <p role="alert" className="text-[13px]">
              {error}
            </p>
            <Button variant="secondary" size="sm" onClick={() => void refresh(tab)}>
              Retry
            </Button>
          </div>
        </Card>
      ) : tickets.length === 0 ? (
        <Card>
          <div className="p-4">
            <EmptyState message={`No ${tab === "all" ? "" : CATEGORY_LABEL[tab]?.toLowerCase() + " "}tickets in this view.`} />
          </div>
        </Card>
      ) : (
        <Card>
          <ul className="grid">
            {tickets.map((ticket) => (
              <li
                key={ticket.id}
                className="row-hover border-t first:border-t-0"
                style={{ borderColor: "var(--m-border)" }}
              >
                <Link
                  href={`/tickets/${ticket.id}`}
                  className="flex min-w-0 items-center gap-2 px-3 py-2 no-underline sm:gap-3 sm:px-4"
                  style={{ minHeight: "var(--m-density-row)" }}
                >
                  <span
                    className="w-20 shrink-0 font-mono text-[12.5px] font-semibold sm:w-24 sm:text-[13px]"
                    style={{ color: "var(--m-text)" }}
                  >
                    {ticket.number}
                  </span>
                  <span
                    className="min-w-0 flex-1 truncate text-[13px]"
                    style={{ color: "var(--m-text)" }}
                  >
                    {ticket.title}
                  </span>
                  <StatusTag tone={priorityTone(ticket.priority)}>
                    {ticket.priority.toUpperCase()}
                  </StatusTag>
                  <span
                    className="hidden shrink-0 text-xs md:inline"
                    style={{ color: "var(--m-text-muted)" }}
                  >
                    {CATEGORY_LABEL[ticket.category] ?? ticket.category}
                  </span>
                  <span
                    className="hidden w-20 shrink-0 text-right text-xs sm:block"
                    style={{ color: "var(--m-text-muted)" }}
                  >
                    {relativeTime(ticket.updated_at)}
                  </span>
                </Link>
              </li>
            ))}
          </ul>
        </Card>
      )}
    </div>
  );
}
