"use client";
/**
 * TranscriptPanel — the "describe it" composer.
 *
 * One free-text field, named as the sentence it wants (Pages' "Describe your
 * change", Craft's review prompt), plus the one button that starts a draft. The
 * hint is load-bearing and deliberately plain: **text only**. Voice-note capture
 * is a different lane's job (Wave 46 ruling), so this panel has no microphone, no
 * recorder and no audio input to disable — there is nothing here to grey out.
 *
 * The count is a character count, not a token count: it tells the reviewer when
 * the description is getting long without pretending to know the model's
 * budget.
 */
import { Button, Spinner } from "@marula-core/ui";
import { Field, Textarea } from "@marula-core/ui/kit";

export interface TranscriptPanelProps {
  value: string;
  onChange: (next: string) => void;
  onGenerate: () => void;
  busy: boolean;
  /** The transcript was cleared or never filled: Generate stays off, not dead. */
  disabled: boolean;
}

export function TranscriptPanel({
  value,
  onChange,
  onGenerate,
  busy,
  disabled,
}: TranscriptPanelProps) {
  return (
    <section aria-label="Describe the report" className="grid gap-3">
      <Field
        label="What should this report answer?"
        hint="Write it as you would say it — the data it reads, what it groups by, what it counts, and how far back it looks. Text only: a voice note is captured elsewhere."
      >
        <Textarea
          value={value}
          onChange={(event) => onChange(event.target.value)}
          placeholder="Tickets opened this month by priority, as a bar chart."
          rows={4}
          className="h-auto min-h-[92px]"
          disabled={busy}
        />
      </Field>
      <div className="flex flex-wrap items-center gap-3">
        <Button variant="primary" onClick={onGenerate} disabled={disabled || busy}>
          {busy ? <Spinner /> : null} Generate a draft
        </Button>
        <span className="text-[12px] tabular-nums text-muted-foreground">
          {value.trim().length} characters
        </span>
        <span className="text-[12px] text-muted-foreground">
          A draft is validated against the same registry the builder saves through.
          Nothing is stored until you accept it.
        </span>
      </div>
    </section>
  );
}
