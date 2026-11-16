// /projects/templates — the template gallery (S11.1, S11.2, S11.4, S11.5,
// S11.6, S11.7, S11.8, S11.11).
//
// The card shows the SHAPE of the job first: phases as a proportional strip,
// then tasks, duration and effort range. Shipped and yours are marked, archived
// rows are hidden unless asked for, and a billable template says what it needs
// before it can start. Without authoring permission the gallery is a reading
// room: use and preview, no fork or new.

"use client";

import {
  Badge,
  Button,
  Dialog,
  DialogContent,
  DialogTrigger,
  EmptyState,
  Spinner,
  SurfaceHeader,
} from "@marula-core/ui";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useAuth, useRequireAuth } from "../../lib/api";
import {
  archiveTemplate,
  fetchTemplateGallery,
  fetchTemplatePreview,
  type GalleryTemplate,
  type TemplatePreview,
} from "./delivery-api";

const AUTHORING = "projects:create";
const EDITING = "projects:edit";

function minutesLabel(minutes: number | null): string {
  if (minutes === null) return "—";
  const hours = Math.round((minutes / 60) * 10) / 10;
  return `${hours}h`;
}

function rangeLabel(template: GalleryTemplate): string {
  if (template.effort_min_minutes === null && template.effort_max_minutes === null) {
    return "not recorded";
  }
  if (template.effort_min_minutes !== null && template.effort_max_minutes !== null) {
    return `${minutesLabel(template.effort_min_minutes)} – ${minutesLabel(
      template.effort_max_minutes,
    )}`;
  }
  return minutesLabel(template.effort_min_minutes ?? template.effort_max_minutes);
}

/** The phases as a proportional strip. No hue per phase: the strip is a shape
 * cue, and the word "phases" carries the meaning. */
function ShapeStrip({ template }: { template: GalleryTemplate }) {
  const blocks = Math.min(template.phase_count, 8);
  return (
    <div className="flex items-center gap-2" aria-hidden={template.phase_count === 0}>
      <div className="flex h-[6px] flex-1 gap-[2px] overflow-hidden rounded-pill">
        {Array.from({ length: blocks }, (_, index) => (
          <span
            key={index}
            className="h-full flex-1 rounded-pill"
            style={{
              background:
                index === 0 ? "var(--m-accent)" : "color-mix(in srgb, var(--m-border) 70%, transparent)",
            }}
          />
        ))}
        {template.phase_count === 0 ? (
          <span className="h-full w-full rounded-pill bg-muted" />
        ) : null}
      </div>
      <span className="shrink-0 text-[12px] tabular-nums text-muted-foreground">
        {template.phase_count} phases · {template.task_count} tasks
      </span>
    </div>
  );
}

