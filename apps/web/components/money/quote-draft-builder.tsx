"use client";
/**
 * The quote builder's draft-aware parts: the item table with provenance, the
 * context column (Summary, Describe, Notes), and the change proposal card.
 *
 * Purpose: Q09–Q11 are one screen — an ordinary quote editor whose only
 * difference is that every line says where it came from, the summary says what
 * to check, and a change can be described rather than clicked. The table, the
 * marks and the proposal card are here so the route keeps its create logic and
 * this file keeps the drawing.
 *
 * Non-purpose: no state of its own beyond the tab and the change composer. The
 * route owns the lines and the totals; the server owns the proposal.
 */
import { AssistantCard, Button, Kbd } from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";

import type {
  ChangeOperation,
  LineProvenance,
  MoneySource,
  SourceSpan,
  Understanding,
} from "../../lib/money-authoring";
import { formatMoneyMinor, lineAmountMinor, sectionsOf, sliceChars } from "./draft-math";
import { FieldValue, LineMark, ProvenanceDot, ThingsToCheck } from "./draft-provenance";

export type EditableLine = {
  description: string;
  quantity_milli: number;
  unit_price_minor: number;
  unit?: string | null;
  catalog_item_id?: string | null;
  taxable?: boolean;
  optional?: boolean;
  section?: string | null;
  source?: LineProvenance;
  source_span?: SourceSpan | null;
  quantity_assumed?: boolean;
  price_assumed?: boolean;
};

export type BuilderTab = "summary" | "describe" | "notes";

/**
 * The item table. One row per line, grouped by section, with the provenance dot
 * on the left, assumed numbers on blush, and the note number that matches the
 * highlight on the Notes tab.
 */
export function QuoteItemsTable({
  lines,
  currency,
  onChange,
  showProvenance,
  noteNumbers,
  proposed,
  editable,
  onRemove,
  testIdPrefix,
}: {
  lines: EditableLine[];
  currency: string;
  onChange: (index: number, patch: Partial<EditableLine>) => void;
  onRemove?: (index: number) => void;
  showProvenance: boolean;
  noteNumbers: (number | null)[];
  proposed: Set<number>;
  editable: boolean;
  testIdPrefix: string;
}): React.JSX.Element {
  const sections = sectionsOf(lines);
  return (
    <table className="m-dsc-items" data-testid={`${testIdPrefix}-items`}>
      <thead>
        <tr>
          <th>Item</th>
          <th className="m-dsc-num">Qty</th>
          <th className="m-dsc-num">Unit price</th>
          <th className="m-dsc-num">Total</th>
          {editable ? <th aria-label="Remove" /> : null}
        </tr>
      </thead>
      <tbody>
        {sections.map((section) => {
          const subtotal = section.lines.reduce(
            (sum, { line }) => sum + lineAmountMinor(line.quantity_milli, line.unit_price_minor),
            0,
          );
          return (
            <SectionRows
              key={section.title}
              title={section.title}
              count={section.lines.length}
              subtotal={subtotal}
              currency={currency}
              optionalOnly={section.lines.every(({ line }) => line.optional === true)}
            >
              {section.lines.map(({ line, index }) => {
                const note = noteNumbers[index] ?? null;
                return (
                  <tr
                    key={`${index}-${line.description}`}
                    data-optional={line.optional === true ? "true" : "false"}
                    data-proposed={proposed.has(index) ? "true" : "false"}
                    data-testid={`${testIdPrefix}-line-${index}`}
                  >
                    <td>
                      <span className="m-dsc-item-name">
                        {showProvenance ? <ProvenanceDot source={line.source} /> : null}
                        {editable ? (
                          <>
                            <label className="sr-only" htmlFor={`${testIdPrefix}-desc-${index}`}>
                              Description for line {index + 1}
                            </label>
                            <input
                              id={`${testIdPrefix}-desc-${index}`}
                              className="m-input"
                              style={{ minWidth: 0, flex: 1 }}
                              value={line.description}
                              placeholder="What is being sold"
                              onChange={(event) => onChange(index, { description: event.target.value })}
                            />
                          </>
                        ) : (
                          <span className="min-w-0">{line.description}</span>
                        )}
                        {line.unit !== null && line.unit !== undefined && line.unit !== "" ? (
                          <span className="m-dsc-item-detail">· {line.unit}</span>
                        ) : null}
                        {showProvenance && note !== null ? <LineMark number={note} /> : null}
                        {line.optional === true ? (
                          <span className="m-dsc-pill m-dsc-pill-muted">Optional</span>
                        ) : null}
                        {proposed.has(index) ? (
                          <span className="m-dsc-proposed-tag">
                            <Icon name="spark" size={12} /> Proposed
                          </span>
                        ) : null}
                      </span>
                    </td>
                    <td className="m-dsc-num">
                      {editable ? (
                        <input
                          className="m-input m-dsc-num"
                          style={{ width: 74 }}
                          inputMode="decimal"
                          aria-label={`Quantity for line ${index + 1}`}
                          value={String(line.quantity_milli / 1000)}
                          onChange={(event) => {
                            const value = Number(event.target.value);
                            onChange(index, {
                              quantity_milli: Number.isNaN(value) ? 0 : Math.round(value * 1000),
                              quantity_assumed: false,
                            });
                          }}
                        />
                      ) : (
                        <FieldValue assumed={line.quantity_assumed}>
                          {line.quantity_milli / 1000}
                        </FieldValue>
                      )}
                    </td>
                    <td className="m-dsc-num">
                      {editable ? (
                        <input
                          className="m-input m-dsc-num"
                          style={{ width: 96 }}
                          inputMode="decimal"
                          aria-label={`Unit price for line ${index + 1}`}
                          value={(line.unit_price_minor / 100).toFixed(2)}
                          onChange={(event) => {
                            const value = Number(event.target.value);
                            onChange(index, {
                              unit_price_minor: Number.isNaN(value) ? 0 : Math.round(value * 100),
                              price_assumed: false,
                            });
                          }}
                        />
                      ) : (
                        <FieldValue assumed={line.price_assumed}>
                          {formatMoneyMinor(line.unit_price_minor, currency)}
                        </FieldValue>
                      )}
                    </td>
                    <td className="m-dsc-num">
                      <b>{formatMoneyMinor(lineAmountMinor(line.quantity_milli, line.unit_price_minor), currency)}</b>
                    </td>
                    {editable ? (
                      <td>
                        <button
                          type="button"
                          className="m-dsc-option"
                          aria-label={`Remove line ${index + 1}`}
                          onClick={() => onRemove?.(index)}
                        >
                          <Icon name="x" size={14} />
                        </button>
                      </td>
                    ) : null}
                  </tr>
                );
              })}
            </SectionRows>
          );
        })}
      </tbody>
    </table>
  );
}

