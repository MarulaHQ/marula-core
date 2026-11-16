"use client";
/**
 * Describe it — the door, the split New control, and the docked 440px panel.
 *
 * Purpose: one quiet entry (Q01), the same capability as the third way to start
 * a document (Q02), and the panel the pack draws for every step between words
 * and a draft (Q03–Q08, Q14, C02, C03, C05). The panel composes typed, spoken
 * and uploaded sources, shows the live read of the words, says what it
 * understood before it builds, and hands the draft to the ordinary builder.
 *
 * The panel is mounted for as long as the route is, and hidden rather than
 * unmounted when it closes, so "Keep working" is real: the request keeps
 * running and the check step is there when it lands.
 *
 * Non-purpose: it never saves. The draft goes to `onBuild`, the route opens its
 * own builder, and the existing create route remains the only write. No grant
 * means none of this renders at all — not a disabled button, not a hint, not a
 * gap (pack DESIGN-NOTES §Decisions, Q12).
 */
import { Button, Kbd } from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import {
  authorContractDraft,
  authorQuoteDraft,
  extractFields,
  moneyAuthoringAvailable,
  type ContractDraft,
  type ContractAuthorResult,
  type ExtractedField,
  type MoneyKind,
  type MoneySource,
  type QuoteAuthorResult,
  type QuoteDraft,
} from "../../lib/money-authoring";
import { DescribeCompose } from "./describe-compose";
import { DescribeFailure, DescribeUnderstood } from "./describe-understood";
import "./authoring.css";

export type AnyDraft = QuoteDraft | ContractDraft;
export type AuthorResult = QuoteAuthorResult | ContractAuthorResult;

/**
 * True only once the org has granted the surface AND a model is usable.
 * `enabled` lets a caller that already asked (a route that needs the answer for
 * its own New menu) pass the answer down instead of asking twice.
 */
