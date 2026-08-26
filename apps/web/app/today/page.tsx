// /today — the my-work surface.
//
// Purpose: the cross-board priority queue assigned to me (TASKLIST 2.11):
// compact 24px rows — mono ticket number, squared client tag, title,
// source board/status, right-aligned SLA countdown in tabular numerals —
// ordered server-side by priority then earliest of SLA resolution/manual
// due. Plus a docked timer working-set strip (running first). Matches
// design/mockups/today-light.jpeg and today-dark.png: dense table, quiet
// chrome, status color only where it carries meaning.
// Non-purpose: no board management or admin surfaces here.
"use client";

import type { components } from "@meridian/sdk";
import { Button, Spinner, StatusTag, Tag } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { AppHeader } from "../../components/app-header";
import { BoardNav } from "../../components/board-nav";
import { api, useRequireAuth } from "../../lib/api";

type MyWorkItem = components["schemas"]["MyWorkItem"];
type TimerSession = components["schemas"]["TimerSession"];

function PriorityTag({ priority }: { priority: string }) {
  const tone =
    priority === "p1" ? "danger" : priority === "p2" ? "warning" : "neutral";
  return <StatusTag tone={tone}>{priority.toUpperCase()}</StatusTag>;
}

function ClientTag({ name }: { name: string }) {
  // Squared client tag per mockup: initials derived from the client name,
  // outline tone so it stays quiet against status color.
  const initials = name
    .split(/\s+/)
    .slice(0, 2)
    .map((w) => w[0]?.toUpperCase() ?? "")
    .join("");
  return <Tag tone="outline" title={name}>{initials}</Tag>;
}

/// Compact relative countdown ("2d 4h", "3h", "overdue 5h") from now.
function dueLabel(iso: string): { text: string; overdue: boolean } {
  const due = new Date(iso).getTime();
  const diffMs = due - Date.now();
  const mins = Math.round(Math.abs(diffMs) / 60_000);
  const d = Math.floor(mins / 1440);
  const h = Math.floor((mins % 1440) / 60);
  const span = d > 0 ? `${d}d ${h}h` : h > 0 ? `${h}h` : `${mins}m`;
  return diffMs < 0 ? { text: `overdue ${span}`, overdue: true } : { text: span, overdue: false };
}

export default function TodayPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const [items, setItems] = useState<MyWorkItem[] | null>(null);
  const [workingSet, setWorkingSet] = useState<TimerSession[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const mine = await api.api.GET("/api/tickets/my-work");
      const ws = await api.api.GET("/api/timers/working-set");
      if (mine.error !== undefined) throw new Error("failed to load tickets");
      if (ws.error !== undefined) throw new Error("failed to load timers");
      setItems(mine.data);
      setWorkingSet(ws.data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Load failed");
    }
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  async function stopTimer(sessionId: string) {
    await api.api.POST("/api/timers/stop", { body: { session_id: sessionId } });
    void refresh();
  }

  if (auth.status !== "authed") {
    return (
      <main className="flex min-h-screen items-center justify-center">
        <Spinner />
      </main>
    );
  }

  return (
    <div className="flex h-screen flex-col">
      <AppHeader />
      <div className="flex min-h-0 flex-1">
        <BoardNav />
        <main
          className="flex min-w-0 flex-1 flex-col gap-4 overflow-auto p-4"
          style={{ borderLeft: "1px solid var(--m-border)" }}
        >
          <div className="flex items-center justify-between">
            <h1 className="text-sm font-semibold">Today</h1>
            <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              {items?.length ?? 0} open · assigned to you · across all boards
            </span>
          </div>

          {/* Docked timer working-set */}
          {workingSet !== null && workingSet.length > 0 && (
            <section
              className="flex flex-wrap items-center gap-2 rounded-[2px] border p-2"
              style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
              aria-label="Timer working set"
            >
              {workingSet.map((s) => (
                <div
                  key={s.id}
                  className="flex items-center gap-2 rounded-[2px] border px-2 py-1"
                  style={{
                    borderColor: s.stopped_at === null ? "var(--m-accent)" : "var(--m-border)",
                  }}
                >
                  <StatusTag tone={s.stopped_at === null ? "warning" : "neutral"}>
                    {s.stopped_at === null ? "RUNNING" : "PAUSED"}
                  </StatusTag>
                  <button
                    className="text-xs underline-offset-2 hover:underline"
                    onClick={() => router.push(`/tickets/${s.ticket_id}`)}
                  >
                    session {s.id.slice(0, 8)}
                  </button>
                  {s.stopped_at === null && (
                    <Button size="sm" variant="ghost" onClick={() => void stopTimer(s.id)}>
                      Stop
                    </Button>
                  )}
                </div>
              ))}
            </section>
          )}

          {error !== null && (
            <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
              {error}
            </p>
          )}

          {items === null ? (
            <div className="flex flex-1 items-center justify-center">
              <Spinner />
            </div>
          ) : items.length === 0 ? (
            <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              Nothing assigned to you yet. Tickets appear here highest-priority first.
            </p>
          ) : (
            <ol role="list" aria-label="My work queue" className="flex flex-col">
              {items.map((t) => {
                const dueIso = t.sla_resolution_due ?? t.due_at;
                const due = dueIso !== null && dueIso !== undefined ? dueLabel(dueIso) : null;
                return (
                  <li key={t.id}>
                    <button
                      type="button"
                      onClick={() => router.push(`/tickets/${t.id}`)}
                      className="row-hover grid h-6 w-full grid-cols-[auto_auto_1fr_auto_auto_auto] items-center gap-2 text-left text-xs"
                    >
                      <span className="font-mono" style={{ color: "var(--m-text-muted)" }}>
                        {t.number}
                      </span>
                      <PriorityTag priority={t.priority} />
                      <span className="truncate">{t.title}</span>
                      <span className="flex items-center gap-2">
                        {t.client_name !== null && t.client_name !== undefined && (
                          <ClientTag name={t.client_name} />
                        )}
                      </span>
                      <span className="truncate text-xs" style={{ color: "var(--m-text-muted)" }}>
                        {t.board_name}
                      </span>
                      <span
                        className="text-right tabular-nums"
                        style={{
                          color:
                            due?.overdue === true
                              ? "var(--m-status-danger-fg)"
                              : "var(--m-text-secondary)",
                          minWidth: "6rem",
                        }}
                        title={
                          dueIso !== null && dueIso !== undefined
                            ? `Due ${new Date(dueIso).toLocaleString()}`
                            : undefined
                        }
                      >
                        {due === null ? "—" : due.text}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ol>
          )}
        </main>
      </div>
    </div>
  );
}