function SectionRows({
  title,
  count,
  subtotal,
  currency,
  optionalOnly,
  children,
}: {
  title: string;
  count: number;
  subtotal: number;
  currency: string;
  optionalOnly: boolean;
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <>
      <tr className="m-dsc-section-row">
        <td colSpan={5}>
          {title}
          <span>
            {" "}
            · {count} {count === 1 ? "item" : "items"} · {formatMoneyMinor(subtotal, currency)}
            {optionalOnly ? ", not in the total" : ""}
          </span>
        </td>
      </tr>
      {children}
    </>
  );
}

/** The context column: the same three tabs on a quote and on a change. */
export function QuoteContextColumn({
  tab,
  onTab,
  showTabs,
  summary,
  notes,
  describe,
}: {
  tab: BuilderTab;
  onTab: (tab: BuilderTab) => void;
  showTabs: boolean;
  summary: React.ReactNode;
  notes: React.ReactNode;
  describe: React.ReactNode;
}): React.JSX.Element {
  return (
    <aside className="m-dsc-context" aria-label={showTabs ? "Draft context" : "Summary"}>
      {showTabs ? (
        <div
          role="tablist"
          aria-label="Draft context"
          className="flex gap-5 border-b px-[18px]"
          style={{ borderColor: "var(--m-border)" }}
        >
          {(
            [
              ["summary", "Summary"],
              ["describe", "Describe"],
              ["notes", "Notes"],
            ] as [BuilderTab, string][]
          ).map(([id, label]) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={tab === id}
              onClick={() => onTab(id)}
              data-testid={`draft-tab-${id}`}
              className="flex items-center gap-1.5 py-2.5 text-[14px]"
              style={{
                color: tab === id ? "var(--m-text)" : "var(--m-text-secondary)",
                fontWeight: tab === id ? 600 : 500,
                borderBottom:
                  tab === id ? "2px solid var(--m-primary)" : "2px solid transparent",
                marginBottom: -1,
              }}
            >
              {id === "describe" ? <Icon name="spark" size={14} /> : null}
              {label}
            </button>
          ))}
        </div>
      ) : (
        <div className="m-dsc-panel-head">
          <h3>Summary</h3>
        </div>
      )}
      <div className="m-dsc-context-body">
        {tab === "summary" ? summary : null}
        {tab === "notes" ? notes : null}
        {tab === "describe" ? describe : null}
      </div>
    </aside>
  );
}

