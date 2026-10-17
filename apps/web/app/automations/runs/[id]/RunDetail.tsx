"use client";
// /automations/runs/[id] — one run, its whole trace, and the retry
// (Wave 43 lane 18, `p43-workflow-closeout-ui`).
//
// Purpose: make a run explain itself. The header says what ran and how it
// ended; the trace lists every step attempt in order with its result, its time
// and its output; the side panel shows the selected step's input, output and
// error in full. A failed run keeps what already applied and offers the
// operator action the ledger was built for — retry from the step that broke, or
// retry the whole document — and reports the server's typed refusal verbatim
// when a repeat would double-apply.
//
// The design of record is the committed pack's run drawer (W05 in progress /
// W06 failed at a step): a status strip over the trace, the finished-prefix
// line above the steps, blush for a failure (red stays reserved for a real
// breach), and the two retry verbs exactly as the executor defines them
// (`resume` and `from_start`).
//
// Non-purpose: no execution and no step writes (the executor owns both), no
// editing the workflow, and no retry offered on a run the server would refuse.
import {
  Button,
  EmptyState,
  Spinner,
  SurfaceHeader,
  Table,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  BlushNote,
  HEAD_CELL_CLASS,
  PageFrame,
  SELECTED_ROW_STYLE,
  SkeletonRows,
} from "../../../../components/knowledge/ledger";
import { api, useRequireAuth } from "../../../../lib/api";
import { envelopeOf, rejectionText } from "../../../../lib/error-copy";
import {
  durationText,
  finishedPrefix,
  isRetryable,
  prettyJson,
  relativeTime,
  runStatusMeta,
  shortId,
  stepOutcome,
  stepSummary,
  triggerLabel,
  type RunDetailView,
  type RunStepView,
} from "../../../../lib/workflow/run-labels";
import { DotPill } from "../RunStatusPill";

type RetryMode = components["schemas"]["WorkflowRetryMode"];

