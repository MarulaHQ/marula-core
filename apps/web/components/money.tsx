"use client";
/**
 * Money-cluster chrome — the frame every surface in the cluster wears.
 *
 * Purpose: the pack files six screens under one sub-navigation and gives them
 * one set of parts (source/approvals.html): the header row with its shortcut
 * pills, the strip above a table, the bulk bar, the entry-in-context pane, the
 * pill grammar and the empty state. Written once here, so a wrong colour
 * meaning or a wrong duration cannot be spelled two ways in two routes.
 *
 * Non-purpose: no fetching, no domain decisions. The words and numbers come
 * from lib/money-ui.ts, the amounts from lib/format.ts, and every colour is a
 * --m-* token — the pack's own pairs (--acc, --due, --gsoft, --note) ARE these
 * tokens, so the drawing and the app agree by construction.
 */
import { SurfaceHeader } from "@marula-core/ui";
import { Icon as KitIcon } from "@marula-core/ui/kit";
import Link from "next/link";
import { usePathname } from "next/navigation";
import type { ReactNode } from "react";

import {
  MONEY_NAV,
  hms,
  moneyItemFor,
  type AgeingBucket,
  type MoneyPill,
} from "../lib/money-ui";

// ---------------------------------------------------------------- icons ----

function Icon({ name }: { name: "check" | "clock" | "alert" | "info" }) {
  return <KitIcon name={name} size={14} className="m-money-ico" />;
}

// ---------------------------------------------------------------- header ----

/**
 * The money sub-navigation. It is the cluster's own table of contents, so it
 * renders on every page in the cluster, including a document page like the
 * invoice detail, which stays under Invoices.
 */
export function MoneySubnav(): React.JSX.Element {
  const pathname = usePathname();
  const active = moneyItemFor(pathname);
  return (
    <nav className="m-money-subnav" aria-label="Money">
      {MONEY_NAV.map((item) => (
        <Link
          key={item.id}
          href={item.href}
          aria-current={item.id === active ? "page" : undefined}
        >
          {item.label}
        </Link>
      ))}
    </nav>
  );
}

export interface MoneyStatPillProps {
  label: string;
  count: number;
  /** The dot's token family: gold for a count, bark for a neutral, blush for
   *  the audit. Named as a family so no hex is written here. */
  tone: "gold" | "bark" | "blush";
  pressed?: boolean;
  onClick?: () => void;
  testId?: string;
}

const STAT_DOT: Record<MoneyStatPillProps["tone"], string> = {
  gold: "var(--m-gold)",
  bark: "var(--m-input)",
  blush: "var(--m-status-due-fg)",
};

export function MoneyStatPill({
  label,
  count,
  tone,
  pressed = false,
  onClick,
  testId,
}: MoneyStatPillProps): React.JSX.Element {
  return (
    <button
      type="button"
      className="m-money-stat"
      aria-pressed={pressed}
      onClick={onClick}
      data-testid={testId}
    >
      <span className="m-money-stat-dot" style={{ background: STAT_DOT[tone] }} aria-hidden="true" />
      {label} <span className="m-money-stat-n">{count}</span>
    </button>
  );
}

export interface MoneyHeaderProps {
  title: string;
  /**
   * The money documents sit in one category, and the category's name goes
   * above the page's name — the pack draws it on every quoting list (Q01, C01).
   * Defaults to the cluster's own label; pass `null` to drop it.
   */
  eyebrow?: ReactNode;
  /** Shortcut pills, already counted by the caller. */
  stats?: ReactNode;
  /** Anything that belongs at the far right of the header. */
  actions?: ReactNode;
  /** The sub-navigation defaults on; a document page keeps it. */
  nav?: boolean;
}

export function MoneyHeader({
  title,
  eyebrow = "Billing",
  stats,
  actions,
  nav = true,
}: MoneyHeaderProps): React.JSX.Element {
  // The frame is the shared one (`SurfaceHeader`, the chrome lane's), so the
  // title size is decided in one place for the whole app rather than spelled
  // here a fifth time. The money sub-navigation rides in `meta`, beside the
  // title on its baseline, which is where the pack draws it.
  return (
    <SurfaceHeader
      title={title}
      eyebrow={eyebrow}
      meta={
        <>
          {nav ? <MoneySubnav /> : null}
          {stats}
        </>
      }
      actions={actions}
      className="m-money-head"
    />
  );
}

// ----------------------------------------------------------------- pills ----

/** A pill in one of the four token families a money surface may use. */
export function MoneyPillTag({
  pill,
  testId,
}: {
  pill: MoneyPill;
  testId?: string;
}): React.JSX.Element {
  return (
    <span className={`m-money-pill m-money-pill-${pill.family}`} data-testid={testId}>
      {pill.icon !== undefined ? <Icon name={pill.icon} /> : null}
      {pill.label}
    </span>
  );
}

/**
 * The ageing bucket. Overdue and due-soon are the same blush: a sent invoice
 * running late is a fact about money, and red on this screen would claim a
 * breach that has not happened (pack §7, and the cluster's colour table).
 * A CURRENT invoice earns no pill at all — 21 days out is not a warning, and
 * painting it blush spends the colour before there is anything to say.
 */
export function AgeingPill({
  bucket,
  label,
  testId,
}: {
  bucket: AgeingBucket;
  label: string | null;
  testId?: string;
}): React.JSX.Element | null {
  if (label === null) return null;
  if (bucket !== "overdue" && bucket !== "due-soon") return null;
  const late = bucket === "overdue";
  return (
    <span
      className="m-money-pill m-money-pill-blush"
      data-testid={testId}
      title={late ? "Overdue" : "Due soon"}
    >
      {late ? <Icon name="clock" /> : null}
      {label}
    </span>
  );
}

