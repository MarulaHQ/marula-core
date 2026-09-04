// /boards — board index (nav-pass companion surface).
//
// Purpose: the landing page for the nav's "Boards" entry: every board the
// caller can reach, grouped by board group (groups in order, ungrouped
// last), with the caller's board role and default marker. Reads only the
// existing GET /boards/nav + GET /board-groups endpoints.
// Non-purpose: no board administration here (board admin lives under
// Settings per the UI backlog); no board tree — BoardNav keeps that job on
// the surfaces that host it.
"use client";

import { Button, Card, EmptyState, Spinner, Tag } from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";

type NavEntry = components["schemas"]["BoardNavEntry"];
type Group = { id: string; name: string; position: number };

export default function BoardsIndexPage() {
  const auth = useRequireAuth();
  const [entries, setEntries] = useState<NavEntry[] | null>(null);
  const [groups, setGroups] = useState<Group[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const nav = await api.api.GET("/api/v1/boards/nav");
    if (nav.error !== undefined || nav.data === undefined) {
      setError("Failed to load boards. Check your connection and retry.");
      setEntries([]);
      return;
    }
    setEntries(nav.data);
    setError(null);
    const gs = await api.api.GET("/api/v1/board-groups");
    if (gs.error === undefined && gs.data !== undefined) setGroups([...gs.data]);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  if (auth.status !== "authed") return null;

  const sections: { key: string; label: string; boards: NavEntry[] }[] = [
    ...[...groups]
      .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))
      .map((g) => ({
        key: g.id,
        label: g.name,
        boards: (entries ?? []).filter((e) => e.group_id === g.id),
      })),
    {
      key: "_ungrouped",
      label: "Ungrouped boards",
      boards: (entries ?? []).filter((e) => e.group_id === null),
    },
  ].filter((s) => s.boards.length > 0);

  return (
    <div className="mx-auto grid max-w-3xl gap-6 p-6">
      <div>
        <h1 className="text-lg font-semibold">Boards</h1>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Every board you can reach, grouped by the org's board groups.
        </p>
      </div>

      {entries === null ? (
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
            <Button variant="secondary" size="sm" onClick={() => void refresh()}>
              Retry
            </Button>
          </div>
        </Card>
      ) : entries.length === 0 ? (
        <Card>
          <div className="p-4">
            <EmptyState message="No boards yet. Boards appear here once created." />
          </div>
        </Card>
      ) : (
        sections.map((section) => (
          <Card key={section.key}>
            <h2 className="p-4 pb-2 text-sm font-semibold">{section.label}</h2>
            <ul className="grid gap-0">
              {section.boards.map((board) => (
                <li
                  key={board.id}
                  className="flex items-center gap-3 border-t px-4 py-2"
                  style={{ borderColor: "var(--m-border)" }}
                >
                  <Link
                    href={`/boards/${board.id}`}
                    className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2"
                  >
                    {board.name}
                  </Link>
                  {board.is_default ? <Tag>default</Tag> : null}
                  {board.favorite ? (
                    <span aria-label="Favorite" style={{ color: "var(--m-accent)" }}>
                      ★
                    </span>
                  ) : null}
                  {board.my_role !== null ? (
                    <span
                      className="ml-auto text-xs"
                      title={`Your board role: ${board.my_role}`}
                      style={{ color: "var(--m-text-muted)" }}
                    >
                      {board.my_role}
                    </span>
                  ) : null}
                </li>
              ))}
            </ul>
          </Card>
        ))
      )}
    </div>
  );
}
