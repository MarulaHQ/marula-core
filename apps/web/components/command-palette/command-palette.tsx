// The command palette (pack states K01 search, K02 chat).
//
// Purpose: Cmd/Ctrl+K or the top-bar field opens this overlay. Search mode is
// the default: one field, a Search/Ask switch when general chat is granted,
// results grouped in the /search page's order, the first row selected, and a
// footer that teaches the keys. Ask mode is the same box: a context chip, the
// conversation, sources under each answer, dashed proposal rows that wait for
// approval, and "Open as a panel".
//
// Non-purpose: no AI affordance without the grant — not a disabled switch, not
// a hint, not an empty gap. Search results never gain an Ask control. The
// full-page /search surface stays AI-free (its P4 rule); this is a separate
// surface that happens to search the same endpoint.
//
// Geometry: 680 px wide, centred, 84 px from the top (the pack's K01 frame),
// never a full page. The dialog owns the focus trap and restores focus on
// close.

"use client";

import { Button, Spinner } from "@marula-core/ui";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  Icon,
  Kbd,
  Segmented,
  SegmentedItem,
  type IconName,
} from "@marula-core/ui/kit";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";

import { sendChatTurn } from "./chat-turn";
import { ChatLog } from "./chat-log";
import type { GrantStatus } from "./grant";
import { describeScreenContext, type ScreenContext } from "./screen-context";
import {
  ACTIONS,
  flatOpenable,
  groupResults,
  resultHref,
  resultMeta,
  rowKey,
  type PaletteAction,
  type ResultGroup,
} from "./search-model";
import { actions, getServerSnapshot, getSnapshot, subscribe, type ChatMessage } from "./store";
import { usePaletteSearch } from "./use-search";

export interface CommandPaletteProps {
  grant: GrantStatus;
  context: ScreenContext;
}

