"use client";
/**
 * ViewSwitch — reimplemented (wave41 lane 20) over the handoff kit's
 * `Segmented`, so every surface that switches between named views draws the
 * same control the mockups do, instead of restyling a local copy.
 *
 * The active view is a pressed segment, not a selected dropdown item. The
 * public API is unchanged.
 */
import { Segmented, SegmentedCount, SegmentedItem } from "../kit";

export interface ViewOption {
  id: string;
  label: string;
  /** Rendered after the label, e.g. a row count. */
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
    <Segmented
      type="single"
      value={activeId}
      onValueChange={(value) => value && onChange(value)}
      aria-label={label}
      className={className}
    >
      {views.map((view) => (
        <SegmentedItem key={view.id} value={view.id}>
          {view.label}
          {view.hint ? <SegmentedCount>{view.hint}</SegmentedCount> : null}
        </SegmentedItem>
      ))}
    </Segmented>
  );
}