function idempotencyKey(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return `run-retry-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export default function RunDetail({ runId }: { runId: string }) {
  const auth = useRequireAuth();
  const router = useRouter();

  const [detail, setDetail] = useState<RunDetailView | null>(null);
  const [loading, setLoading] = useState(true);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [notFound, setNotFound] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [retryBusy, setRetryBusy] = useState<RetryMode | null>(null);
  const [refusal, setRefusal] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setErrorMsg(null);
    setNotFound(false);
    const { data, error } = await api.api.GET("/api/v1/workflows/runs/{id}", {
      params: { path: { id: runId } },
    });
    setLoading(false);
    if (error !== undefined || data === undefined) {
      // The house pattern: the SDK carries the typed envelope under
      // `error.error` (bare or parked) and the HTTP status at the top level.
      // Reading `error.response.status` silently missed, which is why the
      // not-found surface never rendered.
      const env = envelopeOf(error);
      const status = (error as { status?: number } | null | undefined)?.status;
      if (status === 404 || env.reason === "not_found" || env.reason === "rls_sparse") {
        setNotFound(true);
        return;
      }
      setErrorMsg(rejectionText(env, "The run did not load. Try again."));
      return;
    }
    const view = data as RunDetailView;
    setDetail(view);
    // Start on the step the operator came for: the failure, else the parked
    // step, else the last one that ran.
    const steps = view.steps ?? [];
    const focus =
      steps.find((s) => s.status === "failed") ??
      steps.find((s) => s.status === "waiting") ??
      steps.find((s) => s.status === "running") ??
      steps[steps.length - 1] ??
      null;
    setSelectedId(focus ? focus.id : null);
  }, [runId]);

  useEffect(() => {
    if (auth.status === "authed") void load();
  }, [auth.status, load]);

  const steps = useMemo(() => detail?.steps ?? [], [detail]);
  const selected = useMemo(
    () => steps.find((s) => s.id === selectedId) ?? null,
    [steps, selectedId],
  );

  const retry = async (mode: RetryMode) => {
    if (detail === null) return;
    setRetryBusy(mode);
    setRefusal(null);
    const { data, error } = await api.api.POST("/api/v1/workflows/runs/{id}/retry", {
      params: { path: { id: detail.id } },
      body: { mode, idempotency_key: idempotencyKey() } as never,
    });
    setRetryBusy(null);
    if (error !== undefined || data === undefined) {
      // The refusal is the point of the typed 409: it names every node that
      // would double-apply. Shown as the server wrote it, never paraphrased.
      const reason = (error as { error?: { reason?: string; message?: string } } | undefined)?.error;
      if (reason?.reason === "run_retry_refused" && typeof reason.message === "string") {
        setRefusal(reason.message);
        return;
      }
      setRefusal(rejectionText(envelopeOf(error), "The retry was refused."));
      return;
    }
    // The retry is a NEW run row; the failed one is untouched. Go to it.
    router.push(`/automations/runs/${data.run.id}`);
  };

  if (loading) {
    return (
      <PageFrame>
        <SurfaceHeader eyebrow="Workflows / Runs" title="Run" />
        <div
          className="min-h-0 flex-1 overflow-auto rounded-[16px]"
          style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)" }}
        >
          <SkeletonRows rows={6} columns={4} sentence="Loading the run and its trace…" />
        </div>
      </PageFrame>
    );
  }

  if (notFound) {
    return (
      <PageFrame>
        <SurfaceHeader eyebrow="Workflows / Runs" title="Run not found" />
        <div role="alert">
          <BlushNote title="That run is not visible in your workspace.">
            <p className="mt-1">
              It may have aged out of the retention window, or it belongs to a workflow you cannot
              see.
            </p>
            <Button className="mt-2" size="sm" variant="secondary" asChild>
              <Link href="/automations/runs">Back to runs</Link>
            </Button>
          </BlushNote>
        </div>
      </PageFrame>
    );
  }

  if (detail === null) {
    return (
      <PageFrame>
        <SurfaceHeader eyebrow="Workflows / Runs" title="Run" />
        <div role="alert">
          <BlushNote title="The run did not load.">
            <div className="flex items-center gap-3">
              <span>{errorMsg ?? "Try again."}</span>
              <Button size="sm" variant="secondary" onClick={() => void load()}>
                Try again
              </Button>
            </div>
          </BlushNote>
        </div>
      </PageFrame>
    );
  }

  const run = detail;
  const meta = runStatusMeta(run.status);
  const retryable = isRetryable(run.status);
  const prefix = finishedPrefix(steps, run);

  return (
    <PageFrame>
      <SurfaceHeader
        eyebrow={
          <span className="flex items-center gap-1.5">
            <Link href="/automations" className="no-underline">
              Workflows
            </Link>
            <span aria-hidden="true">/</span>
            <Link href="/automations/runs" className="no-underline">
              Runs
            </Link>
            <span aria-hidden="true">/</span>
            <span className="font-mono">{shortId(run.id)}</span>
          </span>
        }
        title={run.rule_name}
        meta={
          <>
            <DotPill tone={meta.tone}>{meta.label}</DotPill>
            <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              version {run.rule_version}
            </span>
            <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              {triggerLabel(run.trigger)}
            </span>
            <span className="text-[13px] tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
              {durationText(run.started_at, run.finished_at)}
            </span>
          </>
        }
        actions={
          <Button variant="secondary" asChild>
            <Link href="/automations/runs">Back to runs</Link>
          </Button>
        }
      />

      <RunBanner run={run} steps={steps} />

      {run.retry_of != null || run.resumed_from != null ? (
        <p className="mb-2 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {run.retry_of != null ? (
            <>
              This run retries{" "}
              <Link href={`/automations/runs/${run.retry_of}`} className="font-mono">
                {shortId(run.retry_of)}
              </Link>
              .
            </>
          ) : null}
          {run.retry_of != null && run.resumed_from != null ? " " : null}
          {run.resumed_from != null ? (
            <>
              This run continues{" "}
              <Link href={`/automations/runs/${run.resumed_from}`} className="font-mono">
                {shortId(run.resumed_from)}
              </Link>
              .
            </>
          ) : null}
        </p>
      ) : null}

      {refusal !== null ? (
        <div className="mb-2" role="alert">
          <BlushNote title="The retry was refused.">{refusal}</BlushNote>
        </div>
      ) : null}

      <div className="grid min-h-0 flex-1 gap-3 lg:grid-cols-[minmax(0,1fr)_380px]">
        {/* The trace */}
        <div
          className="min-h-0 overflow-auto rounded-[16px]"
          style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)" }}
        >
          {steps.length === 0 ? (
            <div data-testid="run-steps-empty">
              <EmptyState message="No steps recorded yet — this run has not started executing." />
            </div>
          ) : (
            <>
              {prefix !== null ? (
                <p className="px-3 pt-3 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  {prefix}
                </p>
              ) : null}
              <Table data-testid="run-steps-table">
                <thead>
                  <tr>
                    <Th className={HEAD_CELL_CLASS}>Step</Th>
                    <Th className={HEAD_CELL_CLASS}>Result</Th>
                    <Th className={HEAD_CELL_CLASS}>Time</Th>
                    <Th className={HEAD_CELL_CLASS}>Output</Th>
                  </tr>
                </thead>
                <tbody>
                  {steps.map((step) => {
                    const outcome = stepOutcome(step, run, steps);
                    const isSelected = step.id === selectedId;
                    return (
                      <Tr
                        key={step.id}
                        className="row-hover cursor-pointer"
                        style={isSelected ? SELECTED_ROW_STYLE : undefined}
                        aria-selected={isSelected}
                        data-testid={`run-step-row-${step.step_index}`}
                        onClick={() => setSelectedId(step.id)}
                      >
                        <Td className="text-[13px]">
                          <span
                            className="tabular-nums"
                            style={{ color: "var(--m-text-muted)" }}
                          >
                            {step.step_index}.
                          </span>{" "}
                          <span className="font-medium">{step.node_label}</span>
                          {step.attempt > 1 ? (
                            <span
                              className="ml-2 text-[12px]"
                              style={{ color: "var(--m-text-muted)" }}
                            >
                              attempt {step.attempt}
                            </span>
                          ) : null}
                        </Td>
                        <Td>
                          <DotPill tone={outcome.tone}>{outcome.label}</DotPill>
                        </Td>
                        <Td className="text-[13px] tabular-nums">
                          <span style={{ color: "var(--m-text-secondary)" }}>
                            {stepStepTime(step)}
                          </span>
                        </Td>
                        <Td className="max-w-[320px] text-[13px]">
                          <span
                            className={step.status === "failed" ? "font-mono" : ""}
                            style={{
                              color:
                                step.status === "failed"
                                  ? "var(--m-status-due-fg)"
                                  : "var(--m-text-secondary)",
                            }}
                          >
                            {stepSummary(step, outcome)}
                          </span>
                        </Td>
                      </Tr>
                    );
                  })}
                </tbody>
              </Table>
            </>
          )}
        </div>

        {/* The selected step's detail and the operator action */}
        <aside
          key={selected?.id ?? "none"}
          className="min-h-0 overflow-auto rounded-[16px] p-4 motion-reduce:animate-none animate-[m-rise_120ms_cubic-bezier(0.2,0,0,1)]"
          style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)" }}
          aria-live="polite"
        >
          {selected === null ? (
            <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              Select a step to read its input, output and timing.
            </p>
          ) : (
            <StepPanel
              step={selected}
              stepCount={steps.length}
              run={run}
              steps={steps}
              retryable={retryable}
              retryBusy={retryBusy}
              onRetry={retry}
            />
          )}
        </aside>
      </div>
    </PageFrame>
  );
}

function stepStepTime(step: RunStepView): string {
  if (step.finished_at == null) {
    if (step.status === "running") return "running";
    if (step.status === "waiting") return "waiting";
    return "—";
  }
  return durationText(step.started_at, step.finished_at);
}

/**
 * The status strip over the trace. Three honest states: a failure (blush) says
 * where it broke and what is kept; a parked run (gold) says what it waits on;
 * a running run (neutral) says it is going. Nothing is rendered for a clean
 * finish — the status chip and the trace already say that.
 */
function RunBanner({ run, steps }: { run: RunDetailView; steps: RunStepView[] }) {
  if (run.status === "failed") {
    const failed = steps.find((s) => s.status === "failed") ?? null;
    // Steps are 0-indexed; the kept prefix is every step BEFORE the failure, so
    // the count is how many of them sort earlier — never `step_index - 1`, which
    // reads as "nothing was kept" on the first break after step 1.
    const kept = failed ? steps.filter((s) => s.step_index < failed.step_index).length : steps.length;
    return (
      <div
        className="mb-2 flex flex-wrap items-center gap-2 rounded-[12px] px-3 py-2 text-[13px]"
        style={{ background: "var(--m-status-due-bg)", color: "var(--m-status-due-fg)" }}
        role="status"
        data-testid="run-banner"
      >
        <span>
          Run failed{failed ? ` at ${failed.node_label}` : ""}.
          {kept === 0
            ? " No step completed."
            : kept === 1
              ? " Step 1 is kept."
              : ` Steps 1 to ${kept} are kept.`}
        </span>
        {run.error_code != null || run.error != null ? (
          <span className="font-mono">
            {run.error_code ?? ""}
            {run.error_code != null && run.error != null ? ": " : ""}
            {run.error ?? ""}
          </span>
        ) : null}
      </div>
    );
  }
  if (run.status === "waiting") {
    return (
      <div
        className="mb-2 flex flex-wrap items-center gap-2 rounded-[12px] px-3 py-2 text-[13px]"
        // The status "waiting" pair, the same one the Waiting pill wears. It
        // flips per mode (7.37:1 light, 8.06:1 dark); the fixed --m-gold-* pair
        // it replaced measured 4.26:1 in both, below AA at this size, and read
        // as a bright strip in dark.
        style={{ background: "var(--mc-st-wait)", color: "var(--mc-st-wait-fg)" }}
        role="status"
        data-testid="run-banner"
      >
        <span>
          Waiting{run.wake_on != null ? ` for ${run.wake_on}` : ""}
          {run.wake_at != null ? ` until ${new Date(run.wake_at).toLocaleString()}` : ""}.
        </span>
        <span>The wait is stored, not held in memory — it resumes when the event arrives.</span>
      </div>
    );
  }
  if (run.status === "running") {
    return (
      <div
        className="mb-2 rounded-[12px] px-3 py-2 text-[13px]"
        style={{ background: "var(--m-surface-2)", color: "var(--m-text-secondary)" }}
        role="status"
        data-testid="run-banner"
      >
        Running now — this view is a snapshot. Reopen it for the latest step.
      </div>
    );
  }
  return null;
}

function StepPanel({
  step,
  stepCount,
  run,
  steps,
  retryable,
  retryBusy,
  onRetry,
}: {
  step: RunStepView;
  stepCount: number;
  run: RunDetailView;
  steps: RunStepView[];
  retryable: boolean;
  retryBusy: RetryMode | null;
  onRetry: (mode: RetryMode) => void;
}) {
  const outcome = stepOutcome(step, run, steps);
  const failed = step.status === "failed";
  // `min-w-0` on every child: a grid item defaults to `min-width: auto`, so a
  // long JSON line or a sentence in a 380px panel widens its own track and the
  // text is clipped at the panel edge instead of wrapping or scrolling in place.
  return (
    <div className="grid min-w-0 gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-semibold">{step.node_label}</span>
        <DotPill tone={outcome.tone}>{outcome.label}</DotPill>
        <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
          step {step.step_index} of {stepCount}
          {step.attempt > 1 ? ` · attempt ${step.attempt}` : ""}
          {step.iteration != null ? ` · iteration ${step.iteration}` : ""}
        </span>
      </div>

      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-[12px]">
        <dt style={{ color: "var(--m-text-muted)" }}>Node</dt>
        <dd className="font-mono">{step.node_id}</dd>
        <dt style={{ color: "var(--m-text-muted)" }}>Started</dt>
        <dd className="tabular-nums" title={new Date(step.started_at).toLocaleString()}>
          {relativeTime(step.started_at)}
        </dd>
        <dt style={{ color: "var(--m-text-muted)" }}>Time</dt>
        <dd className="tabular-nums">{stepStepTime(step)}</dd>
      </dl>

      {failed || step.status === "abandoned" ? (
        <div
          className="rounded-[12px] px-3 py-2 text-[12px]"
          style={{ background: "var(--m-status-due-bg)", color: "var(--m-status-due-fg)" }}
        >
          {step.error_code != null ? <div className="font-mono">{step.error_code}</div> : null}
          <div>{step.error ?? "No detail was recorded for this failure."}</div>
        </div>
      ) : null}

      <JsonBlock label="Input" value={step.input} />
      {step.output != null ? <JsonBlock label="Output" value={step.output} /> : null}

      {retryable && failed ? (
        <div className="grid min-w-0 gap-2 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
          <Button
            onClick={() => onRetry("resume")}
            disabled={retryBusy !== null}
            data-testid="run-retry-resume"
          >
            {retryBusy === "resume" ? <Spinner /> : null} Retry from this step
          </Button>
          <Button
            variant="secondary"
            onClick={() => onRetry("from_start")}
            disabled={retryBusy !== null}
            data-testid="run-retry-from-start"
          >
            {retryBusy === "from_start" ? <Spinner /> : null} Retry whole run
          </Button>
          <p className="min-w-0 break-words text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            Steps before this one are kept. The retry is recorded as a new run; this one keeps
            saying failed. A repeat that would apply something twice is refused.
          </p>
        </div>
      ) : null}
    </div>
  );
}

function JsonBlock({ label, value }: { label: string; value: unknown }) {
  return (
    <div className="min-w-0">
      <div
        className="mb-1 text-[12px] font-semibold uppercase tracking-[0.06em]"
        style={{ color: "var(--m-text-muted)" }}
      >
        {label}
      </div>
      <pre
        className="m-0 max-h-64 max-w-full overflow-y-auto whitespace-pre-wrap break-words rounded-[10px] p-2 font-mono text-[12px]"
        style={{ background: "var(--m-surface-2)", color: "var(--m-text)" }}
      >
        {prettyJson(value)}
      </pre>
    </div>
  );
}
