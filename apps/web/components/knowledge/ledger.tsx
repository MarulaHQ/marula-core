// Wave 41 lane 10 (`p41-ui-knowledge`) — the ledger frame every surface in
// this lane copies from the mockup pack's Tickets list.
//
// Purpose: one place for the five things DERIVATION-GUIDE.md line 33 says the
// knowledge cluster copies rather than invents — the 64px page header with its
// attention shortcuts, the one-row control strip, the bordered table card, the
// 52px footer carrying the row count and the density control, and the
// collapsed assistant stub (AI ladder state 2). The numbers are the pack's,
// taken from source/tickets-list.html: `--ph` 64, `--ctl` 44 at comfortable,
// `--row` 48, `--hdr` 44, `.ft2` 52, `.tbl` 16px radius, and the stub's 40px.
//
// Non-purpose: this is presentation. It fetches nothing, holds no filter state
// and knows nothing about knowledge bases, vaults or rules — the landed
// primitives (ViewSwitch, FilterBar, ColumnChooser, ExportControl,
// DensitySwitch, Table, Tag, Badge, Button) stay independent, and a surface
// uses only the ones it needs.
import type { ReactNode } from "react";
import { EmptyState, Tag } from "@marula-core/ui";

/* ---------- icons (the pack's own geometry, 18px, 1.75 stroke) ---------- */

function Glyph({ children, size = 18 }: { children: ReactNode; size?: number }): ReactNode {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      style={{ flexShrink: 0 }}
    >
      {children}
    </svg>
  );
}

export function SparkIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z" />
    </Glyph>
  );
}

export function AlertIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v6M12 16h.01" />
    </Glyph>
  );
}

export function ClockIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <circle cx="12" cy="12" r="8" />
      <path d="M12 8v4l3 2" />
    </Glyph>
  );
}

export function ColumnsIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M9 4v16M15 4v16" />
    </Glyph>
  );
}

export function FunnelIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <path d="M3 5h18l-7 8v6l-4-2v-4z" />
    </Glyph>
  );
}

export function DownloadIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <path d="M12 4v11M7 11l5 5 5-5M5 20h14" />
    </Glyph>
  );
}

export function UploadIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <path d="M12 19V8M7 12l5-5 5 5M5 20h14" />
    </Glyph>
  );
}

export function PlusIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <path d="M12 5v14M5 12h14" />
    </Glyph>
  );
}

export function XIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <path d="M6 6l12 12M18 6L6 18" />
    </Glyph>
  );
}

export function CheckIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <path d="M5 12l5 5L20 7" />
    </Glyph>
  );
}

export function ChevronIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <path d="M9 6l6 6-6 6" />
    </Glyph>
  );
}

export function PenIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <path d="M4 20h4L19 9l-4-4L4 16z" />
    </Glyph>
  );
}

export function LockIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <rect x="5" y="11" width="14" height="9" rx="2" />
      <path d="M8 11V8a4 4 0 0 1 8 0v3" />
    </Glyph>
  );
}

export function FileIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <path d="M6 3h9l4 4v14H6z" />
      <path d="M14 3v5h5M9 13h6M9 17h6" />
    </Glyph>
  );
}

export function BookIcon({ size = 18 }: { size?: number }): ReactNode {
  return (
    <Glyph size={size}>
      <path d="M4 5a2 2 0 0 1 2-2h13v16H6a2 2 0 0 0-2 2z" />
      <path d="M4 19V5" />
    </Glyph>
  );
}

export function SortIcon({ dir }: { dir: "asc" | "desc" }): ReactNode {
  return dir === "asc" ? (
    <Glyph size={14}>
      <path d="M12 19V6M7 11l5-5 5 5" />
    </Glyph>
  ) : (
    <Glyph size={14}>
      <path d="M12 5v13M7 13l5 5 5-5" />
    </Glyph>
  );
}

/* ---------- export ---------- */

/**
 * The pack's small export: exactly the rows and columns on screen, as a real
 * file (DESIGN-NOTES-tickets.md §8). Built in the browser because these
 * surfaces hold records the client already has — the ledger's own rows, in the
 * user's column order — so there is nothing for a server round trip to add.
 */
