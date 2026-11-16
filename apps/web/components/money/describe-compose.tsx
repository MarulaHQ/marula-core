"use client";
/**
 * The compose half of Describe it: the kind, the client, one composer, and the
 * three ways in — typing, talking and uploading.
 *
 * Purpose: the panel is one place that combines sources (pack DESIGN-NOTES
 * §Rules applied: "One panel, one composer"). Typing, a spoken transcript and
 * an uploaded document all end up as sources in the same list, and the chips
 * under the box say what the words already contain before anything is drafted.
 *
 * Non-purpose: it never drafts and never saves. The panel owns the request; this
 * file owns the words.
 */
import { Button } from "@marula-core/ui";
import { Segmented, SegmentedItem } from "@marula-core/ui/kit";
import { Icon } from "@marula-core/ui/kit";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type {
  ExtractedField,
  MoneyKind,
  MoneySource,
  SourceKind,
} from "../../lib/money-authoring";
import { speechRecognitionCtor, uploadSource } from "../../lib/money-authoring";

const EXAMPLES: Record<MoneyKind, string[]> = {
  quote: [
    "Quote 10 laptops for Kestrel, installed by 20 Nov",
    "Replace six access points at Acme's Cape Town HQ, survey first, weekend install",
    "Renew Acme's backup quote with a 5% uplift",
  ],
  contract: [
    "Managed Silver for Fernwood Pharmacy, 12 months from 1 Nov, 12 users and 14 devices",
    "Retainer, 20 hours a month, P1 in 2 hours, Net 14",
    "Managed Gold renewal for Acme with a 5% uplift",
  ],
};

const KIND_SUB: Record<MoneyKind, { title: string; sub: string; placeholder: string }> = {
  quote: {
    title: "Describe it",
    sub: "Say or paste what was agreed. I draft it, you review it.",
    placeholder:
      "Describe the work, the price and when it should happen. Notes from a call work well.",
  },
  contract: {
    title: "Describe it",
    sub: "Say or paste the agreement. I fill it from your clause library.",
    placeholder:
      "Describe the agreement: who it is for, what it covers, how long, and what it costs.",
  },
};

export type ClientOption = { id: string; name: string };