/** The summary column's own blocks, in the pack's order. */
export function QuoteSummary({
  clientName,
  totalMinor,
  optionalMinor,
  currency,
  validForDays,
  checks,
  budgetMinor,
  understanding,
}: {
  clientName: string | null;
  totalMinor: number;
  optionalMinor: number;
  currency: string;
  validForDays: number | null;
  checks: string[];
  budgetMinor: number | null;
  understanding: Understanding | null;
}): React.JSX.Element {
  const approver = understanding?.questions.find((question) =>
    /approv/i.test(question.text),
  );
  return (
    <>
      <div>
        <p className="m-dsc-lab">Client</p>
        <div className="m-dsc-kv">
          <span>Client</span>
          <span>{clientName ?? "Choose it before sending"}</span>
        </div>
        {approver !== undefined ? (
          <div className="m-dsc-kv">
            <span>Approver</span>
            <span>{approver.chosen ?? approver.options[0]}</span>
          </div>
        ) : null}
      </div>
      <div>
        <p className="m-dsc-lab">Total</p>
        <div className="m-dsc-total" data-testid="draft-total">
          {formatMoneyMinor(totalMinor, currency)}
        </div>
        {optionalMinor > 0 ? (
          <p className="m-dsc-hint" style={{ marginTop: 4 }}>
            Optional extras of {formatMoneyMinor(optionalMinor, currency)} are not included.
          </p>
        ) : null}
        {budgetMinor !== null && budgetMinor > 0 && totalMinor > budgetMinor ? (
          <p
            className="m-dsc-failure"
            style={{ marginTop: 8, fontSize: 13 }}
            data-testid="draft-budget-over"
          >
            <Icon name="alert" size={15} />
            <span>
              {formatMoneyMinor(totalMinor - budgetMinor, currency)} over the budget in your notes (about{" "}
              {formatMoneyMinor(budgetMinor, currency)}).
            </span>
          </p>
        ) : null}
      </div>
      <div>
        <p className="m-dsc-lab">Payment and timing</p>
        <div className="m-dsc-kv">
          <span>Valid for</span>
          <span>{validForDays === null ? "30 days from sending" : `${validForDays} days from sending`}</span>
        </div>
      </div>
      <ThingsToCheck checks={checks} />
    </>
  );
}

/** The Notes tab: the words, with the numbered highlights that match the rows. */
export function QuoteNotesTab({
  sources,
  lines,
  noteNumbers,
  emptyHint,
}: {
  sources: MoneySource[];
  lines: EditableLine[];
  noteNumbers: (number | null)[];
  emptyHint: string;
}): React.JSX.Element {
  const highlights = lines
    .map((line, index) => ({ line, index, number: noteNumbers[index] }))
    .filter((row) => row.number !== null && row.line.source_span != null)
    .map((row) => ({ number: row.number as number, span: row.line.source_span as SourceSpan }));

  // A constraint that became no line of its own lives in the words, not in the
  // item table; the notes tab quotes it so nothing silently disappeared.
  const constraint = sources
    .flatMap((source) => source.text.split(/[.\n]/))
    .map((sentence) => sentence.trim())
    .find((sentence) => /no downtime|zero downtime|stays as is|not included|exclud/i.test(sentence));

  if (sources.length === 0) {
    return <p className="m-dsc-hint">{emptyHint}</p>;
  }
  return (
    <div className="grid gap-4" data-testid="quote-source-notes">
      <div className="m-dsc-chips">
        {sources.map((source) => (
          <span className="m-dsc-chip m-dsc-chip-neutral" key={source.id ?? source.name}>
            <Icon name={source.kind === "transcript" ? "mic" : "file"} size={13} />
            {source.name ?? source.kind}
          </span>
        ))}
      </div>
      {sources.map((source) => (
        <div key={source.id ?? source.name}>
          <p className="m-dsc-lab" style={{ marginBottom: 6 }}>
            {source.name ?? source.kind}
          </p>
          <p className="m-dsc-notes">
            {renderHighlights(
              source,
              highlights.filter((highlight) => highlight.span.source_id === source.id),
            )}
          </p>
        </div>
      ))}
      <div>
        <p className="m-dsc-lab" style={{ marginBottom: 6 }}>
          Not turned into a line
        </p>
        <p className="m-dsc-hint">
          {constraint === undefined
            ? "Nothing was left out. Every sentence either became a line or was kept as a term."
            : `A constraint with no line of its own was kept as a term: “${constraint}”.`}
        </p>
      </div>
    </div>
  );
}

