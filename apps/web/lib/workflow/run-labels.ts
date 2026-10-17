// Run-ledger vocabulary (Wave 43 lane 18, `p43-workflow-closeout-ui`).
//
// Purpose: one place turns the run ledger's two closed status vocabularies, its
// trigger vocabulary and its raw timestamps into the words, the pill tone and
// the one-line summaries the run list and the step trace render. Pure, so both
// surfaces and the unit tests read the same mapping and cannot drift.
//
// The tones are the kit's own Pill tones (`@marula-core/ui/kit`), never raw
// colours: a failed step is blush `attention`, because red is reserved for a
// real breach (docs/theme.md §6, design pack W06).
//
// Non-purpose: no fetching, no React, no status invention. Every status this
// module names is one the server can actually send.
import type { components } from "@marula-core/sdk";

export type WorkflowRunStatus = components["schemas"]["WorkflowRunStatus"];
export type WorkflowStepStatus = components["schemas"]["WorkflowStepStatus"];
export type WorkflowRunTrigger = components["schemas"]["WorkflowRunTrigger"];
export type RunView = components["schemas"]["RunView"];
export type RunStepView = components["schemas"]["RunStepView"];
export type RunDetailView = components["schemas"]["RunDetailView"];

/** The kit Pill tones this surface uses, as `@marula-core/ui/kit` names them. */
export type PillTone =
  | "progress"
  | "waiting"
  | "success"
  | "attention"
  | "neutral"
  | "gold";

export interface StatusMeta {
  label: string;
  tone: PillTone;
}

const RUN_STATUS: Record<WorkflowRunStatus, StatusMeta> = {
  running: { label: "Running", tone: "progress" },
  waiting: { label: "Waiting", tone: "waiting" },
  succeeded: { label: "Succeeded", tone: "success" },
  failed: { label: "Failed", tone: "attention" },
  abandoned: { label: "Abandoned", tone: "neutral" },
};

/** A step's own status, before the run's fate is taken into account. */
const STEP_STATUS: Record<WorkflowStepStatus, StatusMeta> = {
  running: { label: "Running", tone: "progress" },
  waiting: { label: "Waiting", tone: "waiting" },
  succeeded: { label: "Done", tone: "success" },
  failed: { label: "Failed", tone: "attention" },
  abandoned: { label: "Abandoned", tone: "neutral" },
  skipped: { label: "Skipped", tone: "neutral" },
};

export const RUN_STATUS_ORDER: WorkflowRunStatus[] = [
  "running",
  "waiting",
  "succeeded",
  "failed",
  "abandoned",
];

export function runStatusMeta(status: WorkflowRunStatus): StatusMeta {
  return RUN_STATUS[status] ?? { label: status, tone: "neutral" };
}

const RUN_TRIGGER_LABEL: Record<WorkflowRunTrigger, string> = {
  event: "From an event",
  resume: "Resumed",
  operator_retry: "Operator retry",
  manual: "Manual run",
};

export function triggerLabel(trigger: WorkflowRunTrigger): string {
  return RUN_TRIGGER_LABEL[trigger] ?? trigger;
}

/**
 * May an operator retry this run? Mirrors `RunStatus::is_retryable` on the
 * server: it stopped, and it did not stop clean. The server is the authority —
 * this only decides whether the button is offered, never whether the write is
 * allowed.
 */
export function isRetryable(status: WorkflowRunStatus): boolean {
  return status === "failed" || status === "abandoned";
}

export interface StepOutcome extends StatusMeta {
  /** True when the label came from the run's fate rather than the step's own status. */
  derived: boolean;
}

/**
 * How a step reads in the trace. A `skipped` step is two different facts and
 * the pack draws them apart (W06): the branch was not taken, or the run stopped
 * before reaching it. The server records both as `skipped`, so the distinction
 * is derived from the run's fate and the index of the step it failed at — stated
 * plainly, never hidden.
 */
export function stepOutcome(
  step: RunStepView,
  run: Pick<RunView, "status">,
  steps: RunStepView[],
): StepOutcome {
  if (step.status === "skipped") {
    const failedAt = firstFailedIndex(steps);
    if (run.status === "failed" && failedAt !== null && step.step_index > failedAt) {
      return { label: "Not reached", tone: "neutral", derived: true };
    }
    return { ...STEP_STATUS.skipped, derived: false };
  }
  return { ...(STEP_STATUS[step.status] ?? { label: step.status, tone: "neutral" as PillTone }), derived: false };
}