export function DescribeCompose({
  kind,
  kinds,
  onKind,
  clients,
  clientId,
  onClientId,
  text,
  onText,
  sources,
  onSources,
  instruction,
  onInstruction,
  fields,
  canDraft,
  onDraft,
  footerHint,
  draftLabel,
  testId,
}: {
  kind: MoneyKind;
  /** The kinds this route can write. One means the choice is not drawn. */
  kinds: MoneyKind[];
  onKind: (kind: MoneyKind) => void;
  clients: ClientOption[];
  clientId: string;
  onClientId: (id: string) => void;
  text: string;
  onText: (text: string) => void;
  sources: MoneySource[];
  onSources: (sources: MoneySource[]) => void;
  instruction: string;
  onInstruction: (instruction: string) => void;
  fields: ExtractedField[];
  canDraft: boolean;
  onDraft: () => void;
  footerHint: string;
  draftLabel: string;
  testId: string;
}): React.JSX.Element {
  const [talking, setTalking] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [reading, setReading] = useState(false);
  const fileInput = useRef<HTMLInputElement | null>(null);
  const copy = KIND_SUB[kind];

  const speakable = useMemo(() => speechRecognitionCtor() !== null, []);

  const addFiles = useCallback(
    async (files: FileList | File[]) => {
      setUploadError(null);
      setReading(true);
      const next = [...sources];
      for (const file of Array.from(files)) {
        const outcome = await uploadSource(file);
        if (outcome.accepted) {
          next.push(outcome.source);
        } else {
          setUploadError(outcome.message);
        }
      }
      setReading(false);
      onSources(next);
    },
    [onSources, sources],
  );

  const stopAndReview = useCallback(
    (transcript: string) => {
      setTalking(false);
      const spoken = transcript.trim();
      if (spoken !== "") {
        const joined = text.trim() === "" ? spoken : `${text.trim()}\n\n${spoken}`;
        onText(joined);
      }
    },
    [onText, text],
  );

  return (
    <div className="flex flex-col gap-4">
      {kinds.length > 1 ? (
        <Segmented
          type="single"
          value={kind}
          onValueChange={(value) => {
            if (value === "quote" || value === "contract") onKind(value);
          }}
          aria-label="What to draft"
        >
          {kinds.includes("quote") ? <SegmentedItem value="quote">Quote</SegmentedItem> : null}
          {kinds.includes("contract") ? (
            <SegmentedItem value="contract">Contract</SegmentedItem>
          ) : null}
        </Segmented>
      ) : null}

      <label className="m-dsc-field">
        <span className="m-dsc-lab">For</span>
        <select
          className="m-dsc-select"
          value={clientId}
          onChange={(event) => onClientId(event.target.value)}
          data-testid="describe-client"
        >
          <option value="">Optional. I will look for the client in what you give me.</option>
          {clients.map((client) => (
            <option key={client.id} value={client.id}>
              {client.name}
            </option>
          ))}
        </select>
      </label>

      {talking ? (
        <TalkCard onStop={stopAndReview} onCancel={() => setTalking(false)} />
      ) : (
        <div className="m-dsc-composer">
          <label className="sr-only" htmlFor="describe-text">
            What should I read?
          </label>
          <textarea
            id="describe-text"
            className="m-dsc-textarea"
            value={text}
            onChange={(event) => onText(event.target.value)}
            placeholder={copy.placeholder}
            data-testid="describe-transcript"
          />
          <div className="m-dsc-composer-bar">
            {speakable ? (
              <Button
                size="sm"
                variant="secondary"
                onClick={() => setTalking(true)}
                data-testid="describe-talk"
              >
                <Icon name="mic" size={15} /> Talk
              </Button>
            ) : null}
            <Button
              size="sm"
              variant="secondary"
              onClick={() => fileInput.current?.click()}
              data-testid="describe-upload"
            >
              <Icon name="paperclip" size={15} /> Upload notes
            </Button>
            <span className="m-dsc-sp" />
            <span className="m-dsc-hint">Paste works too</span>
          </div>
        </div>
      )}

      <input
        ref={fileInput}
        type="file"
        multiple
        accept=".txt,.md,.csv,.vtt,.srt,.json,.log,text/plain,text/csv,text/vtt,text/markdown"
        className="sr-only"
        aria-label="Upload notes"
        onChange={(event) => {
          if (event.target.files !== null && event.target.files.length > 0) {
            void addFiles(event.target.files);
          }
          event.target.value = "";
        }}
      />

      {reading ? <p className="m-dsc-hint">Reading the file…</p> : null}
      {uploadError !== null ? (
        <div className="m-dsc-failure" role="alert" data-testid="describe-upload-error">
          <Icon name="alert" size={16} />
          <span>{uploadError}</span>
        </div>
      ) : null}

      {sources.length > 0 ? (
        <div className="flex flex-col gap-2">
          <p className="m-dsc-lab">
            {sources.length} source{sources.length === 1 ? "" : "s"}
          </p>
          {sources.map((source, index) => (
            <div className="m-dsc-source" key={`${source.id ?? source.name ?? index}`}>
              <Icon name={source.kind === "transcript" ? "mic" : "file"} size={16} />
              <span className="min-w-0 flex-1">
                <span className="m-dsc-source-name">{source.name ?? label(source.kind)}</span>
                <span className="m-dsc-source-meta">
                  {Array.from(source.text).length.toLocaleString()} characters
                </span>
              </span>
              <span className="m-dsc-chip">
                <Icon name="check" size={13} /> Read
              </span>
              <button
                type="button"
                className="m-dsc-option"
                aria-label={`Remove ${source.name ?? label(source.kind)}`}
                onClick={() => onSources(sources.filter((_, i) => i !== index))}
              >
                <Icon name="x" size={14} />
              </button>
            </div>
          ))}
          <button
            type="button"
            className="m-dsc-drop"
            onClick={() => fileInput.current?.click()}
            data-testid="describe-drop-more"
          >
            <Icon name="up" size={18} />
            <span>
              Drop more files, or <strong>browse</strong>. Text, Markdown, CSV and VTT or SRT
              transcripts.
            </span>
          </button>
        </div>
      ) : null}

      {sources.length > 0 ? (
        <label className="m-dsc-field">
          <span className="m-dsc-lab">Anything to add? Optional</span>
          <textarea
            className="m-dsc-textarea"
            style={{ minHeight: 64 }}
            value={instruction}
            onChange={(event) => onInstruction(event.target.value)}
            placeholder="They want the optional support plan. Ignore the part about printers."
            data-testid="describe-instruction"
          />
        </label>
      ) : null}

      {fields.length > 0 ? (
        <div className="flex flex-col gap-1.5" data-testid="describe-seen">
          <p className="m-dsc-lab">I can see</p>
          <div className="m-dsc-chips">
            {fields.map((field) => (
              <span
                className={`m-dsc-chip${field.key === "exclusion" ? " m-dsc-chip-gold" : ""}`}
                key={`${field.key}-${field.value}`}
              >
                <Icon name={field.key === "exclusion" ? "alert" : "check"} size={13} />
                {field.value}
              </span>
            ))}
          </div>
        </div>
      ) : null}

      {text.trim() === "" && sources.length === 0 ? (
        <div className="flex flex-col gap-1.5">
          <p className="m-dsc-lab">Or start from</p>
          {EXAMPLES[kind].map((example) => (
            <button
              type="button"
              className="m-dsc-drop"
              key={example}
              onClick={() => onText(example)}
            >
              <Icon name="spark" size={15} />
              <span>{example}</span>
            </button>
          ))}
        </div>
      ) : null}

      <div className="m-dsc-foot" style={{ borderTop: "none", padding: 0, background: "transparent" }}>
        <span className="m-dsc-foot-hint">{footerHint}</span>
        <Button
          variant="primary"
          disabled={!canDraft}
          onClick={onDraft}
          data-testid={testId}
        >
          <Icon name="spark" size={16} /> {draftLabel}
        </Button>
      </div>
      <p className="sr-only" aria-live="polite">
        {fields.length > 0 ? `I can see ${fields.length} things in these words.` : ""}
      </p>
    </div>
  );
}

