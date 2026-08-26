// /today — the my-work surface.
//
// Purpose: priority-ordered list of the caller's open tickets plus a docked
// timer working-set strip (running first); start/switch/stop inline. Matches
// design/mockups/today-light.jpeg and today-dark.png: dense table, quiet
// chrome, status color only where it carries meaning.
// Non-purpose: no board management or admin surfaces here.
"use client";

import type { components } from "@meridian/sdk";
import {
  Button,
  EmptyState,
  Spinner,
  StatusTag,
  Table,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { AppHeader } from "../../components/app-header";
import { SetupBanner } from "../../components/setup-banner";
import { api, useRequireAuth } from "../../lib/api";

type Ticket = components["schemas"]["Ticket"];
type TimerSession = components["schemas"]["TimerSession"];

const PRIORITY_ORDER: Record<string, number> = { p1: 0, p2: 1, p3: 2, p4: 3 };

function PriorityTag({ priority }: { priority: string }) {
  const tone =
    priority === "p1" ? "danger" : priority === "p2" ? "warning" : "neutral";
  return <StatusTag tone={tone}>{priority.toUpperCase()}</StatusTag>;
}

function categoryTone(category: string): "success" | "warning" | "neutral" {
  if (category === "closed") return "success";
  if (category === "waiting") return "warning";
  return "neutral";
}

export default function TodayPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const [tickets, setTickets] = useState<Ticket[] | null>(null);
  const [workingSet, setWorkingSet] = useState<TimerSession[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const mine = await api.api.GET("/api/tickets/my-work");
      const ws = await api.api.GET("/api/timers/working-set");
      if (mine.error !== undefined) throw new Error("failed to load tickets");
      if (ws.error !== undefined) throw new Error("failed to load timers");
      setTickets(
        [...mine.data].sort(
          (a, b) =>
            (PRIORITY_ORDER[a.priority] ?? 9) -
            (PRIORITY_ORDER[b.priority] ?? 9),
        ),
      );
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

  const running = workingSet?.find((s) => s.stopped_at === null);
  void running;

  return (
    <div className="flex h-screen flex-col">
      <AppHeader />
      <main className="flex min-h-0 flex-1 flex-col gap-4 overflow-auto p-4">
        <div className="flex items-center justify-between">
          <h1 className="text-sm font-semibold">Today</h1>
          <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
            {tickets?.length ?? 0} open · assigned to you
          </span>
        </div>

        <SetupBanner />

        {/* Docked timer working-set */}
        {workingSet !== null && workingSet.length > 0 && (
          <section
            className="flex flex-wrap items-center gap-2 rounded-[2px] border p-2"
            style={{
              borderColor: "var(--m-border)",
              background: "var(--m-surface)",
            }}
            aria-label="Timer working set"
          >
            {workingSet.map((s) => (
              <div
                key={s.id}
                className="flex items-center gap-2 rounded-[2px] border px-2 py-1"
                style={{
                  borderColor:
                    s.stopped_at === null
                      ? "var(--m-accent)"
                      : "var(--m-border)",
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
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => void stopTimer(s.id)}
                  >
                    Stop
                  </Button>
                )}
              </div>
            ))}
          </section>
        )}

        {error !== null && (
          <p
            className="text-xs"
            style={{ color: "var(--m-status-danger-fg)" }}
            role="alert"
          >
            {error}
          </p>
        )}

        {tickets === null ? (
          <div className="flex flex-1 items-center justify-center">
            <Spinner />
          </div>
        ) : tickets.length === 0 ? (
          <EmptyState message="Nothing assigned to you yet. Tickets appear here highest-priority first." />
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Number</Th>
                <Th>Priority</Th>
                <Th>Title</Th>
                <Th>Status</Th>
                <Th>Due</Th>
              </tr>
            </thead>
            <tbody>
              {tickets.map((t) => (
                <Tr key={t.id} onClick={() => router.push(`/tickets/${t.id}`)}>
                  <Td>{t.number}</Td>
                  <Td>
                    <PriorityTag priority={t.priority} />
                  </Td>
                  <Td>{t.title}</Td>
                  <Td>
                    <StatusTag tone={categoryTone(t.category)}>
                      {t.category}
                    </StatusTag>
                  </Td>
                  <Td>
                    {t.due_at !== null && t.due_at !== undefined
                      ? new Date(t.due_at).toLocaleDateString()
                      : "—"}
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </main>
    </div>
  );
}
