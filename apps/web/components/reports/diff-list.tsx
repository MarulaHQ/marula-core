"use client";
/**
 * DiffList — the structural diff between the current draft and the proposal.
 *
 * Composition taken from the review surfaces this lane studied: a change list
 * groups by the part of the report it touches (`view`, `group_by`, `measures`,
 * `filters`, `card`), and each line reads `member: before → after`. The arrows
 * are the reviewers' own vocabulary (Versus-style diff lists, Notion's AI
 * suggestion block), and the op is spelled as a word in the accessible name —
 * `+ add` is a glyph for the eye and "added" for the screen reader.
 *
 * Tokens only: the three ops use the product's status words, never a literal
 * hex, and an op is never signalled by colour alone (each row carries its sign
 * and its word).
 */
import { cn } from "@marula-core/ui";
import {
  diffPathParts,
  diffValue,
  type ReportDiffEntry,
} from "../../lib/report-authoring";

const OP_WORD: Record<ReportDiffEntry["op"], string> = {
  add: "added",
  remove: "removed",
  change: "changed",
};

const OP_SIGN: Record<ReportDiffEntry["op"], string> = {
  add: "+",
  remove: "−",
  change: "~",
};

const SECTION_LABEL: Record<string, string> = {
  view: "Data",
  viz: "Presentation",
  name: "Name",
  group_by: "Grouped by",
  measures: "Measures",
  filters: "Filters",
  card: "Card",
};

function sectionLabel(section: string): string {
  return SECTION_LABEL[section] ?? section.replace(/_/g, " ");
}

/** Ops grouped in the order a reviewer reads a report: data, then shape, then card. */
const SECTION_ORDER = ["name", "view", "viz", "filters", "group_by", "measures", "card"];

export function DiffList({ diff }: { diff: ReportDiffEntry[] }) {
  if (diff.length === 0) return null;
  const sections = new Map<string, ReportDiffEntry[]>();
  for (const entry of diff) {
    const { section } = diffPathParts(entry);
    const bucket = sections.get(section);
    if (bucket === undefined) sections.set(section, [entry]);
    else bucket.push(entry);
  }
  const ordered = [...sections.entries()].sort(
    (a, b) => rank(a[0]) - rank(b[0]) || a[0].localeCompare(b[0]),
  );

  return (
    <section aria-label="Changes against the current draft" className="grid gap-1 py-2">
      <div className="flex items-center justify-between gap-2 px-4 pb-1">
        <h3 className="text-[11px] font-semibold uppercase tracking-[.06em] text-muted-foreground">
          Against the current draft
        </h3>
        <span className="text-[12px] tabular-nums text-muted-foreground">
          {diff.length} change{diff.length === 1 ? "" : "s"}
        </span>
      </div>
      {ordered.map(([section, entries]) => (
        <div key={section} className="grid gap-1 px-4 py-1.5">
          <h4 className="text-[11px] font-semibold uppercase tracking-[.06em] text-muted-foreground">
            {sectionLabel(section)}
          </h4>
          <ul className="grid gap-1">
            {entries.map((entry) => (
              <DiffRow key={`${entry.op}:${entry.path}`} entry={entry} />
            ))}
          </ul>
        </div>
      ))}
    </section>
  );
}

function rank(section: string): number {
  const index = SECTION_ORDER.indexOf(section);
  return index === -1 ? SECTION_ORDER.length : index;
}

function DiffRow({ entry }: { entry: ReportDiffEntry }) {
  const { member } = diffPathParts(entry);
  return (
    <li
      data-op={entry.op}
      className="grid grid-cols-[16px_minmax(0,1fr)] items-baseline gap-2 text-[13px] leading-[1.5]"
    >
      <span
        aria-hidden="true"
        className={cn(
          "text-center font-semibold tabular-nums",
          entry.op === "add" && "text-mc-green-text",
          entry.op === "remove" && "text-mc-attn-fg",
          entry.op === "change" && "text-foreground",
        )}
      >
        {OP_SIGN[entry.op]}
      </span>
      <span className="min-w-0">
        <span className="sr-only">{OP_WORD[entry.op]}: </span>
        <span className="font-medium">{member === "" ? entry.path : member}</span>
        {entry.op === "change" ? (
          <>
            {" "}
            <span className="text-muted-foreground">{diffValue(entry.before)}</span>
            <span aria-hidden="true" className="text-muted-foreground">
              {" → "}
            </span>
            <span className="sr-only"> changed to </span>
            <span>{diffValue(entry.after)}</span>
          </>
        ) : entry.op === "add" ? (
          <span className="text-muted-foreground"> {diffValue(entry.after)}</span>
        ) : (
          <span className="text-muted-foreground line-through">{diffValue(entry.before)}</span>
        )}
      </span>
    </li>
  );
}
