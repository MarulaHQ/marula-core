"use client";
/**
 * RecordFrame — the three-column record skeleton every record-detail page in
 * the app uses (Wave 41 lane 7 `p41-ui-clients`).
 *
 * Purpose: the client record established the shape and the pack reuses it
 * for every other record rather than letting each page invent one. Three
 * columns, one job each:
 *
 * - Left, 290px — **Customer**: who this is. Identity, status, the contract
 *   summary, the people, the places, the owner.
 * - Middle, fluid — **Content**: the work of the record. Breadcrumb and
 *   actions, the title with its pills, the tabs, the tab.
 * - Right, 340px — **Right now**: what needs attention. A plain summary of
 *   cards, never an assistant.
 *
 * The column widths are the pack's, not guesses: the context column is the
 * kit's client-context variant at 290px and the right-hand summary is 340px.
 * Below `lg` both side columns collapse to stacked blocks in source order
 * (customer, content, right now), because a 290px column next to a fluid
 * one is unreadable at tablet width and a horizontal scroller hides the
 * summary the record exists to show.
 *
 * Non-purpose: no fetching, no tabs, no state. It is geometry and the two
 * labelled landmarks, so a record page that renders a different thing in
 * each column still reads as the same page shape.
 */

/** The pack's exact record metrics, lifted from the surface source so the
 * numbers here are the pack's and not approximations:
 *
 * - the grid is `290px minmax(0,1fr) 340px`, full-bleed (it cancels the
 *   page's own 24px gutter, which is why the columns reach the content edges);
 * - the customer column sits on the **canvas**, not on a card, carries a
 *   single hairline on its right, and pads `8px 18px`;
 * - its sections are `14px 0` with a `6px` gap and a hairline between them;
 * - the summary column sits on the **surface**, carries a single hairline on
 *   its left, pads `18px`, and gaps its cards by `14px`;
 * - a card header is `14px 18px`, a card body `14px 18px` with an `8px` gap,
 *   and a table row is `52px` tall with `18px` of side padding.
 */
export interface RecordFrameProps {
  /** The left column's contents: who this record is about. */
  context: React.ReactNode;
  /** The middle column's contents: the work of the record. */
  content: React.ReactNode;
  /** The right column's contents: what needs attention. */
  summary: React.ReactNode;
  className?: string;
}

export function RecordFrame({
  context,
  content,
  summary,
  className,
}: RecordFrameProps): React.JSX.Element {
  return (
    <div
      className={[
        // Stacked below xl with a real gap; from xl up the columns sit flush
        // and are divided by their hairlines, so the gap goes to zero.
        "grid grid-cols-1 gap-4",
        "xl:grid-cols-[290px_minmax(0,1fr)_340px] xl:gap-0",
        // Full bleed: the columns own the page gutter from xl up, exactly as
        // the pack does, which is what makes the hairline read as a division
        // rather than as the edge of a floating box.
        "xl:-mx-6",
        className ?? "",
      ].join(" ")}
      data-testid="record-frame"
    >
      {/* The customer column is a COLUMN, not a card: canvas ground, one
          hairline on the right, no rounded box. Wrapping it in a bordered
          panel is what made the first pass look like a set of empty boxes. */}
      <aside
        aria-label="Customer"
        data-testid="record-context"
        className="min-w-0 xl:max-h-[calc(100vh-11rem)] xl:overflow-y-auto xl:border-r xl:px-[18px] xl:pt-2"
        style={{ borderColor: "var(--m-border)" }}
      >
        {context}
      </aside>

      {/* The middle column keeps its own 24px inset: the pack's card sits 24px
          clear of the customer column's hairline, not against it. */}
      <section
        aria-label="Record"
        data-testid="record-content"
        className="min-w-0 xl:px-6"
      >
        {content}
      </section>

      {/* The summary column is surface ground with a single hairline on its
          left. Its cards provide the framing, so an outer border would frame
          the frames. */}
      <aside
        aria-label="Right now"
        data-testid="record-summary"
        className="flex min-w-0 flex-col gap-3.5 xl:max-h-[calc(100vh-11rem)] xl:overflow-y-auto xl:border-l xl:bg-[var(--m-surface-2)] xl:px-[18px]"
        style={{ borderColor: "var(--m-border)" }}
      >
        {summary}
      </aside>
    </div>
  );
}

