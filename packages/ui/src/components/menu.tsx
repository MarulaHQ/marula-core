"use client";
/**
 * Menu — the anchored action list the product had been improvising.
 *
 * Purpose: one keyboard-navigable menu for record and message overflow, a
 * capability chip's provider list, and any other "more than one thing you can
 * do here" slot. `@marula-core/ui` shipped `Dialog`, `Select` and `Tooltip`
 * and no menu, so every surface that needed one either borrowed a `Select`
 * (which announces itself as a listbox and expects a value) or hand-rolled
 * absolute positioning. This is the one definition.
 *
 * Three decisions worth naming:
 *
 * 1. **In place, not portalled.** The panel is an absolutely positioned child
 *    of the trigger's own wrapper. A portal would escape the clipping of the
 *    surface that owns it and would then need collision handling to stay on
 *    screen; every place this menu is used (a card, a chip row, a page header)
 *    has room below the trigger. The wrapper is `position: relative` and the
 *    panel is `top: calc(100% + 6px)`, the same 6px the column chooser uses.
 *    `align` picks the edge, because a control at the right of a header opens
 *    its menu inward.
 * 2. **`aria-activedescendant`, not roving focus.** Focus stays on the
 *    `role="menu"` container for the whole interaction and the active row is
 *    described by id. A menu item is a button that the user presses, not a
 *    tab stop; keeping the tab stop on the container means Escape and Tab are
 *    handled in one place and a row can be added without re-wiring focus. It
 *    also keeps pointer hover on the same `activeIndex` path as the keyboard,
 *    so hover and arrows never disagree about which row Enter would run.
 * 3. **Motion is one 90ms opacity + 2px rise, and none at all when the reader
 *    asked for reduced motion.** It is under the 120ms ceiling, it touches
 *    transform and opacity only, and the media-query check is read at open
 *    time rather than assumed. Closing is instant: an exiting panel that lingers
 *    over the row underneath it is a second target, not a polish detail.
 *
 * Non-purpose: no submenus, no checkable items, no typeahead and no portal.
 * A caller that needs those is asking for a different control, and this one
 * stays small enough to audit. Disabled items stay in the list on purpose —
 * an item that is absent reads as "this product cannot do that", which is the
 * wrong answer for an action the current user simply lacks a scope for.
 *
 * Design research (Calendly, Chili Piper, LaunchNotes): the roster pattern
 * that holds up is one action per row, with everything else behind one
 * overflow affordance, never a toolbar of equivalent buttons. That is why
 * alternative providers and message actions come through this menu instead
 * of a second row of controls.
 */
import {
  cloneElement,
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactElement,
  type ReactNode,
  type Ref,
} from "react";
import { cn } from "../lib/cn";

/** What an item's press does. `danger` is the destructive pair, not a second
 * neutral; a removal belongs here so the menu reads the same everywhere. */
export type MenuItemTone = "default" | "danger";

export interface MenuItem {
  /**
   * The row's content. A string renders as plain text; a node is for a row
   * that carries a status or a secondary line. The accessible name of the row
   * is its text content, so put the words in the node, not in a `title`.
   */
  label: ReactNode;
  /** Runs on press, then the menu closes and focus returns to the trigger. */
  onSelect: () => void;
  /** A leading 16px mark. Decorative: the label carries the meaning. */
  icon?: ReactNode;
  /** Rendered, announced as unavailable, skipped by the arrow keys. */
  disabled?: boolean;
  tone?: MenuItemTone;
  /**
   * When set, the row is an anchor rather than a button. A menu row that
   * leaves the product is a link: the browser owns the new tab, so a popup
   * blocker never eats an action the user asked for, and the row can be
   * opened in a new window the way every other vendor link can.
   */
  href?: string;
  /** Stable id for tests and for a caller that reopens onto one row. */
  id?: string;
}

type TriggerAriaProps = {
  ref?: Ref<HTMLElement>;
  onClick?: (event: ReactMouseEvent<HTMLElement>) => void;
  onKeyDown?: (event: ReactKeyboardEvent<HTMLElement>) => void;
  "aria-haspopup"?: "menu";
  "aria-expanded"?: boolean;
  "aria-controls"?: string;
};