function label(kind: SourceKind): string {
  if (kind === "transcript") return "Transcript";
  if (kind === "recording") return "Recording";
  if (kind === "typed") return "Typed notes";
  return "Document";
}

// --- talk -----------------------------------------------------------------

/**
 * The live card. The recogniser lives here, not in the panel: it starts when
 * the card opens, stops while paused, and is aborted when the card goes away,
 * so no microphone stays open behind a closed panel.
 *
 * Audio never leaves the browser: the recogniser returns text and the audio is
 * discarded by the browser itself, which is the promise the card states.
 */
function TalkCard({
  onStop,
  onCancel,
}: {
  onStop: (transcript: string) => void;
  onCancel: () => void;
}): React.JSX.Element {
  const [finalText, setFinalText] = useState("");
  const [interim, setInterim] = useState("");
  const [paused, setPaused] = useState(false);
  const [seconds, setSeconds] = useState(0);
  const finalRef = useRef("");
  const recognitionRef = useRef<{
    start: () => void;
    stop: () => void;
    abort: () => void;
  } | null>(null);
  const speakable = speechRecognitionCtor();

  useEffect(() => {
    const timer = window.setInterval(() => setSeconds((value) => value + 1), 1000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (speakable === null) return;
    const recognition = new speakable();
    recognition.lang = typeof navigator === "undefined" ? "en-US" : navigator.language;
    recognition.continuous = true;
    recognition.interimResults = true;
    recognition.onresult = (event) => {
      let final = finalRef.current;
      let pending = "";
      for (let index = event.resultIndex; index < event.results.length; index += 1) {
        const result = event.results[index];
        if (result === undefined) continue;
        const words = result[0]?.transcript ?? "";
        if (result.isFinal) final += `${words.trim()} `;
        else pending += words;
      }
      finalRef.current = final;
      setFinalText(final);
      // The newest words are lighter until the recogniser settles them.
      setInterim(pending);
    };
    recognitionRef.current = recognition;
    if (!paused) {
      try {
        recognition.start();
      } catch {
        // A recogniser that refuses to start leaves the card usable: the words
        // can still be typed into the box behind it.
      }
    }
    return () => {
      recognition.abort();
      recognitionRef.current = null;
    };
    // The recogniser is created once; pause is handled by the effect below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const recognition = recognitionRef.current;
    if (recognition === null) return;
    if (paused) {
      recognition.stop();
    } else {
      try {
        recognition.start();
      } catch {
        // Already running: Radix-free browsers throw on a double start.
      }
    }
  }, [paused]);

  return (
    <div className="m-dsc-talk" data-testid="describe-talk-card">
      <div className="m-dsc-talk-head">
        <span className="m-dsc-chip m-dsc-chip-gold">
          <Icon name="mic" size={13} /> {paused ? "Paused" : "Listening"}
        </span>
        <span className="m-dsc-hint">Audio is turned into text, then discarded</span>
        <span className="m-dsc-talk-clock">{clock(seconds)}</span>
      </div>
      <div className="m-dsc-wave" aria-hidden="true">
        {Array.from({ length: 12 }, (_, index) => (
          <i key={index} />
        ))}
      </div>
      <p className="m-dsc-transcript" aria-live="polite" data-testid="describe-running-transcript">
        {finalText}
        <span className="m-dsc-dim">{interim}</span>
        {finalText === "" && interim === "" ? (
          <span className="m-dsc-dim">Say it the way you would tell a colleague.</span>
        ) : null}
      </p>
      <p className="m-dsc-hint">
        Say the client, what is included, quantities, prices, dates and terms. You can fix any word
        before I draft.
      </p>
      <div className="flex items-center gap-2">
        <Button size="sm" variant="secondary" onClick={() => setPaused((value) => !value)}>
          <Icon name={paused ? "play" : "pause"} size={15} />
          {paused ? "Resume" : "Pause"}
        </Button>
        <Button size="sm" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <span className="m-dsc-sp flex-1" />
        <Button
          size="sm"
          variant="primary"
          onClick={() => onStop(finalRef.current)}
          data-testid="describe-stop"
        >
          <Icon name="stop" size={15} /> Stop and review
        </Button>
      </div>
    </div>
  );
}

function clock(seconds: number): string {
  const minutes = Math.floor(seconds / 60)
    .toString()
    .padStart(2, "0");
  const rest = (seconds % 60).toString().padStart(2, "0");
  return `00:${minutes}:${rest}`;
}