function renderHighlights(source: MoneySource, spans: { number: number; span: SourceSpan }[]): React.ReactNode[] {
  const ordered = [...spans].sort((a, b) => a.span.start - b.span.start);
  const nodes: React.ReactNode[] = [];
  let cursor = 0;
  for (const highlight of ordered) {
    if (highlight.span.start < cursor) continue;
    if (highlight.span.start > cursor) nodes.push(sliceChars(source.text, cursor, highlight.span.start));
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
  const length = Array.from(source.text).length;
  if (cursor < length) nodes.push(sliceChars(source.text, cursor, length));
  return nodes;
}

/**
 * The Describe tab (Q11): what the person asked for, and what the server would
 * do about it. Nothing changes until Apply, and the rows it touches are marked
 * on the left whether or not the card is read.
 */
export function ChangeProposalCard({
  instruction,
  sentence,
  operations,
  resultingTotalMinor,
  currency,
  busy,
  error,
  onInstruction,
  onSubmit,
  onApply,
  onDismiss,
}: {
  instruction: string;
  sentence: string | null;
  operations: ChangeOperation[];
  resultingTotalMinor: number | null;
  currency: string;
  busy: boolean;
  error: string | null;
  onInstruction: (value: string) => void;
  onSubmit: () => void;
  onApply: () => void;
  onDismiss: () => void;
}): React.JSX.Element {
  return (
    <div className="flex flex-col gap-3" data-testid="change-proposal">
      {instruction.trim() !== "" ? (
        <p
          className="self-end rounded-[14px_14px_4px_14px] px-3 py-2 text-[13.5px]"
          style={{ background: "var(--m-surface-2)", maxWidth: 320, lineHeight: 1.45 }}
        >
          {instruction}
        </p>
      ) : null}

      {operations.length > 0 ? (
        <AssistantCard
          eyebrow="Proposed change"
          heading={sentence ?? "Here is what I would change"}
          action={
            <Button variant="assistant" onClick={onApply} data-testid="change-apply">
              <Icon name="check" size={15} /> Apply {operations.length} change
              {operations.length === 1 ? "" : "s"}
            </Button>
          }
          secondaryActions={
            <div className="flex gap-2">
              <Button variant="assistant-outline" onClick={onDismiss}>
                Not this
              </Button>
            </div>
          }
          footer={
            <p className="m-dsc-hint" style={{ color: "inherit", opacity: 0.85 }}>
              The rows it touches are marked on the left. Nothing changes until you apply.
              {resultingTotalMinor !== null
                ? ` The new total would be ${formatMoneyMinor(resultingTotalMinor, currency)}.`
                : ""}
            </p>
          }
        >
          <div className="flex flex-col gap-2">
            {operations.map((operation) => (
              <div className="flex gap-2" key={`${operation.path}-${operation.summary}`}>
                <span className="mt-0.5 flex size-[18px] shrink-0 items-center justify-center rounded-full border">
                  <Icon name="plus" size={12} />
                </span>
                <span>{operation.summary}</span>
              </div>
            ))}
          </div>
        </AssistantCard>
      ) : null}

      {error !== null ? (
        <p className="m-dsc-hint" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      ) : null}

      <div className="m-dsc-bar">
        <Icon name="spark" size={18} />
        <label className="sr-only" htmlFor="change-composer">
          Describe a change
        </label>
        <input
          id="change-composer"
          value={instruction}
          placeholder="Describe a change"
          onChange={(event) => onInstruction(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              onSubmit();
            }
          }}
          data-testid="change-instruction"
        />
        <Kbd>D</Kbd>
      </div>
      <Button
        variant="secondary"
        size="sm"
        onClick={onSubmit}
        disabled={busy || instruction.trim() === ""}
        data-testid="change-submit"
      >
        {busy ? "Reading the change…" : "Propose the change"}
      </Button>
    </div>
  );
}
