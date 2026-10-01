// The portal's component set: the shapes its pages are built from.
//
// Purpose: the portal is a narrow surface — a column of cards, a list of
// plain-worded rows, a few forms — so its kit is small on purpose. Everything
// here hangs off a class in app/portal/portal.css, which carries the pack's
// real px values (48px primary, 40px secondary, 16px cards, 26px titles).
//
// THE FOUR RULES THIS KIT ENFORCES, so no page can forget them:
// 1. No red on this surface. `StatusPill` has no red variant to reach for, and
//    `FieldError` is blush. Red means a genuine breach and the client is never
//    shown one, because a breach is not hers to fix.
// 2. Colour is never alone. Every pill pairs its colour with the word, and the
//    two that mean "you act" also carry a clock icon.
// 3. Gold is where you act: "Waiting for you" and "Waiting for your answer".
// 4. An error always says what to do next, never just that something is wrong.
"use client";

import { Icon, type IconName } from "@marula-core/ui/kit";
import {
  CLIENT_STATUS,
  clientStatusFor,
  type ClientStatus,
  type PortalTicketCategory,
} from "../../lib/portal-copy";

/** The icons the portal needs. Stroke-based, 1.75px, sized in px. */
export function PortalIcon({ name, size = 18 }: { name: string; size?: number }) {
  // Wave41 lane 20: the portal draws from the kit's one icon door.
  const MAP: Record<string, IconName> = {
    clock: "clock",
    check: "check",
    plus: "plus",
    down: "down",
    back: "back",
    chev: "chev",
    x: "x",
    download: "download",
    info: "info",
    alert: "alert",
    search: "search",
    mail: "mail",
    file: "file",
  };
  return <Icon name={MAP[name] ?? "info"} size={size} />;
}

/** A pill in one of the client's five words. Never red. */
export function StatusPill({
  status,
  note,
}: {
  status: ClientStatus;
  /** A longer form for where the number matters: "Overdue 12 days". */
  note?: string;
}) {
  const word = CLIENT_STATUS[status];
  // Colour is never alone, so every pill carries a mark whose KIND carries the
  // meaning: a dot for the informational words, a clock for the two that need
  // her, a check for done. See portal/sand/states/02-tickets.png.
  if (status === "waiting" || status === "scheduled") {
    return (
      <span className={`portal-pill portal-pill--${status}`}>
        {status === "waiting" ? <PortalIcon name="clock" size={14} /> : <i className="portal-pill__dot" />}
        {note ?? word.label}
      </span>
    );
  }
  if (status === "resolved") {
    return (
      <span className="portal-pill portal-pill--resolved">
        <PortalIcon name="check" size={14} />
        {note ?? word.label}
      </span>
    );
  }
  return (
    <span className={`portal-pill portal-pill--${status}`}>
      <i className="portal-pill__dot" />
      {note ?? word.label}
    </span>
  );
}

/** Overdue is blush with the number of days. Never red, never bare. */
export function OverduePill({ days }: { days: number }) {
  return (
    <span className="portal-pill portal-pill--overdue">
      <PortalIcon name="clock" size={14} />
      Overdue {days} {days === 1 ? "day" : "days"}
    </span>
  );
}

export function PaidPill() {
  return (
    <span className="portal-pill portal-pill--paid">
      <PortalIcon name="check" size={14} />
      Paid
    </span>
  );
}

/** Confirmed: a person said yes, so it is green and checked. */
export function ConfirmedPill() {
  return (
    <span className="portal-pill portal-pill--resolved">
      <PortalIcon name="check" size={14} />
      Confirmed
    </span>
  );
}

/** Waiting on the service desk to confirm a requested visit. Neutral, not a
 *  failure: the client asked, a person has to answer. */
export function RequestedPill() {
  return (
    <span className="portal-pill portal-pill--received">
      <PortalIcon name="clock" size={14} />
      Waiting to be confirmed
    </span>
  );
}

/** A card: the portal's only container. */
export function PortalCard({
  title,
  aside,
  children,
  className,
  testId,
}: {
  title?: string;
  aside?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  testId?: string;
}) {
  return (
    <section className={`portal-card${className ? ` ${className}` : ""}`} data-testid={testId}>
      {title !== undefined && (
        <div className="portal-card__hd">
          <h2>{title}</h2>
          {aside !== undefined && aside}
        </div>
      )}
      {children}
    </section>
  );
}

