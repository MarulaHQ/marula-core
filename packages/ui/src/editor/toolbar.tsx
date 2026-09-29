"use client";
/**
 * Editor toolbar primitives (ADR 0007, Wave 16 lane 2) — presentation +
 * interaction only: every element renders in the Marula tokens,
 * is keyboard-reachable with focus-visible, and marks interactivity with
 * the semantic tokens. Editor STATE never lives here — the shell drives
 * Tiptap commands and passes `active`/`disabled` down.
 *
 * Non-purpose: no Tiptap imports — primitives stay valid for any headless
 * editor; no palette drafts (theme.md §9) and motion beyond the 120ms law.
 */
import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cn } from "../lib/cn";

/** One slim toolbar row; groups are separated by hairlines via CSS. */
export interface EditorToolbarProps {
  className?: string;
  "aria-label": string;
  children: React.ReactNode;
}

export const EditorToolbar = forwardRef<HTMLDivElement, EditorToolbarProps>(
  ({ className, children, ...props }, ref) => (
    <div
      ref={ref}
      className={cn("m-editor-toolbar", className)}
      role="toolbar"
      aria-orientation="horizontal"
      {...props}
    >
      {children}
    </div>
  ),
);

EditorToolbar.displayName = "EditorToolbar";

/** A group of related tools (renders the separator via CSS). */
export function EditorToolbarGroup({
  className,
  children,
}: {
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div className={cn("m-editor-toolbar-group", className)} role="group">
      {children}
    </div>
  );
}

export interface EditorToolProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** On = the corresponding mark/node is active at the caret. */
  active?: boolean;
  /** Icon-only tools MUST carry a label for WCAG 2.2 AA + a tooltip. */
  label: string;
}

export const EditorTool = forwardRef<HTMLButtonElement, EditorToolProps>(
  ({ className, active, label, ...props }, ref) => (
    <button
      ref={ref}
      type="button"
      className={cn("m-editor-tool", className)}
      aria-pressed={Boolean(active)}
      aria-label={label}
      title={label}
      {...props}
    />
  ),
);

EditorTool.displayName = "EditorTool";
