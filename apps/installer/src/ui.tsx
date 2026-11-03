/**
 * Installer-specific compositions of the canonical kit.
 *
 * Purpose: the two things a wizard needs that a general component library does
 * not carry — a value table grouped by the descriptor's own subjects, and the
 * plan rendered as a numbered list with the reason for each step. Both are
 * built from kit primitives (`Card`, `Pill`, `Icon`), so the installer looks
 * like the product rather than like a page that borrowed its colours.
 */

import { Card, CardBody, CardHeader, CardTitle, Icon, Pill } from "@marula-core/ui/kit";

import { transport } from "./api";
import { groupValues, humanLabel } from "./lib/model";
import type { ConfigRow, PlanStep } from "./types";

/**
 * A link to a published docs page.
 *
 * Purpose: the wizard explains a decision in one sentence and the docs page
 * explains it at length. In the native app there is no address bar and no
 * browser chrome, so the click goes through the same transport the review
 * screen uses to open the installed product — which accepts http and https and
 * nothing else. The browser host opens a tab instead, and both keep the
 * fallback of a real href if the transport refuses.
 */
export function DocsLink({ href, children }: { href: string; children: React.ReactNode }) {
  return (
    <a
      href={href}
      rel="noreferrer"
      target="_blank"
      onClick={(event) => {
        event.preventDefault();
        void transport.open(href).catch(() => {
          window.open(href, "_blank", "noopener,noreferrer");
        });
      }}
      className="inline-flex items-center gap-1 text-[12.5px] font-medium text-[var(--mc-gold-fg)] underline decoration-[var(--mc-gold)]/40 underline-offset-2 hover:decoration-[var(--mc-gold)]"
    >
      {children}
      <Icon name="ext" size={12} />
    </a>
  );
}

const SOURCE_LABEL: Record<string, string> = {
  you: "you",
  kept: "kept",
  generated: "generated",
  profile: "profile",
  default: "default",
};

const SOURCE_TONE: Record<string, "gold" | "success" | "neutral"> = {
  you: "gold",
  kept: "success",
  generated: "neutral",
  profile: "neutral",
  default: "neutral",
};

/** Where a value came from, in one word and one colour. */
export function Proof({ source }: { source: string }) {
  return (
    <Pill tone={SOURCE_TONE[source] ?? "neutral"} className="text-[11px] uppercase tracking-[0.04em]">
      {SOURCE_LABEL[source] ?? source}
    </Pill>
  );
}

/**
 * The config, grouped by what it configures. Every row leads with the words a
 * person would use; the descriptor's name sits under it for anyone who wants the
 * exact lever.
 */
export function ValueTable({ rows }: { rows: ConfigRow[] }) {
  const groups = groupValues(rows);
  return (
    <div className="flex flex-col gap-5">
      {groups.map((group) => (
        <div key={group.group} className="flex flex-col gap-1.5">
          <div className="flex items-baseline justify-between gap-3">
            <h3 className="m-0 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">
              {group.title}
            </h3>
            <span className="text-[11px] tabular-nums text-muted-foreground">{group.rows.length}</span>
          </div>
          <div className="overflow-hidden rounded-card border border-border">
            {group.rows.map((row) => (
              <div
                key={row.name}
                className="grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 border-b border-border px-3.5 py-2.5 last:border-b-0"
              >
                <div className="min-w-0">
                  <div className="truncate text-[13px] font-medium">{humanLabel(row.name)}</div>
                  <code className="block truncate font-mono text-[11px] text-muted-foreground" title={row.description}>
                    {row.name}
                  </code>
                </div>
                <code
                  className="max-w-[240px] truncate font-mono text-[12px] tabular-nums"
                  title={row.secret ? "masked" : row.value}
                >
                  {row.value}
                </code>
                <Proof source={row.source} />
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

/** The plan: what runs, why, and the descriptor field behind it. */
export function PlanList({ steps }: { steps: PlanStep[] }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-[14px]">What the installer will do</CardTitle>
      </CardHeader>
      <CardBody className="gap-0">
        {steps.map((step, index) => (
          <div
            key={`${step.headline}-${index}`}
            className="grid grid-cols-[22px_minmax(0,1fr)] gap-3 border-b border-border py-2.5 last:border-b-0"
          >
            <span className="pt-0.5 font-mono text-[11px] tabular-nums text-muted-foreground">
              {String(index + 1).padStart(2, "0")}
            </span>
            <div className="min-w-0">
              <div className="flex items-start gap-2">
                <Icon name={step.kind === "write" ? "file" : "bolt"} size={14} className="mt-0.5 text-muted-foreground" />
                <code className="break-words font-mono text-[11.5px] leading-[1.55]">{step.headline}</code>
              </div>
              <p className="m-0 mt-1 text-[12.5px] leading-[1.5] text-muted-foreground">{step.reason}</p>
              <p className="m-0 mt-1 font-mono text-[10px] text-muted-foreground/80">{step.source}</p>
            </div>
          </div>
        ))}
      </CardBody>
    </Card>
  );
}

/** A section heading with an optional action on the right. */
export function Section({
  title,
  hint,
  action,
  children,
}: {
  title: string;
  hint?: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-end justify-between gap-3">
        <div>
          <h2 className="m-0 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">
            {title}
          </h2>
          {hint ? <p className="m-0 mt-1 text-[12.5px] text-muted-foreground">{hint}</p> : null}
        </div>
        {action}
      </div>
      {children}
    </section>
  );
}
