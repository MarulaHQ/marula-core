"use client";
/**
 * DescribeMoney — the pack's "Describe it" door beside New quote / New
 * contract (Q01–Q09, C01–C04).
 *
 * Purpose: turn a sentence or pasted notes into a draft the ordinary builder
 * can carry. The door only exists when the surface is granted and a model is
 * usable, because the pack's rule is explicit: no grant means no trace of the
 * capability, not a greyed-out button.
 *
 * Non-purpose: it never saves. "Build the draft" hands the parsed draft to the
 * caller, which fills its own create dialog; the existing create route remains
 * the only write.
 */
import { Button, Dialog, DialogContent, DialogTrigger, Textarea } from "@marula-core/ui";
import { useEffect, useState } from "react";
import {
  authorContractDraft,
  authorQuoteDraft,
  moneyAuthoringAvailable,
  type ContractDraft,
  type QuoteDraft,
} from "../../lib/money-authoring";
import { money } from "../../lib/format";

type Kind = "quote" | "contract";
type Draft = QuoteDraft | ContractDraft;

const COPY: Record<Kind, { button: string; title: string; placeholder: string; examples: string[] }> = {
  quote: {
    button: "Describe it",
    title: "Describe a quote",
    placeholder:
      "e.g. Replace the switch stack in the Acme warehouse on Saturday. Two switches, eight hours of labour, patch leads. They also asked about a wireless survey.",
    examples: [
      "Two switches and a day of labour for the Acme warehouse",
      "Onsite support visit, half a day, parts extra",
      "Five new laptops with setup and migration",
    ],
  },
  contract: {
    button: "Describe it",
    title: "Describe a contract",
    placeholder:
      "e.g. Acme wants managed support for a year: helpdesk, patching, P1 within an hour. Twelve users, two sites, billed monthly.",
    examples: [
      "Managed support, 12 users, monthly, annual term",
      "Retainer for 20 hours a month with P1 cover",
      "Fixed-fee network refresh over three months",
    ],
  },
};

function isQuote(value: Draft): value is QuoteDraft {
  return "lines" in value;
}

export function DescribeMoney({
  kind,
  disabled,
  onDraft,
  testId,
}: {
  kind: Kind;
  disabled?: boolean;
  onDraft: (draft: Draft) => void;
  testId?: string;
}) {
  const [available, setAvailable] = useState(false);
  const [open, setOpen] = useState(false);
  const [run, setRun] = useState(0);

  useEffect(() => {
    let cancelled = false;
    const surface = kind === "quote" ? "quote_authoring" : "contract_authoring";
    void moneyAuthoringAvailable(surface).then((ok) => {
      if (!cancelled) setAvailable(ok);
    }).catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [kind]);

  if (!available) return null;
  const copy = COPY[kind];

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) setRun((n) => n + 1);
      }}
    >
      <DialogTrigger asChild>
        <Button variant="secondary" size="sm" disabled={disabled} data-testid={testId ?? `describe-${kind}`}>
          {copy.button}
        </Button>
      </DialogTrigger>
      <DialogContent
        title={copy.title}
        description="I will show you what I understood before I build anything. Nothing is saved until you save the draft."
        width={620}
      >
        <DescribePanel key={run} kind={kind} copy={copy} onBuild={(draft) => {
          setOpen(false);
          onDraft(draft);
        }} />
      </DialogContent>
    </Dialog>
  );
}

