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
import { Field, Icon, Textarea } from "@marula-core/ui/kit";

export interface TranscriptPanelProps {
  value: string;
  onChange: (next: string) => void;
  onGenerate: () => void;
  busy: boolean;
  /** The transcript was cleared or never filled: Generate stays off, not dead. */
  disabled: boolean;
}

/**
 * The mic affordance, drawn as the typed state it is.
 *
 * Voice-note capture is its own lane by ruling, so there is no recorder behind
 * this control and no audio input to grey out. Rendering nothing would leave a
 * person hunting for a mic; rendering a live button would lie. It states the
 * reason, stays out of the tab order's action path, and points at the sentence
 * that says what to do instead.
 */
function VoiceNotAvailable({ describedBy }: { describedBy: string }) {
  return (
    <Button
      variant="ghost"
      size="sm"
      aria-disabled="true"
      aria-describedby={describedBy}
      data-testid="transcript-voice-unavailable"
      data-state="unavailable"
      title="Voice capture is not available yet. Describe the report in text."
      onClick={(event) => event.preventDefault()}
    >
      <Icon name="mic" size={14} />
      <span className="ml-1.5">Voice note</span>
    </Button>
  );
}

export function TranscriptPanel({
  value,
  onChange,
  onGenerate,
  busy,
  disabled,
}: TranscriptPanelProps) {
  const voiceHintId = "transcript-voice-note";
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
        <VoiceNotAvailable describedBy={voiceHintId} />
        <span className="text-[12px] tabular-nums text-muted-foreground">
          {value.trim().length} characters
        </span>
        <span className="text-[12px] text-muted-foreground">
          A draft is validated against the same registry the builder saves through.
          Nothing is stored until you accept it.
        </span>
      </div>
      <p id={voiceHintId} className="text-[12px] leading-[1.5] text-muted-foreground">
        Voice capture is not available yet, and audio is never recorded here.
        Describe the report in text.
      </p>
    </section>
  );
}
