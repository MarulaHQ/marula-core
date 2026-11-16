// /projects/templates/[id] — the template document, its preview, its
// validation, its lessons and its versions (S11.4, S11.9, S11.10, S11.11,
// S12.1–S12.11 where the API carries them).
//
// Two tabs do the heavy lifting: "Plan" (the tree the author reads) and
// "Document" (the spec they edit, validated before it can be saved or
// published). Publishing is its own step with a note, and restoring an old
// version makes a new draft — history is never rewritten.

"use client";

import {
  Badge,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  FieldError,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  SurfaceHeader,
  Table,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import Link from "next/link";
import { useParams, useRouter, useSearchParams } from "next/navigation";
import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import { api, useAuth, useRequireAuth } from "../../lib/api";
import {
  createFromTemplate,
  decideTemplateLesson,
  deliveryFetch,
  fetchTemplateLessons,
  fetchTemplatePreview,
  fetchTemplateValidation,
  fetchTemplateGallery,
  forkTemplate,
  setTemplateMeta,
  type GalleryTemplate,
  type TemplateLesson,
  type TemplatePreview,
  type ValidationReport,
} from "./delivery-api";

interface VersionRow {
  id: string;
  template_id: string;
  version: number;
  spec: Record<string, unknown>;
  status: string;
  source: string;
  note: string | null;
  published_at: string | null;
  created_at: string;
  current: boolean;
}

interface ClientRow {
  id: string;
  name: string;
}

function minutesLabel(minutes: number | null | undefined): string {
  if (minutes === null || minutes === undefined) return "—";
  return `${Math.round((minutes / 60) * 10) / 10}h`;
}

export function TemplateEditor() {
  const params = useParams<{ id: string }>();
  const search = useSearchParams();
  const router = useRouter();
  const auth = useRequireAuth();
  const { me } = useAuth();
  const canEdit = (me?.permissions ?? []).includes("projects:edit");
  const canCreate = (me?.permissions ?? []).includes("projects:create");

  const [card, setCard] = useState<GalleryTemplate | null>(null);
  const [preview, setPreview] = useState<TemplatePreview | null>(null);
  const [validation, setValidation] = useState<ValidationReport | null>(null);
  const [lessons, setLessons] = useState<TemplateLesson[]>([]);
  const [versions, setVersions] = useState<VersionRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionNote, setActionNote] = useState<string | null>(null);
  const [tab, setTab] = useState<"plan" | "document" | "validation" | "lessons" | "versions">(
    "plan",
  );
  const [draft, setDraft] = useState("");
  const [draftNote, setDraftNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [clients, setClients] = useState<ClientRow[]>([]);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [nextCard, nextPreview, nextValidation, nextLessons] = await Promise.all([
        deliveryFetch<GalleryTemplate>(`/api/v1/project-templates/${params.id}/gallery-card`),
        fetchTemplatePreview(params.id),
        fetchTemplateValidation(params.id),
        fetchTemplateLessons(params.id),
      ]);
      const versionBody = await deliveryFetch<{ versions: VersionRow[] }>(
        `/api/v1/project-templates/${params.id}/versions`,
      );
      setCard(nextCard);
      setPreview(nextPreview);
      setValidation(nextValidation);
      setLessons(nextLessons);
      setVersions(versionBody.versions);
      const current = versionBody.versions.find((version) => version.current);
      setDraft(JSON.stringify(current?.spec ?? {}, null, 2));
      const clientBody = await api.api.GET("/api/v1/clients");
      if (clientBody.data) setClients(clientBody.data as ClientRow[]);
    } catch (e) {
      setError((e as Error).message);
    }
  }, [params.id]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void load();
  }, [auth.status, load]);

  const unheld = useMemo(
    () => (validation?.issues ?? []).filter((issue) => issue.code === "role_unheld"),
    [validation],
  );

  async function onSaveDraft() {
    setBusy(true);
    setActionError(null);
    try {
      const spec = JSON.parse(draft) as Record<string, unknown>;
      await deliveryFetch(`/api/v1/project-templates/${params.id}/versions`, {
        method: "POST",
        body: JSON.stringify({ spec, note: draftNote || null }),
      });
      setActionNote("Draft saved. Nothing has changed for anyone using the template.");
      await load();
    } catch (e) {
      setActionError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function onPublish(version: number) {
    setBusy(true);
    setActionError(null);
    try {
      await deliveryFetch(
        `/api/v1/project-templates/${params.id}/versions/${version}/publish`,
        { method: "POST", body: "{}" },
      );
      setActionNote(`Version ${version} is now what a new project reads.`);
      await load();
    } catch (e) {
      setActionError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function onRestore(version: VersionRow) {
    setDraft(JSON.stringify(version.spec, null, 2));
    setTab("document");
    setActionNote(
      `Version ${version.version} is in the editor. Saving makes a new draft; it does not rewrite history.`,
    );
  }

  async function onDecide(lesson: TemplateLesson, state: "accepted" | "dismissed") {
    setActionError(null);
    try {
      await decideTemplateLesson(
        params.id,
        lesson.id,
        state,
        state === "dismissed" ? "Not true for the work we deliver" : undefined,
      );
      setLessons(await fetchTemplateLessons(params.id));
    } catch (e) {
      setActionError((e as Error).message);
    }
  }

  async function onFork() {
    if (card === null) return;
    const name = window.prompt("Name for the fork", `${card.name} (ours)`);
    if (name === null || name.trim() === "") return;
    setBusy(true);
    try {
      const forked = await forkTemplate(card.id, name.trim());
      router.push(`/projects/templates/${forked.id}`);
    } catch (e) {
      setActionError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function onMeta(values: {
    category: string;
    typical: string;
    min: string;
    max: string;
  }) {
    setBusy(true);
    setActionError(null);
    try {
      await setTemplateMeta(params.id, {
        category: values.category.trim() || null,
        typical_days: values.typical.trim() === "" ? null : Number(values.typical),
        effort_min_minutes:
          values.min.trim() === "" ? null : Math.round(Number(values.min) * 60),
        effort_max_minutes:
          values.max.trim() === "" ? null : Math.round(Number(values.max) * 60),
      });
      await load();
    } catch (e) {
      setActionError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  if (auth.status === "loading" || (card === null && error === null)) {
    return (
      <div className="flex items-center gap-2 p-8 text-[13px] text-muted-foreground">
        <Spinner /> Reading the template…
      </div>
    );
  }
  if (error !== null || card === null) {
    return (
      <div className="p-6">
        <div role="alert" className="flex items-center gap-3">
          <span className="flex-1">This template could not be read: {error}</span>
          <Button variant="secondary" onClick={() => void load()}>
            Try again
          </Button>
          <Button asChild variant="ghost">
            <Link href="/projects/templates">Back to templates</Link>
          </Button>
        </div>
      </div>
    );
  }

  const currentVersion = versions.find((version) => version.current) ?? null;

  return (
    <div className="flex flex-col gap-5 p-6">
      <SurfaceHeader
        eyebrow={
          <Link href="/projects/templates" className="hover:underline">
            Templates
          </Link>
        }
        title={card.name}
        description={card.description ?? "No description"}
        meta={
          <div className="flex flex-wrap items-center gap-2">
            {card.is_shipped ? <Badge tone="neutral">Shipped</Badge> : <Badge tone="outline">Yours</Badge>}
            {card.archived ? <Badge tone="offline">Archived</Badge> : null}
            <span className="text-[12px] text-muted-foreground">
              {card.phase_count} phases · {card.task_count} tasks ·{" "}
              {currentVersion === null ? "no published version" : `v${currentVersion.version}`}
            </span>
          </div>
        }
        actions={
          <div className="flex items-center gap-2">
            {canEdit && !card.is_shipped ? (
              <Button variant="secondary" disabled={busy} onClick={() => void onFork()}>
                Fork
              </Button>
            ) : null}
            {canCreate && !card.archived ? (
              <CreateDialog
                templateId={card.id}
                templateName={card.name}
                clients={clients}
                openByDefault={search.get("use") === "1"}
              />
            ) : null}
          </div>
        }
      />

      {actionError !== null ? (
        <p role="alert" className="rounded-ctl border border-destructive px-3 py-2 text-[13px] text-destructive">
          {actionError}
        </p>
      ) : null}
      {actionNote !== null ? (
        <p className="rounded-ctl bg-muted px-3 py-2 text-[13px] text-muted-foreground">{actionNote}</p>
      ) : null}

      <div role="tablist" aria-label="Template sections" className="flex gap-1 border-b border-border">
        {(
          [
            ["plan", "Plan"],
            ["document", "Document"],
            ["validation", `Validation${validation?.publishable === false ? " (!)" : ""}`],
            ["lessons", `Lessons${lessons.filter((l) => l.state === "open").length > 0 ? ` (${lessons.filter((l) => l.state === "open").length})` : ""}`],
            ["versions", `Versions (${versions.length})`],
          ] as const
        ).map(([key, label]) => (
          <button
            key={key}
            type="button"
            role="tab"
            aria-selected={tab === key}
            onClick={() => setTab(key)}
            className={
              tab === key
                ? "border-b-2 border-foreground px-3 py-2 text-[13px] font-semibold"
                : "border-b-2 border-transparent px-3 py-2 text-[13px] text-muted-foreground hover:text-foreground"
            }
          >
            {label}
          </button>
        ))}
      </div>

      {tab === "plan" && preview !== null ? (
        <section aria-label="Plan" className="flex flex-col gap-4">
          <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
            <Stat label="Phases" value={String(preview.phases.length)} />
            <Stat label="Tasks" value={String(preview.tasks.length)} />
            <Stat label="Effort" value={minutesLabel(preview.total_effort_minutes)} />
            <Stat
              label="Dated"
              value={`${preview.dated_tasks} of ${preview.tasks.length}`}
            />
          </div>
          <Table>
            <thead>
              <Tr>
                <Th>Phase / task</Th>
                <Th>Role</Th>
                <Th className="text-right">Days</Th>
                <Th className="text-right">Effort</Th>
                <Th>Depends on</Th>
              </Tr>
            </thead>
            <tbody>
              {preview.phases.map((phase) => (
                <Fragment key={phase.key}>
                  <Tr style={{ background: "var(--m-surface-2)" }}>
                    <Td className="font-semibold">{phase.name}</Td>
                    <Td />
                    <Td />
                    <Td className="text-right font-mono tabular-nums">
                      {minutesLabel(phase.effort_minutes)}
                    </Td>
                    <Td />
                  </Tr>
                  {preview.tasks
                    .filter((task) => task.phase_key === phase.key)
                    .map((task) => (
                      <Tr key={task.key}>
                        <Td className="pl-6">
                          {task.title}
                          {task.optional ? (
                            <Badge tone="outline" className="ml-2">
                              Optional
                            </Badge>
                          ) : null}
                        </Td>
                        <Td><span className="text-muted-foreground">{task.role_key ?? "—"}</span></Td>
                        <Td className="text-right font-mono tabular-nums">
                          {task.duration_days ?? "—"}
                        </Td>
                        <Td className="text-right font-mono tabular-nums">
                          {minutesLabel(task.effort_minutes)}
                        </Td>
                        <Td><span className="text-[12px] text-muted-foreground">
                          {task.depends_on.length === 0 ? "—" : task.depends_on.join(", ")}
                        </span></Td>
                      </Tr>
                    ))}
                </Fragment>
              ))}
            </tbody>
          </Table>
        </section>
      ) : null}

      {tab === "document" ? (
        <section aria-label="Document" className="flex flex-col gap-3">
          <p className="text-[13px] text-muted-foreground">
            The template is data. Edit it here, save a draft, then publish when
            validation is clean. Projects already running stay on the version they
            started from.
          </p>
          <Label htmlFor="spec">Document</Label>
          <textarea
            id="spec"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            spellCheck={false}
            className="min-h-[360px] w-full rounded-ctl border border-border bg-card p-3 font-mono text-[12px] leading-[1.5]"
          />
          <div className="flex flex-wrap items-end gap-3">
            <div className="flex-1">
              <Label htmlFor="note">Version note</Label>
              <Input
                id="note"
                value={draftNote}
                onChange={(event) => setDraftNote(event.target.value)}
                placeholder="What changed, and why"
              />
              <FieldHint>Says what a reader should know before using this version.</FieldHint>
            </div>
            <Button disabled={busy || !canEdit} onClick={() => void onSaveDraft()}>
              Save draft
            </Button>
          </div>
          {!canEdit ? (
            <FieldError>Editing a template needs the projects edit permission.</FieldError>
          ) : null}
        </section>
      ) : null}

      {tab === "validation" && validation !== null ? (
        <section aria-label="Validation" className="flex flex-col gap-3">
          <p className="text-[13px]">
            {validation.publishable
              ? "Nothing here blocks a publish."
              : "Fix the errors before publishing. Warnings are honest facts, not blockers."}
          </p>
          {unheld.length > 0 ? (
            <p className="rounded-ctl bg-muted px-3 py-2 text-[13px] text-muted-foreground">
              {unheld.length} role{unheld.length === 1 ? "" : "s"} named here are not
              roles in this organisation yet. Tasks will be created unassigned until
              they are.
            </p>
          ) : null}
          {validation.issues.length === 0 ? (
            <EmptyState message="No issues. The document is internally consistent." />
          ) : (
            <ul className="flex flex-col gap-2">
              {validation.issues.map((issue, index) => (
                <li
                  key={`${issue.code}-${index}`}
                  className="flex items-start gap-3 rounded-ctl border border-border px-3 py-2 text-[13px]"
                >
                  <Badge
                    tone={
                      issue.severity === "error"
                        ? "danger"
                        : issue.severity === "warning"
                          ? "warning"
                          : "neutral"
                    }
                  >
                    {issue.severity}
                  </Badge>
                  <span className="flex-1">{issue.message}</span>
                  <span className="font-mono text-[11px] text-muted-foreground">{issue.code}</span>
                </li>
              ))}
            </ul>
          )}
        </section>
      ) : null}

      {tab === "lessons" ? (
        <section aria-label="Lessons" className="flex flex-col gap-3">
          <p className="text-[13px] text-muted-foreground">
            What closed projects taught about this template. Each suggestion names its
            sample and stays a suggestion until someone accepts it into the draft.
          </p>
          {lessons.length === 0 ? (
            <EmptyState message="No lessons yet. They appear after projects created from this template close, with their time logged." />
          ) : (
            <ul className="flex flex-col gap-2">
              {lessons.map((lesson) => (
                <li key={lesson.id} className="rounded-ctl border border-border p-3 text-[13px]">
                  <div className="flex items-center gap-2">
                    <Badge tone="neutral">{lesson.kind.replaceAll("_", " ")}</Badge>
                    <span className="flex-1">{lesson.message}</span>
                    <Badge tone={lesson.state === "open" ? "warning" : "success"}>
                      {lesson.state}
                    </Badge>
                  </div>
                  {lesson.state === "open" ? (
                    <div className="mt-2 flex gap-2">
                      <Button size="sm" variant="secondary" onClick={() => void onDecide(lesson, "accepted")}>
                        Accept into draft
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => void onDecide(lesson, "dismissed")}>
                        Dismiss
                      </Button>
                    </div>
                  ) : lesson.dismiss_reason !== null ? (
                    <p className="mt-1 text-[12px] text-muted-foreground">
                      Dismissed: {lesson.dismiss_reason}
                    </p>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
        </section>
      ) : null}

      {tab === "versions" ? (
        <section aria-label="Versions" className="flex flex-col gap-3">
          <Table>
            <thead>
              <Tr>
                <Th>Version</Th>
                <Th>Status</Th>
                <Th>Source</Th>
                <Th>Note</Th>
                <Th>Published</Th>
                <Th />
              </Tr>
            </thead>
            <tbody>
              {versions.map((version) => (
                <Tr key={version.id}>
                  <Td className="font-mono tabular-nums">v{version.version}</Td>
                  <Td>
                    <Badge tone={version.current ? "success" : "neutral"}>
                      {version.current ? "current" : version.status}
                    </Badge>
                  </Td>
                  <Td><span className="text-muted-foreground">{version.source}</span></Td>
                  <Td><span className="text-muted-foreground">{version.note ?? "—"}</span></Td>
                  <Td className="font-mono text-[12px]">
                    {version.published_at?.slice(0, 10) ?? "—"}
                  </Td>
                  <Td>
                    <div className="flex justify-end gap-1">
                      <Button size="sm" variant="ghost" onClick={() => void onRestore(version)}>
                        Restore
                      </Button>
                      {canEdit && !version.current && version.status !== "published" ? (
                        <Button
                          size="sm"
                          variant="secondary"
                          disabled={busy}
                          onClick={() => void onPublish(version.version)}
                        >
                          Publish
                        </Button>
                      ) : null}
                    </div>
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
          <MetaForm card={card} disabled={busy || !canEdit} onSave={onMeta} />
        </section>
      ) : null}
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-card border border-border bg-card px-3 py-2">
      <div className="text-[12px] text-muted-foreground">{label}</div>
      <div className="font-mono text-[16px] tabular-nums">{value}</div>
    </div>
  );
}

function MetaForm({
  card,
  disabled,
  onSave,
}: {
  card: GalleryTemplate;
  disabled: boolean;
  onSave: (values: { category: string; typical: string; min: string; max: string }) => Promise<void>;
}) {
  const [category, setCategory] = useState(card.category ?? "");
  const [typical, setTypical] = useState(card.typical_days?.toString() ?? "");
  const [min, setMin] = useState(
    card.effort_min_minutes === null ? "" : (card.effort_min_minutes / 60).toString(),
  );
  const [max, setMax] = useState(
    card.effort_max_minutes === null ? "" : (card.effort_max_minutes / 60).toString(),
  );
  return (
    <div className="rounded-card border border-border p-4">
      <h3 className="mb-3 text-[13px] font-semibold">Catalogue facts</h3>
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <div>
          <Label htmlFor="category">Category</Label>
          <Input id="category" value={category} onChange={(e) => setCategory(e.target.value)} />
        </div>
        <div>
          <Label htmlFor="typical">Typical days</Label>
          <Input id="typical" inputMode="numeric" value={typical} onChange={(e) => setTypical(e.target.value)} />
        </div>
        <div>
          <Label htmlFor="effort-min">Effort min (h)</Label>
          <Input id="effort-min" inputMode="decimal" value={min} onChange={(e) => setMin(e.target.value)} />
        </div>
        <div>
          <Label htmlFor="effort-max">Effort max (h)</Label>
          <Input id="effort-max" inputMode="decimal" value={max} onChange={(e) => setMax(e.target.value)} />
        </div>
      </div>
      <Button
        className="mt-3"
        variant="secondary"
        disabled={disabled}
        onClick={() => void onSave({ category, typical, min, max })}
      >
        Save catalogue facts
      </Button>
    </div>
  );
}

function CreateDialog({
  templateId,
  templateName,
  clients,
  openByDefault,
}: {
  templateId: string;
  templateName: string;
  clients: ClientRow[];
  openByDefault: boolean;
}) {
  const router = useRouter();
  const [open, setOpen] = useState(openByDefault);
  const [name, setName] = useState(templateName);
  const [clientId, setClientId] = useState<string>("");
  const [startDate, setStartDate] = useState("");
  const [dueDate, setDueDate] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onCreate() {
    setBusy(true);
    setError(null);
    try {
      const created = await createFromTemplate(templateId, {
        name: name.trim(),
        client_id: clientId === "" ? null : clientId,
        start_date: startDate === "" ? null : startDate,
        due_date: dueDate === "" ? null : dueDate,
        assignee_map: {},
      });
      router.push(`/projects/${created.id}`);
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button>Start a project</Button>
      </DialogTrigger>
      <DialogContent
        title="Start a project"
        description="Name it, choose the client and the dates. What will be created is counted for you."
      >
        <div className="flex flex-col gap-3">
          <div>
            <Label htmlFor="project-name">Project name</Label>
            <Input id="project-name" value={name} onChange={(e) => setName(e.target.value)} />
          </div>
          <div>
            <Label htmlFor="client">Client</Label>
            <Select value={clientId} onValueChange={setClientId}>
              <SelectTrigger id="client">
                <SelectValue placeholder="No client (internal)" />
              </SelectTrigger>
              <SelectContent>
                {clients.map((client) => (
                  <SelectItem key={client.id} value={client.id}>
                    {client.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <FieldHint>A billable template needs a client with an active contract.</FieldHint>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <Label htmlFor="start">Start</Label>
              <Input id="start" type="date" value={startDate} onChange={(e) => setStartDate(e.target.value)} />
            </div>
            <div>
              <Label htmlFor="due">Due</Label>
              <Input id="due" type="date" value={dueDate} onChange={(e) => setDueDate(e.target.value)} />
            </div>
          </div>
          {error !== null ? <FieldError>{error}</FieldError> : null}
        </div>
        <DialogActions>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button disabled={busy || name.trim() === ""} onClick={() => void onCreate()}>
            {busy ? "Creating…" : "Create project"}
          </Button>
        </DialogActions>
      </DialogContent>
    </Dialog>
  );
}
