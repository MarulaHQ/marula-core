"use client";

// Decision workbench suggestion card (Wave 23 lane 3, decision_workbench).
//
// Purpose: the inline AI suggestion card on the ticket detail workbench.
// Reads the newest recorded decision answers per surface for this ticket
// (GET /api/v1/tickets/{id}/decision-suggestions) and renders each
// answer with its threshold state — act (above the act threshold),
// pending (between act and review), ignore (below) — plus an honest
// shadow badge: shadow-mode rows drive nothing and say so.
//
// Posture: degrade-quiet. No answers ⇒ the card does not render at all
// (visibly-absent doctrine, never an empty shell or dead button).
// Suggestions are suggestion-grade: the card offers Apply only for
// answers above the act threshold; a pending suggestion shows its
// pending state and offers nothing. Applying rides the existing ticket
// PATCH (optimistic: local state hides the suggestion before the
// request settles; failure restores it).
//
// Design: reference pattern from the lazyweb Agentic Search (Hiver AI
// copilot inline card, Front suggestions, Intercom Fin — finalized
// agentic-search id cfce9bfc-fb3e-4559-ad3c-dfaa97c50231), rendered with
// Workbench tokens only; motion law: transform/opacity ≤120ms,
// prefers-reduced-motion honored (m-wb-slide-in).

import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";
import { resultErrorText } from "../../lib/error-copy";
import { announce } from "../a11y/live-region";

type SuggestionAnswer = {
  question_id: string;
  answer: {
    kind?: string;
    picked?: number;
    picked_label?: string;
    level?: number;
    probability?: number;
    confidence?: number;
    distribution?: number[];
  };
  act: number | null;
  review: number | null;
  state: "act" | "pending" | "ignore" | null;
};

type SuggestionSurface = {
  surface: string;
  mode: string;
  provider: string;
  model: string;
  recorded_at: string;
  answers: SuggestionAnswer[];
};

type DecisionSuggestions = {
  ticket_id: string;
  suggestions: SuggestionSurface[];
};

const QUESTION_LABELS: Record<string, string> = {
  actionable: "Actionable",
  urgency: "Urgency",
  frustrated: "Frustration",
  board: "Suggested board",
  queue: "Suggested queue",
  escalation_urgency: "Escalation urgency",
  severity: "Severity",
  coverage: "MSA coverage",
  after_hours_surcharge: "After-hours surcharge",
};

const STATE_TONES: Record<string, { bg: string; fg: string; label: string }> = {
  act: { bg: "var(--m-status-success-bg)", fg: "var(--m-status-success-fg)", label: "CONFIDENT" },
  pending: { bg: "var(--m-status-warning-bg)", fg: "var(--m-status-warning-fg)", label: "PENDING" },
  ignore: { bg: "var(--m-status-neutral-bg)", fg: "var(--m-status-neutral-fg)", label: "LOW SIGNAL" },
};

function answerLabel(answer: SuggestionAnswer): string {
  const a = answer.answer;
  if (a.kind === "choice" && a.picked_label != null && a.picked_label !== "") return a.picked_label;
  if (a.kind === "score" && a.level != null) return `Level ${a.level + 1} of 5`;
  if (a.kind === "noul" && a.probability != null) return `${Math.round(a.probability * 100)}% likely`;
  if (a.confidence != null) return `${Math.round(a.confidence * 100)}%`;
  return "—";
}

function confidenceOf(answer: SuggestionAnswer): number | null {
  if (answer.answer.kind === "noul" && answer.answer.probability != null) return answer.answer.probability;
  return answer.answer.confidence ?? null;
}

function StateBadge({ state, mode }: { state: string | null; mode: string }) {
  const tone = STATE_TONES[state ?? "ignore"] ?? {
    bg: "var(--m-status-neutral-bg)",
    fg: "var(--m-status-neutral-fg)",
    label: "LOW SIGNAL",
  };
  return (
    <span
      data-testid={state != null ? `wb-decision-state-${state}` : undefined}
      className="inline-flex h-[18px] shrink-0 items-center rounded-full px-2 text-[9px] font-bold tracking-wide"
      style={{ background: tone.bg, color: tone.fg }}
    >
      {mode === "shadow" ? "SHADOW" : tone.label}
    </span>
  );
}

