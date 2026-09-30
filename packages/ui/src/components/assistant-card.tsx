"use client";
/**
 * AssistantCard — Wave 41 lane 2b.
 *
 * Purpose: the ONE inverted surface a screen is allowed (DESIGN-NOTES §5).
 * The card carries the assistant's recommendation, its evidence, and the
 * action a person has to approve; the five `--m-as*` tokens exist for it and
 * for nothing else, which is what keeps "one inverted surface per screen"
 * checkable instead of aspirational.
 *
 * The three states below are the spec's assistant rules, not decoration:
 * a card that has RUN reports what ran, a card that has been DISMISSED
 * collapses to a dashed row carrying Undo (dismissing must be reversible
 * without opening anything), and a card asking a QUESTION shows the answer
 * once it is given. Nothing here sends anything client-facing — the assistant
 * drafts, a person approves.
 *
 * Non-purpose: this is presentational. It does not call the assistant, hold
 * a conversation, or decide what is client-facing; `AssistantPanel` in the app
 * owns all of that and renders this.
 */
import {
  forwardRef,
  type HTMLAttributes,
  type ReactNode,
} from "react";
import { cn } from "../lib/cn";

export interface AssistantCardProps extends HTMLAttributes<HTMLDivElement> {
  /** Small caps label above the recommendation. */
  eyebrow?: ReactNode;
  heading: ReactNode;
  children?: ReactNode;
  /** The primary action, usually a `Button variant="assistant"`. */
  action?: ReactNode;
  /** Secondary actions: "View evidence", "Not this". */
  secondaryActions?: ReactNode;
  /** Rendered under a hairline inside the card (--m-asline). */
  footer?: ReactNode;
}

export const AssistantCard = forwardRef<HTMLDivElement, AssistantCardProps>(
  (
    { className, eyebrow, heading, children, action, secondaryActions, footer, ...props },
    ref,
  ) => (
    <div ref={ref} className={cn("m-assistant-card", className)} {...props}>
      {eyebrow ? <p className="m-assistant-eyebrow">{eyebrow}</p> : null}
      <p className="m-assistant-title">{heading}</p>
      {children ? <div className="m-assistant-body">{children}</div> : null}
      {action || secondaryActions ? (
        <div className="m-assistant-actions">
          {action}
          {secondaryActions}
        </div>
      ) : null}
      {footer ? <div className="m-assistant-footer">{footer}</div> : null}
    </div>
  ),
);

AssistantCard.displayName = "AssistantCard";

/**
 * The dismissed state: the card collapses to a dashed row that keeps only
 * what was dismissed and the way back. `--m-asf` is the card's own
 * foreground, so a dashed border in it reads as "this was here" rather than
 * as a second control.
 */
export function AssistantCardDismissed({
  summary,
  onUndo,
  className,
  ...props
}: HTMLAttributes<HTMLDivElement> & {
  summary: ReactNode;
  onUndo: () => void;
}): React.JSX.Element {
  return (
    <div className={cn("m-assistant-dismissed", className)} {...props}>
      <span className="m-assistant-dismissed-summary">{summary}</span>
      <button type="button" className="m-assistant-undo" onClick={onUndo}>
        Undo
      </button>
    </div>
  );
}

/**
 * A question the assistant is waiting on, with the answer once it is given.
 * The answered state keeps the question visible with a check beside it: a
 * question that disappears on answer leaves the timeline with no trace of
 * what was asked.
 */
export function AssistantCardQuestion({
  question,
  answer,
  className,
  ...props
}: HTMLAttributes<HTMLDivElement> & {
  question: ReactNode;
  /** Absent while the question is open. */
  answer?: ReactNode;
}): React.JSX.Element {
  const answered = answer !== undefined;
  return (
    <div
      className={cn("m-assistant-question", answered && "is-answered", className)}
      {...props}
    >
      <span className="m-assistant-question-mark" aria-hidden="true">
        {answered ? (
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
            <path d="M4 12.5 9.5 18 20 6.5" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        ) : (
          "?"
        )}
      </span>
      <span className="m-assistant-question-body">
        <span className="m-assistant-question-text">{question}</span>
        {answered ? <span className="m-assistant-answer">{answer}</span> : null}
      </span>
    </div>
  );
}