export function downloadCsv(
  filename: string,
  columns: string[],
  rows: string[][],
): void {
  const escape = (cell: string): string => {
    if (/[",\n]/.test(cell)) return `"${cell.replace(/"/g, '""')}"`;
    return cell;
  };
  const csv = [
    columns.map(escape).join(","),
    ...rows.map((row) => row.map((cell) => escape(cell ?? "")).join(",")),
  ].join("\n");
  const blob = new Blob([`﻿${csv}`], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(url);
}

/* ---------- the frame ---------- */

export type StatTone = "gold" | "blush" | "red" | "neutral";

const STAT_DOT: Record<StatTone, string> = {
  gold: "var(--m-gold)",
  blush: "var(--m-blush)",
  red: "var(--m-destructive)",
  neutral: "var(--m-text-muted)",
};

/**
 * An attention shortcut (DESIGN-NOTES-tickets.md §3): a 32px pill carrying a
 * dot, a label and a tabular count, pressed when the filter it toggles is on.
 * The dot never travels alone — the label says what it counts.
 */
export function StatPill({
  tone = "neutral",
  label,
  count,
  pressed = false,
  onClick,
  disabled,
}: {
  tone?: StatTone;
  label: string;
  count: number;
  pressed?: boolean;
  onClick?: () => void;
  disabled?: boolean;
}): ReactNode {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-pressed={onClick ? pressed : undefined}
      className="inline-flex h-8 shrink-0 items-center gap-2 rounded-full border px-3 text-[13px] font-semibold transition-colors duration-100 disabled:opacity-60"
      style={{
        borderColor: pressed ? "transparent" : "var(--m-border)",
        background: pressed ? "var(--m-accent)" : "transparent",
        color: pressed ? "var(--m-on-accent)" : "var(--m-text)",
      }}
    >
      <span
        aria-hidden="true"
        className="inline-block h-2 w-2 rounded-full"
        style={{ background: STAT_DOT[tone] }}
      />
      {label}
      <span className="tabular-nums">{count.toLocaleString()}</span>
    </button>
  );
}

/** The 64px page header: title, attention shortcuts, then the primary action. */
export function PageHeader({
  title,
  attention,
  actions,
  subtitle,
}: {
  title: string;
  attention?: ReactNode;
  actions?: ReactNode;
  subtitle?: ReactNode;
}): ReactNode {
  return (
    <header className="flex min-h-16 shrink-0 flex-wrap items-center gap-4 pb-2 pt-3">
      <div className="min-w-0">
        <h1
          className="truncate font-semibold tracking-[-0.01em]"
          style={{ fontSize: 26, color: "var(--m-text)" }}
        >
          {title}
        </h1>
        {subtitle ? (
          <p className="mt-0.5 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            {subtitle}
          </p>
        ) : null}
      </div>
      {attention ? (
        <div className="ml-2 flex flex-wrap items-center gap-2" role="group" aria-label="Attention shortcuts">
          {attention}
        </div>
      ) : null}
      <span className="flex-1" />
      {actions ? <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div> : null}
    </header>
  );
}

/** The page body: 24px gutters, full height, canvas background. */
export function PageFrame({ children }: { children: ReactNode }): ReactNode {
  return (
    <div
      className="mx-auto flex h-full w-full max-w-[1600px] flex-col px-6 pb-4"
      style={{ color: "var(--m-text)" }}
    >
      {children}
    </div>
  );
}

/** The bordered table card: 1px border, 16px radius, surface, scrolls alone. */
export function TableCard({
  children,
  className,
  label,
}: {
  children: ReactNode;
  className?: string;
  label?: string;
}): ReactNode {
  return (
    <div
      className={`min-h-0 flex-1 overflow-auto rounded-[16px] ${className ?? ""}`}
      style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)" }}
      aria-label={label}
    >
      {children}
    </div>
  );
}

/** Column header text: 12px, uppercase, .06em, secondary colour. */
export const HEAD_CELL_CLASS =
  "px-3 py-2 text-left text-[12px] font-semibold uppercase tracking-[0.06em]";

/** The 52px footer: shown-of-total on the left, density and key hints right. */
export function LedgerFooter({
  shown,
  total,
  noun,
  density,
  hints,
  trailing,
}: {
  shown: number;
  total: number;
  /** Plural noun for the records, e.g. "articles". */
  noun: string;
  density?: ReactNode;
  hints?: ReactNode;
  trailing?: ReactNode;
}): ReactNode {
  return (
    <div
      className="flex min-h-[52px] shrink-0 flex-wrap items-center gap-3 pt-3 text-[13px]"
      style={{ color: "var(--m-text-secondary)" }}
    >
      <span>
        <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
          {shown.toLocaleString()}
        </b>{" "}
        of{" "}
        <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
          {total.toLocaleString()}
        </b>{" "}
        {noun}
      </span>
      {trailing}
      <span className="flex-1" />
      {hints}
      {density}
    </div>
  );
}

/** A keyboard hint pair, the way the pack's footer prints them. */
export function KeyHint({ keys, label }: { keys: string; label: string }): ReactNode {
  return (
    <span className="inline-flex items-center gap-1.5">
      {keys.split(" ").map((key, i) => (
        <kbd
          key={`${key}-${i}`}
          className="inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-[5px] border px-1 text-[12px] font-semibold leading-none"
          style={{ borderColor: "var(--m-border)" }}
        >
          {key}
        </kbd>
      ))}
      <span>{label}</span>
    </span>
  );
}

/**
 * Selection replaces the column header, in place, so the layout never jumps
 * (DESIGN-NOTES-tickets.md §7). It is a gold WASH, not an inverted surface:
 * the pack's bulk bar is a pale gold field with ordinary surface chips inside
 * it and a gold edge down the left, so the row tint below still reads as the
 * stronger mark. Selected rows carry that same tint plus the gold edge.
 */
export function SelectionBar({
  count,
  noun,
  onClear,
  children,
}: {
  count: number;
  noun: string;
  onClear: () => void;
  children?: ReactNode;
}): ReactNode {
  return (
    <div
      className="sticky top-0 z-[5] flex flex-wrap items-center gap-2 border-b px-3"
      style={{
        height: "var(--m-density-control)",
        background: "var(--m-accent-soft)",
        color: "var(--m-text)",
        borderColor: "var(--m-border)",
        borderLeft: "3px solid var(--m-accent)",
      }}
      role="region"
      aria-label={`${count} ${noun} selected`}
    >
      <span className="tabular-nums font-bold">{count.toLocaleString()}</span>
      <span>selected</span>
      {children}
      <button
        type="button"
        onClick={onClear}
        className="ml-auto inline-flex h-8 items-center rounded-[8px] border px-2.5 text-[13px] font-semibold underline underline-offset-2"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      >
        Clear selection
      </button>
    </div>
  );
}

/** The row style a selected row wears: the gold wash plus the gold edge. */
export const SELECTED_ROW_STYLE = {
  boxShadow: "inset 3px 0 0 var(--m-accent)",
};

/**
 * A status pill the way the pack draws one: a tinted pill, a dot in the same
 * family, and the word. Colour never travels alone — the dot says "this is a
 * status", the label says which.
 */
export function StatusPill({
  tone,
  children,
}: {
  tone: "success" | "warning" | "neutral" | "danger" | "outline";
  children: ReactNode;
}): ReactNode {
  return (
    <Tag tone={tone}>
      <span
        aria-hidden="true"
        className="inline-block h-2 w-2 rounded-full"
        style={{ background: "currentColor" }}
      />
      {children}
    </Tag>
  );
}

/**
 * Loading, as the pack draws it: flat blocks, no shimmer, and a sentence that
 * says what is loading (DESIGN-NOTES shared kit). A spinner alone leaves the
 * reader guessing whether anything is happening.
 */
export function SkeletonRows({
  rows = 8,
  sentence,
  columns = 4,
}: {
  rows?: number;
  sentence: string;
  columns?: number;
}): ReactNode {
  return (
    <div role="status" aria-busy="true" aria-label={sentence}>
      <p className="px-3 py-3 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
        {sentence}
      </p>
      <div className="grid gap-3 px-3 pb-4">
        {Array.from({ length: rows }, (_, i) => (
          <div key={i} className="flex items-center gap-3">
            {Array.from({ length: columns }, (_, c) => (
              <span
                key={c}
                className="h-3 rounded-full"
                style={{
                  background: "var(--m-surface-2)",
                  width: c === 0 ? "26%" : c === 1 ? "34%" : "12%",
                }}
              />
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}

/**
 * "No rows because of what you asked for" — a different sentence from "there is
 * nothing here yet", and the pack draws them differently: a bold title, a line
 * that names the way out, and one primary action.
 *
 * One deviation, on purpose: the pack also shows a secondary "Edit filters".
 * `FilterBar` has no imperative open, so that button could not do anything —
 * and a dead button is worse than a missing one. The Filter control sits in the
 * toolbar directly above, one click away.
 */
export function NoMatches({
  title,
  hint,
  noun,
  onClear,
}: {
  title: string;
  hint: string;
  noun: string;
  onClear: () => void;
}): ReactNode {
  return (
    <EmptyState
      message={`${title} ${hint}`}
      action={
        <button
          type="button"
          onClick={onClear}
          className="inline-flex h-11 items-center rounded-[12px] px-4 text-[13px] font-semibold"
          style={{ background: "var(--m-ink)", color: "var(--m-gold)" }}
        >
          Clear filters
        </button>
      }
    />
  );
}

/**
 * The collapsed assistant stub — AI ladder state 2. In the pack this is a 40px
 * strip down the RIGHT edge of the surface, not a row above it: the spark sits
 * at the top of a narrow column that runs the height of the ledger, so the
 * affordance stays visible without a panel and without taking a row of its
 * own. There is no count when there is nothing to count.
 */
export function AssistantStub({
  count = 0,
  label,
  onOpen,
}: {
  count?: number;
  /** The sentence the spark stands for; it becomes the strip's accessible name. */
  label: string;
  onOpen: () => void;
}): ReactNode {
  return (
    <button
      type="button"
      data-testid="assistant-stub"
      onClick={onOpen}
      aria-label={label}
      title={label}
      className="absolute right-0 top-0 flex h-full w-10 flex-col items-center gap-2 border-l pt-3 transition-colors duration-100"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
    >
      <span style={{ color: "var(--m-gold)" }}>
        <SparkIcon size={16} />
      </span>
      {count > 0 ? (
        <span
          className="tabular-nums inline-flex h-5 min-w-5 items-center justify-center rounded-full px-1 text-[12px] font-bold"
          style={{ background: "var(--m-accent)", color: "var(--m-on-accent)" }}
        >
          {count}
        </span>
      ) : null}
    </button>
  );
}

/**
 * A row mark for an inline verdict (pattern P3): the flag plus the one-click
 * fix, in the row. Never a panel, never an inverted surface.
 */
export function InlineVerdict({
  children,
  onFix,
  fixLabel,
  tone = "blush",
}: {
  children: ReactNode;
  onFix?: () => void;
  fixLabel?: string;
  tone?: "blush" | "green";
}): ReactNode {
  const palette =
    tone === "green"
      ? { background: "var(--m-status-success-bg)", color: "var(--m-status-success-fg)" }
      : { background: "var(--m-status-due-bg)", color: "var(--m-status-due-fg)" };
  return (
    <span
      className="inline-flex items-center gap-2 rounded-full px-2.5 py-1 text-[12px] font-semibold"
      style={palette}
    >
      {tone === "green" ? <CheckIcon size={14} /> : <AlertIcon size={14} />}
      {children}
      {onFix && fixLabel ? (
        <button type="button" onClick={onFix} className="underline underline-offset-2">
          {fixLabel}
        </button>
      ) : null}
    </span>
  );
}

/**
 * The card stack's frame: a step rail above one card. Used by the import
 * wizard, which is a stack rather than a long form — one step a screen, each
 * step resumable.
 */
export function WizardStepRail({
  steps,
  current,
  onStep,
}: {
  steps: { id: string; label: string }[];
  current: number;
  /** Absent while a step is not yet reachable: the step rail only offers
   *  completed steps and the current one. */
  onStep?: (index: number) => void;
}): ReactNode {
  return (
    <ol className="flex flex-wrap items-center gap-2" aria-label="Import steps">
      {steps.map((step, index) => {
        const done = index < current;
        const active = index === current;
        return (
          <li key={step.id} className="flex items-center gap-2">
            <button
              type="button"
              disabled={!done || !onStep}
              onClick={() => onStep?.(index)}
              aria-current={active ? "step" : undefined}
              className="inline-flex h-8 items-center gap-2 rounded-full border px-3 text-[13px] font-semibold transition-colors duration-100 disabled:opacity-70"
              style={{
                borderColor: active ? "transparent" : "var(--m-border)",
                background: active ? "var(--m-accent)" : "var(--m-surface)",
                color: active ? "var(--m-on-accent)" : "var(--m-text-secondary)",
              }}
            >
              {done ? <CheckIcon size={14} /> : <span className="tabular-nums">{index + 1}</span>}
              {step.label}
            </button>
            {index < steps.length - 1 ? <ChevronIcon size={14} /> : null}
          </li>
        );
      })}
    </ol>
  );
}

/**
 * A rule builder step: WHEN / IF / THEN, each a framed block with its own
 * label. The rule reads as a sentence you can argue with.
 */
export function RuleStep({
  word,
  summary,
  children,
  action,
}: {
  word: "WHEN" | "IF" | "THEN";
  summary: ReactNode;
  children?: ReactNode;
  action?: ReactNode;
}): ReactNode {
  return (
    <section
      className="grid gap-2 rounded-[12px] border p-3"
      style={{ borderColor: "var(--m-border)" }}
      aria-label={`${word} ${word === "WHEN" ? "trigger" : word === "IF" ? "conditions" : "actions"}`}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span
          className="rounded-[6px] px-2 py-0.5 text-[12px] font-bold tracking-[0.06em]"
          style={{ background: "var(--m-surface-2)", color: "var(--m-text-secondary)" }}
        >
          {word}
        </span>
        <p className="min-w-0 flex-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {summary}
        </p>
        {action}
      </div>
      {children}
    </section>
  );
}

/**
 * The validation failure the document-type editor shows: blush, an icon and
 * words. Never red — red is a breach (DESIGN-NOTES-tickets.md §9).
 */
export function BlushNote({
  children,
  title,
}: {
  children: ReactNode;
  title?: string;
}): ReactNode {
  return (
    <div
      role="alert"
      className="flex items-start gap-2 rounded-[12px] p-3 text-[13px]"
      style={{ background: "var(--m-status-due-bg)", color: "var(--m-status-due-fg)" }}
    >
      <span style={{ marginTop: 1 }}>
        <AlertIcon size={16} />
      </span>
      <span className="min-w-0">
        {title ? <b className="mr-1">{title}</b> : null}
        {children}
      </span>
    </div>
  );
}

/** A masked value with the reveal control beside it. The value never renders
 *  unmasked without a confirmation first. */
export function MaskedValue({
  masked,
  onReveal,
  revealLabel = "Reveal",
}: {
  masked: string;
  onReveal?: () => void;
  revealLabel?: string;
}): ReactNode {
  return (
    <span className="inline-flex items-center gap-2">
      <code
        className="rounded-[6px] px-1.5 py-0.5 text-[13px]"
        style={{ background: "var(--m-surface-2)", fontFamily: "var(--m-font-mono)" }}
      >
        {masked}
      </code>
      {onReveal ? (
        <button
          type="button"
          onClick={onReveal}
          className="inline-flex items-center gap-1 text-[13px] font-semibold underline underline-offset-2"
        >
          <LockIcon size={14} />
          {revealLabel}
        </button>
      ) : null}
    </span>
  );
}
