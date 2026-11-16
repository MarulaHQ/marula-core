// The slide-up chat panel (pack states K03 open, K04 mid-expansion, K05
// collapsed tab).
//
// Purpose: when the palette is too small, the conversation moves here without
// losing a message. Five hundred and twenty pixels wide, anchored above the
// timer bar (the work bar's measured height, so a wrapped bar still clears
// it), never the whole viewport, and the bar stays visible underneath. It
// translates in from its anchor: K04 is the first frames of that motion, with
// the lower edge still clipped behind the bar. Esc collapses to the tab; the
// tab carries the last answer's first line.
//
// Non-purpose: no floating corner widget, no second conversation, no approval
// control. It only exists with the general_chat grant.

"use client";

import { Button } from "@marula-core/ui";
import { Icon, IconButton } from "@marula-core/ui/kit";
import { useEffect, useRef, useState, useSyncExternalStore } from "react";

import { sendChatTurn } from "./chat-turn";
import { ChatLog } from "./chat-log";
import type { GrantStatus } from "./grant";
import { describeScreenContext, type ScreenContext } from "./screen-context";
import {
  actions,
  getServerSnapshot,
  getSnapshot,
  lastAssistantPreview,
  subscribe,
} from "./store";

const PANEL_WIDTH = 520;
const PANEL_HEIGHT = 470;
/** The pack's mid-expansion offset (K04). */
const ENTER_OFFSET = "translateY(58%)";

/** The transform for a phase: K04's 58% while entering, settled otherwise. */
export function panelTransform(phase: "entering" | "open"): string {
  return phase === "entering" ? ENTER_OFFSET : "translateY(0)";
}

export function ChatPanel({ grant, context }: { grant: GrantStatus; context: ScreenContext }) {
  const state = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
  const barHeight = useWorkBarHeight(state.panel !== "closed");
  const panelRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const tabRef = useRef<HTMLButtonElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const [draft, setDraft] = useState("");
  const phase = state.panel;

  // No grant, no panel — not even a closed remnant.
  useEffect(() => {
    if (grant !== "granted" && phase !== "closed") actions.setPanel("closed");
  }, [grant, phase]);

  // K04 -> K03: the entrance settles one frame after paint, so the transform
  // transition actually runs. Reduced motion lands immediately.
  useEffect(() => {
    if (phase !== "entering") return;
    const reduced =
      typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduced) {
      actions.setPanel("open");
      return;
    }
    let inner = 0;
    const outer = window.requestAnimationFrame(() => {
      inner = window.requestAnimationFrame(() => actions.setPanel("open"));
    });
    return () => {
      window.cancelAnimationFrame(outer);
      if (inner !== 0) window.cancelAnimationFrame(inner);
    };
  }, [phase]);

  useFocusTrap(panelRef, phase === "open" || phase === "entering", () => {
    actions.setPanel("collapsed");
  });

  useEffect(() => {
    if (phase === "open") inputRef.current?.focus();
    if (phase === "collapsed") tabRef.current?.focus();
  }, [phase]);

  useEffect(() => {
    if (typeof bottomRef.current?.scrollIntoView === "function") {
      bottomRef.current.scrollIntoView({ block: "nearest" });
    }
  }, [state.messages]);

  if (grant !== "granted" || phase === "closed") return null;

  const preview = lastAssistantPreview(state.messages);

  if (phase === "collapsed") {
    return (
      <div className="fixed right-4 z-40" style={{ bottom: barHeight }}>
        <button
          ref={tabRef}
          type="button"
          data-testid="chat-panel-tab"
          aria-label="Expand the chat"
          title="Expand the chat (Esc closes)"
          onClick={() => actions.setPanel("entering")}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              actions.setPanel("closed");
            }
          }}
          className="flex h-10 max-w-[360px] cursor-pointer items-center gap-2.5 rounded-t-ctl border border-b-0 border-border bg-card px-3.5 font-semibold text-foreground shadow-mc"
        >
          <Icon name="chat" size={18} />
          <span>Ask</span>
          <span className="max-w-[190px] truncate font-normal text-muted-foreground">
            {preview === "" ? "No answer yet" : preview}
          </span>
          <Icon name="up" size={16} />
        </button>
      </div>
    );
  }

  const parts = describeScreenContext(context);
  const panelHeight = `min(${PANEL_HEIGHT}px, calc(100dvh - ${barHeight}px - 24px))`;
  const lastUserMessage = [...state.messages].reverse().find((message) => message.role === "user");
  const hasError = state.messages.some((message) => message.error !== null);

  return (
    <div
      className="fixed right-4 z-40 overflow-hidden"
      style={{ bottom: barHeight, width: `min(${PANEL_WIDTH}px, calc(100vw - 32px))`, height: panelHeight }}
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-label="Ask about this screen"
        data-testid="chat-panel"
        data-phase={phase}
        className="flex h-full flex-col rounded-t-[18px] border border-b-0 border-border bg-card text-foreground shadow-mc transition-transform duration-[120ms] ease-out motion-reduce:transition-none"
        style={{ transform: phase === "entering" ? panelTransform("entering") : panelTransform("open") }}
      >
        <div className="flex shrink-0 items-center gap-2.5 border-b border-border px-4 py-3.5">
          <Icon name="chat" size={18} />
          <h2 className="m-0 flex-1 text-[15px] font-semibold">Ask</h2>
          <span
            data-testid="chat-panel-context"
            className="inline-flex min-w-0 items-center gap-2 rounded-pill bg-muted px-3 py-1 text-[13px] font-medium"
          >
            <Icon name="ticket" size={14} className="shrink-0" />
            <span className="truncate">{parts.join(" · ")}</span>
          </span>
          <IconButton
            aria-label="Collapse the chat"
            title="Collapse the chat"
            className="size-8"
            onClick={() => actions.setPanel("collapsed")}
          >
            <Icon name="down" size={16} />
          </IconButton>
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-3.5 overflow-y-auto px-4 py-4">
          <ChatLog
            messages={state.messages}
            emptyHint="Ask about what this screen shows. It cannot see your other work."
          />
          {state.busy && state.messages.every((message) => !message.pending) ? (
            <p className="text-[13px] text-muted-foreground">Thinking…</p>
          ) : null}
          {hasError ? (
            <div>
              <Button
                size="sm"
                disabled={state.busy || lastUserMessage === undefined}
                onClick={() => {
                  if (lastUserMessage !== undefined) void sendChatTurn(lastUserMessage.text, context);
                }}
              >
                Try again
              </Button>
            </div>
          ) : null}
          <div ref={bottomRef} />
        </div>

        <div className="flex shrink-0 items-center gap-2 border-t border-border px-3 py-2.5">
          <input
            ref={inputRef}
            type="text"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter") return;
              event.preventDefault();
              const text = draft.trim();
              if (text === "" || state.busy) return;
              setDraft("");
              void sendChatTurn(text, context);
            }}
            placeholder="Ask about this screen…"
            aria-label="Ask about this screen"
            autoComplete="off"
            className="h-11 min-w-0 flex-1 rounded-ctl border border-input bg-transparent px-3.5 text-sm outline-none placeholder:text-muted-foreground"
          />
          <Button
            variant="primary"
            disabled={state.busy || draft.trim() === ""}
            onClick={() => {
              const text = draft.trim();
              if (text === "" || state.busy) return;
              setDraft("");
              void sendChatTurn(text, context);
            }}
          >
            <Icon name="send" size={16} />
            Send
          </Button>
        </div>
      </div>
    </div>
  );
}

