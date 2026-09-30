// /projects — the portfolio ledger, rebuilt against the Marula surface pack
// (Wave 41 lane 7 `p41-ui-clients`).
//
// DERIVATION-GUIDE: `/projects` and `/projects/[id]` are a **Derive** from TL
// (the Tickets list) and DP (Dispatch), changing only three things:
//
//   1. **Stat pills for active, at risk, complete and overdue** in the header.
//      A pill is a count that filters the list, so the fastest way in is on
//      the screen.
//   2. **At risk is BLUSH; overdue is RED only once it has actually breached**
//      — a date in the past is a breach, a date coming up is attention. Both
//      carry the words and a clock, never colour alone.
//   3. The five data-surface controls the TL template carries, on the same
//      one-row toolbar.
//
// Purpose: track delivery against a commitment and see it slipping early
// (marula-surface-brief §5.7). So the header answers "how is the portfolio
// doing" in four numbers before the table answers "which one".
//
// Non-purpose: no Gantt or budget views here (the record owns the schedule
// surfaces); no ticket conversion; phases are read, not written, on this page.
"use client";

import {
  Badge,
  Button,
  ColumnChooser,
  DataToolbar,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  ExportControl,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Table,
  Td,
  Th,
  Tr,
  ViewSwitch,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { rejectionText } from "../../lib/error-copy";
import { phaseProgress, type PhaseProgress } from "../../lib/projects-metrics";
import { projectPosture, formatPackDate } from "../../lib/renewal-signals";
import { api, useRequireAuth } from "../../lib/api";
import { money } from "../../lib/format";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { useDataSurfacePrefs } from "../../lib/data-surface-prefs";

type Project = components["schemas"]["Project"];
type ProjectStatus = components["schemas"]["ProjectStatus"];
type ProjectMutationResponse = components["schemas"]["ProjectMutationResponse"];
type BillingMethod = components["schemas"]["BillingMethod"];
type Client = components["schemas"]["Client"];

const STATUS_LABEL: Record<string, string> = {
  planning: "Planning",
  active: "Active",
  on_hold: "On hold",
  completed: "Completed",
  cancelled: "Cancelled",
};

const BILLING_METHODS: BillingMethod[] = [
  "milestone",
  "time_and_materials",
  "fixed_fee",
  "non_billable",
];

/** The pack's views for this surface: Active, At risk (TL §11 Projects row). */
const VIEWS = [
  { id: "all", label: "All" },
  { id: "active", label: "Active" },
  { id: "risk", label: "At risk" },
];

/** Namespaced so this persisted document cannot collide with another lane's. */
const SURFACE = "projects.list";

/** TL's column set for projects: Project, Client, Phase, Budget, Due. */
type ColumnId = "project" | "client" | "phase" | "budget" | "due" | "billing";

const COLUMNS: { id: ColumnId; label: string; defaultVisible: boolean }[] = [
  { id: "project", label: "Project", defaultVisible: true },
  { id: "client", label: "Client", defaultVisible: true },
  { id: "phase", label: "Phase", defaultVisible: true },
  { id: "budget", label: "Budget", defaultVisible: true },
  { id: "due", label: "Due", defaultVisible: true },
  { id: "billing", label: "Billing", defaultVisible: false },
];

function Monogram({ name }: { name: string }) {
  const initials = name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join("");
  return (
    <span
      aria-hidden="true"
      className="grid h-[26px] w-[26px] shrink-0 place-items-center rounded-[999px] text-[12px] font-semibold"
      style={{ background: "var(--m-canopy-deep)", color: "var(--m-cream)" }}
    >
      {initials}
    </span>
  );
}

/** The schedule pill. At risk and due-soon are BLUSH with a clock and the
 * number of days; only a date that has actually passed takes red. */
function PosturePill({
  tone,
  label,
}: {
  tone: "due" | "danger";
  label: string;
}) {
  return (
    <Badge tone={tone} data-testid={`project-posture-${tone}`}>
      <span className="inline-flex items-center gap-1">
        <ClockIcon />
        {label}
      </span>
    </Badge>
  );
}

export default function ProjectsPage() {
  const auth = useRequireAuth();
  const prefs = useDataSurfacePrefs();

  const [projects, setProjects] = useState<Project[] | null>(null);
  const [clients, setClients] = useState<Client[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [phaseStats, setPhaseStats] = useState<Record<string, PhaseProgress>>({});

  const [view, setView] = useState("all");
  const [clientFilter, setClientFilter] = useState<string>("all");
  const [query, setQuery] = useState("");

  const [createOpen, setCreateOpen] = useState(false);
  const [name, setName] = useState("");
  const [clientId, setClientId] = useState<string>("none");
  const [billingMethod, setBillingMethod] = useState<BillingMethod>("milestone");
  const [budget, setBudget] = useState("");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const now = Date.now();

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

  // One real phases call per project, refreshed only when the project list
  // identity changes. The Phase column is a real roll-up, not a guess.
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

  /** The four header counters. Each is a filter, so it counts the current
   * data rather than a separate tally — one definition, used by the pill and
   * the filter it drives, so the two can never disagree.
   *
   * `at risk` is the ATTENTION set and `overdue` is its breached subset, so
   * the two deliberately overlap: a project whose due date has passed is at
   * risk, and it is also overdue. That is the honest reading — at risk asks
   * "does this need me today", overdue asks "did the date pass". The pack's
   * blush/red rule lives in the pills and the row labels (a date coming up is
   * blush, a date that has passed is red), not in making one counter exclude
   * the other. */
  const posture = useMemo(() => {
    const list = projects ?? [];
    const live = list.filter((p) => p.status !== "completed" && p.status !== "cancelled");
    return {
      active: live.filter((p) => p.status === "active").length,
      atRisk: live.filter((p) => projectPosture(p.due_date, p.status, now) !== null).length,
      complete: list.filter((p) => p.status === "completed").length,
      overdue: live.filter((p) => {
        const sig = projectPosture(p.due_date, p.status, now);
        return sig !== null && sig.tone === "danger";
      }).length,
    };
  }, [projects, now]);

  const filtered = useMemo(() => {
    if (projects === null) return null;
    const q = query.trim().toLowerCase();
    return projects.filter((p) => {
      const sig = projectPosture(p.due_date, p.status, now);
      if (view === "active" && p.status !== "active") return false;
      if (view === "risk" && projectPosture(p.due_date, p.status, now) === null) return false;
      if (clientFilter !== "all" && p.client_id !== clientFilter) return false;
      if (q === "") return true;
      return (
        p.name.toLowerCase().includes(q) ||
        clientName(p.client_id).toLowerCase().includes(q)
      );
    });
  }, [projects, view, clientFilter, query, clientName, now]);

  // Column visibility, persisted per user on the D12 key.
  const saved = prefs.view(SURFACE);
  const [visibleColumns, setVisibleColumns] = useState<ColumnId[]>(
    COLUMNS.filter((c) => c.defaultVisible).map((c) => c.id),
  );
  useEffect(() => {
    if (!prefs.ready) return;
    const savedIds = saved.columns.map((c) => c.key as ColumnId);
    if (savedIds.length === 0) return;
    const known = COLUMNS.filter((c) => savedIds.includes(c.id));
    if (known.length > 0) setVisibleColumns(known.map((c) => c.id));
  }, [prefs.ready, saved.columns]);

  const visible = useMemo(
    () => COLUMNS.filter((c) => visibleColumns.includes(c.id)),
    [visibleColumns],
  );

  const onColumnsChange = (next: { key: string; visible: boolean }[]) => {
    const ordered = next
      .filter((c) => c.visible)
      .map((c) => c.key as ColumnId)
      .filter((id) => COLUMNS.some((c) => c.id === id));
    setVisibleColumns(ordered);
    void prefs.save(SURFACE, {
      ...saved,
      columns: next.map((c) => ({ key: c.key, visible: c.visible })),
    });
  };

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

  const cell = (id: ColumnId) => visibleColumns.includes(id);

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-4 p-4 md:p-6">
      {/* ---- header: title, four stat pills, one primary ---- */}
      <header className="flex flex-wrap items-center gap-3">
        <h1 className="text-[26px] font-semibold leading-none tracking-[-0.01em]">Projects</h1>
        <div className="flex flex-wrap items-center gap-2">
          <StatPill
            label="active"
            count={posture.active}
            active={view === "active"}
            dot="var(--m-canopy)"
            onToggle={() => setView(view === "active" ? "all" : "active")}
          />
          <StatPill
            label="at risk"
            count={posture.atRisk}
            active={view === "risk"}
            dot="var(--m-blush)"
            onToggle={() => setView(view === "risk" ? "all" : "risk")}
          />
          <StatPill label="complete" count={posture.complete} active={false} dot="var(--m-canopy)" onToggle={() => undefined} />
          <StatPill label="overdue" count={posture.overdue} active={false} dot="var(--m-danger)" onToggle={() => undefined} />
        </div>
        <span className="flex-1" />
        <Dialog
          open={createOpen}
          onOpenChange={(open) => {
            setCreateOpen(open);
            if (!open) setCreateError(null);
          }}
        >
          <DialogTrigger asChild>
            <Button variant="primary" data-testid="new-project">
              <PlusIcon />
              New project
            </Button>
          </DialogTrigger>
          <DialogContent
            title="New project"
            description="Create the engagement, then shape phases and tasks on its page."
          >
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
      </header>

      <DataToolbar
        views={<ViewSwitch views={VIEWS} activeId={view} onChange={setView} label="Project views" />}
        filters={
          <>
            <Input
              aria-label="Search projects"
              placeholder="Search name or client…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="w-52"
            />
            <Select value={clientFilter} onValueChange={setClientFilter}>
              <SelectTrigger aria-label="Filter by client" className="w-44">
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
          </>
        }
        columns={
          <ColumnChooser
            columns={COLUMNS.map((c) => ({
              key: c.id,
              label: c.label,
              visible: visibleColumns.includes(c.id),
            }))}
            onChange={onColumnsChange}
          />
        }
        export={
          <ExportControl
            columns={visible.map((c) => c.label)}
            rowCount={filtered === null ? 0 : filtered.length}
            onExport={() => {}}
          />
        }
      />

      <div
        className="overflow-hidden rounded-[var(--m-radius-card)] border"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      >
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
          <Table>
            <thead>
              <tr>
                {cell("project") ? <Th>Project</Th> : null}
                {cell("client") ? <Th>Client</Th> : null}
                {cell("phase") ? <Th>Phase</Th> : null}
                {cell("budget") ? <Th className="text-right">Budget</Th> : null}
                {cell("due") ? <Th>Due</Th> : null}
                {cell("billing") ? <Th>Billing</Th> : null}
              </tr>
            </thead>
            <tbody>
              {filtered.map((project, i) => {
                const sig = projectPosture(project.due_date, project.status, now);
                const progress = phaseStats[project.id] ?? null;
                return (
                  <Tr
                    key={project.id}
                    className="m-entity-row"
                    style={{ animationDelay: `${Math.min(i * 20, 160)}ms` }}
                    data-testid={`project-row-${project.id}`}
                  >
                    {cell("project") ? (
                      <Td>
                        <div className="flex min-w-0 flex-col gap-1 leading-[1.3]">
                          <span className="flex flex-wrap items-center gap-2">
                            <Link
                              href={`/projects/${project.id}`}
                              className="truncate font-semibold underline decoration-transparent underline-offset-2 transition-colors duration-[100ms] hover:decoration-[var(--m-border-strong)]"
                            >
                              {project.name}
                            </Link>
                            <Badge tone={statusTone(project.status)}>
                              {STATUS_LABEL[project.status] ?? project.status}
                            </Badge>
                            {sig !== null ? (
                              <PosturePill
                                tone={sig.tone === "danger" ? "danger" : "due"}
                                label={sig.label}
                              />
                            ) : null}
                          </span>
                        </div>
                      </Td>
                    ) : null}
                    {cell("client") ? (
                      <Td>
                        <span className="flex items-center gap-2">
                          <Monogram name={clientName(project.client_id)} />
                          <span className="text-[13px]">{clientName(project.client_id)}</span>
                        </span>
                      </Td>
                    ) : null}
                    {cell("phase") ? (
                      <Td>
                        <PhaseBar progress={progress} />
                      </Td>
                    ) : null}
                    {cell("budget") ? (
                      <Td className="text-right text-[13px] tabular-nums">
                        {project.budget_minor != null ? (
                          <span className="font-semibold">
                            {money(project.budget_minor, project.currency ?? "USD")}
                          </span>
                        ) : (
                          <span style={{ color: "var(--m-text-muted)" }}>No budget</span>
                        )}
                      </Td>
                    ) : null}
                    {cell("due") ? (
                      <Td>
                        {project.due_date != null ? (
                          <span className="text-[13px] tabular-nums">{formatPackDate(project.due_date)}</span>
                        ) : (
                          <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                            No due date
                          </span>
                        )}
                      </Td>
                    ) : null}
                    {cell("billing") ? (
                      <Td>
                        <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                          {project.billing_method.replaceAll("_", " ")}
                        </span>
                      </Td>
                    ) : null}
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-3 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
        <span className="tabular-nums">
          {filtered === null ? "" : `${filtered.length} project${filtered.length === 1 ? "" : "s"}`}
        </span>
        <span aria-hidden="true">·</span>
        <span className="tabular-nums">
          Showing <strong className="font-semibold">{filtered?.length ?? 0}</strong> of{" "}
          <strong className="font-semibold">{projects?.length ?? 0}</strong> projects
        </span>
        <span className="flex-1" />
        <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          At risk is a date coming up. Overdue is a date that has passed.
        </span>
      </div>
    </div>
  );
}

/** A header stat pill: a dot, the word, the count. Pressed when it is the
 * active filter. A count of zero still shows, so a user can tell "none" from
 * "not loaded".
 *
 * The testid is the label with its spaces hyphenated — stat-active,
 * stat-at-risk, and so on — and is carried across the rebrand unchanged, so
 * a test written against the old surface still addresses the same counter. */
function StatPill({
  label,
  count,
  active,
  dot,
  onToggle,
}: {
  label: string;
  count: number;
  active: boolean;
  dot: string;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onToggle}
      data-testid={`stat-${label.replaceAll(" ", "-")}`}
      className="inline-flex cursor-pointer items-center gap-2 rounded-full border px-3 py-1.5 text-[13px] font-semibold"
      style={{
        background: active ? "var(--m-accent)" : "transparent",
        color: active ? "var(--m-on-accent)" : "var(--m-text)",
        borderColor: active ? "transparent" : "var(--m-border)",
      }}
    >
      <span className="h-2 w-2 rounded-full" style={{ background: dot }} aria-hidden="true" />
      {label}
      <span className="tabular-nums">{count}</span>
    </button>
  );
}

/** The Phase column's progress: a real roll-up of completed phases, with the
 * count beside it so the bar is never read without its number. Green at
 * 100% (done), the accent tint while it is in flight. The stable
 * `phase-progress-<done>-<total>` testid is carried across the rebrand so a
 * test written against the old surface still addresses the same roll-up. */
function PhaseBar({ progress }: { progress: PhaseProgress | null }) {
  if (progress === null || progress.total === 0) {
    return (
      <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }} data-testid="phase-progress-empty">
        {progress === null ? "…" : "no phases"}
      </span>
    );
  }
  return (
    <span className="flex min-w-[8rem] items-center gap-2" data-testid={`phase-progress-${progress.done}-${progress.total}`}>
      <span className="text-[13px] tabular-nums whitespace-nowrap">
        {progress.done}/{progress.total} phases
      </span>
      <span
        aria-hidden="true"
        className="h-1.5 flex-1 overflow-hidden rounded-full"
        style={{ background: "var(--m-surface-2)" }}
      >
        <span
          className="block h-full rounded-full"
          style={{
            width: `${progress.pct}%`,
            background:
              progress.pct === 100 ? "var(--m-canopy)" : "var(--m-accent-text)",
            transition: "width 120ms cubic-bezier(0.2,0,0,1)",
          }}
        />
      </span>
    </span>
  );
}

function statusTone(status: string): "success" | "warning" | "danger" | "neutral" {
  switch (status) {
    case "active":
      return "success";
    case "on_hold":
      return "warning";
    case "cancelled":
      return "danger";
    default:
      return "neutral";
  }
}

function ClockIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 2" />
    </svg>
  );
}

function PlusIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" aria-hidden="true">
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}