"use client";
/**
 * Slash / block menu primitive (ADR 0007) — a keyboard-accessible listbox
 * for block insertion: presentation only. Filtering, Tiptap command
 * invocation, and position stay in the shell. Motion rides the shared
 * transition class tokens (120ms, background only).
 */
import { forwardRef, useEffect, useState } from "react";
import { cn } from "../lib/cn";

export interface EditorMenuItem {
  key: string;
  label: string;
  hint?: string;
  icon?: React.ReactNode;
}

export interface EditorMenuProps {
  className?: string;
  "aria-label": string;
  items: EditorMenuItem[];
  /** Fired on click, Enter, or Space at the initial/following cursor. */
  onPick(item: EditorMenuItem): void;
  /** Set from the outside when the menu should not render nothing. */
  children?: React.ReactNode;
}

function filtered(items: EditorMenuItem[], query: string): EditorMenuItem[] {
  if (query.trim() === "") return items;
    return items.filter((item) =>
        item.label.toLowerCase().includes(query.trim().toLowerCase()),
    );
}

function EditorMenuImpl({
  className,
  "aria-label": label,
  items,
  onPick,
  query,
  children,
}: {
  className?: string;
  "aria-label": string;
  items: EditorMenuItem[];
  onPick(item: EditorMenuItem): void;
  /** Optional query (slash menu typing); picks and keydowns apply. */
  query?: string;
  children?: React.ReactNode;
}) {
  const [activeIndex, setActiveIndex] = useState(0);
  const visible = filtered(items, query ?? "");

  useEffect(() => {
    setActiveIndex(0);
  }, [query]);

  if (visible.length === 0) return null;

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActiveIndex((i) => (i + 1) % visible.length);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActiveIndex((i) => (i + visible.length - 1) % visible.length);
    } else if (event.key === "Enter" || event.key === "Tab") {
      event.preventDefault();
      const item = visible[activeIndex];
      if (item) onPick(item);
    }
  };

  return (
    <ul
      className={cn("m-editor-menu", className)}
      role="listbox"
      aria-label={label}
      onKeyDown={onKeyDown}
      tabIndex={-1}
    >
      {visible.map((item, index) => (
        <li key={item.key} role="none">
          <button
            type="button"
            role="option"
            className="m-editor-menu-item"
            data-active={index === activeIndex}
            aria-selected={index === activeIndex}
            onMouseEnter={() => setActiveIndex(index)}
            onClick={() => onPick(item)}
          >
            {item.icon !== undefined ? item.icon : null}
            <span>{item.label}</span>
            {item.hint !== undefined ? (
              <span className="m-editor-menu-hint">{item.hint}</span>
            ) : null}
          </button>
        </li>
      ))}
      {children !== undefined ? <li role="none">{children}</li> : null}
    </ul>
  );
}

/** The slash/block menu. `filtered` is intentionally simple: exact
 * substring matching keeps the runtime light; keyboard affordances
 * remain native listbox keys. */
export const EditorMenu = forwardRef<
  unknown,
  Omit<React.ComponentPropsWithoutRef<typeof EditorMenuImpl>, "ref">
>(function EditorMenu(props, _ref) {
  return EditorMenuImpl(props);
});
