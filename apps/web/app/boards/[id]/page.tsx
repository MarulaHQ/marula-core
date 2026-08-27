// /boards/[id] — minimal board surface backing the left-nav tree links.
//
// Purpose: the board's status pipeline and its ticket queue rendered from
// real API data, tokens only, both themes. Deliberately v1-lean: workflow
// editing lives in the API; richer views arrive with later stages.
// Non-purpose: no board administration (statuses/transitions/members are
// managed through the API's ConfigureBoard endpoints, not this page).
"use client";

import type { components } from "@meridian/sdk";
import { Button, Spinner, StatusTag } from "@meridian/ui";
import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { AppHeader } from "../../../components/app-header";
import { BoardNav } from "../../../components/board-nav";
import { api, useRequireAuth } from "../../../lib/api";

type BoardStatus = components["schemas"]["BoardStatus"];
type Ticket = components["schemas"]["Ticket"];
type BoardMemberView = components["schemas"]["BoardMemberView"];

function categoryTone(category: string): "success" | "warning" | "neutral" {
  if (category === "closed") return "success";
  if (category === "waiting") return "warning";
  return "neutral";
}

const ROLES = ["viewer", "member", "admin"] as const;

/// Compact board-admin roster: existing members with inline role change.
/// Adding NEW members waits on an org user-directory endpoint (deferred;
/// see stage notes) — re-roling works fully today.
function MembersPanel({ boardId }: { boardId: string }) {
  const [members, setMembers] = useState<BoardMemberView[] | null>(null);
  const [hidden, setHidden] = useState(false);

  const refresh = useCallback(async () => {
    const res = await api.api.GET("/api/v1/boards/{id}/members", {
      params: { path: { id: boardId } },
    });
    if (res.response.status === 403) {
      setHidden(true);
      return;
    }
    if (res.error !== undefined) return;
    setMembers(res.data);
  }, [boardId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function changeRole(userId: string, role: string) {
    await api.api.PUT("/api/v1/boards/{id}/members", {
      params: { path: { id: boardId } },
      body: { user_id: userId, role: role as (typeof ROLES)[number] },
    });
    void refresh();
  }

  if (hidden || members === null) {
    return null;
  }
  return (
    <section aria-label="Board members" className="flex flex-col gap-1">
      <h2
        className="text-[10px] font-semibold uppercase tracking-wide"
        style={{ color: "var(--m-text-muted)" }}
      >
        Members ({members.length})
      </h2>
      {members.map((m) => (
        <div key={m.user_id} className="row-hover flex h-6 items-center gap-2 text-xs">
          <span className="truncate">{m.display_name}</span>
          <span className="ml-auto" />
          <select
            aria-label={`Role for ${m.display_name}`}
            className="rounded-[2px] border px-1 py-0 text-xs"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
            value={m.role}
            onChange={(e) => void changeRole(m.user_id, e.target.value)}
          >
            {ROLES.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
        </div>
      ))}
    </section>
  );
}

export default function BoardPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const params = useParams<{ id: string }>();
  const [statuses, setStatuses] = useState<BoardStatus[] | null>(null);
  const [tickets, setTickets] = useState<Ticket[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (typeof params?.id !== "string") return;
    try {
      const st = await api.api.GET("/api/v1/boards/{id}/statuses", {
        params: { path: { id: params.id } },
      });
      if (st.error !== undefined) throw new Error("failed to load board");
      setStatuses(st.data);
      const tk = await api.api.GET("/api/v1/tickets", {
        params: { query: { board_id: params.id, limit: 100 } },
      });
      if (tk.error !== undefined) throw new Error("failed to load tickets");
      setTickets(tk.data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Load failed");
    }
  }, [params?.id]);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

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
          <div className="flex items-center gap-3">
            <h1 className="text-sm font-semibold">Board</h1>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => router.push(`/boards/${params?.id ?? ""}/fields`)}
            >
              Custom fields
            </Button>
          </div>

          <MembersPanel boardId={params?.id ?? ""} />

          {error !== null && (
            <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
              {error}
            </p>
          )}

          {statuses !== null && (
            <div className="flex flex-wrap items-center gap-2">
              {[...statuses]
                .sort((a, b) => a.sort_order - b.sort_order)
                .map((s) => (
                  <StatusTag key={s.id} tone={categoryTone(s.category)}>
                    {s.name}
                  </StatusTag>
                ))}
            </div>
          )}

          {tickets === null ? (
            <div className="flex flex-1 items-center justify-center">
              <Spinner />
            </div>
          ) : tickets.length === 0 ? (
            <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              No tickets on this board yet.
            </p>
          ) : (
            <ol role="list" aria-label="Board tickets" className="flex flex-col">
              {tickets.map((t) => (
                <li key={t.id}>
                  <button
                    type="button"
                    onClick={() => router.push(`/tickets/${t.id}`)}
                    className="row-hover grid h-6 w-full grid-cols-[auto_1fr_auto_auto] items-center gap-2 text-left text-xs"
                  >
                    <span className="font-mono" style={{ color: "var(--m-text-muted)" }}>
                      {t.number}
                    </span>
                    <span className="truncate">{t.title}</span>
                    <StatusTag tone={categoryTone(t.category)}>{t.category}</StatusTag>
                    <span
                      className="tabular-nums"
                      style={{ color: "var(--m-text-secondary)" }}
                    >
                      {t.priority.toUpperCase()}
                    </span>
                  </button>
                </li>
              ))}
            </ol>
          )}
        </main>
      </div>
    </div>
  );
}