export function DecisionSuggestionsCard({
  ticketId,
  onApplied,
}: {
  ticketId: string;
  onApplied?: () => void;
}) {
  const [data, setData] = useState<DecisionSuggestions | null>(null);
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(new Set());
  const [applying, setApplying] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    const res = await api.api.GET("/api/v1/tickets/{id}/decision-suggestions", {
      params: { path: { id: ticketId } },
    });
    if (res.data !== undefined) {
      setData(res.data as unknown as DecisionSuggestions);
      setError(null);
      const fresh = (res.data as unknown as DecisionSuggestions).suggestions?.length ?? 0;
      if (fresh > 0) {
        announce(`${fresh} AI suggestion${fresh === 1 ? "" : "s"} ready for review`, "polite");
      }
    } else {
      setError(resultErrorText(res as never, "Couldn't load AI suggestions."));
    }
  }, [ticketId]);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Optimistic dismissal: hide instantly; nothing to settle server-side
  // (suggestions are append-only audit rows — dismissal is local view state).
  const dismiss = (surface: string) => {
    setDismissed((prev) => new Set(prev).add(surface));
  };

  // Optimistic board apply: hide the suggestion instantly, settle the
  // board move in the background; a failure restores the card view.
  const applyBoard = async (surface: string, answer: SuggestionAnswer) => {
    const label = answer.answer.picked_label ?? "";
    setApplying(surface);
    setDismissed((prev) => new Set(prev).add(surface));
    const nav = await api.api.GET("/api/v1/boards/nav");
    const board = (nav.data ?? []).find((b) => (b as { name?: string }).name === label);
    if (board == null) {
      setApplying(null);
      setDismissed((prev) => {
        const next = new Set(prev);
        next.delete(surface);
        return next;
      });
      setError("Couldn't resolve the suggested board.");
      return;
    }
    const res = await api.api.PATCH("/api/v1/tickets/{id}", {
      params: { path: { id: ticketId } },
      body: { board_id: board.id } as never,
    });
    setApplying(null);
    if (res.data !== undefined) onApplied?.();
    else {
      setDismissed((prev) => {
        const next = new Set(prev);
        next.delete(surface);
        return next;
      });
      setError(resultErrorText(res as never, "Applying the board suggestion failed."));
    }
  };

  if (data == null) return null;
  const visible = data.suggestions.filter(
    (s) => s.answers.length > 0 && !dismissed.has(s.surface),
  );
  if (visible.length === 0) return null;

  return (
    <aside
      data-testid="wb-decision-card"
      aria-label="AI triage suggestions"
      className="rounded-xl border p-3"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
    >
      <div className="mb-1.5 flex items-center justify-between gap-2">
        <h2
          className="text-[10px] font-bold uppercase tracking-wide"
          style={{ color: "var(--m-text-muted)" }}
        >
          AI triage suggestions
        </h2>
        <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }} title={visible[0]?.model}>
          {visible[0]?.model}
        </span>
      </div>
      {error != null && (
        <p role="alert" className="mb-1.5 text-[11px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      )}
      <div className="grid gap-1.5">
        {visible.flatMap((surface) =>
          surface.answers.map((answer) => {
            const confidence = confidenceOf(answer);
            const canApply =
              surface.mode === "live" &&
              answer.state === "act" &&
              (answer.question_id === "board" || answer.question_id === "queue");
            return (
              <div
                key={`${surface.surface}-${answer.question_id}`}
                className="flex items-center justify-between gap-2 rounded-lg px-2.5 py-1.5"
                style={{ border: "1px solid var(--m-border)" }}
              >
                <div className="min-w-0">
                  <p className="truncate text-[12px] font-semibold" style={{ color: "var(--m-text)" }}>
                    {QUESTION_LABELS[answer.question_id] ?? answer.question_id}:{" "}
                    {answerLabel(answer)}
                  </p>
                  {confidence != null && (
                    <p className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                      {Math.round(confidence * 100)}% confidence
                    </p>
                  )}
                </div>
                <div className="flex shrink-0 items-center gap-1.5">
                  <StateBadge state={answer.state} mode={surface.mode} />
                  {canApply && (
                    <button
                      type="button"
                      disabled={applying === surface.surface}
                      data-testid={`wb-decision-apply-${answer.question_id}`}
                      onClick={() => void applyBoard(surface.surface, answer)}
                      className="h-[22px] cursor-pointer rounded-lg border px-2 text-[10px] font-bold disabled:cursor-wait"
                      style={{
                        borderColor: "var(--m-accent)",
                        background: "var(--m-accent)",
                        color: "var(--m-on-accent)",
                      }}
                    >
                      {applying === surface.surface ? "Applying…" : "Apply"}
                    </button>
                  )}
                  {answer.state !== "act" && surface.mode === "live" && (
                    <button
                      type="button"
                      onClick={() => dismiss(surface.surface)}
                      className="h-[22px] cursor-pointer rounded-lg border px-2 text-[10px] font-semibold"
                      style={{
                        borderColor: "var(--m-border)",
                        background: "var(--m-surface)",
                        color: "var(--m-text-muted)",
                      }}
                    >
                      Dismiss
                    </button>
                  )}
                </div>
              </div>
            );
          }),
        )}
      </div>
    </aside>
  );
}
