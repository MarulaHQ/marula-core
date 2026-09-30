"use client";
/**
 * SurfaceTabs — the pack's view control for a data surface.
 *
 * Purpose: the pack draws views as TABS, not as a segmented control — the
 * inactive ones are plain text on the canvas and the active one is a raised
 * surface card with a hairline border. The sub-nav beside a page title is
 * the same shape (Dispatch: Board / Planning / Travel). This is that shape,
 * scoped to the Insight surfaces; the shared `ViewSwitch` in @marula-core/ui
 * draws the segmented variant, and changing a primitive ten lanes are
 * building against is not this lane's call.
 *
 * A count rides on the tab the way the pack rides one: quiet text after the
 * label, `tabular-nums` so a changing count does not shuffle the row.
 *
 * Non-purpose: this holds no state and fetches nothing. It is a tablist.
 */

export interface SurfaceTab {
  id: string;
  label: string;
  /** Optional count shown after the label. */
  hint?: string;
  /**
   * Overrides the accessible name where the visible label does not say what
   * activating the tab DOES — "Team load" reads as a noun, "Apply Team load
   * arrangement" reads as an action.
   */
  ariaLabel?: string;
}

export function SurfaceTabs({
  tabs,
  activeId,
  onChange,
  label,
  className,
}: {
  tabs: readonly SurfaceTab[];
  activeId: string;
  onChange: (id: string) => void;
  label: string;
  className?: string;
}): React.JSX.Element {
  return (
    <div
      className={className}
      role="tablist"
      aria-label={label}
      style={{ display: "flex", alignItems: "center", gap: 4, flexWrap: "wrap" }}
    >
      {tabs.map((tab) => {
        const active = tab.id === activeId;
        return (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={active}
            aria-label={tab.ariaLabel}
            onClick={() => onChange(tab.id)}
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: 6,
              height: "var(--m-density-control-sm)",
              padding: "0 12px",
              borderRadius: "var(--m-radius-control)",
              border: `1px solid ${active ? "var(--m-border)" : "transparent"}`,
              background: active ? "var(--m-surface)" : "transparent",
              color: active ? "var(--m-text)" : "var(--m-text-secondary)",
              fontSize: 13,
              fontWeight: active ? 600 : 500,
              whiteSpace: "nowrap",
              cursor: "pointer",
            }}
          >
            {tab.label}
            {tab.hint !== undefined ? (
              <span className="tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                {tab.hint}
              </span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}
