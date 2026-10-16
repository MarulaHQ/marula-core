"use client";
// /automations/runs — the workflow run ledger (Wave 43 lane 18,
// `p43-workflow-closeout-ui`).
//
// Purpose: the operator's read of what the workflows have been doing — one row
// per execution attempt, newest first, filterable by status and by workflow,
// each row opening the run's own trace. It is the twin of the webhook delivery
// ledger in /settings/integrations: same keyset cursor, same page shape, same
// empty-state honesty, so an operator who can read one can read the other.
//
// Non-purpose: no authoring (that is /automations), no webhook delivery UI
// (that already exists at /settings/integrations and this surface only points
// at it), and no invented totals — the list is paged, so the footer says how
// many rows are shown and nothing more.
import { Button, EmptyState, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Spinner, SurfaceHeader, Table, Td, Th, Tr, ViewSwitch } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  BlushNote,
  HEAD_CELL_CLASS,
  NoMatches,
  PageFrame,
  SkeletonRows,
} from "../../../components/knowledge/ledger";
import { api, useRequireAuth } from "../../../lib/api";
import { rejectionText } from "../../../lib/error-copy";
import {
  durationText,
  relativeTime,
  runStatusMeta,
  shortId,
  RUN_STATUS_ORDER,
  triggerLabel,
  type RunView,
  type WorkflowRunStatus,
} from "../../../lib/workflow/run-labels";
import { DotPill } from "./RunStatusPill";

type AutomationRuleRow = { id: string; name: string };

const PAGE_LIMIT = 50;

const STATUS_VIEWS = [
  { id: "all", label: "All runs" },
  ...RUN_STATUS_ORDER.map((status) => ({ id: status, label: runStatusMeta(status).label })),
];

function queryFor(status: string, ruleId: string | undefined, cursor: string | null) {
  return {
    ...(status === "all" ? {} : { status: status as WorkflowRunStatus }),
    ...(ruleId === undefined ? {} : { rule_id: ruleId }),
    ...(cursor === null ? {} : { cursor }),
    limit: PAGE_LIMIT,
  };
}

