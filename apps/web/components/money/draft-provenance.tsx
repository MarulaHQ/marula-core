"use client";
/**
 * The draft editor's provenance parts: the dots, the assumed marks, the
 * "things to check" list and the notes tab's numbered highlights.
 *
 * Purpose: make "where did this line come from" one implemented idea, used by
 * the quote builder, the contract builder and the change proposal cards. Gold is
 * from the notes, blush is assumed, canopy is the catalogue — and every one of
 * them carries words or a number as well, because colour is never the only
 * signal (docs/theme.md §3.4, pack DESIGN-NOTES-quoting §Rules applied).
 *
 * Non-purpose: it decides nothing. The source of a line is computed by the
 * server (`source`, `source_span`), and this file only draws it.
 */
import { Icon } from "@marula-core/ui/kit";

import type { DraftLine, LineProvenance, MoneySource, SourceSpan } from "../../lib/money-authoring";
import { sliceChars } from "./draft-math";

const SOURCE_WORD: Record<LineProvenance, string> = {
  notes: "From your notes",
  assumed: "Assumed, please check",
  catalogue: "Priced from your catalogue",
};

export function ProvenanceDot({
  source,
  size = 8,
}: {
  source: LineProvenance | undefined;
  size?: number;
}): React.JSX.Element {
  const resolved: LineProvenance = source ?? "assumed";
  return (
    <span
      className="m-dsc-dot"
      data-source={resolved}
      style={size === 8 ? undefined : { width: size, height: size }}
      role="img"
      aria-label={SOURCE_WORD[resolved]}
      title={SOURCE_WORD[resolved]}
    />
  );
}

/**
 * A number that may have been guessed. An assumed number sits on blush and the
 * dot beside the row says the same thing in words, so neither is a lone hint.
 */
export function FieldValue({
  children,
  assumed,
  testId,
}: {
  children: React.ReactNode;
  assumed?: boolean;
  testId?: string;
}): React.JSX.Element {
  if (assumed !== true) return <span data-testid={testId}>{children}</span>;
  return (
    <span className="m-dsc-assumed-value" data-testid={testId} title="Assumed, please check">
      {children}
    </span>
  );
}

/** The legend above the item table. It is also the summary's way in. */
export function ProvenanceLegend({
  checks,
  onOpenChecks,
  withCatalogue,
}: {
  checks: string[];
  onOpenChecks?: () => void;
  withCatalogue?: boolean;
}): React.JSX.Element {
  return (
    <div className="m-dsc-legend">
      <span>
        <ProvenanceDot source="notes" /> From your notes
      </span>
      <span>
        <ProvenanceDot source="assumed" /> Assumed, please check
      </span>
      {withCatalogue === true ? (
        <span>
          <ProvenanceDot source="catalogue" /> From the catalogue
        </span>
      ) : null}
      <span className="m-dsc-sp" />
      {checks.length > 0 ? (
        <button
          type="button"
          className="m-dsc-option"
          onClick={onOpenChecks}
          data-testid="draft-checks-count"
        >
          {checks.length} to check
        </button>
      ) : null}
    </div>
  );
}

/** "Things to check", in the summary column, each sentence from the server. */
export function ThingsToCheck({ checks }: { checks: string[] }): React.JSX.Element | null {
  if (checks.length === 0) return null;
  return (
    <div data-testid="draft-checks">
      <p className="m-dsc-lab" style={{ marginBottom: 6 }}>
        Needs a look, {checks.length}
      </p>
      {checks.map((check) => (
        <div className="m-dsc-check" key={check}>
          <Icon name="alert" size={14} />
          <span>{check}</span>
        </div>
      ))}
    </div>
  );
}

/** One numbered highlight, as it appears in the notes. */
export type NoteHighlight = { number: number; span: SourceSpan };

/**
 * The notes tab: the original words, with each line's span highlighted and
 * numbered to match the mark on that line. Text outside a span stays plain.
 */
export function SourceNotes({
  sources,
  highlights,
}: {
  sources: MoneySource[];
  highlights: NoteHighlight[];
}): React.JSX.Element {
  if (sources.length === 0) {
    return (
      <p className="m-dsc-hint">
        The words are not attached to this draft any more. They stay on screen until you close the
        builder.
      </p>
    );
  }
  return (
    <div className="grid gap-4" data-testid="draft-source-notes">
      {sources.map((source) => (
        <div key={source.id ?? source.name ?? source.text.slice(0, 12)}>
          <p className="m-dsc-lab" style={{ marginBottom: 6 }}>
            {source.name ?? source.kind}
          </p>
          <p className="m-dsc-notes">
            {renderSource(source, highlights.filter((h) => h.span.source_id === source.id))}
          </p>
        </div>
      ))}
    </div>
  );
}

/**
 * The one span renderer: the source text with each span highlighted and
 * numbered. The quote notes tab and the contract notes both draw through it, so
 * overlap and cursor handling cannot drift between them.
 */
export function renderSource(source: MoneySource, spans: NoteHighlight[]): React.ReactNode[] {
  const ordered = [...spans].sort((a, b) => a.span.start - b.span.start);
  const nodes: React.ReactNode[] = [];
  let cursor = 0;
  for (const highlight of ordered) {
    if (highlight.span.start < cursor) continue;
    if (highlight.span.start > cursor) {
      nodes.push(sliceChars(source.text, cursor, highlight.span.start));
    }
    nodes.push(
      <span key={`${highlight.number}-${highlight.span.start}`}>
        <span className="m-dsc-highlight">
          {sliceChars(source.text, highlight.span.start, highlight.span.end)}
        </span>
        <span className="m-dsc-mark-num" aria-label={`Note ${highlight.number}`}>
          {highlight.number}
        </span>
      </span>,
    );
    cursor = highlight.span.end;
  }
  if (cursor < Array.from(source.text).length) {
    nodes.push(sliceChars(source.text, cursor, Array.from(source.text).length));
  }
  return nodes;
}

/** The number a line carries on the notes tab. */
export function LineMark({ number }: { number: number }): React.JSX.Element {
  return (
    <span className="m-dsc-mark-num" aria-label={`Note ${number}`}>
      {number}
    </span>
  );
}

/** Highlights for a quote's lines: one per line that has a located span. */
export function highlightsOf(lines: DraftLine[]): NoteHighlight[] {
  const highlights: NoteHighlight[] = [];
  lines.forEach((line) => {
    if (line.source_span === undefined || line.source_span === null) return;
    highlights.push({ number: highlights.length + 1, span: line.source_span });
  });
  return highlights;
}

/** The map from a line's index to its note number, or null when it has none. */
export function lineNumbers(lines: DraftLine[]): (number | null)[] {
  let next = 0;
  return lines.map((line) => {
    if (line.source_span === undefined || line.source_span === null) return null;
    next += 1;
    return next;
  });
}