export function useMoneyAuthoring(surfaceKey: string, enabled = true): boolean {
  const [available, setAvailable] = useState(false);
  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    void moneyAuthoringAvailable(surfaceKey)
      .then((ok) => {
        if (!cancelled) setAvailable(ok);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [enabled, surfaceKey]);
  return available;
}

function surfaceOf(kind: MoneyKind): string {
  return kind === "quote" ? "quote_authoring" : "contract_authoring";
}

/**
 * The quiet door beside the New control: an outline button, the key, and
 * nothing else. It renders nothing when the surface is not granted.
 */
export function DescribeMoney({
  kind,
  onOpen,
  testId,
  available,
}: {
  kind: MoneyKind;
  onOpen: () => void;
  testId?: string;
  available?: boolean;
}): React.JSX.Element | null {
  // The hook runs on every render, in the same order, whoever supplies the
  // answer; a caller may hand `available` down or leave it to this component.
  const own = useMoneyAuthoring(surfaceOf(kind), available === undefined);
  const show = available ?? own;
  if (!show) return null;
  return (
    <Button
      variant="secondary"
      onClick={onOpen}
      aria-keyshortcuts="d"
      data-testid={testId ?? `describe-${kind}`}
    >
      <Icon name="spark" size={16} />
      Describe it
      <Kbd style={{ marginLeft: 4 }}>D</Kbd>
    </Button>
  );
}

/**
 * The split New control (Q02). One primary action plus a menu of the ways to
 * start: blank, and — when it is granted — describing it. "From a template"
 * joins this menu when quote templates exist; the product has no template
 * concept today, so the menu does not offer one it cannot honour.
 */
export function NewMoneySplit({
  label,
  onBlank,
  onDescribe,
  showDescribe,
  testId,
  menuLabel,
}: {
  label: string;
  onBlank: () => void;
  onDescribe: () => void;
  showDescribe: boolean;
  testId: string;
  menuLabel: string;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      if (wrap.current !== null && !wrap.current.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div className="relative inline-flex" ref={wrap}>
      <div className="inline-flex">
        <Button
          variant="primary"
          onClick={onBlank}
          data-testid={testId}
          style={{ borderTopRightRadius: 0, borderBottomRightRadius: 0 }}
        >
          {label}
        </Button>
        <Button
          variant="primary"
          aria-label={menuLabel}
          aria-expanded={open}
          aria-haspopup="menu"
          onClick={() => setOpen((value) => !value)}
          data-testid={`${testId}-menu`}
          style={{ borderTopLeftRadius: 0, borderBottomLeftRadius: 0, paddingInline: 12 }}
        >
          <Icon name="down" size={16} />
        </Button>
      </div>
      {open ? (
        <div
          role="menu"
          aria-label={menuLabel}
          className="absolute right-0 top-[calc(100%+8px)] z-40 w-[330px] overflow-hidden rounded-[14px] border p-1.5"
          style={{
            background: "var(--m-overlay-surface)",
            borderColor: "var(--m-border)",
            boxShadow: "var(--m-shadow-overlay)",
          }}
        >
          <MenuItem
            icon="file"
            title={`Blank ${kindOf(label)}`}
            sub="Start with an empty page"
            onClick={() => {
              setOpen(false);
              onBlank();
            }}
            testId={`${testId}-blank`}
          />
          {showDescribe ? (
            <MenuItem
              icon="spark"
              title="Describe it"
              sub="Type, talk or upload meeting notes"
              selected
              hint="D"
              onClick={() => {
                setOpen(false);
                onDescribe();
              }}
              testId={`${testId}-describe`}
            />
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function kindOf(label: string): string {
  return label.toLowerCase().includes("contract") ? "contract" : "quote";
}

function MenuItem({
  icon,
  title,
  sub,
  onClick,
  selected,
  hint,
  testId,
}: {
  icon: "file" | "spark";
  title: string;
  sub: string;
  onClick: () => void;
  selected?: boolean;
  hint?: string;
  testId: string;
}): React.JSX.Element {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      data-testid={testId}
      className="flex w-full items-center gap-3 rounded-[10px] px-2.5 py-2 text-left"
      style={{
        background: selected === true ? "var(--m-accent)" : "transparent",
        color: selected === true ? "var(--m-on-accent)" : "var(--m-text)",
        minHeight: 52,
      }}
    >
      <span
        className="flex size-7 shrink-0 items-center justify-center rounded-[8px]"
        style={{
          background: selected === true ? "var(--m-surface)" : "var(--m-surface-2)",
          color: selected === true ? "var(--m-on-accent)" : "var(--m-text-secondary)",
        }}
      >
        <Icon name={icon} size={15} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-[14px] font-semibold">{title}</span>
        <span
          className="block text-[12.5px]"
          style={{ color: selected === true ? "var(--m-on-accent)" : "var(--m-text-secondary)" }}
        >
          {sub}
        </span>
      </span>
      {hint !== undefined ? <Kbd>{hint}</Kbd> : null}
    </button>
  );
}

// --- the panel ------------------------------------------------------------

type Mode = "compose" | "reading" | "understood" | "failed";

export type DescribePanelProps = {
  kind: MoneyKind;
  open: boolean;
  onClose: () => void;
  clients: { id: string; name: string }[];
  /** The draft and the words it came from, for the builder and its notes tab. */
  onBuild: (draft: AnyDraft, sources: MoneySource[], kind: MoneyKind) => void;
  /** "Start a blank quote/contract" from the failure card: same words, no draft. */
  onStartBlank: (sources: MoneySource[]) => void;
  initialText?: string;
  initialClientId?: string;
  /** Renewal (C05): the current contract's name, so the panel opens prefilled. */
  prefilled?: { text: string; using: { label: string; icon: "file" | "ticket" | "chart" }[] } | null;
  /**
   * The record kinds this route can actually write. A panel opened from the
   * quotes list builds quotes; offering Contract there would produce a draft
   * with no home, so the choice is only drawn when there is a real choice.
   */
  kinds?: MoneyKind[];
  /** Called when a draft lands while the panel is closed ("Keep working"). */
  onReady?: () => void;
  testId?: string;
};

export function DescribePanel({
  kind,
  open,
  onClose,
  clients,
  onBuild,
  onStartBlank,
  initialText,
  initialClientId,
  prefilled,
  kinds,
  onReady,
  testId,
}: DescribePanelProps): React.JSX.Element | null {
  const [panelKind, setPanelKind] = useState<MoneyKind>(kind);
  const [mode, setMode] = useState<Mode>("compose");
  const [clientId, setClientId] = useState(initialClientId ?? "");
  const [text, setText] = useState(initialText ?? prefilled?.text ?? "");
  const [sources, setSources] = useState<MoneySource[]>([]);
  const [instruction, setInstruction] = useState("");
  const [fields, setFields] = useState<ExtractedField[]>([]);
  const [result, setResult] = useState<AuthorResult | null>(null);
  const [failure, setFailure] = useState<{ message: string; reference: string | null } | null>(null);
  const [answers, setAnswers] = useState<Record<string, string>>({});
  const [seenOpen, setSeenOpen] = useState(false);
  const abort = useRef<AbortController | null>(null);
  // The request outlives the render that started it, and the person may close
  // the panel while it runs. Read the latest `open` when it lands, not the one
  // captured at click time.
  const openRef = useRef(open);
  openRef.current = open;

  // The panel keeps the kind it was opened with; a renewal always opens as a
  // contract even if the list's own kind is a quote.
  useEffect(() => {
    if (open) {
      setPanelKind(kind);
      setSeenOpen(true);
      // The panel is not a modal, so focus has to be moved on purpose: the
      // person clicked a door, and the next thing they want is the box.
      const timer = window.setTimeout(() => {
        const box = document.getElementById("describe-text");
        if (box instanceof HTMLTextAreaElement) box.focus();
      }, 0);
      return () => window.clearTimeout(timer);
    }
    return undefined;
  }, [open, kind]);

  useEffect(() => {
    if (prefilled !== undefined && prefilled !== null) {
      setText(prefilled.text);
    }
  }, [prefilled]);

  // Live extraction: debounced, and it never blocks the composer. This is the
  // pre-commit read (Q04), deliberately separate from the check step (Q08).
  useEffect(() => {
    if (!seenOpen) return;
    const typed = text.trim();
    if (typed === "" && sources.length === 0) {
      setFields([]);
      return;
    }
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      void extractFields(
        panelKind,
        {
          transcript: typed,
          sources,
          client_id: clientId === "" ? null : clientId,
        },
        controller.signal,
      ).then((outcome) => {
        if (outcome.ok) setFields(outcome.result.fields);
      });
    }, 450);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [text, sources, clientId, panelKind, seenOpen]);

  const canDraft = text.trim() !== "" || sources.length > 0;

  const draftIt = useCallback(async () => {
    if (!canDraft) return;
    setMode("reading");
    setFailure(null);
    setResult(null);
    setAnswers({});
    const controller = new AbortController();
    abort.current = controller;
    const input = {
      transcript: text.trim(),
      sources,
      instruction: instruction.trim() === "" ? undefined : instruction.trim(),
      client_id: clientId === "" ? null : clientId,
    };
    const outcome =
      panelKind === "quote"
        ? await authorQuoteDraft(input, controller.signal)
        : await authorContractDraft(input, controller.signal);
    abort.current = null;
    if (!outcome.ok) {
      setFailure({ message: outcome.message, reference: null });
      setMode("failed");
      return;
    }
    if (outcome.result.draft === null) {
      const issue = outcome.result.issues[0]?.message;
      setFailure({
        message: issue ?? outcome.result.error ?? "I could not build a draft from those words.",
        reference: outcome.result.reference ?? null,
      });
      setMode("failed");
      return;
    }
    setResult(outcome.result);
    setMode("understood");
    // The panel may have been closed to keep working: the draft arrives, and
    // the route brings the check step back rather than leaving it hidden.
    if (!openRef.current) onReady?.();
    setAnswers(
      Object.fromEntries(
        (outcome.result.draft.understanding?.questions ?? []).map((question) => [
          question.id,
          question.chosen ?? question.options[0] ?? "",
        ]),
      ),
    );
  }, [canDraft, clientId, instruction, onReady, panelKind, sources, text]);

  useEffect(() => () => abort.current?.abort(), []);

  const understanding = useMemo(() => result?.draft?.understanding ?? null, [result]);
  const draft = result?.draft ?? null;
  const copy = HEAD_COPY[mode === "reading" ? "reading" : mode];

  if (!open && !seenOpen) return null;

  return (
    <aside
      className="m-dsc-panel"
      aria-label="Describe it"
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          onClose();
        }
      }}
      hidden={!open}
      data-testid={testId ?? "describe-panel"}
      style={open ? undefined : { display: "none" }}
    >
      <div className="m-dsc-head">
        <span className="m-dsc-mark">
          <Icon name="spark" size={17} />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="m-dsc-head-title">{copy.title}</h2>
          <span className="m-dsc-head-sub">{copy.sub}</span>
        </div>
        <Button
          variant="ghost"
          size="sm"
          aria-label="Close panel"
          onClick={onClose}
          data-testid="describe-close"
        >
          <Icon name="x" size={16} />
        </Button>
      </div>

      <div className="m-dsc-body">
        {/* Only while the panel is open: the panel is hidden rather than
            unmounted, and a Talk card inside it would keep the microphone
            running behind a closed panel. Its words live in the parent, so
            closing loses nothing that was already settled. */}
        {mode === "compose" && open ? (
          <>
            {prefilled !== undefined && prefilled !== null && prefilled.using.length > 0 ? (
              <div className="flex flex-col gap-1.5">
                <p className="m-dsc-lab">Using</p>
                <div className="m-dsc-chips">
                  {prefilled.using.map((item) => (
                    <span className="m-dsc-chip m-dsc-chip-neutral" key={item.label}>
                      <Icon name={item.icon} size={13} />
                      {item.label}
                    </span>
                  ))}
                </div>
              </div>
            ) : null}
            <DescribeCompose
              kind={panelKind}
              kinds={kinds ?? [kind]}
              onKind={setPanelKind}
              clients={clients}
              clientId={clientId}
              onClientId={setClientId}
              text={text}
              onText={setText}
              sources={sources}
              onSources={setSources}
              instruction={instruction}
              onInstruction={setInstruction}
              fields={fields}
              canDraft={canDraft}
              onDraft={() => void draftIt()}
              footerHint="I will show you what I understood before I build anything."
              draftLabel="Draft it"
              testId="describe-draft"
            />
          </>
        ) : null}

        {mode === "reading" ? (
          <Reading
            sources={sources}
            typed={text.trim() !== ""}
            fields={fields}
            onKeepWorking={onClose}
            onCancel={() => {
              abort.current?.abort();
              setMode("compose");
            }}
          />
        ) : null}

        {mode === "understood" ? (
          <>
            <DescribeUnderstood
              understanding={understanding}
              assumptions={draft?.assumptions ?? []}
              answers={answers}
              onAnswer={(id, option) =>
                setAnswers((current) => ({ ...current, [id]: option }))
              }
              fallbackStatement={
                panelKind === "quote"
                  ? "A quote from the words you gave me."
                  : "A contract from the words you gave me."
              }
            />
            <div className="m-dsc-foot" style={{ borderTop: "none", padding: 0, background: "transparent" }}>
              <span className="m-dsc-foot-hint">Nothing is saved until you build it.</span>
              <Button variant="secondary" size="sm" onClick={() => setMode("compose")}>
                Edit my notes
              </Button>
              <Button
                variant="primary"
                size="sm"
                onClick={() => {
                  if (draft === null) return;
                  onBuild(withAnswers(draft, answers), sourceList(text, sources), panelKind);
                  onClose();
                }}
                data-testid="describe-build"
              >
                Build the draft
              </Button>
            </div>
          </>
        ) : null}

        {mode === "failed" && failure !== null ? (
          <DescribeFailure
            kind={panelKind}
            message={failure.message}
            reference={failure.reference}
            sourceCount={sourceList(text, sources).length}
            onRetry={() => void draftIt()}
            onEditNotes={() => setMode("compose")}
            onStartBlank={() => {
              onStartBlank(sourceList(text, sources));
              onClose();
            }}
          />
        ) : null}
      </div>
    </aside>
  );
}

const HEAD_COPY: Record<Mode | "reading", { title: string; sub: string }> = {
  compose: {
    title: "Describe it",
    sub: "Say or paste what was agreed. I draft it, you review it.",
  },
  reading: {
    title: "Reading your notes",
    sub: "This takes about 20 seconds. You can close this and keep working.",
  },
  understood: {
    title: "Here is what I understood",
    sub: "Check this before I build. It takes ten seconds.",
  },
  failed: {
    title: "I couldn't build that draft",
    sub: "Nothing was saved. Your notes are still here.",
  },
};

/** The typed words become a source too, so the notes tab can point at them. */
function sourceList(text: string, sources: MoneySource[]): MoneySource[] {
  const typed = text.trim();
  if (typed === "") return sources;
  return [{ id: "typed-1", kind: "typed", name: "Your words", text: typed }, ...sources];
}

/**
 * The check step's answers are part of the draft the builder receives.
 * Otherwise the person picks a signer or an approver and the draft still
 * carries the server's first guess.
 */
function withAnswers(draft: AnyDraft, answers: Record<string, string>): AnyDraft {
  const understanding = draft.understanding;
  if (understanding === null || understanding === undefined) return draft;
  return {
    ...draft,
    understanding: {
      ...understanding,
      questions: understanding.questions.map((question) => ({
        ...question,
        chosen: answers[question.id] ?? question.chosen,
      })),
    },
  };
}

/**
 * The progress list (Q07). Its first three rows are facts from the pre-read the
 * panel already ran; the last rows say what is still happening rather than
 * inventing a percentage.
 */
export function Reading({
  sources,
  typed,
  fields,
  onKeepWorking,
  onCancel,
}: {
  sources: MoneySource[];
  typed: boolean;
  fields: ExtractedField[];
  onKeepWorking: () => void;
  onCancel: () => void;
}): React.JSX.Element {
  const sourceCount = sources.length + (typed ? 1 : 0);
  const client = fields.find((field) => field.key === "client")?.value;
  const things = fields.filter((field) => field.key === "item" || field.key === "quantity").length;
  const budget = fields.find((field) => field.key === "budget")?.value;

  const steps: { state: "done" | "working" | "todo"; title: string; sub?: string }[] = [
    {
      state: "done",
      title: `Read ${sourceCount} source${sourceCount === 1 ? "" : "s"}`,
      sub: typed && sourceCount === 1 ? "your words" : undefined,
    },
    {
      state: "done",
      title: client === undefined ? "Looking for the client" : "Found the client",
      sub: client,
    },
    {
      state: "done",
      title: things === 0 ? "Looking for what to quote" : `Found ${things} thing${things === 1 ? "" : "s"}`,
    },
    {
      state: "working",
      title: "Pricing from your catalogue and past quotes",
      sub: "a line the notes never priced comes back at zero",
    },
    {
      state: "todo",
      title: "Checking budget and terms",
      sub: budget,
    },
    { state: "todo", title: "Building the draft" },
  ];

  return (
    <div className="flex flex-col gap-3" aria-live="polite">
      <div>
        {steps.map((step) => (
          <div className="m-dsc-step" key={step.title}>
            <span className="m-dsc-step-dot" data-state={step.state} aria-hidden="true">
              {step.state === "done" ? <Icon name="check" size={14} /> : null}
              {step.state === "working" ? <Icon name="refresh" size={14} /> : null}
            </span>
            <div>
              <b>{step.title}</b>
              {step.sub !== undefined ? <span>{step.sub}</span> : null}
            </div>
          </div>
        ))}
      </div>
      <p className="m-dsc-hint">I will tell you when it is ready.</p>
      <div className="flex items-center gap-2">
        <Button variant="secondary" size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <span className="m-dsc-sp flex-1" />
        <Button variant="primary" size="sm" onClick={onKeepWorking} data-testid="describe-keep-working">
          Keep working
        </Button>
      </div>
    </div>
  );
}