/** The index of the first failed step, or null when none failed. */
export function firstFailedIndex(steps: RunStepView[]): number | null {
  const failed = steps.find((s) => s.status === "failed");
  return failed ? failed.step_index : null;
}

/**
 * The pack's "Steps 1 to 6 finished" line above the trace: the count of steps
 * that reached a terminal, non-failed state before the trace's first break. A
 * run with no failure reports its finished tail honestly instead of inventing
 * the phrase.
 */
export function finishedPrefix(steps: RunStepView[], run: Pick<RunView, "status">): string | null {
  if (steps.length === 0) return null;
  const failedAt = firstFailedIndex(steps);
  if (failedAt === null) {
    if (run.status === "succeeded") {
      return steps.length === 1 ? "1 step finished" : `All ${steps.length} steps finished`;
    }
    return null;
  }
  const before = steps.filter((s) => s.step_index < failedAt).length;
  if (before === 0) return null;
  return before === 1 ? "Step 1 finished" : `Steps 1 to ${before} finished`;
}

// --- time ------------------------------------------------------------------

/** A compact duration: "0.5s", "1m 12s", "1h 04m". */
export function durationText(startIso: string, endIso: string | null | undefined): string {
  if (!endIso) return "still going";
  const ms = Date.parse(endIso) - Date.parse(startIso);
  if (!Number.isFinite(ms) || ms < 0) return "—";
  return spanText(ms);
}

/** The same shape from a raw millisecond span, so a live timer can share it. */
export function spanText(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;
  const minutes = Math.floor(seconds / 60);
  const remSeconds = Math.round(seconds % 60);
  if (minutes < 60) return `${minutes}m ${String(remSeconds).padStart(2, "0")}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${String(minutes % 60).padStart(2, "0")}m`;
}

/** "2 minutes ago" / "3 days ago" / "just now", from a fixed clock for tests. */
export function relativeTime(iso: string, nowMs: number = Date.now()): string {
  const diff = nowMs - Date.parse(iso);
  if (!Number.isFinite(diff)) return "—";
  if (diff < 45_000) return "just now";
  const minutes = Math.round(diff / 60_000);
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.round(hours / 24);
  return `${days} day${days === 1 ? "" : "s"} ago`;
}

/** A short id for a row label: the leading 8 hex characters, or the whole id. */
export function shortId(id: string): string {
  return id.length > 8 ? id.slice(0, 8) : id;
}

// --- structured output -----------------------------------------------------

/**
 * One honest line for a step's OUTPUT cell. A failed step names its typed code
 * and message; a succeeded step gets a compact read of whatever the node
 * returned; a step that did not run says so. Nothing is invented, and a payload
 * too long to read at a glance is cut with an ellipsis rather than silently
 * truncated.
 */
export function stepSummary(step: RunStepView, outcome: StepOutcome): string {
  if (step.status === "failed" || step.status === "abandoned") {
    const code = step.error_code ?? null;
    const message = step.error ?? "no detail was recorded";
    return code ? `${code}: ${message}` : message;
  }
  if (outcome.derived || step.status === "skipped") return "—";
  if (step.status === "running" || step.status === "waiting") return "—";
  return compactValue(step.output);
}

/** A compact, single-line read of a JSON value: scalars as-is, objects as pairs. */
export function compactValue(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "string") return clip(value);
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (Array.isArray(value)) {
    if (value.length === 0) return "—";
    return clip(value.map((v) => scalarText(v)).join(", "));
  }
  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>).filter(
      ([, v]) => v !== null && v !== undefined,
    );
    if (entries.length === 0) return "—";
    return clip(entries.map(([k, v]) => `${k} ${scalarText(v)}`).join(" · "));
  }
  return clip(String(value));
}

function scalarText(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (Array.isArray(value)) return `[${value.length}]`;
  if (typeof value === "object") return "{…}";
  return String(value);
}

function clip(text: string, max = 140): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

/** Pretty JSON for the detail panel's input/output blocks. */
export function prettyJson(value: unknown): string {
  if (value === null || value === undefined) return "—";
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}