/** A labelled section inside a record column. The pack's kit uses these for
 * every block of the context and summary columns, and the label is the same
 * 12px uppercase eyebrow in both. */
export function RecordSection({
  label,
  children,
  className,
}: {
  label: string;
  children: React.ReactNode;
  className?: string;
}): React.JSX.Element {
  return (
    <section
      className={`flex flex-col gap-1.5 border-b py-3.5 first:pt-2 last:border-b-0 last:pb-0 ${className ?? ""}`}
      style={{ borderColor: "var(--m-border)" }}
      data-testid={`record-section-${label.toLowerCase().replaceAll(/\s+/g, "-")}`}
    >
      <p
        className="text-[12px] font-semibold uppercase tracking-[0.08em]"
        style={{ color: "var(--m-text-muted)" }}
      >
        {label}
      </p>
      {children}
    </section>
  );
}

/** A label/value row inside a record column: the key quiet, the value
 * semibold and right-aligned so a column of them lines up. */
export function RecordRow({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <div className="flex items-baseline justify-between gap-3 text-[13px]">
      <span style={{ color: "var(--m-text-muted)" }}>{label}</span>
      <span className="text-right font-semibold">{children}</span>
    </div>
  );
}

/** The right column's card. A plain summary surface: flat surface colour and
 * a 1px border, never an inversion (the one inverted surface on a record is
 * the assistant card, and this column is deliberately not it). */
export function SummaryCard({
  title,
  trailing,
  children,
}: {
  title: string;
  trailing?: React.ReactNode;
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <div
      className="overflow-hidden rounded-2xl border"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      data-testid={`summary-card-${title.toLowerCase().replaceAll(/\s+/g, "-")}`}
    >
      <div
        className="flex items-center gap-2.5 border-b px-[18px] py-3.5"
        style={{ borderColor: "var(--m-border)" }}
      >
        <h3 className="text-[13px] font-semibold">{title}</h3>
        <span className="flex-1" />
        {trailing}
      </div>
      <div className="flex flex-col gap-2 px-[18px] py-3.5">{children}</div>
    </div>
  );
}

/** The tab bar under a record's title. The page header does not change
 * between tabs, so the record never feels like it moved: the tabs sit
 * inside the content column, under a header that stays put.
 *
 * The tabs wrap rather than scroll. A six-tab record inside a fluid middle
 * column overflows at 1440px, and a clipped tab label ("Docum…") reads as a
 * broken surface rather than as "there is more, scroll". Wrapping keeps every
 * tab name whole and visible at any width, which is the whole point of naming
 * them. */
export function RecordTabs({
  tabs,
  active,
  onChange,
}: {
  tabs: { id: string; label: string; count?: number | null }[];
  active: string;
  onChange: (id: string) => void;
}): React.JSX.Element {
  return (
    <nav
      aria-label="Record sections"
      data-testid="record-tabs"
      className="flex flex-wrap gap-x-4 gap-y-1 border-b"
      style={{ borderColor: "var(--m-border)" }}
    >
      {tabs.map((tab) => {
        const isActive = tab.id === active;
        return (
          <button
            key={tab.id}
            type="button"
            onClick={() => onChange(tab.id)}
            aria-current={isActive ? "page" : undefined}
            className="shrink-0 cursor-pointer border-none bg-transparent px-0.5 pb-2 pt-1 text-[13px] font-medium whitespace-nowrap"
            style={{
              color: isActive ? "var(--m-text)" : "var(--m-text-secondary)",
              borderBottom: `2px solid ${isActive ? "var(--m-text)" : "transparent"}`,
              fontWeight: isActive ? 600 : 500,
            }}
            data-testid={`record-tab-${tab.id}`}
          >
            {tab.label}
            {tab.count != null ? (
              <span className="ml-1.5 tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                {tab.count}
              </span>
            ) : null}
          </button>
        );
      })}
    </nav>
  );
}