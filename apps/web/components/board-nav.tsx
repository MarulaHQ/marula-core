// BoardNav — the left-nav board tree (TASKLIST 2.11).
//
// Purpose: collapsible org board groups with favorite stars, rendered
// exclusively from Workbench tokens so light/dark parity is automatic.
// Starred boards float into a "Starred" section at the top; starring is
// per-user presentation state and never touches board data.
// Non-purpose: no board administration here (members/roles live behind
// the API's board-admin endpoints, not this surface).
"use client";

import type { components } from "@meridian/sdk";
import { Button } from "@meridian/ui";
import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";

type NavEntry = components["schemas"]["BoardNavEntry"];

type Group = { id: string; name: string; position: number };

function Star({ filled }: { filled: boolean }) {
  return (
    <span aria-hidden="true" style={{ color: filled ? "var(--m-accent)" : "var(--m-text-muted)" }}>
      {filled ? "★" : "☆"}
    </span>
  );
}

export function BoardNav() {
  const [entries, setEntries] = useState<NavEntry[] | null>(null);
  const [groups, setGroups] = useState<Group[]>([]);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const nav = await api.api.GET("/api/v1/boards/nav");
      if (nav.error !== undefined) throw new Error("failed to load boards");
      setEntries(nav.data);
      const gs = await api.api.GET("/api/v1/board-groups");
      if (gs.error === undefined) setGroups([...gs.data]);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Load failed");
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function toggleFavorite(entry: NavEntry) {
    await api.api.POST("/api/v1/boards/favorite", {
      body: { board_id: entry.id, favorite: !entry.favorite },
    });
    void refresh();
  }

  function toggleGroup(key: string) {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }

  if (entries === null) {
    return (
      <nav aria-label="Boards" className="hidden w-56 shrink-0 overflow-auto p-3 text-xs md:block">
        {error !== null && (
          <p role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
            {error}
          </p>
        )}
        {!error && <span style={{ color: "var(--m-text-muted)" }}>Loading…</span>}
      </nav>
    );
  }

  const starred = entries.filter((e) => e.favorite);
  const byId = new Map(entries.map((e) => [e.id, e]));
  const sections: { key: string; label: string; boards: NavEntry[] }[] = [
    ...[...groups]
      .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))
      .map((g) => ({
        key: g.id,
        label: g.name,
        boards: entries.filter((e) => e.group_id === g.id),
      })),
    { key: "_ungrouped", label: "Boards", boards: entries.filter((e) => e.group_id === null) },
  ].filter((s) => s.boards.length > 0);

  function renderEntry(e: NavEntry) {
    return (
      <div key={e.id} className="flex h-6 items-center gap-1 pl-4">
        <a
          href={`/boards/${e.id}`}
          title={e.description ?? e.name}
          className="truncate text-xs hover:underline"
          style={{ color: "var(--m-text)" }}
        >
          {e.name}
          {e.is_default && (
            <span className="ml-1" style={{ color: "var(--m-text-muted)" }}>
              ·
            </span>
          )}
        </a>
        <span className="ml-auto" />
        {e.my_role !== null && (
          <span
            className="text-[10px]"
            title={`Your board role: ${e.my_role}`}
            style={{ color: "var(--m-text-muted)" }}
          >
            {e.my_role === "admin" ? "A" : e.my_role === "member" ? "M" : "V"}
          </span>
        )}
        <Button
          variant="ghost"
          size="sm"
          className="!px-1 !py-0"
          aria-label={`${e.favorite ? "Unfavorite" : "Favorite"} ${e.name}`}
          aria-pressed={e.favorite}
          onClick={() => void toggleFavorite(e)}
        >
          <Star filled={e.favorite} />
        </Button>
      </div>
    );
  }

  return (
    <nav
      aria-label="Boards"
      className="hidden w-56 shrink-0 overflow-auto p-3 md:block"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
    >
      {starred.length > 0 && (
        <section className="mb-2">
          <h2 className="text-[10px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
            Starred
          </h2>
          {starred.map((e) => (
            <div key={`star-${e.id}`} className="flex h-6 items-center gap-1">
              <Star filled />
              <a href={`/boards/${e.id}`} className="truncate text-xs hover:underline" style={{ color: "var(--m-text)" }}>
                {byId.get(e.id)?.name ?? ""}
              </a>
            </div>
          ))}
        </section>
      )}

      {sections.map((s) => (
        <section key={s.key} className="mb-2">
          <button
            type="button"
            className="flex w-full items-center gap-1 text-left text-[10px] font-semibold uppercase tracking-wide hover:underline"
            style={{ color: "var(--m-text-muted)" }}
            aria-expanded={!collapsed.has(s.key)}
            onClick={() => toggleGroup(s.key)}
          >
            <span aria-hidden="true">{collapsed.has(s.key) ? "▸" : "▾"}</span>
            {s.label}
          </button>
          {!collapsed.has(s.key) && s.boards.map(renderEntry)}
        </section>
      ))}

      {entries.length === 0 && (
        <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          No boards yet.
        </p>
      )}
    </nav>
  );
}
