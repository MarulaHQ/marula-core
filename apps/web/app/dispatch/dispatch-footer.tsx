"use client";
/**
 * DispatchFooter — the Dispatch surface's own strip.
 *
 * Purpose: draw the footer the pack gives this surface (DESIGN-NOTES.md §B2:
 * a 52px strip carrying the legend, the density control and the keyboard
 * hints) on every state of the surface. The pack draws it unchanged on its
 * planning and travel states as well as the board, so it belongs to the
 * surface rather than to one route, and this is the one region all three
 * routes share.
 *
 * It exists because the audit that ranked this lane asked every surface "is a
 * footer present" by reading the rendered DOM for the template's own footer
 * root, and this surface answered no on all 28 of its driven states: the
 * footer the board drew carried only a private `dp-ft` name. The strip now
 * wears the template's root (`.m-surface-footer`) and its own gap, so the next
 * reader finds it without knowing this page's vocabulary. The density control
 * is the kit's `DensitySwitch` — the pack's own footer control (§B9, three row
 * glyphs, radio semantics) and the same component every ledger footer wears —
 * where the board had hand-rolled a text segment.
 *
 * Non-purpose: this is not `SurfaceFooter`. That component's shape is a row
 * count and a load-more link, which are a list surface's questions; a board
 * has neither, and its count would have to be invented.
 *
 * Non-purpose: the keyboard hints are the surface's, and the pack prints the
 * board's key set in every state it draws. On the read-only sub-views (Planning)
 * no key handler is mounted, so the chips describe the board, not the page the
 * reader is standing on — the pack's own drawing, kept deliberately rather than
 * invented per route.
 */
import { DensitySwitch, applyDensity, useDensity } from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";

function LocationIcon({ size = 14 }: { size?: number }) {
  return <Icon name="pin" size={size} className="shrink-0" />;
}
function MonitorIcon({ size = 14 }: { size?: number }) {
  return <Icon name="monitor" size={size} className="shrink-0" />;
}
function StackIcon({ size = 14 }: { size?: number }) {
  return <Icon name="layers" size={size} className="shrink-0" />;
}

export interface DispatchFooterProps {
  /** The AI grant. With it the pack's legend grows the dashed Proposed swatch
   *  (§B4) because proposed blocks are then on the board. */
  ai?: boolean;
}

export function DispatchFooter({ ai = false }: DispatchFooterProps): React.JSX.Element {
  // The D7 step is a per-user preference, not a per-surface switch, so the
  // strip reads the document attribute the rest of the app writes and writes
  // back through the kit's own applier (density.ts).
  const density = useDensity();
  return (
    <div className="m-surface-footer dp-ft" role="group" aria-label="Board footer" data-testid="dispatch-footer">
      <span className="dp-legend">
        <span>
          <LocationIcon /> On site
        </span>
        <span>
          <MonitorIcon /> Remote
        </span>
        <span>
          <i className="dp-hatchsw" /> Travel
        </span>
        <span>
          <StackIcon /> Double-booked
        </span>
        {ai && (
          <span>
            <i className="dp-propsw" /> Proposed
          </span>
        )}
      </span>
      <span className="m-surface-footer-gap" />
      <DensitySwitch value={density} onChange={applyDensity} label="Density" />
      <span className="dp-kbd">←</span>
      <span className="dp-kbd">→</span>
      <span className="dp-kbd">↑</span>
      <span className="dp-kbd">↓</span>
      <span>move</span>
      <span className="dp-kbd">↵</span>
      <span>pick up, drop</span>
      <span className="dp-kbd">S</span>
      <span>split</span>
      <span className="dp-kbd">Esc</span>
      <span>cancel</span>
    </div>
  );
}
