// /projects — staff project engagements (Wave 17 lane 4 UX pass).
//
// Purpose: the portfolio list with health signals and phase progress:
// stat strip (active / at-risk / on hold), client-side name+client search,
// status + client filters, and per-project phase-progress bars fetched
// from the real /projects/{id}/phases endpoints (bounded: one call per
// visible project, cached until the list reloads). Create rides the
// IF1–IF3 envelope as before.
// Non-purpose: no Gantt/Budget views here (the detail page owns schedule
// surfaces); no ticket conversion.
"use client";

import { rejectionText } from "../../lib/error-copy";
import {
  projectRisk,
  projectStatusTone,
  phaseProgress,
  type PhaseProgress,
} from "../../lib/projects-metrics";
import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
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
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { money } from "../../lib/format";
import { runEnvelopeMutation } from "../../lib/envelope-write";

type Project = components["schemas"]["Project"];
type ProjectStatus = components["schemas"]["ProjectStatus"];
type ProjectMutationResponse = components["schemas"]["ProjectMutationResponse"];
type BillingMethod = components["schemas"]["BillingMethod"];
type Client = components["schemas"]["Client"];

const STATUSES: (ProjectStatus | "all")[] = [
  "all",
  "planning",
  "active",
  "on_hold",
  "completed",
  "cancelled",
];

const BILLING_METHODS: BillingMethod[] = [
  "milestone",
  "time_and_materials",
  "fixed_fee",
  "non_billable",
];

const STATUS_LABEL: Record<string, string> = {
  planning: "Planning",
  active: "Active",
  on_hold: "On hold",
  completed: "Completed",
  cancelled: "Cancelled",
};