function DescribePanel({
  kind,
  copy,
  onBuild,
}: {
  kind: Kind;
  copy: (typeof COPY)[Kind];
  onBuild: (draft: Draft) => void;
}) {
  const [transcript, setTranscript] = useState("");
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [valid, setValid] = useState(false);
  const [assumptions, setAssumptions] = useState<string[]>([]);
  const [issues, setIssues] = useState<{ field: string; message: string }[]>([]);
  const [message, setMessage] = useState<string | null>(null);

  const draftIt = async () => {
    setBusy(true);
    setDraft(null);
    setValid(false);
    setMessage(null);
    setIssues([]);
    const outcome = kind === "quote"
      ? await authorQuoteDraft(transcript)
      : await authorContractDraft(transcript);
    setBusy(false);
    if (!outcome.ok) {
      setMessage(outcome.message);
      return;
    }
    if (outcome.result.draft === null) {
      setIssues(outcome.result.issues);
      setMessage(
        outcome.result.error ??
          "I could not build a draft from those notes. Nothing was saved; edit the notes and try again.",
      );
      return;
    }
    setDraft(outcome.result.draft);
    setValid(outcome.result.valid);
    setAssumptions(outcome.result.assumptions ?? []);
    setIssues(outcome.result.issues);
  };

  return (
    <div className="flex flex-col gap-4">
      {draft === null ? (
        <>
          <label className="flex flex-col gap-1.5">
            <span className="text-[13px] font-medium" style={{ color: "var(--m-text-secondary)" }}>
              What is the job?
            </span>
            <Textarea
              value={transcript}
              onChange={(e) => setTranscript(e.target.value)}
              rows={5}
              placeholder={copy.placeholder}
              data-testid="describe-transcript"
            />
          </label>
          <div className="flex flex-wrap gap-1.5">
            {copy.examples.map((example) => (
              <button
                key={example}
                type="button"
                onClick={() => setTranscript(example)}
                className="rounded-[10px] border px-2.5 py-1 text-left text-[12px]"
                style={{ borderColor: "var(--m-border)", color: "var(--m-text-secondary)" }}
              >
                {example}
              </button>
            ))}
          </div>
          {message !== null && (
            <p className="text-[13px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
              {message}
            </p>
          )}
          <div className="flex items-center justify-end gap-2 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
            <Button
              variant="primary"
              size="sm"
              disabled={busy || transcript.trim() === ""}
              onClick={() => void draftIt()}
              data-testid="describe-draft"
            >
              {busy ? "Reading the notes…" : "Draft it"}
            </Button>
          </div>
        </>
      ) : (
        <>
          <div
            className="rounded-[var(--m-radius-container)] border px-4 py-3"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface-2)" }}
          >
            <p className="text-[12px] font-semibold uppercase tracking-[0.08em]" style={{ color: "var(--m-text-muted)" }}>
              What I understood
            </p>
            <DraftSummary kind={kind} draft={draft} />
          </div>
          {assumptions.length > 0 && (
            <div className="flex flex-col gap-1">
              <span className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                What I assumed
              </span>
              <ul className="list-disc pl-5 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                {assumptions.map((line) => (
                  <li key={line}>{line}</li>
                ))}
              </ul>
            </div>
          )}
          {issues.length > 0 && (
            <ul className="list-disc pl-5 text-[13px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
              {issues.map((issue) => (
                <li key={`${issue.field}-${issue.message}`}>{issue.message}</li>
              ))}
            </ul>
          )}
          {message !== null && (
            <p className="text-[13px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
              {message}
            </p>
          )}
          <div className="flex items-center justify-between gap-2 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                setDraft(null);
                setValid(false);
                setIssues([]);
                setMessage(null);
              }}
            >
              Edit my notes
            </Button>
            <Button variant="primary" size="sm" disabled={!valid} onClick={() => onBuild(draft)} data-testid="describe-build">
              Build the draft
            </Button>
          </div>
        </>
      )}
    </div>
  );
}

function DraftSummary({ kind, draft }: { kind: Kind; draft: Draft }) {
  if (isQuote(draft)) {
    return (
      <div className="mt-2 flex flex-col gap-1.5">
        <p className="text-[15px] font-semibold">{draft.title}</p>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {draft.lines.length} line{draft.lines.length === 1 ? "" : "s"} · {draft.currency}
          {draft.valid_for_days ? ` · valid ${draft.valid_for_days} days` : ""}
        </p>
        <ul className="flex flex-col divide-y" style={{ borderColor: "var(--m-border)" }}>
          {draft.lines.map((line, index) => (
            <li key={`${line.description}-${index}`} className="flex items-baseline justify-between gap-3 py-1.5 text-[13px]">
              <span className="min-w-0">{line.description}</span>
              <span className="shrink-0 tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                {(line.quantity_milli / 1000).toLocaleString(undefined, { maximumFractionDigits: 3 })} ×{" "}
                {money(line.unit_price_minor, draft.currency)}
              </span>
            </li>
          ))}
        </ul>
      </div>
    );
  }
  return (
    <div className="mt-2 flex flex-col gap-1.5">
      <p className="text-[15px] font-semibold">{draft.name}</p>
      <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
        {draft.contract_type.replaceAll("_", " ")}
        {draft.billing_frequency ? ` · ${draft.billing_frequency}` : ""}
        {draft.value_minor != null ? ` · ${money(draft.value_minor, draft.currency)}` : " · value to set"}
        {draft.term_months != null ? ` · ${draft.term_months} months` : ""}
      </p>
      {draft.clauses.length > 0 && (
        <ul className="list-disc pl-5 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {draft.clauses.slice(0, 4).map((clause) => (
            <li key={clause}>{clause}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
