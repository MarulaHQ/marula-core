"use client";
/**
 * The check step: what the draft understood, and what it could not decide.
 *
 * Purpose: the pack's rule is that this always happens — "After reading, the
 * agent always says what it understood before it builds anything. There is no
 * setting for this and no switch in the panel." So this card is not optional
 * chrome: it is the one inverted surface on the screen, and the only place the
 * draft is buildable from.
 *
 * Non-purpose: it does not build. The panel's footer runs the build; this file
 * states the understanding, asks the questions, and lists the assumptions.
 */
import { Icon } from "@marula-core/ui/kit";

import type { DraftQuestion, Understanding } from "../../lib/money-authoring";

export function DescribeUnderstood({
  understanding,
  assumptions,
  answers,
  onAnswer,
  fallbackStatement,
}: {
  understanding: Understanding | null;
  assumptions: string[];
  answers: Record<string, string>;
  onAnswer: (id: string, option: string) => void;
  fallbackStatement: string;
}): React.JSX.Element {
  const statement = understanding?.statement?.trim() ?? "";
  const facts = understanding?.facts ?? [];
  const questions: DraftQuestion[] = understanding?.questions ?? [];
  const assumed = [...(understanding?.assumptions ?? []), ...assumptions];

  return (
    <div className="flex flex-col gap-4">
      <div className="m-dsc-understanding" data-testid="describe-understanding">
        <p className="m-dsc-lab">Here is what I understood</p>
        <p className="m-dsc-understanding-sentence">{statement === "" ? fallbackStatement : statement}</p>
        {facts.length > 0 ? (
          <dl className="m-dsc-facts">
            {facts.map((fact) => (
              <div key={fact.label} className="contents">
                <dt>{fact.label}</dt>
                <dd>{fact.value}</dd>
              </div>
            ))}
          </dl>
        ) : null}
      </div>

      {questions.length > 0 ? (
        <>
          <p className="m-dsc-lab">Needs your answer, {questions.length}</p>
          {questions.map((question) => {
            const chosen = answers[question.id] ?? question.chosen ?? question.options[0];
            return (
              <div className="m-dsc-question" key={question.id}>
                <span className="m-dsc-question-text">{question.text}</span>
                <div className="m-dsc-options" role="group" aria-label={question.text}>
                  {question.options.map((option) => (
                    <button
                      type="button"
                      className="m-dsc-option"
                      key={option}
                      aria-pressed={chosen === option}
                      onClick={() => onAnswer(question.id, option)}
                      data-testid={`describe-answer-${question.id}-${option}`}
                    >
                      {option}
                    </button>
                  ))}
                </div>
              </div>
            );
          })}
        </>
      ) : null}

      {assumed.length > 0 ? (
        <>
          <p className="m-dsc-lab">I assumed, {assumed.length}</p>
          <div className="m-dsc-assumptions" data-testid="describe-assumptions">
            {assumed.map((line) => (
              <div className="m-dsc-assumption" key={line}>
                <Icon name="alert" size={15} />
                <span>{line}</span>
              </div>
            ))}
          </div>
        </>
      ) : null}
    </div>
  );
}

/**
 * The failure card (Q14). Plain, blush, never red: the draft failed the app's
 * own checks, nothing was saved, and the words are still here. The reference is
 * the one thing support needs and the one thing the person cannot reconstruct.
 */
export function DescribeFailure({
  kind,
  message,
  reference,
  sourceCount,
  onRetry,
  onEditNotes,
  onStartBlank,
}: {
  /** Which record the blank action starts: a quote or a contract. */
  kind: "quote" | "contract";
  message: string;
  reference: string | null;
  sourceCount: number;
  onRetry: () => void;
  onEditNotes: () => void;
  onStartBlank: () => void;
}): React.JSX.Element {
  return (
    <div className="flex flex-col gap-4" data-testid="describe-failure">
      <div className="m-dsc-failure">
        <Icon name="alert" size={16} />
        <span>
          <b>{message}</b> Nothing was saved, and your {sourceCount === 1 ? "words are" : "notes are"} still here.
        </span>
      </div>
      <div className="grid gap-2">
        <p className="m-dsc-hint">
          The draft I produced did not pass my own checks, so I threw it away instead of showing you
          something half right.
        </p>
        <p className="m-dsc-hint">
          Try again, or edit your notes and I will read them again. You can also start a blank{" "}
          {kind} with these notes attached.
        </p>
        {reference !== null ? (
          <p className="m-dsc-ref">
            Reference <span>{reference}</span>
            <button
              type="button"
              className="m-dsc-option"
              onClick={() => void navigator.clipboard?.writeText(reference)}
            >
              <Icon name="copy" size={14} /> Copy
            </button>
          </p>
        ) : null}
        <p className="m-dsc-hint">Give the reference to support if it keeps happening.</p>
      </div>
      <div className="flex flex-wrap gap-2">
        <button type="button" className="m-dsc-option" onClick={onEditNotes}>
          Edit my notes
        </button>
        <button type="button" className="m-dsc-option" onClick={onStartBlank}>
          Start a blank {kind}
        </button>
        <button
          type="button"
          className="m-dsc-option"
          onClick={onRetry}
          data-testid="describe-retry"
        >
          <Icon name="refresh" size={14} /> Try again
        </button>
      </div>
    </div>
  );
}
