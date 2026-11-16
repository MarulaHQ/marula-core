// /projects/[id] — one engagement's delivery surface.
//
// The record follows the surface pack's project template: an identity header
// (breadcrumb, title, status/client/billing/PM meta, and the four figures a PM
// reads first), the promise strip under it (started, promised go-live, next
// milestone, the commitment at risk, the baseline), the tab strip, and then the
// work. The header stays put while the tabs change, so the record never feels
// like it moved.
//
// The Plan tab is <ProjectPlanSurface/>: the S3 plan grid with phase rows, task
// rows, undated work called out honestly, inline creation, the impact preview
// before any date moves, named undo after a move, bulk scale, the task
// inspector (a bottom sheet at 1024 and below) and the density control.
// Timeline is <ProjectTimelineSurface/>: the S4 chart with its preview, tray
// and table view. Tasks keeps the phase-grouped
// list with the needs-attention triage filter and the dependency editor. Time
// and budget keeps the billing reads: phase burn, released billing, the linked
// contract and the purchase bills.
//
// Reads: the project, its client, phases, tasks and per-task dependencies
// through the typed SDK; the project calendar, risk signals and latest baseline
// through lib/projects-plan-types (the Wave 68 endpoints the generated client
// does not carry yet). Writes keep the IF1–IF3 envelope with optimistic status
// reconcile, except the schedule moves, which preview and apply through the
// schedule endpoints and undo through the revert list the apply returns.
//
// Non-purpose: no budget-burn editing, no ticket conversion, no client update
// drafting. Those are separate surfaces or separate lanes.
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
  Badge,
  Button,
  Card,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
  Diamond,
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
} from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useAuth, useRequireAuth } from "../../../lib/api";
import { ProjectTimelineSurface } from "../../../components/project-timeline-surface";
import { dueLabel, money } from "../../../lib/format";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import {
  CommitFlexible,
  CommitKey,
  CommitSep,
  CommitmentBar,
  CrumbSep,
  Kpi,
  ProjectShellHeader,
} from "../../../components/project-shell-header";
import { ProjectPlanSurface } from "../../../components/project-plan-surface";
import {
  effortOf,
  formatDay,
  formatWeekday,
  loadBaseline,
  loadCalendar,
  loadRisk,
  workingDaysBetween,
  type BaselineView,
  type PlanPhase,
  type PlanTask,
  type RiskResponse,
  type WorkingCalendar,
} from "../../../lib/projects-plan-types";

