"use client";
/**
 * ViewSwitch — Wave 41 lane 2b, D12: named views, per user.
 *
 * Purpose: switch a surface between the views a user has saved (or the
 * defaults the surface ships). The active view is a pressed segment, not a
 * selected dropdown item: a view switch is a small closed set the user holds
 * in their head, and a dropdown makes them reopen a menu to get back to the
 * view they were just in.
 *
 * Non-purpose: this does not create, rename or delete views. It reports the
 * choice; a surface that offers view management renders that next to it.
 */
import { cn } from "../lib/cn";

export interface ViewOption {
  id: string;
  label: string;
  /** Rendered on the segment when it is active, e.g. a row count. */
  hint?: string;
}

export interface ViewSwitchProps {
  views: ViewOption[];
  activeId: string;
  onChange: (id: string) => void;
  /** The surface's own identity, for the accessible name. */
  label?: string;
  className?: string;
}

export function ViewSwitch({
  views,
  activeId,
  onChange,
  label = "Views",
  className,
}: ViewSwitchProps): React.JSX.Element | null {
  if (views.length < 2) return null;
  return (
    <div
      className={cn("m-view-switch", className)}
      role="tablist"
      aria-label={label}
    >
      {views.map((view) => {
        const active = view.id === activeId;
        return (
          <button
            key={view.id}
            type="button"
            role="tab"
            aria-selected={active}
            className={cn("m-view-switch-item", active && "is-active")}
            onClick={() => onChange(view.id)}
          >
            {view.label}
            {view.hint ? (
              <span className="m-view-switch-hint" aria-hidden="true">
                {view.hint}
              </span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}
