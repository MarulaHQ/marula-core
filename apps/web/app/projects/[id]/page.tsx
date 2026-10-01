// /projects/[id] — one engagement's delivery surface (Wave 17 lane 4 UX
// pass; Wave 41 lane 7 `p41-ui-clients` reshaped it to the pack's record
// template).
//
// Identity header, glanceable roll-up strip (phases / tasks / milestones +
// schedule-risk counts), a sequence rail over the phases (gantt: the dated
// timeline is <ProjectGantt/>, mounted below — phases carry nullable
// start/end dates since Wave 34, and the timeline draws only the ones that
// HAVE both. The sequence rail stays, because an undated phase is still an
// ordered stage).
//
// gantt-lite: phases are ordered stages, not dated bars — the schema has
// sort_order + status, no dates), phase/task groups with per-phase
// progress, a needs-attention triage filter (overdue / due-soon / blocked),
// and the finish-to-start dependency editor (PUT full set).
// Mutations keep the IF1–IF3 envelope with optimistic status reconcile.
// Non-purpose: no budget burn EDITING here, no ticket conversion.
//
// **What Wave 41 changed** (the frame and the signals, not the behaviour):
//   1. The page is now the pack's three-column RECORD shape — the client and
//      the owner on the left, the delivery surface in the middle, what needs
//      attention on the right — reusing the skeleton the client record
//      established rather than inventing a second one for projects.
//   2. **At risk is BLUSH; overdue is RED only once the due date has actually
//      passed** — the existing `projectRisk` already split these, and the
//      left column now shows both sides with their words, never colour alone.
//   3. The right column carries a **budget bar on the Dispatch utilisation
//      bar's language** (flat 1px track, blush overflow), fed by the real
//      cost-rollup — one bar rule transferred rather than a new one invented.
//
// Every existing capability — the roll-up strip, the phase rail, the gantt,
// phase-grouped tasks with blocked-by chips, the attention filter, the
// dependency editor and the optimistic complete-task envelope — is kept
// exactly as it was. A visual rebrand does not remove function.
"use client";

import { rejectionText } from "../../../lib/error-copy";
import {
  blockedTasks,
  dueSoonTasks,
  overdueTasks,
  phaseProgress,
  projectRisk,
  projectStatusTone,
  taskOpenness,
  taskProgress,
} from "../../../lib/projects-metrics";
import {
  Button,
  Card,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Tag,
} from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { ProjectGantt } from "../../../components/project-gantt";
import { dueLabel, money } from "../../../lib/format";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import {
  RecordFrame,
  RecordRow,
  RecordSection,
  SummaryCard,
} from "../../../components/records/record-frame";

type Project = components["schemas"]["Project"];
type ProjectPhase = components["schemas"]["ProjectPhase"];
type ProjectTask = components["schemas"]["ProjectTask"];
type Client = components["schemas"]["Client"];
type ProjectMutationResponse = components["schemas"]["ProjectMutationResponse"];
type PhaseMutationResponse = components["schemas"]["PhaseMutationResponse"];
type TaskMutationResponse = components["schemas"]["TaskMutationResponse"];
type DependencyListResponse = components["schemas"]["DependencyListResponse"];
type Contract = components["schemas"]["Contract"];
type PhaseBurn = components["schemas"]["PhaseBurn"];
type BillingRelease = components["schemas"]["ProjectBillingRelease"];

const STATUS_LABEL: Record<string, string> = {
  planning: "Planning",
  active: "Active",
  on_hold: "On hold",
  completed: "Completed",
  cancelled: "Cancelled",
  pending: "Pending",
};