type Project = components["schemas"]["Project"];
type ProjectPhase = components["schemas"]["ProjectPhase"];
type ProjectTask = components["schemas"]["ProjectTask"];
type Client = components["schemas"]["Client"];
type ProjectMutationResponse = components["schemas"]["ProjectMutationResponse"];
type PhaseMutationResponse = components["schemas"]["PhaseMutationResponse"];
type TaskMutationResponse = components["schemas"]["TaskMutationResponse"];
type DependencyListResponse = components["schemas"]["DependencyListResponse"];
type Contract = components["schemas"]["Contract"];
type PhaseBurn = components["schemas"]["PhaseBurnView"];
type BillingRelease = components["schemas"]["BillingReleaseView"];

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
  const { me } = useAuth();
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
  // The plan's own reads: the project's working pattern, the deterministic risk
  // signals, and the last baseline. Each answers null without failing the page
  // because they are separate endpoints with separate permissions.
  const [calendar, setCalendar] = useState<WorkingCalendar | null>(null);
  const [riskSignals, setRiskSignals] = useState<RiskResponse | null>(null);
  const [baseline, setBaseline] = useState<BaselineView | null>(null);
  const [planLoaded, setPlanLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [attentionOnly, setAttentionOnly] = useState(false);
  // The record's tabs (DESIGN-NOTES-clients §7): the delivery plan, the
  // tasks, and time and budget. The header above them does not change
  // between tabs, so the record never feels like it moved.
  const [tab, setTab] = useState<"plan" | "timeline" | "tasks" | "money">("plan");

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

  // A hold is a status change plus the note the client will read. The reason
  // is optional, and it is the only hold text that crosses the portal boundary.
  const [holdOpen, setHoldOpen] = useState(false);
  const [holdReason, setHoldReason] = useState("");

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
    const [calendarRead, riskRead, baselineRead] = await Promise.all([
      loadCalendar(projectId).catch(() => null),
      loadRisk(projectId).catch(() => null),
      loadBaseline(projectId).catch(() => null),
    ]);
    setCalendar(calendarRead);
    setRiskSignals(riskRead);
    setBaseline(baselineRead);
    setPlanLoaded(true);
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
            start_at: null,
            duration_minutes: null,
            effort_estimate_minutes: null,
            effort_remaining_minutes: null,
            work_type: null,
            priority: null,
            role_key: null,
            required_skills: [],
            is_critical: false,
            baseline_start_at: null,
            baseline_duration_minutes: null,
            external_owner: null,
            calendar_pin: null,
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

  // Putting a project on hold carries the one reason the client may read.
  // Leaving hold takes it out of the status and the server clears the stored
  // reason with it, so the client never resends a stale one.
  const putOnHold = async () => {
    setBusy(true);
    setActionError(null);
    const reason = holdReason.trim();
    const result = await runEnvelopeMutation<
      components["schemas"]["PatchProjectRequest"],
      ProjectMutationResponse
    >({
      describe: "Put project on hold",
      entityId: projectId,
      optimistic: () => undefined,
      revert: () => undefined,
      buildBody: (base) => ({
        ...base,
        status: "on_hold",
        client_hold_reason: reason === "" ? null : reason,
      }),
      request: (c, body) =>
        c.PATCH("/api/v1/projects/{id}", { params: { path: { id: projectId } }, body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setActionError(rejectionText(result, "The hold was rejected. The project is unchanged."));
      return;
    }
    setHoldOpen(false);
    setHoldReason("");
    void refresh();
  };

  const resumeWork = async () => {
    setBusy(true);
    setActionError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["PatchProjectRequest"],
      ProjectMutationResponse
    >({
      describe: "Resume project",
      entityId: projectId,
      optimistic: () => undefined,
      revert: () => undefined,
      buildBody: (base) => ({ ...base, status: "active" }),
      request: (c, body) =>
        c.PATCH("/api/v1/projects/{id}", { params: { path: { id: projectId } }, body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setActionError(rejectionText(result, "The resume was rejected. The project is still on hold."));
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
  // The account owner is the signed-in user's real name. "You" collapsed
  // the owner line to a pronoun on the one surface that exists to say who
  // owns the account; the monogram carries the same person the client
  // record draws.
  const ownerName = me?.display_name ?? me?.email ?? "";
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
  // The plan reads the Wave 68 task fields the SDK has not been regenerated
  // for yet; the rows carry them at runtime. The cast stays here, at the one
  // boundary, and disappears when the client is regenerated.
  const planTasks = tasks as unknown as PlanTask[];
  const planPhases = sortedPhases as unknown as PlanPhase[];
  const planHours = effortOf(planTasks);
  const planWorkingDays = workingDaysBetween(
    project.start_date ?? null,
    project.due_date ?? null,
    calendar?.weekdays ?? 31,
  );
  const nextMilestone =
    [...planPhases]
      .filter((phase) => phase.is_milestone && phase.status !== "completed" && phase.end_date)
      .sort((a, b) => String(a.end_date).localeCompare(String(b.end_date)))[0] ?? null;
  const atRiskSignal = riskSignals?.signals.find((signal) => signal.severity !== "info") ?? null;
  const atRiskSentence =
    atRiskSignal?.message ??
    (overdue.length > 0
      ? `${overdue.length} task${overdue.length === 1 ? " has" : "s have"} passed the date it was promised for.`
      : null);
  const hoursUsedPct =
    planHours.estimateMinutes === 0
      ? 0
      : Math.min(100, Math.round((planHours.loggedMinutes / planHours.estimateMinutes) * 100));

  return (
    <div className="mx-auto w-full max-w-[1600px] pb-6">
      <ProjectShellHeader
        breadcrumb={
          <>
            <Link href="/projects" className="hover:text-foreground">
              Projects
            </Link>
            <CrumbSep />
            {client === null ? (
              <span>Internal</span>
            ) : (
              <Link href={`/clients/${client.id}`} className="hover:text-foreground">
                {client.name}
              </Link>
            )}
            <CrumbSep />
            <span className="truncate">{project.name}</span>
          </>
        }
        title={project.name}
        meta={
          <>
            <Badge tone={projectStatusTone(project.status)}>
              {STATUS_LABEL[project.status] ?? project.status.replaceAll("_", " ")}
            </Badge>
            {project.status === "on_hold" ? (
              <Button
                size="sm"
                variant="secondary"
                disabled={busy}
                onClick={() => void resumeWork()}
                data-testid="project-resume"
              >
                Resume work
              </Button>
            ) : (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  setHoldReason("");
                  setHoldOpen(true);
                }}
                data-testid="project-hold"
              >
                Put on hold
              </Button>
            )}
            {risk !== null ? (
              <Badge tone={risk.tone === "danger" ? "danger" : "warning"}>{risk.label}</Badge>
            ) : null}
            {client === null ? (
              <span>Internal</span>
            ) : (
              <Link href={`/clients/${client.id}`} className="max-w-[400px] truncate hover:text-foreground">
                {client.name}
              </Link>
            )}
            <span aria-hidden="true">·</span>
            <span className="whitespace-nowrap">{project.billing_method.replaceAll("_", " ")}</span>
            <span aria-hidden="true">·</span>
            <span className="whitespace-nowrap">{ownerName === "" ? "No PM recorded" : `PM ${ownerName}`}</span>
            {overdue.length > 0 ? (
              <Badge tone="danger" data-testid="risk-overdue">
                {overdue.length} overdue task{overdue.length === 1 ? "" : "s"}
              </Badge>
            ) : null}
            {blocked.length > 0 ? (
              <Badge tone="warning" data-testid="risk-blocked">
                {blocked.length} blocked
              </Badge>
            ) : null}
            {dueSoon.length > 0 ? (
              <Badge tone="warning" data-testid="risk-due-soon">
                {dueSoon.length} due in 3d
              </Badge>
            ) : null}
          </>
        }
        stats={
          <>
            <Kpi
              label="Due"
              mono
              value={formatWeekday(project.due_date)}
              sub={planWorkingDays === 0 ? "No working days set" : `${planWorkingDays} working days`}
            />
            <Kpi
              label="Hours"
              width="156px"
              value={
                planHours.estimateMinutes === 0 ? (
                  "Not estimated"
                ) : (
                  <span className="font-mono tabular-nums">
                    {Math.round(planHours.loggedMinutes / 60)} /{" "}
                    {Math.round(planHours.estimateMinutes / 60)} h
                  </span>
                )
              }
            >
              {planHours.estimateMinutes === 0 ? (
                <span className="whitespace-nowrap text-[12px] text-muted-foreground">
                  Add effort to tasks to see burn
                </span>
              ) : (
                <span className="flex items-center gap-2">
                  <span
                    role="meter"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={hoursUsedPct}
                    aria-label={`Hours used: ${hoursUsedPct}%`}
                    className="block h-[6px] min-w-[40px] flex-1 overflow-hidden rounded-pill"
                    style={{ background: "var(--m-border)" }}
                  >
                    <span
                      className="block h-full rounded-pill"
                      style={{
                        width: `${hoursUsedPct}%`,
                        background:
                          hoursUsedPct >= 80
                            ? "var(--m-status-warning-fg)"
                            : "var(--mc-stem)",
                      }}
                    />
                  </span>
                  <span className="whitespace-nowrap text-[12px] text-muted-foreground">
                    {hoursUsedPct}% used, {tProgress.pct}% done
                  </span>
                </span>
              )}
            </Kpi>
            <Kpi
              label="Budget"
              value={
                (project.budget_minor ?? budgetCap) === null
                  ? "No budget"
                  : money(project.budget_minor ?? budgetCap ?? 0, project.currency ?? budgetCurrency)
              }
              sub={
                budgetSpent === null
                  ? "Nothing billed yet"
                  : `${money(budgetSpent, budgetCurrency)} billed`
              }
            />
          </>
        }
        tabs={[
          { id: "plan", label: "Plan", testId: "record-tab-plan" },
          { id: "timeline", label: "Timeline" },
          { id: "tasks", label: "Tasks", count: tProgress.total, testId: "record-tab-tasks" },
          { id: "money", label: "Time and budget", testId: "record-tab-money" },
        ]}
        activeTab={tab}
        onTab={(id) => setTab(id as typeof tab)}
        commitment={
          <CommitmentBar>
            <span>
              <CommitKey>Started</CommitKey>
              <span className="font-mono tabular-nums">{formatWeekday(project.start_date)}</span>
            </span>
            <CommitSep />
            <span>
              <CommitKey>Promised go-live</CommitKey>
              <span className="font-mono tabular-nums">{formatWeekday(project.due_date)}</span>
            </span>
            <CommitSep />
            {nextMilestone === null ? (
              <span>
                <CommitKey>Next milestone</CommitKey>
                None set
              </span>
            ) : (
              <span className="flex items-center gap-2">
                <Diamond />
                <CommitKey className="m-0">Next milestone</CommitKey>
                <span className="max-w-[280px] truncate">{nextMilestone.name},</span>
                <span className="font-mono tabular-nums">{formatWeekday(nextMilestone.end_date)}</span>
              </span>
            )}
            <CommitSep />
            <CommitFlexible>
              {atRiskSentence === null ? (
                <>
                  <Icon name="check" size={14} />
                  No commitment at risk.
                </>
              ) : (
                <>
                  <Icon name="alert" size={14} />
                  <b className="font-semibold">1 commitment at risk:</b>
                  <span className="min-w-0 truncate">{atRiskSentence}</span>
                </>
              )}
            </CommitFlexible>
            <span className="text-muted-foreground">
              {baseline === null
                ? "No baseline yet"
                : `Baseline ${formatDay(baseline.captured_at)}, ${baseline.dated_tasks} of ${baseline.total_tasks} tasks dated`}
            </span>
          </CommitmentBar>
        }
      />

      {/* The delivery record's own pages. Change orders are their own document
          because approval, apply and bill are three separate acts (S13.6). */}
      <nav aria-label="Project records" className="flex gap-4 px-6 pt-3 text-[13px]">
        <Link href={`/projects/${projectId}/change-orders`} className="font-semibold hover:underline">
          Change orders
        </Link>
        <Link href={`/projects/${projectId}/timeline`} className="text-muted-foreground hover:underline">
          Timeline
        </Link>
      </nav>

      <div className="px-6 pt-4">
        {project.status === "on_hold" ? (
          <div
            data-testid="project-hold-note"
            className="mb-3 flex flex-wrap items-baseline gap-2 rounded-ctl border px-3 py-2 text-[12px]"
            style={{ borderColor: "var(--m-border)", color: "var(--m-text-secondary)" }}
          >
            <b className="font-semibold" style={{ color: "var(--m-text)" }}>
              Work is on hold.
            </b>
            {project.client_hold_reason ? (
              <span>Client reason: {project.client_hold_reason}</span>
            ) : (
              <span>The client sees only that work is paused.</span>
            )}
          </div>
        ) : null}
        {actionError !== null ? (
          <p role="alert" className="mb-3 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {actionError}
          </p>
        ) : null}

        {tab === "plan" ? (
          <ProjectPlanSurface
            projectId={projectId}
            projectDueDate={project.due_date ?? null}
            tasks={planTasks}
            phases={planPhases}
            deps={deps as unknown as Record<string, PlanTask[]>}
            calendar={calendar}
            risk={riskSignals}
            baseline={baseline}
            loading={!planLoaded}
            onChanged={refresh}
            onAddPhase={() => setPhaseOpen(true)}
          />
        ) : null}

        {tab === "timeline" ? (
          <div data-testid="project-timeline">
            <ProjectTimelineSurface projectId={projectId} />
          </div>
        ) : null}

        {tab === "money" ? (
          <>
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
            <Link
              href="/billing"
              className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2"
              style={{ color: "var(--m-text-secondary)" }}
            >
              Billing →
            </Link>
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
                  {burn.map((row) => {
                    // Money is financial data: without `financial:read` the
                    // route nulls the amount and the currency, and a null
                    // budget means either "redacted" or "none", so the strip
                    // says only that the amount is hidden. Minutes are not
                    // money and stay visible.
                    const moneyVisible =
                      row.consumed_minor !== null &&
                      row.consumed_minor !== undefined &&
                      row.currency !== null &&
                      row.currency !== undefined;
                    return (
                      <li
                        key={row.phase_id}
                        data-testid={`phase-burn-${row.phase_id}`}
                        className="flex flex-wrap items-center justify-between gap-2 text-[12px]"
                      >
                        <span className="flex items-center gap-2">
                          {row.alert === "over" ? (
                            <Badge tone="danger">over</Badge>
                          ) : row.alert === "warn" ? (
                            <Badge tone="warning">warn</Badge>
                          ) : null}
                          {row.phase_name}
                        </span>
                        <span className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                          {moneyVisible
                            ? money(row.consumed_minor ?? 0, row.currency ?? "USD")
                            : "Amount hidden"}
                          {moneyVisible
                            ? row.budget_minor !== null && row.budget_minor !== undefined
                              ? ` of ${money(row.budget_minor, row.currency ?? "USD")}`
                              : " · no budget"
                            : null}
                          <span style={{ color: "var(--m-text-muted)" }}> · {row.consumed_minutes} min</span>
                        </span>
                      </li>
                    );
                  })}
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
                        <Badge tone={releaseTone(release.status)}>{release.status}</Badge>
                        {release.label}
                        <span style={{ color: "var(--m-text-muted)" }}>{release.released_on}</span>
                      </span>
                      <span className="flex items-center gap-2">
                        <span className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                          {release.amount_minor === null || release.amount_minor === undefined || release.currency === null || release.currency === undefined ? "Amount hidden" : money(release.amount_minor, release.currency)}
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
          </>
        ) : null}

            {tab === "tasks" ? (
            <Card className="m-rise p-0" data-testid="project-tasks">
              <div className="flex flex-wrap items-center justify-between gap-2 border-b p-4" style={{ borderColor: "var(--m-border)" }}>
                <div className="flex items-center gap-3">
                  <h2 className="text-sm font-semibold">Delivery</h2>
                  <span className="text-[12px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                    {pProgress.done}/{pProgress.total} phases · {tProgress.done}/{tProgress.total} tasks
                  </span>
                </div>
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
                      <Badge tone={group.phase.status === "completed" ? "success" : "neutral"}>
                        {STATUS_LABEL[group.phase.status] ?? group.phase.status}
                      </Badge>
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
            </div>
          </div>
        )}
            </Card>
        ) : null}
      </div>

      {/* The dialogs live outside the tab panels, so a dialog opened from one
          tab survives switching to another rather than surprising. */}
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
      <DependencyDialog
        depTaskId={depTaskId}
        onClose={() => setDepTaskId(null)}
        selection={depSelection}
        setSelection={setDepSelection}
        tasks={tasks}
        busy={busy}
        onSave={() => void saveDependencies()}
      />
      <HoldDialog
        open={holdOpen}
        onOpenChange={setHoldOpen}
        reason={holdReason}
        setReason={setHoldReason}
        busy={busy}
        onHold={() => void putOnHold()}
      />
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
        <Badge tone={done ? "success" : task.status === "cancelled" ? "danger" : "neutral"}>
          {task.status.replaceAll("_", " ")}
        </Badge>
        {blocked ? <Badge tone="warning">blocked</Badge> : null}
        {overdue ? <Badge tone="danger">{due?.text ?? "overdue"}</Badge> : dueSoon && due !== null ? <Badge tone="warning">{due.text}</Badge> : null}
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

function HoldDialog(props: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  reason: string;
  setReason: (v: string) => void;
  busy: boolean;
  onHold: () => void;
}) {
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent
        title="Put this project on hold"
        description="Work pauses. The client sees the status and, only if you write one, the reason below."
      >
        <div className="grid gap-3 p-4">
          <div>
            <Label htmlFor="hold-reason">Reason the client can see (optional)</Label>
            <Input
              id="hold-reason"
              value={props.reason}
              maxLength={500}
              onChange={(e) => props.setReason(e.target.value)}
              placeholder="Waiting on the new switchgear delivery"
              data-testid="hold-reason"
            />
            <FieldHint>
              Shown in the client portal while the project is held. Leave it empty and the portal
              says only that work is paused. Your own note stays staff-side.
            </FieldHint>
          </div>
          <DialogActions>
            <Button variant="ghost" onClick={() => props.onOpenChange(false)}>
              Cancel
            </Button>
            <Button disabled={props.busy} onClick={props.onHold} data-testid="hold-confirm">
              {props.busy ? <Spinner /> : null} Put on hold
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
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