export function CommandPalette({ grant, context }: CommandPaletteProps) {
  const state = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
  const search = usePaletteSearch(state.query);
  const groups = useMemo(
    () => groupResults(search.response?.results ?? []),
    [search.response],
  );
  const flat = useMemo(() => flatOpenable(groups, ACTIONS), [groups]);
  const [cursor, setCursor] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const askAvailable = grant === "granted";
  const mode = state.mode === "ask" && askAvailable ? "ask" : "search";

  // The cursor is clamped, never lost: a shorter answer keeps the person on
  // the nearest row instead of jumping to the top.
  useEffect(() => {
    setCursor((current) => {
      if (flat.length === 0) return 0;
      return Math.min(current, flat.length - 1);
    });
  }, [flat.length]);

  // A fresh search keeps the first row selected, as the pack draws it.
  useEffect(() => {
    if (search.response !== null) setCursor(0);
  }, [search.response]);

  useEffect(() => {
    if (state.open) {
      window.requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [state.open, mode]);

  const active = flat[cursor];
  const activeId = active === undefined ? undefined : `palette-row-${rowKey(active)}`;
  const router = useRouter();

  function openRow(href: string): void {
    actions.close();
    router.push(href);
  }

  function send(): void {
    if (mode !== "ask" || state.busy || state.query.trim() === "") return;
    const text = state.query;
    actions.setQuery("");
    void sendChatTurn(text, context);
  }

  function onKeyDown(event: React.KeyboardEvent<HTMLDivElement>): void {
    const fromInput = document.activeElement === inputRef.current;
    // Tab asks only from the field; from anywhere else it moves focus
    // normally, so the footer link and the mode switch stay reachable.
    if (event.key === "Tab" && fromInput && mode === "search" && askAvailable && !event.shiftKey) {
      event.preventDefault();
      actions.setMode("ask");
      return;
    }
    if (mode === "ask" || !fromInput) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setCursor((current) => (flat.length === 0 ? 0 : Math.min(current + 1, flat.length - 1)));
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setCursor((current) => (flat.length === 0 ? 0 : Math.max(current - 1, 0)));
      return;
    }
    if (event.key === "Enter" && active !== undefined) {
      event.preventDefault();
      openRow(active.href);
    }
  }

  const lastUserMessage = [...state.messages].reverse().find((message) => message.role === "user");

  return (
    <Dialog open={state.open} onOpenChange={(open) => (open ? actions.open(state.mode) : actions.close())}>
      <DialogContent
        width={680}
        aria-describedby={undefined}
        className="m-0 self-start"
        style={{ marginTop: "84px" }}
        onKeyDown={onKeyDown}
      >
        <DialogTitle className="sr-only">
          {mode === "ask" ? "Ask about this screen" : "Command palette"}
        </DialogTitle>
        <div className="flex max-h-[calc(100dvh-140px)] flex-col overflow-hidden rounded-modal border border-border bg-card text-foreground shadow-mc">
          {/* Field row: search glyph, the field, the mode switch. */}
          <div className="flex h-15 shrink-0 items-center gap-3 border-b border-border px-4.5">
            <span className="shrink-0 text-muted-foreground">
              <Icon name={mode === "ask" ? "chat" : "search"} size={18} />
            </span>
            <input
              ref={inputRef}
              type="text"
              role={mode === "search" ? "combobox" : undefined}
              aria-expanded={mode === "search" ? true : undefined}
              aria-controls={mode === "search" ? "palette-results" : undefined}
              aria-autocomplete={mode === "search" ? "list" : undefined}
              aria-activedescendant={mode === "search" ? activeId : undefined}
              aria-label={mode === "ask" ? "Ask about this screen" : "Search or jump to"}
              placeholder={mode === "ask" ? "Ask about this screen…" : "Search tickets, clients, devices…"}
              value={state.query}
              onChange={(event) => actions.setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && mode === "ask") {
                  event.preventDefault();
                  send();
                }
              }}
              className="min-w-0 flex-1 border-0 bg-transparent text-[17px] outline-none placeholder:text-muted-foreground"
              autoComplete="off"
              spellCheck={false}
            />
            {askAvailable ? (
              <Segmented
                type="single"
                value={mode}
                onValueChange={(value) => {
                  if (value === "search" || value === "ask") actions.setMode(value);
                }}
                aria-label="Search or ask"
              >
                <SegmentedItem value="search" size="viewer">
                  Search
                </SegmentedItem>
                <SegmentedItem value="ask" size="viewer">
                  Ask
                </SegmentedItem>
              </Segmented>
            ) : null}
          </div>

          {mode === "search" ? (
            <SearchBody
              groups={groups}
              actions={ACTIONS}
              activeId={activeId}
              cursorRow={active}
              onHover={(row) => {
                const index = flat.findIndex((candidate) => rowKey(candidate) === rowKey(row));
                if (index !== -1) setCursor(index);
              }}
              onOpen={openRow}
              query={search.query}
              loading={search.loading}
              error={search.error}
              total={search.response?.total ?? null}
              onRetry={search.retry}
            />
          ) : (
            <AskBody
              messages={state.messages}
              context={context}
              busy={state.busy}
              onRetry={() => {
                if (lastUserMessage !== undefined) void sendChatTurn(lastUserMessage.text, context);
              }}
            />
          )}

          {/* Footer: the keys in search mode, the panel offer in ask mode. */}
          {mode === "search" ? (
            <div className="flex shrink-0 items-center gap-2.5 border-t border-border bg-background px-4.5 py-2.5 text-[13px] text-muted-foreground">
              <Kbd>↑</Kbd>
              <Kbd>↓</Kbd>
              <span>navigate</span>
              <Kbd>↵</Kbd>
              <span>open</span>
              {askAvailable ? (
                <>
                  <Kbd>Tab</Kbd>
                  <span>ask instead</span>
                </>
              ) : null}
              <span className="flex-1" />
              <Link href="/search" className="font-semibold text-foreground underline" onClick={() => actions.close()}>
                Full search page
              </Link>
              <Kbd>Esc</Kbd>
              <span>close</span>
            </div>
          ) : (
            <div className="flex shrink-0 items-center gap-2.5 border-t border-border bg-background px-4.5 py-2.5 text-[13px] text-muted-foreground">
              <Icon name="info" size={14} className="shrink-0" />
              <span>The chat sees this screen, not your other work.</span>
              <span className="flex-1" />
              <Button
                size="sm"
                onClick={() => {
                  actions.setPanel("entering");
                  actions.close();
                }}
              >
                <Icon name="expand" size={14} />
                Open as a panel
              </Button>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function SearchBody({
  groups,
  actions: actionList,
  activeId,
  cursorRow,
  onHover,
  onOpen,
  query,
  loading,
  error,
  total,
  onRetry,
}: {
  groups: ResultGroup[];
  actions: PaletteAction[];
  activeId: string | undefined;
  cursorRow: ReturnType<typeof flatOpenable>[number] | undefined;
  onHover: (row: ReturnType<typeof flatOpenable>[number]) => void;
  onOpen: (href: string) => void;
  query: string;
  loading: boolean;
  error: string | null;
  total: number | null;
  onRetry: () => void;
}) {
  const isEmptyAnswer = !loading && error === null && query.length >= 2 && total === 0;
  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
      {loading ? (
        <div className="flex items-center gap-2 px-3 py-3 text-[13px] text-muted-foreground">
          <Spinner /> Searching…
        </div>
      ) : null}

      {error !== null ? (
        <div
          role="alert"
          className="mx-1 mt-2 flex items-center gap-3 rounded-ctl px-3 py-2 text-[13px] font-medium"
          style={{ background: "var(--m-status-danger-bg)", color: "var(--m-status-danger-fg)" }}
        >
          <Icon name="alert" size={16} />
          <span className="flex-1">{error}</span>
          <Button size="sm" onClick={onRetry}>
            Try again
          </Button>
        </div>
      ) : null}

      {query.length < 2 && error === null ? (
        <p className="px-3 py-3 text-[13px] text-muted-foreground">Type at least two characters.</p>
      ) : null}

      {isEmptyAnswer ? (
        <div className="px-3 py-3">
          <p className="text-sm font-semibold">Nothing matches “{query}”</p>
          <p className="mt-0.5 text-[13px] text-muted-foreground">
            Check the spelling, or search for fewer words.
          </p>
        </div>
      ) : null}

      {/* A listbox may contain only options and groups; the status lines above
          live outside it, and each group heading is presentational. */}
      <div id="palette-results" role="listbox" aria-label="Search results">
        {groups.map((group) => (
          <div key={group.key} role="group" aria-label={`${group.label}, ${group.hits.length}`}>
            <div
              role="presentation"
              className="px-3 pb-1 pt-2 text-xs font-semibold uppercase tracking-[0.08em] text-muted-foreground"
            >
              {group.label} · {group.hits.length}
            </div>
            {group.hits.map((hit) => {
              const href = resultHref(hit);
              const selected = cursorRow?.kind === "result" && cursorRow.hit === hit;
              const body = (
                <>
                  <span
                    className={
                      selected
                        ? "flex size-7.5 shrink-0 items-center justify-center rounded-md bg-card text-mc-act-fg"
                        : "flex size-7.5 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground"
                    }
                  >
                    <Icon name={iconForKind(hit.entity_kind)} size={16} />
                  </span>
                  <span className="min-w-0 flex-1 truncate font-medium">{hit.title}</span>
                  <span
                    className={
                      selected
                        ? "max-w-[45%] shrink-0 truncate text-[13px] text-mc-act-fg"
                        : "max-w-[45%] shrink-0 truncate text-[13px] text-muted-foreground"
                    }
                  >
                    {resultMeta(hit)}
                  </span>
                </>
              );
              const shared =
                "flex h-11 w-full items-center gap-3 rounded-lg px-3 text-left text-sm";
              if (href === null) {
                // Sites and contacts have no record route yet, so they are
                // shown but not offered as an action Enter cannot honour.
                return (
                  <div
                    key={`${hit.entity_kind}-${hit.entity_id}`}
                    role="option"
                    aria-disabled="true"
                    aria-selected="false"
                    className={`${shared} text-muted-foreground`}
                  >
                    {body}
                  </div>
                );
              }
              return (
                <div
                  key={`${hit.entity_kind}-${hit.entity_id}`}
                  id={`palette-row-result:${hit.entity_kind}:${hit.entity_id}`}
                  role="option"
                  aria-selected={selected}
                  tabIndex={-1}
                  data-testid={`palette-result-${hit.entity_kind}-${hit.entity_id}`}
                  className={
                    selected
                      ? `${shared} cursor-pointer bg-mc-act text-mc-act-fg`
                      : `${shared} cursor-pointer hover:bg-muted`
                  }
                  onMouseEnter={() => onHover({ kind: "result", href, hit })}
                  onClick={() => onOpen(href)}
                >
                  {body}
                  {selected ? <Kbd tone="inherit">↵</Kbd> : null}
                </div>
              );
            })}
          </div>
        ))}

        <div role="group" aria-label="Actions">
          <div
            role="presentation"
            className="px-3 pb-1 pt-2 text-xs font-semibold uppercase tracking-[0.08em] text-muted-foreground"
          >
            Actions
          </div>
          {actionList.map((action) => {
            const selected = cursorRow?.kind === "action" && cursorRow.action.id === action.id;
            return (
              <div
                key={action.id}
                id={`palette-row-action:${action.id}`}
                role="option"
                aria-selected={selected}
                tabIndex={-1}
                data-testid={`palette-${action.id}`}
                className={
                  selected
                    ? "flex h-11 cursor-pointer items-center gap-3 rounded-lg bg-mc-act px-3 text-sm text-mc-act-fg"
                    : "flex h-11 cursor-pointer items-center gap-3 rounded-lg px-3 text-sm hover:bg-muted"
                }
                onMouseEnter={() => onHover({ kind: "action", href: action.href, action })}
                onClick={() => onOpen(action.href)}
              >
                <span
                  className={
                    selected
                      ? "flex size-7.5 shrink-0 items-center justify-center rounded-md bg-card text-mc-act-fg"
                      : "flex size-7.5 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground"
                  }
                >
                  <Icon name={action.icon} size={16} />
                </span>
                <span className="min-w-0 flex-1 truncate font-medium">{action.label}</span>
                <span className={selected ? "text-[13px] text-mc-act-fg" : "text-[13px] text-muted-foreground"}>
                  {action.hint}
                </span>
                {selected ? <Kbd tone="inherit">↵</Kbd> : null}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function AskBody({
  messages,
  context,
  busy,
  onRetry,
}: {
  messages: ChatMessage[];
  context: ScreenContext;
  busy: boolean;
  onRetry: () => void;
}) {
  const parts = describeScreenContext(context);
  const [label, ...detail] = parts;
  return (
    <>
      <div className="shrink-0 px-4.5 pt-3">
        <span
          data-testid="palette-context-chip"
          className="inline-flex min-h-7.5 flex-wrap items-center gap-2 rounded-pill bg-muted px-3 py-1 text-[13px] font-medium"
        >
          <Icon name="ticket" size={14} />
          <span>
            This screen: <b className="font-bold">{label}</b>
            {detail.length > 0 ? ` · ${detail.join(" · ")}` : ""}
          </span>
        </span>
      </div>
      <div className="flex min-h-40 flex-1 flex-col gap-3.5 overflow-y-auto px-4.5 py-4">
        <ChatLog
          messages={messages}
          emptyHint="Ask about what this screen shows. It cannot see your other work."
        />
        {busy && messages.every((message) => !message.pending) ? (
          <div className="flex items-center gap-2 text-[13px] text-muted-foreground">
            <Spinner /> Thinking…
          </div>
        ) : null}
        {messages.some((message) => message.error !== null) ? (
          <div>
            <Button size="sm" onClick={onRetry} disabled={busy}>
              Try again
            </Button>
          </div>
        ) : null}
      </div>
    </>
  );
}

function iconForKind(kind: string): IconName {
  if (kind === "ticket") return "ticket";
  if (kind === "comment") return "chat";
  if (kind === "client") return "users";
  if (kind === "document") return "file";
  if (kind === "kb_article") return "book";
  if (kind === "site") return "monitor";
  if (kind === "contact") return "user";
  return "search";
}
