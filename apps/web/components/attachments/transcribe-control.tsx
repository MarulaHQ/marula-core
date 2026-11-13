// Transcription UI (Wave 22): the ADR 0008 `transcription` plugin
// surface made actionable on audio attachments. Typed honest states —
// the availability posture comes from the SERVER refusals (typed reasons
// carry the "why not"), so the button renders and decline states are
// explicit, never fabricated transcripts.
"use client";

import { Button, Spinner } from "@marula-core/ui";
import { useEffect, useState } from "react";
import type { components } from "@marula-core/sdk";
import { api } from "../../lib/api";
import { reasonCopy } from "../../lib/error-copy";

type TranscriptionResponse = components["schemas"]["TranscriptionResponse"];

const TYPED_STATES: Record<string, string> = {
  ai_features_disabled: "AI features are switched off for this organization — transcription stays silent until an admin switches them on.",
  ai_surface_disabled: "The transcription surface is off for this workspace.",
  ai_surface_not_granted: "No grant covers you on the transcription surface (deny-by-default) — ask an admin to grant it.",
  ai_surface_internal: "The transcription surface is internal-only.",
  ai_no_credential: "No transcription provider credential is configured yet.",
  ai_hard_cap_reached: "Your usage cap is binding — transcription is unavailable until the cap window resets.",
  ai_model_not_allowed: "The allowlist has no transcription-capable model yet.",
  ai_model_not_capable: "No transcription endpoint is wired for the configured provider.",
  ai_provider_error: "The transcription provider refused or failed this call.",
  not_audio: "Only audio attachments can be transcribed.",
};

export function isTranscribableAudio(contentType: string | undefined | null): boolean {
  if (!contentType) return false;
  const base = contentType.split(";")[0]?.trim() ?? "";
  return [
    "audio/wav",
    "audio/wave",
    "audio/x-wav",
    "audio/webm",
    "audio/mpeg",
    "audio/mp3",
    "audio/mp4",
    "audio/x-m4a",
    "audio/ogg",
    "audio/flac",
  ].includes(base);
}

export function TranscribeControl({ attachmentId, contentType }: { attachmentId: string; contentType: string }) {
  const [busy, setBusy] = useState(false);
  const [state, setState] = useState<string | null>(null);
  const [result, setResult] = useState<TranscriptionResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setBusy(true);
    setError(null);
    setResult(null);
    const { data, error: postError, response } = await api.api.POST(
      "/api/v1/attachments/{id}/transcribe",
      { params: { path: { id: attachmentId } } },
    );
    setBusy(false);
    if (postError !== undefined || data === undefined) {
      const envelope = (postError as { error?: { reason?: string; message?: string } } | undefined)?.error;
      const reason = envelope?.reason ?? `http_${response?.status ?? 0}`;
      setState(reason);
      setError(
        TYPED_STATES[reason] ??
          reasonCopy(reason, envelope) ??
          envelope?.message ??
          "The transcription call was refused.",
      );
      return;
    }
    setResult(data);
    setState("done");
  };

  if (!isTranscribableAudio(contentType)) return null;

  if (state === "done" && result !== null) {
    return (
      <div className="grid gap-1 rounded-[var(--m-radius-container)] border p-2" style={{ borderColor: "var(--m-border)" }} data-testid={`transcript-${attachmentId}`}>
        <span className="text-[11px] font-semibold" style={{ color: "var(--m-text-secondary)" }}>
          Transcript · {result.provider}/{result.model}
        </span>
        <p className="text-[12px]" style={{ color: "var(--m-text)" }}>{result.text}</p>
      </div>
    );
  }

  return (
    <div className="grid gap-1">
      <Button size="sm" variant="ghost" onClick={() => void run()} disabled={busy} data-testid={`transcribe-${attachmentId}`}>
        {busy ? <Spinner /> : null} Transcribe
      </Button>
      {error !== null ? (
        <p
          role="alert"
          className="text-[11px]"
          // A refused transcription is error copy, not a muted hint (§3.4).
          style={{ color: "var(--m-status-danger-fg)" }}
          data-testid={`transcribe-state-${attachmentId}`}
        >
          {error}
        </p>
      ) : null}
    </div>
  );
}