/** A field with a label, a hint, and an error that is always blush + words. */
export function Field({
  label,
  htmlFor,
  hint,
  error,
  children,
}: {
  label: string;
  htmlFor: string;
  hint?: string;
  error?: string | null;
  children: React.ReactNode;
}) {
  return (
    <div className={`portal-field${error ? " portal-field--error" : ""}`}>
      <label className="portal-field__label" htmlFor={htmlFor}>
        {label}
      </label>
      {children}
      {error ? (
        <p className="portal-field__error" role="alert" id={`${htmlFor}-error`}>
          <PortalIcon name="alert" size={14} />
          <span>{error}</span>
        </p>
      ) : hint !== undefined ? (
        <p className="portal-field__hint" id={`${htmlFor}-hint`}>
          {hint}
        </p>
      ) : null}
    </div>
  );
}

/** A quiet note block: an icon and one sentence that says what happens next. */
export function Note({ children }: { children: React.ReactNode }) {
  return (
    <p className="portal-note">
      <PortalIcon name="info" size={16} />
      <span>{children}</span>
    </p>
  );
}

/** The empty shape the pack draws: a drawing, one sentence, one action. */
export function PortalEmpty({
  title,
  children,
  action,
}: {
  title: string;
  children: React.ReactNode;
  action?: React.ReactNode;
}) {
  return (
    <div className="portal-empty">
      <MarulaDoodle />
      <h2>{title}</h2>
      <p>{children}</p>
      {action}
    </div>
  );
}

/**
 * The brand's line doodle: a round fruit with a leaf and two sparkles.
 *
 * It is the SAME drawing on the portal's empty states
 * (`portal/sand/states/11-empty-tickets.png`) and on the auth family's
 * "check your inbox" screens (`auth/neutral/states/04-magic-link.png`), which
 * is why it lives here and is drawn once. Deliberately a LINE drawing with no
 * fill: it sits on a cream canvas in the portal and on white cards in auth,
 * and a filled fruit would carry a colour from one surface into the other.
 */
export function MarulaDoodle() {
  return (
    <svg
      width="72"
      height="72"
      viewBox="0 0 64 64"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
      style={{ color: "var(--m-accent-text)" }}
    >
      <circle cx="30" cy="36" r="13" />
      <path d="M30 23c0-4 3-7 7-7" />
      <path d="M31 17c4-5 12-5 16-2-3 5-10 7-16 2Z" />
      <path d="M52 20v6M49 23h6" />
      <path d="M14 44v4M12 46h4" />
    </svg>
  );
}

/** The "something always shows" card, for a brand-new ticket. */
export function TicketProgress({ unassigned }: { unassigned: boolean }) {
  return (
    <div className="portal-next" role="status" data-testid="portal-ticket-progress">
      <b className="portal-next__lead">We have it, and someone is looking.</b>
      <div className="portal-step portal-step--done">
        <span className="portal-step__n">
          <PortalIcon name="check" size={14} />
        </span>
        <span>
          <b>Received</b>
          <span>{unassigned ? "Today, not yet assigned" : "Logged with your service desk"}</span>
        </span>
      </div>
      <div className="portal-step portal-step--now">
        <span className="portal-step__n">2</span>
        <span>
          <b>A technician is looking at it</b>
          <span>
            We usually reply within an hour for something like this. You'll get an email the moment
            we do.
          </span>
        </span>
      </div>
      <div className="portal-step">
        <span className="portal-step__n">3</span>
        <span>
          <b>Our first reply</b>
          <span>It will appear here.</span>
        </span>
      </div>
    </div>
  );
}

/** A single row in a card list. The grid is explicit because the pack draws
 *  several row shapes off one row component. */
export function PortalRow({
  title,
  meta,
  trailing,
  to,
  onClick,
  highlight,
  columns,
  testId,
}: {
  title: string;
  meta?: string;
  trailing?: React.ReactNode;
  to?: string;
  onClick?: () => void;
  /** Gold: this row needs her answer. The only highlighted row in the list. */
  highlight?: boolean;
  columns?: string;
  testId?: string;
}) {
  const body = (
    <>
      <span className="portal-row__title">
        <b>{title}</b>
        {meta !== undefined && <span>{meta}</span>}
      </span>
      {trailing}
    </>
  );
  const classes = `portal-row${highlight ? " portal-row--waiting" : ""}`;
  const style = columns === undefined ? undefined : { gridTemplateColumns: columns };
  if (to !== undefined) {
    return (
      <a className={classes} href={to} style={style} data-testid={testId}>
        {body}
      </a>
    );
  }
  return (
    <button
      type="button"
      className={classes}
      style={style}
      onClick={onClick}
      data-testid={testId}
    >
      {body}
    </button>
  );
}

/** The ticket's line under its title: what is happening, in words. */
export function ticketNote(category: PortalTicketCategory): string {
  return CLIENT_STATUS[clientStatusFor(category)].note;
}