export default function ProjectDetailPage() {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const projectId = params.id;

  const [project, setProject] = useState<Project | null>(null);
  const [client, setClient] = useState<Client | null>(null);
  const [phases, setPhases] = useState<ProjectPhase[]>([]);
  const [tasks, setTasks] = useState<ProjectTask[]>([]);
  const [deps, setDeps] = useState<Record<string, ProjectTask[]>>({});
  const [rollup, setRollup] = useState<components["schemas"]["ProjectCostRollupView"] | null>(null);
  // Wave 39 lane 4's billing reads: per-phase burn against the linked
  // contract's rate card, and the release ledger of what has been handed
  // to invoicing. Each answers 403 without financial:read, so the card
  // renders empty rather than the page failing.
  const [burn, setBurn] = useState<PhaseBurn[]>([]);
  const [releases, setReleases] = useState<BillingRelease[]>([]);
  const [contracts, setContracts] = useState<Contract[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [attentionOnly, setAttentionOnly] = useState(false);

  const [phaseOpen, setPhaseOpen] = useState(false);
  const [phaseName, setPhaseName] = useState("");
  const [phaseMilestone, setPhaseMilestone] = useState(false);
  const [phaseAmount, setPhaseAmount] = useState("");
  const [phaseBudget, setPhaseBudget] = useState("");
  const [phaseFee, setPhaseFee] = useState("");

  const [taskOpen, setTaskOpen] = useState(false);
  const [taskTitle, setTaskTitle] = useState("");
  const [taskPhase, setTaskPhase] = useState<string>("none");
  const [taskMilestone, setTaskMilestone] = useState(false);
  const [taskAmount, setTaskAmount] = useState("");
  const [taskDependsOn, setTaskDependsOn] = useState<Set<string>>(new Set());

  const [depTaskId, setDepTaskId] = useState<ProjectTask | null>(null);
  const [depSelection, setDepSelection] = useState<Set<string>>(new Set());

  const refresh = useCallback(async () => {
    const p = await api.api.GET("/api/v1/projects/{id}", {
      params: { path: { id: projectId } },
    });
    if (p.error !== undefined || p.data === undefined) {
      setLoadError("Failed to load this project. It may not exist or you may not have access.");
      setProject(null);
      return;
    }
    setProject(p.data);
    const clientRef = p.data.client_id;
    if (clientRef !== null && clientRef !== undefined) {
      const c = await api.api.GET("/api/v1/clients/{id}", {
        params: { path: { id: clientRef } },
      });
      setClient(c.data ?? null);
    }
    setLoadError(null);
    const [ph, t] = await Promise.all([
      api.api.GET("/api/v1/projects/{id}/phases", { params: { path: { id: projectId } } }),
      api.api.GET("/api/v1/projects/{id}/tasks", { params: { path: { id: projectId } } }),
    ]);
    setPhases(ph.data ?? []);
    const taskRows = t.data ?? [];
    setTasks(taskRows);
    const entries = await Promise.all(
      taskRows.map(async (task) => {
        const d = await api.api.GET("/api/v1/projects/{id}/tasks/{task_id}/dependencies", {
          params: { path: { id: projectId, task_id: task.id } },
        });
        return [task.id, d.data?.depends_on ?? []] as const;
      }),
    );
    setDeps(Object.fromEntries(entries));
    // Posted purchase bills in base currency; without financial:read the
    // route answers 403 and the strip simply stays absent.
    const cost = await api.api.GET("/api/v1/projects/{id}/cost-rollup", {
      params: { path: { id: projectId } },
    });
    setRollup(cost.data ?? null);
    const [burnResult, releaseResult, contractResult] = await Promise.all([
      api.api.GET("/api/v1/projects/{id}/phase-burn", { params: { path: { id: projectId } } }),
      api.api.GET("/api/v1/projects/{id}/billing-releases", { params: { path: { id: projectId } } }),
      api.api.GET("/api/v1/contracts"),
    ]);
    setBurn(burnResult.data ?? []);
    setReleases(releaseResult.data ?? []);
    setContracts((contractResult.data ?? []) as Contract[]);
  }, [projectId]);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const sortedPhases = useMemo(
    () => [...phases].sort((a, b) => a.sort_order - b.sort_order),
    [phases],
  );
  const phaseById = useMemo(() => new Map(phases.map((p) => [p.id, p])), [phases]);
  const taskById = useMemo(() => new Map(tasks.map((t) => [t.id, t])), [tasks]);

  const blocked = useMemo(() => blockedTasks(tasks, deps), [tasks, deps]);
  const overdue = useMemo(() => overdueTasks(tasks), [tasks]);
  const dueSoon = useMemo(() => dueSoonTasks(tasks, 3), [tasks]);
  const blockedIds = useMemo(() => new Set(blocked.map((t) => t.id)), [blocked]);
  const overdueIds = useMemo(() => new Set(overdue.map((t) => t.id)), [overdue]);
  const dueSoonIds = useMemo(() => new Set(dueSoon.map((t) => t.id)), [dueSoon]);

  const inAttention = useCallback(
    (task: ProjectTask) =>
      blockedIds.has(task.id) || overdueIds.has(task.id) || dueSoonIds.has(task.id),
    [blockedIds, overdueIds, dueSoonIds],
  );

  const groupedTasks = useMemo(() => {
    const groups: { key: string; label: string; phase: ProjectPhase | null; tasks: ProjectTask[] }[] =
      sortedPhases.map((phase) => ({
        key: phase.id,
        label: phase.name,
        phase,
        tasks: tasks
          .filter((t) => t.phase_id === phase.id)
          .sort((a, b) => a.sort_order - b.sort_order),
      }));
    const unphased = tasks
      .filter((t) => t.phase_id === null || t.phase_id === undefined || !phaseById.has(t.phase_id))
      .sort((a, b) => a.sort_order - b.sort_order);
    if (unphased.length > 0) groups.push({ key: "none", label: "No phase", phase: null, tasks: unphased });
    return groups;
  }, [sortedPhases, tasks, phaseById]);

  const phaseNameOf = useCallback(
    (phaseId: string | null | undefined) =>
      phaseId === null || phaseId === undefined
        ? "No phase"
        : (phases.find((p) => p.id === phaseId)?.name ?? "—"),
    [phases],
  );

  const completeTask = async (task: ProjectTask) => {
    setActionError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["MutationParams"],
      TaskMutationResponse
    >({
      describe: `Complete task ${task.title}`,
      entityId: task.id,
      optimistic: () =>
        setTasks((prev) =>
          prev.map((t) => (t.id === task.id ? { ...t, status: "completed" } : t)),
        ),
      revert: () =>
        setTasks((prev) =>
          prev.map((t) => (t.id === task.id ? { ...t, status: task.status } : t)),
        ),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key, version: base.version }),
      request: (c, body) =>
        c.POST("/api/v1/projects/{id}/tasks/{task_id}/complete", {
          params: { path: { id: projectId, task_id: task.id } },
          body,
        }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    if (!result.ok) {
      void refresh();
      setActionError(
        result.reason === "dependency_not_ready"
          ? "Dependencies must complete first."
          : rejectionText(result, "The completion was rejected. The task is unchanged."),
      );
      return;
    }
    void refresh();
  };

  const completePhase = async (phase: ProjectPhase) => {
    setActionError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["MutationParams"],
      PhaseMutationResponse
    >({
      describe: `Complete phase ${phase.name}`,
      entityId: phase.id,
      optimistic: () =>
        setPhases((prev) =>
          prev.map((p) => (p.id === phase.id ? { ...p, status: "completed" } : p)),
        ),
      revert: () =>
        setPhases((prev) =>
          prev.map((p) => (p.id === phase.id ? { ...p, status: phase.status } : p)),
        ),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key, version: base.version }),
      request: (c, body) =>
        c.POST("/api/v1/projects/{id}/phases/{phase_id}/complete", {
          params: { path: { id: projectId, phase_id: phase.id } },
          body,
        }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    if (!result.ok) {
      void refresh();
      setActionError(rejectionText(result, "The phase completion was rejected."));
      return;
    }
    void refresh();
  };

  const createPhase = async () => {
    if (phaseName.trim() === "") return;
    setBusy(true);
    setActionError(null);
    const tempId = `pending-phase:${phaseName}`;
    const result = await runEnvelopeMutation<
      components["schemas"]["CreatePhaseRequest"],
      PhaseMutationResponse
    >({
      describe: "Create phase",
      entityId: tempId,
      optimistic: () =>
        setPhases((prev) => [
          ...prev,
          {
            id: tempId,
            org_id: "",
            project_id: projectId,
            name: phaseName.trim(),
            sort_order: prev.length,
            status: "pending",
            is_milestone: phaseMilestone,
            milestone_amount_minor:
              phaseMilestone && phaseAmount.trim() !== "" ? Math.round(Number(phaseAmount) * 100) : null,
            milestone_currency:
              phaseMilestone && phaseAmount.trim() !== "" ? "USD" : null,
            description: null,
            completed_at: null,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
          },
        ]),
      revert: () => setPhases((prev) => prev.filter((p) => p.id !== tempId)),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        name: phaseName.trim(),
        is_milestone: phaseMilestone,
        milestone_amount_minor:
          phaseMilestone && phaseAmount.trim() !== "" ? Math.round(Number(phaseAmount) * 100) : null,
        milestone_currency:
          phaseMilestone && phaseAmount.trim() !== "" ? "USD" : null,
        // A budget is what the burn read model measures against; a fee is
        // a fixed-price phase that releases on completion. Both are
        // optional and neither replaces the milestone amount.
        budget_minor:
          phaseBudget.trim() === "" ? null : Math.round(Number(phaseBudget) * 100),
        budget_currency: phaseBudget.trim() === "" ? null : "USD",
        fee_minor:
          !phaseMilestone && phaseFee.trim() !== "" ? Math.round(Number(phaseFee) * 100) : null,
        fee_currency: !phaseMilestone && phaseFee.trim() !== "" ? "USD" : null,
      }),
      request: (c, body) =>
        c.POST("/api/v1/projects/{id}/phases", { params: { path: { id: projectId } }, body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setActionError(rejectionText(result, "The phase was rejected. Nothing was saved."));
      return;
    }
    setPhaseOpen(false);
    setPhaseName("");
    setPhaseMilestone(false);
    setPhaseAmount("");
    setPhaseBudget("");
    setPhaseFee("");
    void refresh();
  };

  const createTask = async () => {
    if (taskTitle.trim() === "") return;
    setBusy(true);
    setActionError(null);
    const tempId = `pending-task:${taskTitle}`;
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateTaskRequest"],
      TaskMutationResponse
    >({
      describe: "Create task",
      entityId: tempId,
      optimistic: () =>
        setTasks((prev) => [
          ...prev,
          {
            id: tempId,
            org_id: "",
            project_id: projectId,
            title: taskTitle.trim(),
            status: "pending",
            sort_order: prev.length,
            is_milestone: taskMilestone,
            milestone_amount_minor:
              taskMilestone && taskAmount.trim() !== "" ? Math.round(Number(taskAmount) * 100) : null,
            milestone_currency: taskMilestone && taskAmount.trim() !== "" ? "USD" : null,
            phase_id: taskPhase === "none" ? null : taskPhase,
            assignee_id: null,
            description: null,
            due_at: null,
            completed_at: null,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
          },
        ]),
      revert: () => setTasks((prev) => prev.filter((t) => t.id !== tempId)),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        title: taskTitle.trim(),
        phase_id: taskPhase === "none" ? null : taskPhase,
        is_milestone: taskMilestone,
        milestone_amount_minor:
          taskMilestone && taskAmount.trim() !== "" ? Math.round(Number(taskAmount) * 100) : null,
        milestone_currency: taskMilestone && taskAmount.trim() !== "" ? "USD" : null,
        depends_on: [...taskDependsOn],
      }),
      request: (c, body) =>
        c.POST("/api/v1/projects/{id}/tasks", { params: { path: { id: projectId } }, body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setActionError(rejectionText(result, "The task was rejected. Nothing was saved."));
      return;
    }
    setTaskOpen(false);
    setTaskTitle("");
    setTaskMilestone(false);
    setTaskAmount("");
    setTaskPhase("none");
    setTaskDependsOn(new Set());
    void refresh();
  };

  const saveDependencies = async () => {
    if (depTaskId === null) return;
    setBusy(true);
    setActionError(null);
    type DepsBody = components["schemas"]["SetDependenciesRequest"] & components["schemas"]["MutationParams"];
    const result = await runEnvelopeMutation<DepsBody, DependencyListResponse>({
      describe: `Set dependencies for ${depTaskId.title}`,
      entityId: depTaskId.id,
      optimistic: () =>
        setDeps((prev) => ({
          ...prev,
          [depTaskId.id]: [...depSelection].map((id) => taskById.get(id)).filter(
            (t): t is ProjectTask => t !== undefined,
          ),
        })),
      revert: () => void refresh(),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key, depends_on: [...depSelection] }),
      request: (c, body) =>
        c.PUT("/api/v1/projects/{id}/tasks/{task_id}/dependencies", {
          params: { path: { id: projectId, task_id: depTaskId.id } },
          body: body as components["schemas"]["SetDependenciesRequest"],
        }),
    });
    setBusy(false);
    setDepTaskId(null);
    if (!result.ok) {
      setActionError(rejectionText(result, "The dependency set was rejected (cycle or foreign task?)."));
      return;
    }
    void refresh();
  };

  // Linking is a tri-state on the wire: an id links, null unlinks, and
  // omitting the field leaves the link alone. The server refuses a
  // contract belonging to another client or a cancelled one, so the
  // picker only ever offers this client's live contracts.
  const setBillingContract = async (contractId: string | null) => {
    setBusy(true);
    setActionError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["PatchProjectRequest"],
      ProjectMutationResponse
    >({
      describe: contractId === null ? "Unlink billing contract" : "Link billing contract",
      entityId: projectId,
      optimistic: () => undefined,
      revert: () => undefined,
      buildBody: (base) => ({ ...base, billing_contract_id: contractId }),
      request: (c, body) =>
        c.PATCH("/api/v1/projects/{id}", { params: { path: { id: projectId } }, body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setActionError(rejectionText(result, "The billing link was rejected. Nothing changed."));
      return;
    }
    void refresh();
  };

  const cancelRelease = async (release: BillingRelease) => {
    setBusy(true);
    setActionError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["MutationParams"],
      BillingRelease
    >({
      describe: `Cancel release ${release.label}`,
      entityId: release.id,
      optimistic: () => undefined,
      revert: () => undefined,
      buildBody: (base) => ({ ...base }),
      request: (c) =>
        // No body on this route: the release id in the path is the whole
        // instruction, and a repeat cancel answers with the same row.
        c.POST("/api/v1/projects/{id}/billing-releases/{release_id}/cancel", {
          params: { path: { id: projectId, release_id: release.id } },
        }),
    });
    setBusy(false);
    if (!result.ok) {
      setActionError(rejectionText(result, "The release could not be cancelled. Nothing changed."));
      return;
    }
    void refresh();
  };

  if (auth.status !== "authed") return null;

  if (loadError !== null) {
    return (
      <div className="grid gap-4 p-6">
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {loadError}
        </p>
        <Link href="/projects" className="text-[13px] underline underline-offset-2">
          Back to projects
        </Link>
      </div>
    );
  }

  if (project === null) {
    return (
      <div className="p-6">
        <Spinner />
      </div>
    );
  }

  const pProgress = phaseProgress(phases);
  const tProgress = taskProgress(tasks);
  const risk = projectRisk(project);
  // A project bills through one of its own client's live contracts, so
  // the picker never offers a contract the server would refuse.
  const clientContracts = contracts.filter(
    (c) =>
      project.client_id !== null &&
      project.client_id !== undefined &&
      c.client_id === project.client_id &&
      c.status !== "cancelled",
  );
  const linkedContract = contracts.find((c) => c.id === project.billing_contract_id) ?? null;

  // The budget bar reads the real cost-rollup, the same figures the roll-up
  // strip prints: spent against the project's own budget, in the rollup's
  // base currency so a multi-currency book is not silently summed wrong.
  const budgetSpent = rollup?.po_bill_base_minor ?? null;
  const budgetCap = rollup?.budget_minor ?? null;
  const budgetCurrency = rollup?.base_currency ?? project.currency ?? "USD";
  const budgetPct =
    budgetSpent === null || budgetCap === null || budgetCap === 0
      ? null
      : Math.round((budgetSpent / budgetCap) * 100);

  return (
    <div className="mx-auto grid w-full max-w-[1600px] gap-4 p-4 md:p-6">
      <RecordFrame
        context={
          <>
            <RecordSection label="Client">
              {client === null ? (
                <p className="text-[16px] font-semibold">Internal</p>
              ) : (
                <Link
                  href={`/clients/${client.id}`}
                  className="text-[16px] font-semibold underline decoration-transparent underline-offset-2 hover:decoration-[var(--m-border-strong)]"
                >
                  {client.name}
                </Link>
              )}
              <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                <Tag tone={projectStatusTone(project.status)}>
                  {STATUS_LABEL[project.status] ?? project.status.replaceAll("_", " ")}
                </Tag>
                {risk !== null ? (
                  /* The pack's rule, on the client-facing side of the
                     record too: a date coming up is blush, a date that has
                     passed is red. Both carry their words. */
                  <Tag tone={risk.tone === "danger" ? "danger" : "warning"}>{risk.label}</Tag>
                ) : null}
              </div>
            </RecordSection>

            <RecordSection label="Commitment">
              <RecordRow label="Starts">{project.start_date ?? "Not set"}</RecordRow>
              <RecordRow label="Due">{project.due_date ?? "Not set"}</RecordRow>
              <RecordRow label="Billing">{project.billing_method.replaceAll("_", " ")}</RecordRow>
            </RecordSection>

            <RecordSection label="Budget">
              {budgetCap === null ? (
                <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  No budget set on this project.
                </p>
              ) : (
                <>
                  <RecordRow label="Budget">{money(budgetCap, budgetCurrency)}</RecordRow>
                  <RecordRow label="Committed">
                    {budgetSpent === null ? "—" : money(budgetSpent, budgetCurrency)}
                  </RecordRow>
                  <div className="mt-2">
                    <BudgetBar pct={budgetPct} />
                  </div>
                </>
              )}
            </RecordSection>

            <RecordSection label="Owner">
              <RecordRow label="Account owner">
                {client === null ? "Unassigned" : "You"}
              </RecordRow>
            </RecordSection>
          </>
        }
        content={
          <>
      <div className="m-rise">
        <Link
          href="/projects"
          className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2"
          style={{ color: "var(--m-text-secondary)" }}
        >
          ← Projects
        </Link>
        <div className="mt-2 flex flex-wrap items-center gap-2.5">
          <h1 className="text-lg font-semibold">{project.name}</h1>
          <Tag tone={projectStatusTone(project.status)}>
            {STATUS_LABEL[project.status] ?? project.status.replaceAll("_", " ")}
          </Tag>
          {risk !== null ? (
            <Tag tone={risk.tone === "danger" ? "danger" : "warning"}>{risk.label}</Tag>
          ) : null}
        </div>
        <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {client !== null ? (
            <>
              Client:{" "}
              <Link href={`/clients/${client.id}`} className="underline decoration-[var(--m-border)] underline-offset-2">
                {client.name}
              </Link>
              {" · "}
            </>
          ) : (
            "Internal · "
          )}
          {project.billing_method.replaceAll("_", " ")}
          {project.budget_minor !== null && project.budget_minor !== undefined
            ? ` · budget ${money(project.budget_minor, project.currency ?? "USD")}`
            : ""}
          {project.due_date !== null ? ` · due ${project.due_date}` : ""}
        </p>
        {project.description !== null && project.description !== undefined ? (
          <p className="mt-1 max-w-[80ch] text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            {project.description}
          </p>
        ) : null}
      </div>

      <div className="m-rise flex flex-wrap items-center gap-x-5 gap-y-2" style={{ animationDelay: "40ms" }}>
        <RollUp label="phases" done={pProgress.done} total={pProgress.total} pct={pProgress.pct} />
        <RollUp label="tasks" done={tProgress.done} total={tProgress.total} pct={tProgress.pct} />
        {pProgress.milestonesTotal > 0 ? (
          <RollUp
            label="milestones"
            done={pProgress.milestonesDone}
            total={pProgress.milestonesTotal}
            pct={pProgress.milestonesTotal === 0 ? 0 : Math.round((pProgress.milestonesDone / pProgress.milestonesTotal) * 100)}
          />
        ) : null}
        {rollup !== null ? (
          <span
            className="text-[12px] tabular-nums"
            style={{ color: "var(--m-text-secondary)" }}
            data-testid="project-cost-rollup"
          >
            PO cost{" "}
            <strong className="">{money(rollup.po_bill_base_minor, rollup.base_currency)}</strong>
            {rollup.budget_minor !== null && rollup.budget_minor !== undefined
              ? ` of ${money(rollup.budget_minor, rollup.budget_currency ?? rollup.base_currency)} budget`
              : ""}
          </span>
        ) : null}
        {overdue.length > 0 ? (
          <Tag tone="danger" data-testid="risk-overdue">
            {overdue.length} overdue task{overdue.length === 1 ? "" : "s"}
          </Tag>
        ) : null}
        {blocked.length > 0 ? (
          <Tag tone="warning" data-testid="risk-blocked">
            {blocked.length} blocked
          </Tag>
        ) : null}
        {dueSoon.length > 0 ? (
          <Tag tone="warning" data-testid="risk-due-soon">
            {dueSoon.length} due in 3d
          </Tag>
        ) : null}
      </div>

      {sortedPhases.length > 0 ? (
        <PhaseRail phases={sortedPhases} />
      ) : null}

      {actionError !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {actionError}
        </p>
      ) : null}

      {rollup !== null && rollup.bills.length > 0 ? (
        <Card className="m-rise p-4" style={{ animationDelay: "70ms" }} data-testid="project-cost-bills">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h2 className="text-sm font-semibold">Purchase bills</h2>
            <span className="text-[12px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
              posted in {rollup.base_currency}
            </span>
          </div>
          <ul className="mt-2 grid gap-1">
            {rollup.bills.map((bill) => (
              <li key={bill.bill_id} className="flex flex-wrap items-center justify-between gap-2 text-[12px]">
                <Link
                  href={`/purchasing/${bill.purchase_order_id}`}
                  className="underline decoration-[var(--m-border)] underline-offset-2"
                >
                  {bill.purchase_order_number}
                </Link>
 <span className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                  {money(bill.amount_minor, bill.currency)}
                  {bill.base_amount_minor !== null &&
                  bill.base_amount_minor !== undefined &&
                  bill.currency !== rollup.base_currency
                    ? ` · ${money(bill.base_amount_minor, rollup.base_currency)} locked`
                    : ""}
                </span>
              </li>
            ))}
          </ul>
        </Card>
      ) : null}

      <Card className="m-rise p-0" style={{ animationDelay: "75ms" }} data-testid="project-billing">
        <div className="flex flex-wrap items-center justify-between gap-2 border-b p-4" style={{ borderColor: "var(--m-border)" }}>
          <div>
            <h2 className="text-sm font-semibold">Billing</h2>
            <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              {linkedContract === null
                ? "No contract linked, so project time is not priced by this card."
                : "Approved time on each phase prices against the linked contract's rate card."}
            </p>
          </div>
          <div className="flex items-center gap-2">
            <Select
              value={project.billing_contract_id ?? "none"}
              onValueChange={(v) => void setBillingContract(v === "none" ? null : v)}
            >
              <SelectTrigger aria-label="Billing contract" className="w-64" data-testid="project-billing-contract">
                <SelectValue placeholder="Link a contract" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">No contract</SelectItem>
                {clientContracts.map((c) => (
                  <SelectItem key={c.id} value={c.id}>
                    {c.number} — {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {linkedContract !== null ? (
              <Link
                href={`/contracts/${linkedContract.id}`}
                className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2"
                style={{ color: "var(--m-text-secondary)" }}
              >
                Open contract
              </Link>
            ) : null}
          </div>
        </div>

        {(
          <div className="grid gap-4 p-4">            {burn.length > 0 ? (
              <div>
                <div className="flex items-baseline justify-between">
                  <h3 className="text-[13px] font-semibold">Phase burn</h3>
                  <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                    approved billable time, priced at the contract card
                  </span>
                </div>
                <ul className="mt-2 grid gap-1">
                  {burn.map((row) => (
                    <li key={row.phase_id} className="flex flex-wrap items-center justify-between gap-2 text-[12px]">
                      <span className="flex items-center gap-2">
                        {row.alert !== "none" ? (
                          <Tag tone={row.alert === "over" ? "danger" : "warning"}>{row.alert}</Tag>
                        ) : null}
                        {row.phase_name}
                      </span>
 <span className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                        {money(row.consumed_minor, row.currency ?? "USD")}
                        {row.budget_minor !== null && row.budget_minor !== undefined
                          ? ` of ${money(row.budget_minor, row.currency ?? "USD")}`
                          : " · no budget"}
                        <span style={{ color: "var(--m-text-muted)" }}> · {row.consumed_minutes} min</span>
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            ) : (
              <div>
                <h3 className="text-[13px] font-semibold">Phase burn</h3>
                <p className="mt-1 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  No phase budgets on this project yet. Add a budget to a phase and its approved
                  time is measured against it.
                </p>
              </div>
            )}

            <div>
              <div className="flex items-baseline justify-between">
                <h3 className="text-[13px] font-semibold">Released to billing</h3>
                <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                  a pending release bills on the first run that reaches its date
                </span>
              </div>
              {/* The ledger renders even when it is empty. Hiding it would
                  leave "nothing has been released" and "the ledger did not
                  load" looking identical. */}
              {releases.length === 0 ? (
                <p className="mt-1 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  Nothing released yet. A completed milestone or a fixed-fee phase stages its
                  release here, and the next run bills it.
                </p>
              ) : (
                <ul className="mt-2 grid gap-1">
                  {releases.map((release) => (
                    <li key={release.id} className="flex flex-wrap items-center justify-between gap-2 text-[12px]">
                      <span className="flex items-center gap-2">
                        <Tag tone={releaseTone(release.status)}>{release.status}</Tag>
                        {release.label}
                        <span style={{ color: "var(--m-text-muted)" }}>{release.released_on}</span>
                      </span>
                      <span className="flex items-center gap-2">
 <span className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                          {money(release.amount_minor, release.currency)}
                        </span>
                        {release.invoice_id === null || release.invoice_id === undefined ? (
                          release.status === "pending" ? (
                            <Button size="sm" variant="ghost" disabled={busy} onClick={() => void cancelRelease(release)}>
                              Cancel
                            </Button>
                          ) : null
                        ) : (
                          <Link href={`/billing/${release.invoice_id}`} className="underline underline-offset-2">
                            invoice
                          </Link>
                        )}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        )}
      </Card>

      <Card className="m-rise p-0" style={{ animationDelay: "80ms" }}>
        <div className="flex flex-wrap items-center justify-between gap-2 border-b p-4" style={{ borderColor: "var(--m-border)" }}>
          <div className="flex items-center gap-3">
            <h2 className="text-sm font-semibold">Delivery</h2>
            <span className="text-[12px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
              {pProgress.done}/{pProgress.total} phases · {tProgress.done}/{tProgress.total} tasks
            </span>
          </div>
          <div className="flex items-center gap-2">
            <Link
              href="/billing"
              className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2"
              style={{ color: "var(--m-text-secondary)" }}
            >
              Billing →
            </Link>
            <Button size="sm" variant="secondary" onClick={() => setPhaseOpen(true)} data-testid="add-phase">
              Add phase
            </Button>
            <AddPhaseDialog
              open={phaseOpen}
              onOpenChange={setPhaseOpen}
              name={phaseName}
              setName={setPhaseName}
              milestone={phaseMilestone}
              setMilestone={setPhaseMilestone}
              amount={phaseAmount}
              setAmount={setPhaseAmount}
              budget={phaseBudget}
              setBudget={setPhaseBudget}
              fee={phaseFee}
              setFee={setPhaseFee}
              busy={busy}
              onCreate={() => void createPhase()}
            />
          </div>
        </div>

        {/* The dated timeline sits above the ordered rail: a bar tells you
            WHEN, the rail tells you WHAT ORDER, and a planner reads the bar
            first. Undated phases appear in the rail below and are named as
            not being on the timeline. */}
        <div className="p-4">
          <ProjectGantt projectId={projectId} />
        </div>

        {phases.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No phases yet. Phases group tasks into delivery stages; a milestone phase releases its amount to billing on completion." />
          </div>
        ) : (
          <div className="grid grid-cols-1">
            <div className="flex flex-wrap items-center gap-2 border-b px-[18px] py-2.5" style={{ borderColor: "var(--m-border)" }}>
              <div className="flex items-center gap-2 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                <button
                  type="button"
                  onClick={() => setAttentionOnly(false)}
                  className="rounded-full px-2.5 py-1 font-semibold transition-colors"
                  style={
                    !attentionOnly
                      ? { background: "var(--m-accent-soft)", color: "var(--m-accent)" }
                      : { color: "var(--m-text-muted)" }
                  }
                >
                  All tasks
                </button>
                <button
                  type="button"
                  onClick={() => setAttentionOnly(true)}
                  className="rounded-full px-2.5 py-1 font-semibold transition-colors"
                  style={
                    attentionOnly
                      ? { background: "var(--m-accent-soft)", color: "var(--m-accent)" }
                      : { color: "var(--m-text-muted)" }
                  }
                  data-testid="attention-filter"
                >
                  Needs attention
                  {overdue.length + blocked.length + dueSoon.length > 0
                    ? ` (${overdue.length + blocked.length + dueSoon.length})`
                    : ""}
                </button>
              </div>
              <span className="ml-auto text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                finish-to-start dependencies gate completion
              </span>
            </div>
            {groupedTasks.map((group, groupIndex) => {
              const visible = attentionOnly ? group.tasks.filter(inAttention) : group.tasks;
              const gProgress = taskProgress(group.tasks);
              const phaseMilestoneTotal = group.tasks
                .filter((t) => t.is_milestone && t.milestone_amount_minor !== null && t.milestone_amount_minor !== undefined)
                .reduce((sum, t) => sum + (t.milestone_amount_minor ?? 0), 0);
              if (visible.length === 0) return null;
              return (
                <section
                  key={group.key}
                  className="m-rise"
                  style={{ animationDelay: `${Math.min(groupIndex * 40, 160)}ms` }}
                  data-testid={`task-group-${group.key}`}
                >
                  <div
                    className="flex flex-wrap items-center gap-2 px-4 py-2"
                    style={{ background: "var(--m-bg-subtle)" }}
                  >
                    <span className="text-[12px] font-semibold">{group.label}</span>
                    {group.phase !== null ? (
                      <Tag tone={group.phase.status === "completed" ? "success" : "neutral"}>
                        {STATUS_LABEL[group.phase.status] ?? group.phase.status}
                      </Tag>
                    ) : null}
                    {group.phase?.is_milestone && group.phase.milestone_amount_minor !== null && group.phase.milestone_amount_minor !== undefined ? (
                      <span className="text-[11px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                        milestone {money(group.phase.milestone_amount_minor, group.phase.milestone_currency ?? "USD")}
                      </span>
                    ) : null}
                    {phaseMilestoneTotal > 0 ? (
                      <span className="text-[11px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                        +{money(phaseMilestoneTotal, "USD")} task milestones
                      </span>
                    ) : null}
                    <span className="ml-auto text-[11px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                      {gProgress.done}/{gProgress.total} done
                    </span>
                    {group.phase !== null && (group.phase.status === "pending" || group.phase.status === "active") ? (
                      <Button variant="ghost" size="sm" onClick={() => void completePhase(group.phase!)}>
                        Complete phase
                      </Button>
                    ) : null}
                  </div>
                  {visible.map((task) => (
                    <TaskRow
                      key={task.id}
                      task={task}
                      deps={deps[task.id] ?? []}
                      overdue={overdueIds.has(task.id)}
                      dueSoon={dueSoonIds.has(task.id)}
                      blocked={blockedIds.has(task.id)}
                      phaseLabel={phaseNameOf(task.phase_id)}
                      onDependencies={() => {
                        setDepTaskId(task);
                        setDepSelection(new Set((deps[task.id] ?? []).map((b) => b.id)));
                      }}
                      onComplete={() => void completeTask(task)}
                    />
                  ))}
                </section>
              );
            })}
            {attentionOnly &&
              overdue.length + blocked.length + dueSoon.length === 0 ? (
              <div className="p-4">
                <EmptyState message="Nothing needs attention — no overdue, blocked, or due-soon tasks." />
              </div>
            ) : null}
            <div className="flex items-center justify-between border-t px-4 py-2.5" style={{ borderColor: "var(--m-border)" }}>
              <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                {tasks.length} task{tasks.length === 1 ? "" : "s"} total
              </span>
              <Button size="sm" variant="secondary" onClick={() => setTaskOpen(true)} data-testid="add-task">
                Add task
              </Button>
              <AddTaskDialog
                open={taskOpen}
                onOpenChange={setTaskOpen}
                title={taskTitle}
                setTitle={setTaskTitle}
                phase={taskPhase}
                setPhase={setTaskPhase}
                phases={sortedPhases}
                milestone={taskMilestone}
                setMilestone={setTaskMilestone}
                amount={taskAmount}
                setAmount={setTaskAmount}
                dependsOn={taskDependsOn}
                setDependsOn={setTaskDependsOn}
                tasks={tasks}
                busy={busy}
                onCreate={() => void createTask()}
              />
            </div>
          </div>
        )}
      </Card>

      <DependencyDialog
        depTaskId={depTaskId}
        onClose={() => setDepTaskId(null)}
        selection={depSelection}
        setSelection={setDepSelection}
        tasks={tasks}
        busy={busy}
        onSave={() => void saveDependencies()}
      />
          </>
        }
        summary={
          <>
            <p
              className="mb-1 text-[11px] font-semibold uppercase tracking-[0.08em]"
              style={{ color: "var(--m-text-muted)" }}
            >
              Right now
            </p>

            <SummaryCard title="Budget">
              {budgetPct === null ? (
                <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  Set a budget to see this bar.
                </span>
              ) : (
                <>
                  <RecordRow label="Budget">
                    {money(budgetCap as number, budgetCurrency)}
                  </RecordRow>
                  <RecordRow label="Committed">
                    {money(budgetSpent as number, budgetCurrency)}
                  </RecordRow>
                  <div className="mt-1">
                    <BudgetBar pct={budgetPct} />
                  </div>
                </>
              )}
            </SummaryCard>

            <SummaryCard title="Risk">
              {risk === null ? (
                <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  Nothing at risk.
                </span>
              ) : (
                <RecordRow label="Schedule">
                  <Tag tone={risk.tone === "danger" ? "danger" : "warning"}>
                    {risk.label}
                  </Tag>
                </RecordRow>
              )}
              {overdue.length > 0 ? (
                <RecordRow label="Overdue tasks">
                  <Tag tone="danger">{overdue.length} overdue</Tag>
                </RecordRow>
              ) : null}
              {blocked.length > 0 ? (
                <RecordRow label="Blocked">
                  <Tag tone="warning">{blocked.length} blocked</Tag>
                </RecordRow>
              ) : null}
              {dueSoon.length > 0 ? (
                <RecordRow label="Due in 3d">
                  <Tag tone="warning">{dueSoon.length} due in 3d</Tag>
                </RecordRow>
              ) : null}
            </SummaryCard>

            <SummaryCard title="Progress">
              <RecordRow label="Phases">
                {pProgress.done}/{pProgress.total}
              </RecordRow>
              <RecordRow label="Tasks">
                {tProgress.done}/{tProgress.total}
              </RecordRow>
              {pProgress.milestonesTotal > 0 ? (
                <RecordRow label="Milestones">
                  {pProgress.milestonesDone}/{pProgress.milestonesTotal}
                </RecordRow>
              ) : null}
            </SummaryCard>
          </>
        }
      />
    </div>
  );
}

/**
 * The budget bar, on the Dispatch utilisation bar's language: a flat 1px
 * track, a flat fill, and the one rule that transfers with it — past the
 * budget the overflow is BLUSH and the percentage carries an alert icon, so
 * the number is never colour alone.
 */
function BudgetBar({ pct }: { pct: number | null }) {
  if (pct === null) {
    return (
      <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
        Set a budget to see this bar.
      </p>
    );
  }
  const over = pct > 100;
  return (
    <div className="flex items-center gap-2">
      <div
        className="h-1.5 flex-1 overflow-hidden rounded-full"
        style={{ background: "var(--m-border)" }}
        role="meter"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={`Budget ${pct}% committed`}
      >
        <div
          className="h-full rounded-full"
          style={{
            width: `${Math.min(pct, 100)}%`,
            background: over ? "var(--m-accent)" : "var(--m-text-secondary)",
            opacity: over ? 1 : 0.7,
          }}
        />
      </div>
      <span
        className="inline-flex items-center gap-1 text-[13px] font-semibold tabular-nums"
        style={{ color: over ? "var(--m-accent-text)" : "var(--m-text-secondary)" }}
      >
        {over ? <AlertGlyph /> : null}
        {pct}%
      </span>
    </div>
  );
}

function AlertGlyph() {
  return <Icon name="alert" size={13} />;
}

function RollUp({ label, done, total, pct }: { label: string; done: number; total: number; pct: number }) {
  return (
    <div className="min-w-32" data-testid={`rollup-${label}`}>
      <div className="flex items-baseline gap-1.5">
        <span className="text-[13px] font-semibold tabular-nums">{done}</span>
        <span className="text-[12px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
          / {total} {label}
        </span>
      </div>
      <div className="mt-1 h-1 w-28 overflow-hidden rounded-full" style={{ background: "var(--m-bg-subtle)" }}>
        <div
          className="h-full rounded-full"
          style={{
            width: `${pct}%`,
            background: pct === 100 && total > 0 ? "var(--m-status-success-fg)" : "var(--m-accent)",
            transition: "width 120ms cubic-bezier(0.2, 0, 0, 1)",
          }}
        />
      </div>
    </div>
  );
}

function PhaseRail({ phases }: { phases: ProjectPhase[] }) {
  return (
    <div className="m-rise" data-testid="phase-rail" style={{ animationDelay: "60ms" }} role="list" aria-label="Phase sequence">
      <div className="flex gap-1">
        {phases.map((phase) => {
          const fill =
            phase.status === "completed"
              ? "var(--m-status-success-fg)"
              : phase.status === "active"
                ? "var(--m-accent)"
                : "var(--m-bg-subtle)";
          return (
            <div
              key={phase.id}
              role="listitem"
              className="group relative min-w-0 flex-1"
              title={`${phase.name} — ${STATUS_LABEL[phase.status] ?? phase.status}${phase.is_milestone ? " (milestone)" : ""}`}
            >
              <div
                className="h-2 rounded-full"
                style={{
                  background: fill,
                  border: phase.status === "pending" ? "1px solid var(--m-border)" : undefined,
                }}
              />
              <div className="mt-1 flex items-center gap-1 truncate text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                {phase.is_milestone ? <span aria-label="milestone">◆</span> : null}
                <span className="truncate">{phase.name}</span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function TaskRow({
  task,
  deps,
  overdue,
  dueSoon,
  blocked,
  phaseLabel,
  onDependencies,
  onComplete,
}: {
  task: ProjectTask;
  deps: ProjectTask[];
  overdue: boolean;
  dueSoon: boolean;
  blocked: boolean;
  phaseLabel: string;
  onDependencies: () => void;
  onComplete: () => void;
}) {
  const openBlockers = deps.filter((b) => taskOpenness(b) === "open");
  const due = task.due_at !== null && task.due_at !== undefined ? dueLabel(task.due_at) : null;
  const done = taskOpenness(task) !== "open";
  return (
    <div
      className="m-rise row-hover grid grid-cols-1 items-center gap-x-4 gap-y-1.5 border-b px-[18px] py-2.5 last:border-b-0 lg:grid-cols-[minmax(0,2.4fr)_auto_minmax(0,1.4fr)_auto]"
      style={{ borderColor: "var(--m-border)" }}
      data-testid={`task-row-${task.id}`}
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span
            className="truncate text-[13px]"
            style={{
              color: done ? "var(--m-text-muted)" : "var(--m-text)",
              textDecoration: done ? "line-through" : undefined,
            }}
          >
            {task.title}
          </span>
          {task.is_milestone ? <span aria-label="milestone task" style={{ color: "var(--m-accent)" }}>◆</span> : null}
        </div>
        <div className="mt-0.5 flex flex-wrap items-center gap-x-2 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          <span>{phaseLabel}</span>
          {task.milestone_amount_minor !== null && task.milestone_amount_minor !== undefined ? (
            <span className="tabular-nums">{money(task.milestone_amount_minor, task.milestone_currency ?? "USD")}</span>
          ) : null}
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        <Tag tone={done ? "success" : task.status === "cancelled" ? "danger" : "neutral"}>
          {task.status.replaceAll("_", " ")}
        </Tag>
        {blocked ? <Tag tone="warning">blocked</Tag> : null}
        {overdue ? <Tag tone="danger">{due?.text ?? "overdue"}</Tag> : dueSoon && due !== null ? <Tag tone="warning">{due.text}</Tag> : null}
      </div>
      <div className="min-w-0 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
        {deps.length === 0 ? (
          "no dependencies"
        ) : openBlockers.length === 0 ? (
          <span>after {deps.length} done</span>
        ) : (
          <span data-testid={`blocked-by-${task.id}`} style={{ color: "var(--m-status-warning-fg)" }}>
            blocked by {openBlockers.map((b) => b.title).join(", ")}
          </span>
        )}
      </div>
      <div className="flex items-center justify-end gap-1">
        <Button variant="ghost" size="sm" onClick={onDependencies}>
          Dependencies{deps.length > 0 ? ` (${deps.length})` : ""}
        </Button>
        {!done ? (
          <Button variant="ghost" size="sm" onClick={onComplete}>
            Complete
          </Button>
        ) : null}
      </div>
    </div>
  );
}

function releaseTone(status: string): "success" | "warning" | "danger" | "neutral" {
  if (status === "billed") return "success";
  if (status === "pending") return "warning";
  return "neutral";
}

function AddPhaseDialog(props: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  name: string;
  setName: (v: string) => void;
  milestone: boolean;
  setMilestone: (v: boolean) => void;
  amount: string;
  setAmount: (v: string) => void;
  budget: string;
  setBudget: (v: string) => void;
  fee: string;
  setFee: (v: string) => void;
  busy: boolean;
  onCreate: () => void;
}) {
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent title="Add phase" description="Ordered delivery stage; milestone phases release their amount to billing on completion.">
        <div className="grid gap-3 p-4">
          <div>
            <Label htmlFor="phase-name">Name</Label>
            <Input id="phase-name" value={props.name} onChange={(e) => props.setName(e.target.value)} placeholder="Discovery" />
          </div>
          <label className="flex items-center gap-2 text-[13px]">
            <Checkbox checked={props.milestone} onCheckedChange={(v) => props.setMilestone(v === true)} />
            Billing milestone
          </label>
          {props.milestone ? (
            <div>
              <Label htmlFor="phase-amount">Milestone amount (USD)</Label>
              <Input id="phase-amount" type="number" min={0} step="0.01" value={props.amount} onChange={(e) => props.setAmount(e.target.value)} />
              <FieldHint>Released to billing when the phase completes.</FieldHint>
            </div>
          ) : (
            <div>
              <Label htmlFor="phase-fee">Fixed phase fee (USD, optional)</Label>
              <Input id="phase-fee" type="number" min={0} step="0.01" value={props.fee} onChange={(e) => props.setFee(e.target.value)} />
              <FieldHint>
                A completed fixed-fee phase releases this on its own, with no milestone.
              </FieldHint>
            </div>
          )}
          <div>
            <Label htmlFor="phase-budget">Phase budget (USD, optional)</Label>
            <Input id="phase-budget" type="number" min={0} step="0.01" value={props.budget} onChange={(e) => props.setBudget(e.target.value)} />
            <FieldHint>
              What the phase burn is measured against. Leave it empty for an unbudgeted phase —
              no budget is not a zero budget.
            </FieldHint>
          </div>
          <DialogActions>
            <Button variant="ghost" onClick={() => props.onOpenChange(false)}>
              Cancel
            </Button>
            <Button onClick={props.onCreate} disabled={props.busy || props.name.trim() === ""}>
              {props.busy ? <Spinner /> : null} Add phase
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function AddTaskDialog(props: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  title: string;
  setTitle: (v: string) => void;
  phase: string;
  setPhase: (v: string) => void;
  phases: ProjectPhase[];
  milestone: boolean;
  setMilestone: (v: boolean) => void;
  amount: string;
  setAmount: (v: string) => void;
  dependsOn: Set<string>;
  setDependsOn: (update: (prev: Set<string>) => Set<string>) => void;
  tasks: ProjectTask[];
  busy: boolean;
  onCreate: () => void;
}) {
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent title="Add task" description="Tasks carry phase, milestone, and finish-to-start dependencies.">
        <div className="grid gap-3 p-4">
          <div>
            <Label htmlFor="task-title">Title</Label>
            <Input id="task-title" value={props.title} onChange={(e) => props.setTitle(e.target.value)} />
          </div>
          <div>
            <Label htmlFor="task-phase">Phase</Label>
            <Select value={props.phase} onValueChange={props.setPhase}>
              <SelectTrigger id="task-phase" className="w-56">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">No phase</SelectItem>
                {props.phases.map((p) => (
                  <SelectItem key={p.id} value={p.id}>
                    {p.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <label className="flex items-center gap-2 text-[13px]">
            <Checkbox checked={props.milestone} onCheckedChange={(v) => props.setMilestone(v === true)} />
            Billing milestone
          </label>
          {props.milestone ? (
            <div>
              <Label htmlFor="task-amount">Milestone amount (USD)</Label>
              <Input id="task-amount" type="number" min={0} step="0.01" value={props.amount} onChange={(e) => props.setAmount(e.target.value)} />
            </div>
          ) : null}
          <div>
            <Label>Depends on (finish-to-start)</Label>
            <div className="grid max-h-40 gap-1 overflow-auto rounded-[var(--m-radius-container)] border p-2" style={{ borderColor: "var(--m-border)" }}>
              {props.tasks.length === 0 ? (
                <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  No existing tasks to depend on.
                </span>
              ) : (
                props.tasks.map((t) => (
                  <label key={t.id} className="flex items-center gap-2 text-[13px]">
                    <Checkbox
                      checked={props.dependsOn.has(t.id)}
                      onCheckedChange={(v) =>
                        props.setDependsOn((prev) => {
                          const next = new Set(prev);
                          if (v === true) next.add(t.id);
                          else next.delete(t.id);
                          return next;
                        })
                      }
                    />
                    {t.title}
                  </label>
                ))
              )}
            </div>
            <FieldHint>The new task cannot complete until these do.</FieldHint>
          </div>
          <DialogActions>
            <Button variant="ghost" onClick={() => props.onOpenChange(false)}>
              Cancel
            </Button>
            <Button onClick={props.onCreate} disabled={props.busy || props.title.trim() === ""}>
              {props.busy ? <Spinner /> : null} Add task
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function DependencyDialog(props: {
  depTaskId: ProjectTask | null;
  onClose: () => void;
  selection: Set<string>;
  setSelection: (update: (prev: Set<string>) => Set<string>) => void;
  tasks: ProjectTask[];
  busy: boolean;
  onSave: () => void;
}) {
  const { depTaskId } = props;
  return (
    <Dialog
      open={depTaskId !== null}
      onOpenChange={(open) => {
        if (!open) props.onClose();
      }}
    >
      <DialogContent
        title={depTaskId !== null ? `Dependencies — ${depTaskId.title}` : "Dependencies"}
        description="Pick the full set of tasks this one depends on (finish-to-start). Saving replaces the whole set."
      >
        <div className="grid gap-3 p-4">
          <div className="grid max-h-60 gap-1 overflow-auto rounded-[var(--m-radius-container)] border p-2" style={{ borderColor: "var(--m-border)" }}>
            {props.tasks
              .filter((t) => depTaskId === null || t.id !== depTaskId.id)
              .map((t) => {
                const open = taskOpenness(t) === "open";
                return (
                  <label key={t.id} className="flex items-center gap-2 text-[13px]">
                    <Checkbox
                      checked={props.selection.has(t.id)}
                      onCheckedChange={(v) =>
                        props.setSelection((prev) => {
                          const next = new Set(prev);
                          if (v === true) next.add(t.id);
                          else next.delete(t.id);
                          return next;
                        })
                      }
                    />
                    {t.title}
                    <span style={{ color: open ? "var(--m-status-warning-fg)" : "var(--m-text-muted)" }}>
                      {t.status.replaceAll("_", " ")}
                    </span>
                  </label>
                );
              })}
          </div>
          <DialogActions>
            <Button variant="ghost" onClick={props.onClose}>
              Cancel
            </Button>
            <Button onClick={props.onSave} disabled={props.busy || depTaskId === null}>
              {props.busy ? <Spinner /> : null} Save dependencies
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}