export default function RunLedger({ ruleId }: { ruleId?: string }) {
  const auth = useRequireAuth();
  const router = useRouter();

  const [runs, setRuns] = useState<RunView[] | null>(null);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [status, setStatus] = useState<string>("all");
  // Seeded from the URL so a link from a workflow row opens exactly that
  // workflow's history and can be shared. After that the control owns it, and
  // the address bar is kept in step on every change.
  const [ruleFilter, setRuleFilter] = useState<string | undefined>(ruleId);
  const [rules, setRules] = useState<AutomationRuleRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const loadFirst = useCallback(async () => {
    setLoading(true);
    setErrorMsg(null);
    const { data, error } = await api.api.GET("/api/v1/workflows/runs", {
      params: { query: queryFor(status, ruleFilter, null) as never },
    });
    setLoading(false);
    if (error !== undefined || data === undefined) {
      setRuns([]);
      setNextCursor(null);
      setErrorMsg(rejectionText(error as never, "The runs did not load. Try again."));
      return;
    }
    setRuns(data.runs);
    setNextCursor(data.next_cursor ?? null);
  }, [status, ruleFilter]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void loadFirst();
  }, [auth.status, loadFirst]);

  // The workflow filter's options: the org's rules. A failure here leaves the
  // filter empty rather than a half-populated control — the run list itself is
  // unaffected.
  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void (async () => {
      try {
        const { data } = await api.api.GET("/api/v1/automation-rules");
        if (cancelled) return;
        setRules((data as unknown as AutomationRuleRow[]) ?? []);
      } catch {
        if (!cancelled) setRules([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status]);

  const loadMore = async () => {
    if (nextCursor === null || loadingMore) return;
    setLoadingMore(true);
    const { data, error } = await api.api.GET("/api/v1/workflows/runs", {
      params: { query: queryFor(status, ruleFilter, nextCursor) as never },
    });
    setLoadingMore(false);
    if (error !== undefined || data === undefined) {
      setErrorMsg(rejectionText(error as never, "The next page of runs did not load."));
      return;
    }
    setRuns((cur) => [...(cur ?? []), ...data.runs]);
    setNextCursor(data.next_cursor ?? null);
  };

  const selectRule = (value: string) => {
    const next = value === "all" ? undefined : value;
    setRuleFilter(next);
    // Keep the address bar in step so the view is shareable and Back behaves.
    // The component does not re-read the prop, so this never refetches twice.
    router.replace(
      next === undefined ? "/automations/runs" : `/automations/runs?rule=${next}`,
      { scroll: false },
    );
  };

  const clearFilters = () => {
    setStatus("all");
    setRuleFilter(undefined);
    router.replace("/automations/runs", { scroll: false });
  };

  const ruleName = useMemo(
    () => rules.find((r) => r.id === ruleFilter)?.name ?? null,
    [rules, ruleFilter],
  );

  const filtered = status !== "all" || ruleFilter !== undefined;
  const shown = runs?.length ?? 0;

  return (
    <PageFrame>
      <SurfaceHeader
        eyebrow={
          <span className="flex items-center gap-1.5">
            <Link href="/automations" className="no-underline">
              Workflows
            </Link>
            <span aria-hidden="true">/</span>
            <span>Runs</span>
          </span>
        }
        title="Workflow runs"
        actions={
          <>
            <Button variant="secondary" asChild>
              <Link href="/settings/integrations">Webhook activity</Link>
            </Button>
            <Button variant="secondary" asChild>
              <Link href="/automations">Workflows</Link>
            </Button>
          </>
        }
      />

      {errorMsg !== null ? (
        <div className="mb-2" role="alert">
          <BlushNote title="The runs did not load.">
            <div className="flex items-center gap-3">
              <span>{errorMsg}</span>
              <Button size="sm" variant="secondary" onClick={() => void loadFirst()}>
                Try again
              </Button>
            </div>
          </BlushNote>
        </div>
      ) : null}

      <div className="mb-2 flex flex-wrap items-center gap-2">
        <ViewSwitch
          label="Run status"
          views={STATUS_VIEWS}
          activeId={status}
          onChange={(id) => setStatus(id)}
        />
        <div className="w-60">
          <Select value={ruleFilter ?? "all"} onValueChange={selectRule}>
            <SelectTrigger aria-label="Filter by workflow">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All workflows</SelectItem>
              {rules.map((r) => (
                <SelectItem key={r.id} value={r.id}>
                  {r.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      <div
        className="min-h-0 flex-1 overflow-auto rounded-[16px]"
        style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)" }}
      >
        {loading ? (
          <SkeletonRows rows={8} columns={5} sentence="Loading workflow runs…" />
        ) : shown === 0 ? (
          <RunsEmpty
            filtered={filtered}
            ruleName={ruleName}
            onClear={clearFilters}
          />
        ) : (
          <Table data-testid="workflow-runs-table">
            <thead>
              <tr>
                <Th className={HEAD_CELL_CLASS}>Workflow</Th>
                <Th className={HEAD_CELL_CLASS}>Status</Th>
                <Th className={HEAD_CELL_CLASS}>Trigger</Th>
                <Th className={HEAD_CELL_CLASS}>Started</Th>
                <Th className={HEAD_CELL_CLASS}>Duration</Th>
                <Th className={HEAD_CELL_CLASS}>Result</Th>
              </tr>
            </thead>
            <tbody>
              {runs!.map((run) => {
                const meta = runStatusMeta(run.status);
                const href = `/automations/runs/${run.id}`;
                return (
                  <Tr
                    key={run.id}
                    className="row-hover cursor-pointer"
                    data-testid={`workflow-run-row-${run.id}`}
                    onClick={() => router.push(href)}
                  >
                    <Td>
                      <Link
                        href={href}
                        className="no-underline"
                        aria-label={`Open run ${shortId(run.id)} of ${run.rule_name}`}
                      >
                        <span className="font-medium" style={{ color: "var(--m-text)" }}>
                          {run.rule_name}
                        </span>
                        <span
                          className="ml-2 font-mono text-[12px]"
                          style={{ color: "var(--m-text-muted)" }}
                        >
                          {shortId(run.id)}
                        </span>
                      </Link>
                    </Td>
                    <Td>
                      <span className="flex flex-wrap items-center gap-1">
                        <DotPill tone={meta.tone}>{meta.label}</DotPill>
                        {run.superseded ? <DotPill tone="neutral">Continued</DotPill> : null}
                        {run.retried ? <DotPill tone="neutral">Retried</DotPill> : null}
                      </span>
                    </Td>
                    <Td className="text-[13px]" >
                      <span style={{ color: "var(--m-text-secondary)" }}>{triggerLabel(run.trigger)}</span>
                    </Td>
                    <Td className="text-[13px]">
                      <span
                        className="tabular-nums"
                        style={{ color: "var(--m-text-secondary)" }}
                        title={new Date(run.started_at).toLocaleString()}
                      >
                        {relativeTime(run.started_at)}
                      </span>
                    </Td>
                    <Td className="text-[13px]">
                      <span className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                        {durationText(run.started_at, run.finished_at)}
                      </span>
                    </Td>
                    <Td className="max-w-[320px] text-[13px]">
                      {run.error_code !== null && run.error_code !== undefined ? (
                        <span className="font-mono" style={{ color: "var(--m-status-due-fg)" }}>
                          {run.error_code}
                        </span>
                      ) : run.error !== null && run.error !== undefined ? (
                        <span style={{ color: "var(--m-text-secondary)" }}>{run.error}</span>
                      ) : (
                        <span style={{ color: "var(--m-text-muted)" }}>—</span>
                      )}
                    </Td>
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}
      </div>

      <div
        className="flex min-h-[52px] shrink-0 flex-wrap items-center gap-3 pt-3 text-[13px]"
        style={{ color: "var(--m-text-secondary)" }}
      >
        <span>
          Showing{" "}
          <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
            {shown.toLocaleString()}
          </b>{" "}
          run{shown === 1 ? "" : "s"}
        </span>
        {nextCursor !== null ? (
          <Button size="sm" variant="secondary" onClick={() => void loadMore()} disabled={loadingMore}>
            {loadingMore ? <Spinner /> : null} Load more
          </Button>
        ) : null}
      </div>
    </PageFrame>
  );
}

/**
 * The empty card. Three different facts, three different sentences — there is
 * nothing yet, this workflow has never run, or the filters exclude everything.
 * None of them is a dead end, and none of them fakes a run.
 */
function RunsEmpty({
  filtered,
  ruleName,
  onClear,
}: {
  filtered: boolean;
  ruleName: string | null;
  onClear: () => void;
}) {
  if (filtered && ruleName !== null) {
    return (
      <div data-testid="workflow-runs-never-run">
        <EmptyState
          message={`${ruleName} has not run yet. It creates a run the first time its trigger fires.`}
          action={
            <Button asChild>
              <Link href="/automations">Open the workflow</Link>
            </Button>
          }
        />
      </div>
    );
  }
  if (filtered) {
    return (
      <NoMatches
        title="No runs match these filters."
        hint="Widen the status or pick another workflow."
        noun="runs"
        onClear={onClear}
      />
    );
  }
  return (
    <div data-testid="workflow-runs-empty">
      <EmptyState
        message="No workflow runs yet — a workflow creates one when its trigger fires."
        action={
          <Button asChild>
            <Link href="/automations">Set up a workflow</Link>
          </Button>
        }
      />
    </div>
  );
}