export default function ProjectsPage() {
  const auth = useRequireAuth();
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [clients, setClients] = useState<Client[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [phaseStats, setPhaseStats] = useState<Record<string, PhaseProgress>>({});

  const [statusFilter, setStatusFilter] = useState<ProjectStatus | "all">("all");
  const [clientFilter, setClientFilter] = useState<string>("all");
  const [query, setQuery] = useState("");

  const [createOpen, setCreateOpen] = useState(false);
  const [name, setName] = useState("");
  const [clientId, setClientId] = useState<string>("none");
  const [billingMethod, setBillingMethod] = useState<BillingMethod>("milestone");
  const [budget, setBudget] = useState("");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const [p, c] = await Promise.all([
      api.api.GET("/api/v1/projects"),
      api.api.GET("/api/v1/clients"),
    ]);
    if (p.error !== undefined || p.data === undefined) {
      setLoadError("Failed to load projects.");
      setProjects([]);
    } else {
      setProjects(p.data);
      setLoadError(null);
    }
    if (c.error === undefined && c.data !== undefined) setClients(c.data);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  // Phase roll-ups for the progress column: one real phases call per
  // project, refreshed only when the project list identity changes.
  useEffect(() => {
    if (projects === null || projects.length === 0) return;
    let cancelled = false;
    void Promise.all(
      projects.map(async (project) => {
        const res = await api.api.GET("/api/v1/projects/{id}/phases", {
          params: { path: { id: project.id } },
        });
        return [project.id, phaseProgress(res.data ?? [])] as const;
      }),
    ).then((entries) => {
      if (!cancelled) setPhaseStats(Object.fromEntries(entries));
    });
    return () => {
      cancelled = true;
    };
  }, [projects]);

  const clientName = useMemo(() => {
    const map = new Map(clients.map((c) => [c.id, c.name]));
    return (id: string | null | undefined) =>
      id === null || id === undefined ? "Internal" : (map.get(id) ?? "—");
  }, [clients]);

  const filtered = useMemo(() => {
    if (projects === null) return null;
    const q = query.trim().toLowerCase();
    return projects.filter(
      (p) =>
        (statusFilter === "all" || p.status === statusFilter) &&
        (clientFilter === "all" || p.client_id === clientFilter) &&
        (q === "" ||
          p.name.toLowerCase().includes(q) ||
          clientName(p.client_id).toLowerCase().includes(q)),
    );
  }, [projects, statusFilter, clientFilter, query, clientName]);

  const stats = useMemo(() => {
    if (projects === null) return null;
    const active = projects.filter((p) => p.status === "active").length;
    const onHold = projects.filter((p) => p.status === "on_hold").length;
    const atRisk = projects.filter((p) => projectRisk(p, Date.now())?.tone === "danger").length;
    return { active, onHold, atRisk };
  }, [projects]);

  const createProject = async () => {
    if (name.trim() === "") return;
    setCreating(true);
    setCreateError(null);
    const tempId = `pending-project:${name}`;
    const optimistic: Project = {
      id: tempId,
      org_id: "",
      name: name.trim(),
      status: "planning",
      billing_method: billingMethod,
      client_id: clientId === "none" ? null : clientId,
      budget_minor: budget.trim() === "" ? null : Math.round(Number(budget) * 100),
      currency: budget.trim() === "" ? null : "USD",
      description: null,
      start_date: null,
      due_date: null,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    setProjects((prev) => [optimistic, ...(prev ?? [])]);
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateProjectRequest"],
      ProjectMutationResponse
    >({
      describe: "Create project",
      entityId: tempId,
      optimistic: () => {},
      revert: () => setProjects((prev) => (prev ?? []).filter((p) => p.id !== tempId)),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        name: name.trim(),
        client_id: clientId === "none" ? null : clientId,
        billing_method: billingMethod,
        budget_minor: budget.trim() === "" ? null : Math.round(Number(budget) * 100),
        currency: budget.trim() === "" ? null : "USD",
      }),
      request: (client, body) => client.POST("/api/v1/projects", { body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setCreating(false);
    if (!result.ok) {
      setCreateError(rejectionText(result, "The project was rejected. Nothing was saved."));
      return;
    }
    setCreateOpen(false);
    setName("");
    setBudget("");
    setClientId("none");
    void refresh();
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold">Projects</h1>
          <p className="mt-0.5 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Engagements with phases, tasks, and billing hooks.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {stats !== null ? (
            <>
              <HeaderStat label="active" value={stats.active} tone="success" />
              <HeaderStat label="at risk" value={stats.atRisk} tone="danger" />
              <HeaderStat label="on hold" value={stats.onHold} tone="warning" />
            </>
          ) : null}
          <Dialog
            open={createOpen}
            onOpenChange={(open) => {
              setCreateOpen(open);
              if (!open) setCreateError(null);
            }}
          >
            <DialogTrigger asChild>
              <Button>New project</Button>
            </DialogTrigger>
            <DialogContent title="New project" description="Create the engagement, then shape phases and tasks on its page.">
              <div className="grid gap-3 p-4">
                <div>
                  <Label htmlFor="project-name">Name</Label>
                  <Input id="project-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Exchange migration" />
                </div>
                <div>
                  <Label htmlFor="project-client">Client</Label>
                  <Select value={clientId} onValueChange={setClientId}>
                    <SelectTrigger id="project-client" className="w-56">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="none">Internal (no client)</SelectItem>
                      {clients.map((c) => (
                        <SelectItem key={c.id} value={c.id}>
                          {c.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div>
                  <Label htmlFor="project-billing">Billing method</Label>
                  <Select value={billingMethod} onValueChange={(v) => setBillingMethod(v as BillingMethod)}>
                    <SelectTrigger id="project-billing" className="w-56">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {BILLING_METHODS.map((m) => (
                        <SelectItem key={m} value={m}>
                          {m.replaceAll("_", " ")}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div>
                  <Label htmlFor="project-budget">Budget (USD)</Label>
                  <Input id="project-budget" type="number" min={0} step="0.01" value={budget} onChange={(e) => setBudget(e.target.value)} placeholder="Optional" />
                  <FieldHint>Leave empty for an unbudgeted project.</FieldHint>
                </div>
                {createError !== null ? (
                  <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                    {createError}
                  </p>
                ) : null}
                <DialogActions>
                  <Button variant="ghost" onClick={() => setCreateOpen(false)}>
                    Cancel
                  </Button>
                  <Button onClick={() => void createProject()} disabled={creating || name.trim() === ""}>
                    {creating ? <Spinner /> : null} Create project
                  </Button>
                </DialogActions>
              </div>
            </DialogContent>
          </Dialog>
        </div>
      </div>

      <Card className="p-0">
        <div className="flex flex-wrap items-center gap-2 border-b p-3" style={{ borderColor: "var(--m-border)" }}>
          <div className="w-56">
            <Input
              aria-label="Search projects"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search name or client…"
              className="w-full"
            />
          </div>
          <div className="w-40">
            <Select value={statusFilter} onValueChange={(v) => setStatusFilter(v as ProjectStatus | "all")}>
              <SelectTrigger aria-label="Filter by status" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {STATUSES.map((s) => (
                  <SelectItem key={s} value={s}>
                    {s === "all" ? "All statuses" : STATUS_LABEL[s]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="w-44">
            <Select value={clientFilter} onValueChange={setClientFilter}>
              <SelectTrigger aria-label="Filter by client" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All clients</SelectItem>
                {clients.map((c) => (
                  <SelectItem key={c.id} value={c.id}>
                    {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="ml-auto text-[12px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
            {filtered === null ? "" : `${filtered.length} project${filtered.length === 1 ? "" : "s"}`}
          </div>
        </div>

        {filtered === null ? (
          <div className="p-4">
            {loadError !== null ? (
              <p role="alert" className="text-[13px]">{loadError}</p>
            ) : (
              <Spinner />
            )}
          </div>
        ) : filtered.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No projects match. Create the first engagement to start shaping phases and tasks." />
          </div>
        ) : (
          <div className="grid grid-cols-1">
            {filtered.map((project, i) => (
              <ProjectRow
                key={project.id}
                project={project}
                index={i}
                clientLabel={clientName(project.client_id)}
                progress={phaseStats[project.id] ?? null}
              />
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}

function HeaderStat({ label, value, tone }: { label: string; value: number; tone: "success" | "warning" | "danger" }) {
  const fg =
    tone === "success"
      ? "var(--m-status-success-fg)"
      : tone === "warning"
        ? "var(--m-status-warning-fg)"
        : "var(--m-status-danger-fg)";
  return (
    <span
      className="inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-bold"
      style={{ background: "var(--m-bg-subtle)", color: fg }}
      data-testid={`stat-${label.replace(" ", "-")}`}
    >
      <span className="tabular-nums">{value}</span> {label}
    </span>
  );
}

function ProjectRow({
  project,
  index,
  clientLabel,
  progress,
}: {
  project: Project;
  index: number;
  clientLabel: string;
  progress: PhaseProgress | null;
}) {
  const risk = projectRisk(project, Date.now());
  return (
    <div
      className="m-rise row-hover grid grid-cols-1 items-center gap-x-4 gap-y-2 border-b px-4 py-3 transition-colors last:border-b-0 lg:grid-cols-[minmax(0,2.2fr)_auto_minmax(0,1.6fr)_minmax(0,1fr)_auto]"
      style={{ borderColor: "var(--m-border)", animationDelay: `${Math.min(index * 40, 200)}ms` }}
      data-testid={`project-row-${project.id}`}
    >
      <div className="min-w-0">
        <Link
          href={`/projects/${project.id}`}
          className="truncate text-[13px] font-semibold underline decoration-transparent underline-offset-2 transition-colors hover:decoration-[var(--m-border-strong)]"
        >
          {project.name}
        </Link>
        <div className="truncate text-[12px]" style={{ color: "var(--m-text-muted)" }}>
          {clientLabel}
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        <Tag tone={projectStatusTone(project.status)}>
          {STATUS_LABEL[project.status] ?? project.status.replaceAll("_", " ")}
        </Tag>
        {risk !== null ? (
          <Tag tone={risk.tone === "danger" ? "danger" : "warning"}>{risk.label}</Tag>
        ) : null}
      </div>
      <PhaseProgressCell progress={progress} />
      <div className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
        <span>{project.billing_method.replaceAll("_", " ")}</span>
        {project.budget_minor !== null && project.budget_minor !== undefined ? (
          <span className="ml-2 tabular-nums" style={{ color: "var(--m-text-muted)" }}>
            {money(project.budget_minor, project.currency ?? "USD")}
          </span>
        ) : null}
      </div>
      <div className="flex items-center justify-between gap-3 md:justify-end">
        {project.due_date !== null ? (
          <span className="whitespace-nowrap text-[12px] tabular-nums" style={{ color: risk?.tone === "danger" ? "var(--m-status-danger-fg)" : "var(--m-text-secondary)" }}>
            due {project.due_date}
          </span>
        ) : (
          <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>no due date</span>
        )}
        <Link
          href={`/projects/${project.id}`}
          className="text-[13px]"
          style={{ color: "var(--m-text-muted)" }}
          aria-label={`Open ${project.name}`}
        >
          →
        </Link>
      </div>
    </div>
  );
}

function PhaseProgressCell({ progress }: { progress: PhaseProgress | null }) {
  if (progress === null || progress.total === 0) {
    return (
      <div className="text-[12px]" style={{ color: "var(--m-text-muted)" }} data-testid="phase-progress-empty">
        {progress === null ? "…" : "no phases"}
      </div>
    );
  }
  return (
    <div className="min-w-0" data-testid={`phase-progress-${progress.done}-${progress.total}`}>
      <div className="h-1.5 w-full overflow-hidden rounded-full" style={{ background: "var(--m-bg-subtle)" }}>
        <div
          className="h-full rounded-full"
          style={{
            width: `${progress.pct}%`,
            background: progress.pct === 100 ? "var(--m-status-success-fg)" : "var(--m-accent)",
            transition: "width 120ms cubic-bezier(0.2, 0, 0, 1)",
          }}
        />
      </div>
      <div className="mt-1 text-[11px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
        {progress.done}/{progress.total} phases
        {progress.milestonesTotal > 0
          ? ` · milestones ${progress.milestonesDone}/${progress.milestonesTotal}`
          : ""}
      </div>
    </div>
  );
}