// ----------------------------------------------------------------- cards ----

/** The cluster's card: a title row, then the body. */
export function MoneyCard({
  title,
  action,
  children,
  className,
  testId,
}: {
  title?: string;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
  testId?: string;
}): React.JSX.Element {
  return (
    <section className={`m-money-card${className !== undefined ? ` ${className}` : ""}`} data-testid={testId}>
      {title !== undefined ? (
        <div className="m-money-pane-head">
          <h2 className="m-money-lab" style={{ margin: 0 }}>
            {title}
          </h2>
          <span className="m-money-bar-sp" />
          {action}
        </div>
      ) : null}
      {children}
    </section>
  );
}

/** One label, one value, value right and bold — the cluster's fact row. */
export function MoneyKv({
  label,
  children,
  testId,
}: {
  label: string;
  children: ReactNode;
  testId?: string;
}): React.JSX.Element {
  return (
    <div className="m-money-kv" data-testid={testId}>
      <span>{label}</span>
      <span>{children}</span>
    </div>
  );
}

/** "What they wrote" — the tech's own words, in the note tint, never quoted back
 *  as a verdict. */
export function MoneySaid({ children }: { children: ReactNode }): React.JSX.Element {
  return (
    <div className="m-money-said">
      <p style={{ margin: 0 }}>{children}</p>
    </div>
  );
}

/** The live note under a money input: what is about to happen, in words. */
export function MoneyNote({ children }: { children: ReactNode }): React.JSX.Element {
  return (
    <p className="m-money-note">
      <Icon name="info" />
      <span>{children}</span>
    </p>
  );
}

// ---------------------------------------------------------------- strips ----

/**
 * The AI strip above a table (pack §6). Three states and no fourth: something
 * flagged, nothing flagged, and the filter that is currently hiding the rest.
 * It says the answer out loud either way, because a clean result that vanishes
 * leaves the reader wondering whether the audit ran.
 */
export function MoneyStrip({
  tone,
  children,
  action,
  testId,
}: {
  tone: "blush" | "green";
  children: ReactNode;
  action?: ReactNode;
  testId?: string;
}): React.JSX.Element {
  return (
    <div
      className={`m-money-strip${tone === "green" ? " m-money-strip-clean" : ""}`}
      role="status"
      data-testid={testId}
    >
      <Icon name={tone === "green" ? "check" : "alert"} />
      <span>{children}</span>
      <span className="m-money-strip-sp" />
      {action}
    </div>
  );
}

/** The P3 mark: words, never a sparkle, and the fix that sits beside it. */
export function VerdictMark({
  label,
  fix,
  onFix,
  testId,
}: {
  label: string;
  fix?: string;
  onFix?: () => void;
  testId?: string;
}): React.JSX.Element {
  return (
    <div className="m-money-verdict">
      <span className="m-money-mark" data-testid={testId}>
        <Icon name="alert" />
        {label}
      </span>
      {fix !== undefined && onFix !== undefined ? (
        <button type="button" className="m-money-fix" onClick={onFix}>
          {fix}
        </button>
      ) : null}
    </div>
  );
}

// ----------------------------------------------------------------- empty ----

/** The fruit, drawn in the pack's own strokes: bark stem, a leaf, two sparks. */
export function FruitMark(): React.JSX.Element {
  return (
    <svg
      width="88"
      height="88"
      viewBox="0 0 72 72"
      fill="none"
      aria-hidden="true"
      data-testid="money-fruit"
    >
      <circle cx="34" cy="42" r="20" stroke="var(--m-bark)" strokeWidth="2.5" />
      <path
        d="M34 22c1-8 8-12 16-10-1 8-8 12-16 10z"
        stroke="var(--m-input)"
        strokeWidth="2.5"
        strokeLinejoin="round"
      />
      <path
        d="M22 12v6M19 15h6M60 30v5M57.5 32.5h5"
        stroke="var(--m-input)"
        strokeWidth="2.5"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function MoneyEmpty({
  title,
  body,
  action,
  testId,
}: {
  title: string;
  body: string;
  action?: ReactNode;
  testId?: string;
}): React.JSX.Element {
  return (
    <div className="m-money-empty" data-testid={testId}>
      <FruitMark />
      <h2>{title}</h2>
      <p>{body}</p>
      {action !== undefined ? <div style={{ display: "flex", gap: 10, marginTop: 10 }}>{action}</div> : null}
    </div>
  );
}

// ----------------------------------------------------------------- misc -----

/** A duration in the pack's H:MM grammar, tabular so a column lines up. */
export function Hms({ minutes }: { minutes: number }): React.JSX.Element {
  return (
    <span style={{ fontVariantNumeric: "tabular-nums" }}>{hms(minutes)}</span>
  );
}

/** The initials disc the person cell carries. */
export function MoneyAvatar({
  initials,
  size = 26,
  own = false,
}: {
  initials: string;
  size?: number;
  own?: boolean;
}): React.JSX.Element {
  return (
    <span
      aria-hidden="true"
      style={{
        width: size,
        height: size,
        borderRadius: 999,
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        fontSize: 11,
        fontWeight: 700,
        letterSpacing: "0.02em",
        flexShrink: 0,
        background: own ? "var(--m-accent)" : "var(--m-surface-2)",
        color: own ? "var(--m-on-accent)" : "var(--m-text-secondary)",
      }}
    >
      {initials}
    </span>
  );
}
