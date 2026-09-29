// BoardNav — the left-nav board tree (TASKLIST 2.11).
//
// Purpose: collapsible org board groups with favorite stars, rendered
// exclusively from Workbench tokens so light/dark parity is automatic.
// Starred boards float into a "Starred" section at the top; starring is
// per-user presentation state and never touches board data.
// Non-purpose: no board administration here (members/roles live behind
// the API's board-admin endpoints, not this surface).
"use client";

import type { components } from "@marula-core/sdk";
import { Button } from "@marula-core/ui";
import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import { useDialogA11y } from "../lib/a11y/use-dialog-a11y";

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
  const [drawerOpen, setDrawerOpen] = useState(false);
  // Mobile boards drawer is modal: trap + Escape + focus restore.
  const drawerClose = useCallback(() => setDrawerOpen(false), []);
  const drawerRef = useDialogA11y<HTMLDivElement>(drawerOpen, drawerClose);

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

  const drawerToggle = (
    <button
      type="button"
      data-testid="board-nav-toggle"
      className="fixed left-2 bottom-2 z-40 flex items-center gap-1 rounded-full border px-3 py-1.5 text-xs md:hidden"
      style={{
        borderColor: "var(--m-border)",
        background: "var(--m-surface)",
        color: "var(--m-text)",
        boxShadow: "0 1px 4px rgb(0 0 0 / 0.12)",
      }}
      onClick={() => setDrawerOpen(true)}
    >
      <span aria-hidden="true">☰</span> Boards
    </button>
  );

  const drawerScrim = drawerOpen ? (
    <div
      data-testid="board-nav-scrim"
      className="fixed inset-0 z-40 bg-black/40 md:hidden"
      onClick={() => setDrawerOpen(false)}
      aria-hidden="true"
    />
  ) : null;

  function renderTree(entriesArg: NavEntry[] | null, errorArg: string | null) {
    if (entriesArg === null) {
      return errorArg !== null ? (
        <p role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
          {errorArg}
        </p>
      ) : (
        <span style={{ color: "var(--m-text-muted)" }}>Loading…</span>
      );
    }
    const starred = entriesArg.filter((e) => e.favorite);
    const sections: { key: string; label: string; boards: NavEntry[] }[] = [
      ...[...groups]
        .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))
        .map((g) => ({
          key: g.id,
          label: g.name,
          boards: entriesArg.filter((e) => e.group_id === g.id),
        })),
      { key: "_ungrouped", label: "Boards", boards: entriesArg.filter((e) => e.group_id === null) },
    ].filter((s) => s.boards.length > 0);
    const picked = (id: string) => {
      setDrawerOpen(false);
      return id;
    };
    const row = (e: NavEntry, indented: boolean) => {
      if (indented) {
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
        <div key={`star-${e.id}`} className="flex h-6 items-center gap-1">
          <Star filled />
          <a
            href={`/boards/${e.id}`}
            onClick={() => picked(e.id)}
            className="truncate text-xs hover:underline"
            style={{ color: "var(--m-text)" }}
          >
            {e.name}
          </a>
        </div>
      );
    };
    return (
      <>
        {starred.length > 0 && (
          <section className="mb-2">
            <h2 className="text-[10px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Starred
            </h2>
            {starred.map((e) => row(e, false))}
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
            {!collapsed.has(s.key) && s.boards.map((e) => row(e, true))}
          </section>
        ))}
        {entriesArg.length === 0 && (
          <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
            No boards yet.
          </p>
        )}
      </>
    );
  }

  return (
    <>
      {drawerToggle}
      {drawerScrim}
      {drawerOpen && (
        <div
          ref={drawerRef}
          tabIndex={-1}
          data-testid="board-nav-drawer"
          role="dialog"
          aria-modal="true"
          aria-label="Boards"
          className="fixed inset-y-0 left-0 z-50 w-64 overflow-auto p-3 text-xs md:hidden"
          style={{
            background: "var(--m-surface)",
            borderRight: "1px solid var(--m-border)",
            animation: "m-drawer-in 120ms ease-out",
          }}
        >
          <div className="mb-2 flex items-center justify-between">
            <span className="text-[11px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Boards
            </span>
            <Button
              variant="ghost"
              size="sm"
              aria-label="Close boards drawer"
              onClick={() => setDrawerOpen(false)}
            >
              ✕
            </Button>
          </div>
          {renderTree(entries, error)}
        </div>
      )}
      <nav
        aria-label="Boards"
        className="hidden w-56 shrink-0 overflow-auto p-3 md:block"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      >
        {renderTree(entries, error)}
      </nav>
    </>
  );
}
