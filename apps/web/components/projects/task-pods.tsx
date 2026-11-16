// The project task workbench's pods (S10.1–S10.11).
//
// Mount point: the task workbench (owned by another lane) renders
// <TaskPodsPanel projectId taskId initialPod={searchParams.get("pod")} /> in its
// right-hand column. This component is deliberately self-contained so it can be
// mounted without changing the workbench shell.
//
// Held here: a typed blocker with its owner (S10.3), the dependency it waits on
// (S10.4), promotion with both links kept (S10.7), the pod document per role and
// the person's own order (S10.10), and a deep link that outlines one pod
// (S10.11).

"use client";

import {
  Badge,
  Button,
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
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { api, useAuth } from "../../lib/api";
import {
  clearTaskBlocker,
  fetchPodPrefs,
  fetchPodSettings,
  promoteTask,
  putPodPrefs,
  putPodSettings,
  setTaskBlocker,
  type PodPrefs,
  type PodSettings,
  type PromoteResult,
} from "./delivery-api";

const POD_LABELS: Record<string, string> = {
  status: "Status",
  chain: "Where it sits",
  needs: "What it needs",
  notes: "Notes",
  composer: "Composer",
  timer: "Timer",
  time: "Time",
  schedule: "Schedule",
  materials: "Materials",
  client: "Client",
  links: "Links",
};

const BLOCKER_KINDS = [
  ["client", "The client"],
  ["task", "Another task"],
  ["vendor", "A vendor"],
  ["access", "Access"],
  ["parts", "Parts"],
  ["other", "Something else"],
] as const;

/**
 * Which of `pods` the caller's roles may see.
 *
 * The org's pod documents (`PodSettings`) name the pods each role gets, so a pod
 * belongs to a role only while a document grants it there. A pod no document
 * mentions is unrestricted and stays visible. With no documents loaded — or no
 * roles on the caller — the ceiling does not apply, because hiding work on
 * missing data is worse than showing a pod the person can already hide.
 */
export function roleCeiling(
  pods: readonly string[],
  settings: PodSettings[] | null,
  roles: readonly string[],
): Set<string> {
  if (settings === null || settings.length === 0 || roles.length === 0) {
    return new Set(pods);
  }
  const grantedTo = new Map<string, Set<string>>();
  for (const row of settings) {
    for (const pod of row.pods) {
      const holders = grantedTo.get(pod) ?? new Set<string>();
      holders.add(row.role_key);
      grantedTo.set(pod, holders);
    }
  }
  return new Set(
    pods.filter((pod) => {
      const holders = grantedTo.get(pod);
      if (holders === undefined) return true;
      return [...holders].some((role) => roles.includes(role));
    }),
  );
}

interface DepRow {
  id: string;
  title: string;
  status: string;
}

export function TaskPodsPanel({
  projectId,
  taskId,
  taskTitle,
  initialPod = null,
  readOnly = false,
}: {
  projectId: string;
  taskId: string;
  taskTitle: string;
  initialPod?: string | null;
  readOnly?: boolean;
}) {
  const auth = useAuth();
  const canEdit = (auth.me?.permissions ?? []).includes("projects:edit");
  // Pod order is a project setting, so the admin block follows the permission
  // the server checks on `/project-pods/settings`, not the user-admin one.
  const canAdmin = (auth.me?.permissions ?? []).includes("projects:edit");

  const [blockerOpen, setBlockerOpen] = useState(false);
  const [kind, setKind] = useState<string>("client");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [promoted, setPromoted] = useState<PromoteResult | null>(null);
  const [deps, setDeps] = useState<DepRow[] | null>(null);
  const [depsError, setDepsError] = useState<string | null>(null);
  const [depsAttempt, setDepsAttempt] = useState(0);
  const [prefsError, setPrefsError] = useState<string | null>(null);
  const [prefsAttempt, setPrefsAttempt] = useState(0);

  const [settings, setSettings] = useState<PodSettings[] | null>(null);
  const [prefs, setPrefs] = useState<PodPrefs | null>(null);
  const [roleKey, setRoleKey] = useState("project_tech");

  // The route page reuses this panel when the task changes, so the previous
  // task's promotion, blocker form and notice are dropped at the switch.
  useEffect(() => {
    setPromoted(null);
    setBlockerOpen(false);
    setNote("");
    setError(null);
  }, [taskId]);

  // Dependencies belong to the task, so an obsolete response can never
  // commit: the effect is cancelled when the task changes.
  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    setDeps(null);
    setDepsError(null);
    api.api
      .GET("/api/v1/projects/{id}/tasks/{task_id}/dependencies", {
        params: { path: { id: projectId, task_id: taskId } },
      })
      .then((result) => {
        if (cancelled) return;
        // A non-2xx resolves with `error` rather than throwing: leaving `deps`
        // null here would spin forever.
        if (result.error !== undefined || result.data === undefined) {
          setDepsError("The dependencies could not be read, so nothing is claimed about what blocks it.");
          return;
        }
        setDeps(result.data.depends_on as DepRow[]);
      })
      .catch(() => {
        // A failed read is not an empty list: "Nothing blocks the start"
        // would be a false statement in a blocker workflow.
        if (!cancelled) {
          setDepsError("The dependencies could not be read, so nothing is claimed about what blocks it.");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [auth.status, projectId, taskId, depsAttempt]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    setPrefs(null);
    setPrefsError(null);
    fetchPodSettings()
      .then((rows) => {
        if (!cancelled) setSettings(rows);
      })
      .catch(() => {
        if (!cancelled) setSettings([]);
      });
    fetchPodPrefs()
      .then((value) => {
        if (!cancelled) setPrefs(value);
      })
      .catch(() => {
        if (!cancelled) setPrefsError("Your pod settings could not be read.");
      });
    return () => {
      cancelled = true;
    };
  }, [auth.status, prefsAttempt]);

  const onBlock = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await setTaskBlocker(projectId, taskId, {
        kind,
        note: note.trim() === "" ? null : note.trim(),
        owner_id: null,
      });
      setBlockerOpen(false);
      setNote("");
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }, [projectId, taskId, kind, note]);

  async function onPromote() {
    setBusy(true);
    setError(null);
    try {
      setPromoted(await promoteTask(projectId, taskId, { board_id: null, priority: null }));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function onSavePrefs(next: PodPrefs) {
    setPrefs(next);
    try {
      await putPodPrefs(next);
    } catch (e) {
      setError((e as Error).message);
    }
  }

  function togglePod(pod: string) {
    if (prefs === null) return;
    const hidden = prefs.hidden.includes(pod)
      ? prefs.hidden.filter((value) => value !== pod)
      : [...prefs.hidden, pod];
    void onSavePrefs({ ...prefs, hidden });
  }

  // The pods this panel draws, in the person's own order. `order` is a flex
  // property on the aside, so the note and the settings block keep their
  // places around them.
  const SECTIONS: readonly string[] = ["status", "chain", "links"];
  // The role ceiling: the org's pod documents grant a pod to a role. A pod
  // named only for roles the caller does not hold is hidden; a pod no document
  // names is unrestricted. Personal hide/order still layers on top.
  const roleAllowed = roleCeiling(SECTIONS, settings, auth.me?.roles ?? []);
  const podShown = (pod: string): boolean =>
    roleAllowed.has(pod) && (prefs === null || !prefs.hidden.includes(pod));
  const podOrder = (pod: string): number => {
    if (prefs === null) return SECTIONS.indexOf(pod);
    const preferred = prefs.pod_order.indexOf(pod);
    return preferred === -1 ? prefs.pod_order.length + SECTIONS.indexOf(pod) : preferred;
  };
  const everyPodHidden = prefs !== null && SECTIONS.every((pod) => !podShown(pod));

  return (
    <aside aria-label="Task pods" className="flex flex-col gap-3">
      {readOnly ? (
        <p
          style={{ order: -2 }}
          className="rounded-ctl bg-muted px-3 py-2 text-[12px] text-muted-foreground"
        >
          This project is on hold. The workbench stays readable; clear the hold to
          change the task.
        </p>
      ) : null}

      {error !== null ? (
        <p
          role="alert"
          style={{ order: -1 }}
          className="rounded-ctl border border-destructive px-3 py-2 text-[12px] text-destructive"
        >
          {error}
        </p>
      ) : null}

      {everyPodHidden ? (
        <p
          style={{ order: -1 }}
          className="rounded-ctl bg-muted px-3 py-2 text-[12px] text-muted-foreground"
        >
          Every pod is hidden. Turn one back on in Pods below to see it here.
        </p>
      ) : null}

      {podShown("status") ? (
        <section
          aria-label="Status and blocker"
          style={{ order: podOrder("status") }}
          className={
            initialPod === "status"
              ? "rounded-card border border-border p-3 outline outline-2 outline-mc-act"
              : "rounded-card border border-border p-3"
          }
        >
          <div className="flex items-center justify-between">
            <h2 className="text-[13px] font-semibold">Status</h2>
            <Badge tone="neutral">{taskTitle}</Badge>
          </div>
          <div className="mt-2 flex flex-wrap gap-2">
            {!readOnly && canEdit ? (
              <Button
                size="sm"
                variant="secondary"
                onClick={() => setBlockerOpen((open) => !open)}
              >
                Blocked…
              </Button>
            ) : null}
            {!readOnly && canEdit ? (
              <Button
                size="sm"
                variant="ghost"
                disabled={busy}
                onClick={() => {
                  setBusy(true);
                  setError(null);
                  clearTaskBlocker(projectId, taskId)
                    .then(() => setBlockerOpen(false))
                    .catch((e: Error) => setError(e.message))
                    .finally(() => setBusy(false));
                }}
              >
                Clear the blocker
              </Button>
            ) : null}
          </div>
          {blockerOpen ? (
            <div className="mt-2 flex flex-col gap-2 rounded-ctl bg-muted p-2">
              <Label htmlFor="blocker-kind">What is it waiting on?</Label>
              <Select value={kind} onValueChange={setKind}>
                <SelectTrigger id="blocker-kind">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {BLOCKER_KINDS.map(([value, label]) => (
                    <SelectItem key={value} value={value}>
                      {label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Label htmlFor="blocker-note">Who or what, exactly</Label>
              <Input
                id="blocker-note"
                value={note}
                onChange={(e) => setNote(e.target.value)}
                placeholder="Named so the plan can say what it waits on"
              />
              <Button size="sm" disabled={busy} onClick={() => void onBlock()}>
                Set the blocker
              </Button>
            </div>
          ) : null}
        </section>
      ) : null}

      {podShown("chain") ? (
        <section
          aria-label="Waiting on"
          style={{ order: podOrder("chain") }}
          className={
            initialPod === "chain"
              ? "rounded-card border border-border p-3 outline outline-2 outline-mc-act"
              : "rounded-card border border-border p-3"
          }
        >
          <h2 className="text-[13px] font-semibold">Waiting on</h2>
          {depsError !== null ? (
            <div className="mt-1 flex flex-wrap items-center gap-2 text-[12px]">
              <span role="alert" className="text-destructive">
                {depsError}
              </span>
              <Button size="sm" variant="ghost" onClick={() => setDepsAttempt((count) => count + 1)}>
                Try again
              </Button>
            </div>
          ) : deps === null ? (
            <div className="mt-1 flex items-center gap-2 text-[12px] text-muted-foreground">
              <Spinner /> Reading dependencies…
            </div>
          ) : deps.length === 0 ? (
            <p className="mt-1 text-[12px] text-muted-foreground">Nothing blocks the start.</p>
          ) : (
            <ul className="mt-1 flex flex-col gap-1 text-[12px]">
              {deps.map((dep) => (
                <li key={dep.id} className="flex items-center justify-between gap-2">
                  <span className="truncate">{dep.title}</span>
                  <Badge tone={dep.status === "completed" ? "success" : "due"}>{dep.status}</Badge>
                </li>
              ))}
            </ul>
          )}
        </section>
      ) : null}

      {podShown("links") ? (
        <section
          aria-label="Links"
          style={{ order: podOrder("links") }}
          className={
            initialPod === "links"
              ? "rounded-card border border-border p-3 outline outline-2 outline-mc-act"
              : "rounded-card border border-border p-3"
          }
        >
          <h2 className="text-[13px] font-semibold">Links</h2>
          <p className="mt-1 text-[12px] text-muted-foreground">
            Promotion gives this task a ticket on the service desk. Notes, files, time
            and watchers stay shared, and each side links to the other.
          </p>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            {!readOnly && canEdit ? (
              <Button size="sm" variant="secondary" disabled={busy} onClick={() => void onPromote()}>
                {promoted?.created === false ? "Open its ticket" : "Promote to a ticket"}
              </Button>
            ) : null}
            {promoted !== null ? (
              <Link href={`/tickets/${promoted.ticket_id}`} className="text-[13px] underline">
                {promoted.ticket_number}
              </Link>
            ) : null}
          </div>
        </section>
      ) : null}

      <section
        aria-label="Pod settings"
        style={{ order: 99 }}
        className="rounded-card border border-border p-3"
      >
        <h2 className="text-[13px] font-semibold">Pods</h2>
        {prefsError !== null ? (
          <div className="mt-1 flex flex-wrap items-center gap-2 text-[12px]">
            <span role="alert" className="text-destructive">
              {prefsError}
            </span>
            <Button size="sm" variant="ghost" onClick={() => setPrefsAttempt((count) => count + 1)}>
              Try again
            </Button>
          </div>
        ) : prefs === null ? (
          <p className="mt-1 text-[12px] text-muted-foreground">Reading your settings…</p>
        ) : (
          <ul className="mt-1 flex flex-col gap-1">
            {Object.entries(POD_LABELS).map(([pod, label]) => {
              const hidden = prefs.hidden.includes(pod);
              return (
                <li key={pod} className="flex items-center justify-between gap-2 text-[12px]">
                  <span>{label}</span>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={!hidden}
                    aria-label={`Show ${label}`}
                    onClick={() => togglePod(pod)}
                    className={
                      hidden
                        ? "rounded-pill border border-border px-2 py-[2px] text-muted-foreground"
                        : "rounded-pill bg-mc-act px-2 py-[2px] font-semibold"
                    }
                  >
                    {hidden ? "Hidden" : "Shown"}
                  </button>
                </li>
              );
            })}
          </ul>
        )}
        {canAdmin && settings !== null ? (
          <div className="mt-3 border-t border-border pt-2">
            <Label htmlFor="role-key">Role document</Label>
            <Select value={roleKey} onValueChange={setRoleKey}>
              <SelectTrigger id="role-key">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {["project_manager", "project_coordinator", "project_tech"].map((role) => (
                  <SelectItem key={role} value={role}>
                    {role.replaceAll("_", " ")}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <FieldHint>
              The role document is the organisation's; a person can hide pods but never
              widen what their role sees.
            </FieldHint>
            <div className="mt-2 flex flex-wrap gap-1">
              {Object.keys(POD_LABELS).map((pod) => {
                const current = settings.find((row) => row.role_key === roleKey);
                const on = current?.pods.includes(pod) ?? false;
                return (
                  <button
                    key={pod}
                    type="button"
                    className={
                      on
                        ? "rounded-pill bg-mc-act px-2 py-[2px] text-[12px] font-semibold"
                        : "rounded-pill border border-border px-2 py-[2px] text-[12px] text-muted-foreground"
                    }
                    onClick={() => {
                      const pods = on
                        ? (current?.pods ?? []).filter((value) => value !== pod)
                        : [...(current?.pods ?? []), pod];
                      putPodSettings({ surface: "project_task", role_key: roleKey, pods })
                        .then(() => fetchPodSettings())
                        .then(setSettings)
                        .catch((e: Error) => setError(e.message));
                    }}
                  >
                    {POD_LABELS[pod]}
                  </button>
                );
              })}
            </div>
          </div>
        ) : null}
      </section>
    </aside>
  );
}