export interface MenuProps {
  /** Accessible name of the list, e.g. "RMM providers" or "Message actions". */
  label: string;
  /**
   * The button that opens the menu. It is the caller's element — a `Button`,
   * a caret, an overflow glyph — and Menu clones it to attach the trigger
   * semantics (`aria-haspopup`, `aria-expanded`, `aria-controls`) rather than
   * wrapping it in a second button, which would put two press targets in the
   * tab order for one action.
   */
  trigger: ReactElement;
  items: MenuItem[];
  /** Which trigger edge the panel aligns to. Defaults to the leading edge. */
  align?: "start" | "end";
  className?: string;
  /**
   * Controlled open state. A caller that has a second way in — a capability
   * chip's body, which opens the same list the caret owns — passes this so
   * both affordances drive one panel instead of mounting two menus.
   */
  open?: boolean;
  /** Called after the panel opens or closes, for a caller that mirrors state. */
  onOpenChange?: (open: boolean) => void;
}

/** Shared row geometry so the trigger and the rows line up at every density. */
const ROW_HEIGHT = "var(--m-density-control-sm)";

function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(false);
  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    setReduced(query.matches);
    const onChange = (event: MediaQueryListEvent): void => setReduced(event.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return reduced;
}

export function Menu({
  label,
  trigger,
  items,
  align = "start",
  className,
  open: controlledOpen,
  onOpenChange,
}: MenuProps): React.JSX.Element {
  const [internalOpen, setInternalOpen] = useState(false);
  const open = controlledOpen ?? internalOpen;
  const [activeIndex, setActiveIndex] = useState(-1);
  const [entered, setEntered] = useState(false);
  const reducedMotion = usePrefersReducedMotion();

  const rootRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLElement | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const baseId = useId();
  const panelId = `${baseId}-panel`;

  const setOpenState = useCallback(
    (next: boolean) => {
      if (controlledOpen === undefined) setInternalOpen(next);
      if (!next) setActiveIndex(-1);
      onOpenChange?.(next);
    },
    [controlledOpen, onOpenChange],
  );

  /** First row the keyboard may land on, or -1 when every row is disabled. */
  const firstEnabled = useCallback(
    (from: number, step: 1 | -1): number => {
      if (items.length === 0) return -1;
      let index = from;
      for (let visited = 0; visited < items.length; visited += 1) {
        index = (index + step + items.length) % items.length;
        if (!items[index]?.disabled) return index;
      }
      return -1;
    },
    [items],
  );

  const openMenu = useCallback(
    (initial: "first" | "last" | "none" = "first") => {
      setOpenState(true);
      if (initial === "none") {
        setActiveIndex(-1);
        return;
      }
      setActiveIndex(initial === "last" ? firstEnabled(-1, -1) : firstEnabled(-1, 1));
    },
    [firstEnabled, setOpenState],
  );

  const close = useCallback(
    (restoreFocus: boolean) => {
      setOpenState(false);
      setActiveIndex(-1);
      if (restoreFocus) triggerRef.current?.focus();
    },
    [setOpenState],
  );

  const runItem = useCallback(
    (index: number) => {
      const item = items[index];
      if (item === undefined || item.disabled) return;
      // Close before the handler so a handler that navigates or opens a dialog
      // does not leave this panel mounted behind it.
      close(true);
      item.onSelect();
    },
    [close, items],
  );

  // Entry motion: one frame at the resting offset, then settle. Reduced motion
  // skips the frame entirely; the panel is simply there.
  useEffect(() => {
    if (!open) {
      setEntered(false);
      return;
    }
    if (reducedMotion) {
      setEntered(true);
      return;
    }
    const frame = requestAnimationFrame(() => setEntered(true));
    return () => cancelAnimationFrame(frame);
  }, [open, reducedMotion]);

  // The container is the focus target for the whole interaction.
  useEffect(() => {
    if (open) panelRef.current?.focus();
  }, [open]);

  // A menu opened by a second affordance (a chip body) starts on its first
  // enabled row, exactly as if the caret had opened it.
  useEffect(() => {
    if (!open) return;
    setActiveIndex((current) => {
      if (current >= 0 && items[current]?.disabled !== true) return current;
      return firstEnabled(-1, 1);
    });
  }, [firstEnabled, items, open]);

  // Outside pointerdown closes. Capture phase, because a row's own handler or
  // a stopPropagation in the host surface must not keep a detached menu open.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent): void => {
      const target = event.target;
      if (target instanceof Node && rootRef.current?.contains(target)) return;
      close(false);
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    return () => document.removeEventListener("pointerdown", onPointerDown, true);
  }, [close, open]);

  // Any ancestor scrolling moves the trigger away from the panel. Closing is
  // the honest answer: a menu pinned to coordinates that no longer describe a
  // control is a menu that acts on the wrong thing.
  useEffect(() => {
    if (!open) return;
    const onScroll = (event: Event): void => {
      const target = event.target;
      if (target instanceof Node && panelRef.current?.contains(target)) return;
      close(false);
    };
    document.addEventListener("scroll", onScroll, true);
    return () => document.removeEventListener("scroll", onScroll, true);
  }, [close, open]);

  const onTriggerClick = (event: ReactMouseEvent<HTMLElement>): void => {
    event.preventDefault();
    if (open) close(false);
    else openMenu("first");
  };

  const onTriggerKeyDown = (event: ReactKeyboardEvent<HTMLElement>): void => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      openMenu("first");
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      openMenu("last");
    } else if (event.key === "Escape" && open) {
      event.preventDefault();
      close(true);
    }
  };

  const onPanelKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>): void => {
    switch (event.key) {
      case "ArrowDown": {
        event.preventDefault();
        const next = firstEnabled(activeIndex, 1);
        if (next >= 0) setActiveIndex(next);
        break;
      }
      case "ArrowUp": {
        event.preventDefault();
        const next = firstEnabled(activeIndex === -1 ? 0 : activeIndex, -1);
        if (next >= 0) setActiveIndex(next);
        break;
      }
      case "Home": {
        event.preventDefault();
        const next = firstEnabled(-1, 1);
        if (next >= 0) setActiveIndex(next);
        break;
      }
      case "End": {
        event.preventDefault();
        const next = firstEnabled(-1, -1);
        if (next >= 0) setActiveIndex(next);
        break;
      }
      case "Enter":
      case " ": {
        event.preventDefault();
        runItem(activeIndex);
        break;
      }
      case "Escape": {
        event.preventDefault();
        close(true);
        break;
      }
      case "Tab": {
        // Let the browser move focus; the menu is not a tab cycle.
        close(false);
        break;
      }
      default:
        break;
    }
  };

  const ariaProps: TriggerAriaProps = {
    ref: (node: HTMLElement | null) => {
      triggerRef.current = node;
    },
    onClick: onTriggerClick,
    onKeyDown: onTriggerKeyDown,
    "aria-haspopup": "menu",
    "aria-expanded": open,
    "aria-controls": open ? panelId : undefined,
  };

  // A host-supplied handler is composed, not replaced: a trigger often also
  // carries the surface's own analytics or focus behaviour.
  const child = trigger as ReactElement<TriggerAriaProps>;
  const childOnClick = child.props.onClick;
  const childOnKeyDown = child.props.onKeyDown;
  const triggerNode = cloneElement(child, {
    ...ariaProps,
    onClick: (event: ReactMouseEvent<HTMLElement>) => {
      childOnClick?.(event);
      if (!event.defaultPrevented) onTriggerClick(event);
    },
    onKeyDown: (event: ReactKeyboardEvent<HTMLElement>) => {
      childOnKeyDown?.(event);
      if (!event.defaultPrevented) onTriggerKeyDown(event);
    },
  });

  const activeId =
    activeIndex >= 0 ? `${baseId}-item-${String(activeIndex)}` : undefined;

  return (
    <div
      ref={rootRef}
      className={cn("inline-flex", className)}
      style={{ position: "relative", display: "inline-flex" }}
      data-testid="menu-root"
    >
      {triggerNode}
      {open ? (
        <div
          ref={panelRef}
          id={panelId}
          role="menu"
          aria-label={label}
          aria-activedescendant={activeId}
          tabIndex={-1}
          onKeyDown={onPanelKeyDown}
          data-testid="menu-panel"
          style={{
            position: "absolute",
            top: "calc(100% + 6px)",
            ...(align === "start" ? { left: 0 } : { right: 0 }),
            zIndex: 70,
            minWidth: 184,
            maxWidth: "min(320px, calc(100vw - 32px))",
            padding: 4,
            margin: 0,
            background: "var(--m-overlay-surface)",
            border: "1px solid var(--m-border)",
            borderRadius: "var(--m-radius-container)",
            boxShadow: "var(--m-shadow-overlay)",
            outline: "none",
            opacity: entered ? 1 : 0,
            transform: entered ? "none" : "translateY(-2px)",
            transition: reducedMotion
              ? "none"
              : "opacity 90ms ease-out, transform 90ms ease-out",
          }}
        >
          {items.map((item, index) => {
            const active = index === activeIndex;
            const danger = item.tone === "danger";
            const colour = item.disabled
              ? "var(--m-disabled-fg)"
              : danger
                ? "var(--m-danger)"
                : "var(--m-text)";
            const rowStyle = {
              display: "flex",
              alignItems: "center",
              gap: 8,
              width: "100%",
              minHeight: ROW_HEIGHT,
              height: ROW_HEIGHT,
              padding: "0 9px",
              border: 0,
              borderRadius: "calc(var(--m-radius-container) - 6px)",
              background: active
                ? item.disabled
                  ? "transparent"
                  : "var(--m-canvas)"
                : "transparent",
              color: colour,
              font: "inherit",
              fontSize: 13,
              textAlign: "left",
              whiteSpace: "nowrap",
              textDecoration: "none",
              cursor: item.disabled ? "not-allowed" : "pointer",
              opacity: 1,
            } as const;
            const rowChildren = (
              <>
                {item.icon ? (
                  <span
                    aria-hidden="true"
                    style={{
                      display: "inline-flex",
                      width: 16,
                      height: 16,
                      flex: "0 0 16px",
                      alignItems: "center",
                      justifyContent: "center",
                      color: "inherit",
                    }}
                  >
                    {item.icon}
                  </span>
                ) : null}
                <span style={{ minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }}>
                  {item.label}
                </span>
              </>
            );
            const key = item.id ?? `${baseId}-item-${String(index)}`;
            const itemId = `${baseId}-item-${String(index)}`;
            if (item.href !== undefined) {
              return (
                <a
                  key={key}
                  id={itemId}
                  role="menuitem"
                  tabIndex={-1}
                  href={item.href}
                  target="_blank"
                  rel="noreferrer"
                  aria-disabled={item.disabled === true || undefined}
                  data-active={active || undefined}
                  data-testid="menu-item"
                  onClick={(event) => {
                    if (item.disabled === true) {
                      event.preventDefault();
                      return;
                    }
                    // `onSelect` still runs for a link so the caller can log the
                    // view; the navigation itself belongs to the browser.
                    close(true);
                    item.onSelect();
                  }}
                  onPointerEnter={() => {
                    if (!item.disabled) setActiveIndex(index);
                  }}
                  onPointerLeave={() => setActiveIndex(-1)}
                  style={rowStyle}
                >
                  {rowChildren}
                </a>
              );
            }
            return (
              <button
                key={key}
                id={itemId}
                type="button"
                role="menuitem"
                tabIndex={-1}
                aria-disabled={item.disabled === true || undefined}
                data-active={active || undefined}
                data-testid="menu-item"
                onClick={() => runItem(index)}
                onPointerEnter={() => {
                  if (!item.disabled) setActiveIndex(index);
                }}
                onPointerLeave={() => setActiveIndex(-1)}
                style={rowStyle}
              >
                {rowChildren}
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