/** The work bar's height, so the panel's lower edge meets it instead of
 *  covering it. Zero when no timer is running (there is no bar to clear). */
function useWorkBarHeight(active: boolean): number {
  const [height, setHeight] = useState(0);
  useEffect(() => {
    if (!active) return;
    let element: HTMLElement | null = null;
    let sizeObserver: ResizeObserver | null = null;

    const measure = (): void => {
      setHeight(element === null ? 0 : element.getBoundingClientRect().height);
    };

    const attach = (next: HTMLElement | null): void => {
      if (next === element) return;
      sizeObserver?.disconnect();
      sizeObserver = null;
      element = next;
      measure();
      if (element !== null && typeof ResizeObserver !== "undefined") {
        sizeObserver = new ResizeObserver(measure);
        sizeObserver.observe(element);
      }
    };

    attach(document.querySelector<HTMLElement>('[data-testid="work-bar"]'));

    // The bar mounts and unmounts with the timer while the panel stays open,
    // so its presence is watched as well as its size: without this a timer
    // started after the panel opened would be covered by it.
    const presence =
      typeof MutationObserver === "undefined"
        ? null
        : new MutationObserver(() => {
            attach(document.querySelector<HTMLElement>('[data-testid="work-bar"]'));
          });
    presence?.observe(document.body, { childList: true, subtree: true });

    return () => {
      presence?.disconnect();
      sizeObserver?.disconnect();
    };
  }, [active]);
  return height;
}

/** Traps Tab inside an overlay and restores focus when it closes. Esc is the
 *  caller's decision, because the panel collapses while the palette closes. */
function useFocusTrap(
  ref: React.RefObject<HTMLElement | null>,
  active: boolean,
  onEscape: () => void,
): void {
  // The callback is held in a ref so a re-render does not tear the trap down
  // and restore focus mid-interaction.
  const escapeRef = useRef(onEscape);
  useEffect(() => {
    escapeRef.current = onEscape;
  }, [onEscape]);

  useEffect(() => {
    if (!active) return;
    const node = ref.current;
    if (node === null) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        escapeRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = Array.from(
        node.querySelectorAll<HTMLElement>(
          'a[href], button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      );
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (first === undefined || last === undefined) return;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    node.addEventListener("keydown", onKeyDown);
    return () => {
      node.removeEventListener("keydown", onKeyDown);
      previous?.focus();
    };
  }, [active, ref]);
}
