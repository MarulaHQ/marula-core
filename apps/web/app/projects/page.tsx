// /projects — staff project engagements (Workspace-core UI, T3).
//
// Purpose: the project list with status/client filters and the gated
// project-create dialog riding the IF1–IF3 envelope. Links into
// /projects/[id] for phases, tasks, and dependencies.
// Non-purpose: no Gantt/Budget views here — this is the List-first home;
// view switchers land with those views per the UI backlog.
"use client";

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
  Table,
  Tag,
  Td,
  Th,
  Tr,
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

export default function ProjectsPage() {
  const auth = useRequireAuth();
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [clients, setClients] = useState<Client[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [statusFilter, setStatusFilter] = useState<ProjectStatus | "all">("all");
  const [clientFilter, setClientFilter] = useState<string>("all");

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

  const clientName = useMemo(() => {
    const map = new Map(clients.map((c) => [c.id, c.name]));
    return (id: string | null | undefined) =>
      id === null || id === undefined ? "Internal" : (map.get(id) ?? "—");
  }, [clients]);

  const filtered = useMemo(() => {
    if (projects === null) return null;
    return projects.filter(
      (p) =>
        (statusFilter === "all" || p.status === statusFilter) &&
        (clientFilter === "all" || p.client_id === clientFilter),
    );
  }, [projects, statusFilter, clientFilter]);

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
      setCreateError(result.message ?? "The project was rejected. Nothing was saved.");
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
    <div className="grid gap-8">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold">Projects</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Engagements with phases, tasks, and billing hooks.
          </p>
        </div>
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

      <Card>
        <div className="flex items-center justify-between p-4 pb-0">
          <h2 className="text-sm font-semibold">All projects</h2>
          <div className="flex gap-2">
            <Select value={statusFilter} onValueChange={(v) => setStatusFilter(v as ProjectStatus | "all")}>
              <SelectTrigger aria-label="Filter by status" className="w-36">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {STATUSES.map((s) => (
                  <SelectItem key={s} value={s}>
                    {s === "all" ? "All statuses" : s.replaceAll("_", " ")}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
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
          <Table>
            <thead>
              <tr>
                <Th>Name</Th>
                <Th>Client</Th>
                <Th>Status</Th>
                <Th>Billing</Th>
                <Th>Due</Th>
                <Th className="text-right">Budget</Th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((project) => (
                <Tr key={project.id}>
                  <Td>
                    <Link
                      href={`/projects/${project.id}`}
                      className="underline decoration-[var(--m-border)] underline-offset-2"
                    >
                      {project.name}
                    </Link>
                  </Td>
                  <Td>{clientName(project.client_id)}</Td>
                  <Td>
                    <Tag>{project.status.replaceAll("_", " ")}</Tag>
                  </Td>
                  <Td className="text-[13px]">{project.billing_method.replaceAll("_", " ")}</Td>
                  <Td>{project.due_date ?? "—"}</Td>
                  <Td className="text-right tabular-nums">
                    {project.budget_minor !== null && project.budget_minor !== undefined
                      ? money(project.budget_minor, project.currency ?? "USD")
                      : "—"}
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>
    </div>
  );
}