function PreviewBody({ id }: { id: string }) {
  const [preview, setPreview] = useState<TemplatePreview | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetchTemplatePreview(id)
      .then((value) => {
        if (!cancelled) setPreview(value);
      })
      .catch((e: Error) => {
        if (!cancelled) setError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, [id]);

  if (error !== null) {
    return (
      <p role="alert" className="text-[13px] text-destructive">
        {error}
      </p>
    );
  }
  if (preview === null) {
    return (
      <div className="flex items-center gap-2 py-6 text-[13px] text-muted-foreground">
        <Spinner /> Reading the plan…
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-4 text-[13px]">
      <div className="grid grid-cols-3 gap-3">
        <div>
          <div className="text-[12px] text-muted-foreground">Duration</div>
          <div className="font-mono tabular-nums">
            {preview.dated_tasks} dated · {preview.undated_tasks} undated
          </div>
        </div>
        <div>
          <div className="text-[12px] text-muted-foreground">Effort</div>
          <div className="font-mono tabular-nums">{minutesLabel(preview.total_effort_minutes)}</div>
        </div>
        <div>
          <div className="text-[12px] text-muted-foreground">Bills as</div>
          <div>{preview.billing_method ?? "Not billable"}</div>
        </div>
      </div>

      <div>
        <h3 className="mb-1 text-[13px] font-semibold">Roles it expects</h3>
        {preview.roles.length === 0 ? (
          <p className="text-muted-foreground">No role is named on any task.</p>
        ) : (
          <ul className="flex flex-col gap-1">
            {preview.roles.map((role) => (
              <li key={role.role_key} className="flex justify-between gap-3">
                <span>{role.role_key}</span>
                <span className="font-mono tabular-nums text-muted-foreground">
                  {role.task_count} tasks · {minutesLabel(role.effort_minutes)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div>
        <h3 className="mb-1 text-[13px] font-semibold">Plan</h3>
        <ul className="flex flex-col gap-2">
          {preview.phases.map((phase) => (
            <li key={phase.key}>
              <div className="flex items-center justify-between gap-2">
                <span className="font-medium">{phase.name}</span>
                <span className="font-mono tabular-nums text-muted-foreground">
                  {phase.task_count} tasks · {minutesLabel(phase.effort_minutes)}
                </span>
              </div>
              <ul className="ml-3 mt-1 flex flex-col gap-0.5">
                {preview.tasks
                  .filter((task) => task.phase_key === phase.key)
                  .map((task) => (
                    <li key={task.key} className="flex items-center gap-2 text-muted-foreground">
                      <span className="truncate">{task.title}</span>
                      {task.optional ? <Badge tone="outline">Optional</Badge> : null}
                      {task.duration_days !== null ? (
                        <span className="ml-auto shrink-0 font-mono tabular-nums">
                          {task.duration_days}d
                        </span>
                      ) : null}
                    </li>
                  ))}
              </ul>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

export function TemplateGallery() {
  const auth = useRequireAuth();
  const { me } = useAuth();
  const permissions = me?.permissions ?? [];
  const canAuthor = permissions.includes(AUTHORING);
  // Archiving writes through the template record, which the server gates on
  // `projects:edit`; create-only users were shown a control it refuses.
  const canArchive = permissions.includes(EDITING);

  const [templates, setTemplates] = useState<GalleryTemplate[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [includeArchived, setIncludeArchived] = useState(false);
  const [category, setCategory] = useState<string>("all");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const loadSeq = useRef(0);

  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    setError(null);
    try {
      const next = await fetchTemplateGallery(includeArchived);
      // Toggling "Show archived" while another load is in flight can finish
      // in the wrong order; only the newest answer may replace the list.
      if (seq !== loadSeq.current) return;
      setTemplates(next);
    } catch (e) {
      if (seq !== loadSeq.current) return;
      setError((e as Error).message);
    }
  }, [includeArchived]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void load();
  }, [auth.status, load]);

  const categories = useMemo(() => {
    const seen = new Set<string>();
    (templates ?? []).forEach((template) => {
      if (template.category !== null) seen.add(template.category);
    });
    return ["all", ...[...seen].sort()];
  }, [templates]);

  const visible = useMemo(() => {
    return (templates ?? []).filter(
      (template) => category === "all" || template.category === category,
    );
  }, [templates, category]);

  // A category button disappears when its last template is archived; the
  // filter must go back to All instead of leaving an empty gallery with no
  // selected category.
  useEffect(() => {
    setCategory((current) => (categories.includes(current) ? current : "all"));
  }, [categories]);

  const emptyMessage = useMemo(() => {
    if ((templates ?? []).length === 0) {
      return includeArchived
        ? "No templates yet. Ship one with the product, or write your own from a plan you already run well."
        : "No templates yet. Ship one with the product, or write your own from a plan you already run well. Archived templates are hidden — tick “Show archived” to include them.";
    }
    return "No templates in this category. Choose All to see every template.";
  }, [templates, includeArchived]);

  async function onArchive(template: GalleryTemplate) {
    setBusyId(template.id);
    setActionError(null);
    try {
      await archiveTemplate(template.id, !template.archived);
      await load();
    } catch (e) {
      setActionError((e as Error).message);
    } finally {
      setBusyId(null);
    }
  }

  if (auth.status === "loading") {
    return (
      <div className="flex items-center gap-2 p-8 text-[13px] text-muted-foreground">
        <Spinner /> Loading templates…
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-5 p-6">
      <SurfaceHeader
        title="Templates"
        description="A template is how this team starts a job: the phases, the roles and the effort a good delivery of it needs."
      />

      <div className="flex flex-wrap items-center gap-2">
        {categories.map((value) => (
          <button
            key={value}
            type="button"
            onClick={() => setCategory(value)}
            aria-pressed={category === value}
            className={
              category === value
                ? "rounded-pill border border-border bg-mc-act px-3 py-1 text-[13px] font-semibold"
                : "rounded-pill border border-border bg-card px-3 py-1 text-[13px] text-muted-foreground hover:bg-muted"
            }
          >
            {value === "all" ? "All" : value}
          </button>
        ))}
        <label className="ml-auto flex items-center gap-2 text-[13px] text-muted-foreground">
          <input
            type="checkbox"
            checked={includeArchived}
            onChange={(event) => setIncludeArchived(event.target.checked)}
          />
          Show archived
        </label>
      </div>

      {!canAuthor ? (
        <p className="rounded-ctl bg-muted px-3 py-2 text-[13px] text-muted-foreground">
          You can use and preview templates. Creating or forking one needs the
          projects authoring permission.
        </p>
      ) : null}

      {actionError !== null ? (
        <p role="alert" className="rounded-ctl border border-destructive px-3 py-2 text-[13px] text-destructive">
          {actionError}
        </p>
      ) : null}

      {error !== null ? (
        <div role="alert" className="flex items-center gap-3 rounded-ctl border border-border p-4 text-[13px]">
          <span className="flex-1">The templates could not be read: {error}</span>
          <Button variant="secondary" onClick={() => void load()}>
            Try again
          </Button>
        </div>
      ) : null}

      {templates === null && error === null ? (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
          {[0, 1, 2].map((index) => (
            <div key={index} className="h-[188px] animate-pulse rounded-card bg-muted" />
          ))}
        </div>
      ) : null}

      {templates !== null && visible.length === 0 ? (
        <EmptyState message={emptyMessage} />
      ) : null}

      <ul className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
        {visible.map((template) => (
          <li
            key={template.id}
            className={
              template.archived
                ? "flex flex-col gap-3 rounded-card border border-border bg-card p-4 opacity-60"
                : "flex flex-col gap-3 rounded-card border border-border bg-card p-4"
            }
          >
            <div className="flex items-start justify-between gap-2">
              <div className="min-w-0">
                <h2 className="truncate text-[15px] font-semibold">{template.name}</h2>
                <p className="mt-0.5 line-clamp-2 text-[13px] text-muted-foreground">
                  {template.description ?? "No description"}
                </p>
              </div>
              <div className="flex shrink-0 flex-col items-end gap-1">
                {template.is_shipped ? <Badge tone="neutral">Shipped</Badge> : <Badge tone="outline">Yours</Badge>}
                {template.archived ? <Badge tone="offline">Archived</Badge> : null}
              </div>
            </div>

            <ShapeStrip template={template} />

            <div className="flex items-center gap-4 text-[12px] text-muted-foreground">
              <span>
                {template.typical_days === null ? (
                  "No typical duration"
                ) : (
                  <>
                    <span className="font-mono tabular-nums">{template.typical_days}</span>{" "}
                    {template.typical_days === 1 ? "day" : "days"} typical
                  </>
                )}
              </span>
              <span>Effort {rangeLabel(template)}</span>
            </div>

            <div className="mt-auto flex items-center gap-2">
              {!template.archived ? (
                <Button asChild size="sm">
                  <Link href={`/projects/templates/${template.id}?use=1`}>Use</Link>
                </Button>
              ) : null}
              <Button asChild size="sm" variant="secondary">
                <Link href={`/projects/templates/${template.id}`}>Open</Link>
              </Button>
              <Dialog>
                <DialogTrigger asChild>
                  <Button size="sm" variant="ghost">
                    Quick view
                  </Button>
                </DialogTrigger>
                <DialogContent title={template.name} description="What this template will create">
                  <PreviewBody id={template.id} />
                </DialogContent>
              </Dialog>
              {canArchive && !template.is_shipped ? (
                <Button
                  size="sm"
                  variant="ghost"
                  className="ml-auto"
                  disabled={busyId === template.id}
                  onClick={() => void onArchive(template)}
                >
                  {template.archived ? "Restore" : "Archive"}
                </Button>
              ) : null}
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
