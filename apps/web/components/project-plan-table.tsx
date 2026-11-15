"use client";
/**
 * ProjectPlanTable — the plan's rows: phase summary rows, task rows, the
 * undated group and the inline create row (surface pack S3.1–S3.3).
 *
 * Purpose: one table that answers what the plan says. Phase rows carry the
 * date range and the burn strip; task rows carry work type, title, the
 * critical mark, status, person or the role that still needs one, start,
 * working days, effort and what they wait on. Undated work is never drawn as
 * if it were scheduled: it sits in its own group with "No date" and "—".
 *
 * Keyboard: every row is a tab stop. Enter opens the row, `A` starts a task in
 * that row's phase, and the inline row commits on Enter and cancels on Escape.
 * The row menu carries the actions a pointer would reach, in the same order.
 *
 * Non-purpose: no fetching, no writes. The surface owns the drafts and the
 * mutations; this file renders them and reports intent.
 */
import {
  Button,
  Checkbox,
  Input,
  Menu,
  PlanCheckCell,
  PlanCell,
  PlanHeaderCell,
  PlanHeaderRow,
  PlanPhaseRow,
  PlanRow,
  PlanTable,
  BurnStrip,
  Diamond,
  WorkTypeMark,
  PLAN_COLUMNS,
  PLAN_COLUMNS_NARROW,
  PLAN_PHASE_COLUMNS,
  PLAN_PHASE_COLUMNS_NARROW,
  type MenuItem,
} from "@marula-core/ui";
import { Avatar, Icon, Pill } from "@marula-core/ui/kit";
import type { ReactNode } from "react";
import {
  effortOf,
  formatDay,
  formatDayRange,
  hoursLabel,
  isTaskLate,
  roleLabel,
  taskDays,
  taskEnd,
  type PlanPerson,
  type PlanPhase,
  type PlanTask,
} from "../lib/projects-plan-types";

export type PlanFilter = "all" | "unassigned" | "risk" | "blocked" | "critical";

export interface PlanCreateDraft {
  phaseId: string | null;
  title: string;
  role: string;
  days: string;
  effortHours: string;
  /** Dashed suggestion from the project's calendar, in hours. */
  suggestionHours: number;
}

export interface PlanTableProps {
  phases: readonly PlanPhase[];
  tasks: readonly PlanTask[];
  people: ReadonlyMap<string, PlanPerson>;
  deps: Readonly<Record<string, PlanTask[]>>;
  criticalPath: ReadonlySet<string>;
  riskTaskIds: ReadonlySet<string>;
  filter: PlanFilter;
  expanded: ReadonlySet<string>;
  selected: ReadonlySet<string>;
  focusId: string | null;
  creating: PlanCreateDraft | null;
  createError: string | null;
  busy: boolean;
  /** True below 1280: the dependency column drops out first (S3.16). */
  narrow: boolean;
  weekdays: number;
  minutesPerDay: number;
  today: string;
  onTogglePhase: (phaseId: string) => void;
  onToggleSelected: (taskId: string, next: boolean) => void;
  onTogglePhaseSelected: (phaseId: string, next: boolean) => void;
  onToggleAll: (next: boolean) => void;
  onFocusRow: (taskId: string) => void;
  onOpenTask: (task: PlanTask) => void;
  onAddTask: (phaseId: string | null) => void;
  onScalePhase: (phase: PlanPhase) => void;
  onSetDates: () => void;
  onCreatingChange: (patch: Partial<PlanCreateDraft>) => void;
  onCreateCommit: () => void;
  onCreateCancel: () => void;
}

/** The status word and the pill tone, shared by every row that shows one. */
export function TaskStatusPill({
  task,
  today,
  weekdays,
  minutesPerDay,
}: {
  task: PlanTask;
  today: string;
  weekdays: number;
  minutesPerDay: number;
}) {
  if (task.status === "completed") {
    return (
      <Pill tone="success" icon={<Icon name="check" size={13} />}>
        Done
      </Pill>
    );
  }
  if (task.status === "in_progress") {
    return (
      <Pill tone="progress" icon={<Icon name="play" size={13} />}>
        In progress
      </Pill>
    );
  }
  if (task.status === "blocked") {
    return (
      <Pill tone="waiting" icon={<Icon name="lock" size={12} />}>
        Blocked
      </Pill>
    );
  }
  if (task.status === "cancelled") {
    return <Pill tone="neutral">Cancelled</Pill>;
  }
  if (isTaskLate(task, Date.parse(`${today}T23:59:59Z`), weekdays, minutesPerDay)) {
    return (
      <Pill tone="attention" icon={<Icon name="alert" size={13} />}>
        Late
      </Pill>
    );
  }
  return <Pill tone="new">Not started</Pill>;
}

function WhoCell({ task, people }: { task: PlanTask; people: ReadonlyMap<string, PlanPerson> }) {
  const person = task.assignee_id === null || task.assignee_id === undefined
    ? null
    : people.get(task.assignee_id);
  if (task.assignee_id !== null && task.assignee_id !== undefined) {
    const name = person?.display_name ?? "Assigned";
    return (
      <span className="flex min-w-0 items-center gap-2">
        <Avatar initials={person === undefined || person === null ? "?" : `${person.display_name.split(/\s+/).map((w) => w[0]).slice(0, 2).join("")}`.toUpperCase()} title={name} />
        <span className="min-w-0 truncate">{name}</span>
      </span>
    );
  }
  if (task.external_owner !== null && task.external_owner !== undefined && task.external_owner.trim() !== "") {
    return (
      <span className="flex min-w-0 items-center gap-2">
        <Avatar unassigned aria-hidden="true" />
        <span className="min-w-0 truncate text-muted-foreground">{task.external_owner} (external)</span>
      </span>
    );
  }
  const role = roleLabel(task.role_key);
  return (
    <span className="flex min-w-0 items-center gap-2">
      <Avatar unassigned aria-hidden="true" />
      <span className="min-w-0 truncate text-muted-foreground">
        {role === "" ? "Unassigned" : `${role}, open`}
      </span>
    </span>
  );
}

function DependsCell({ task, deps }: { task: PlanTask; deps: Readonly<Record<string, PlanTask[]>> }) {
  const blockers = deps[task.id] ?? [];
  if (blockers.length === 0) return <span className="text-muted-foreground">—</span>;
  const shown = blockers.slice(0, 2);
  return (
    <span className="flex min-w-0 items-center gap-1.5 text-[12px] text-muted-foreground">
      {shown.map((blocker) => (
        <span key={blocker.id} className="inline-flex min-w-0 items-center gap-1 rounded-[6px] border border-border px-1.5 py-[1px]">
          <Icon name="link" size={12} />
          <span className="min-w-0 truncate" title={blocker.title}>
            {blocker.title}
          </span>
        </span>
      ))}
      {blockers.length > shown.length ? <span>+{blockers.length - shown.length}</span> : null}
    </span>
  );
}

function isVisible(
  task: PlanTask,
  filter: PlanFilter,
  options: {
    deps: Readonly<Record<string, PlanTask[]>>;
    criticalPath: ReadonlySet<string>;
    riskTaskIds: ReadonlySet<string>;
    today: string;
  },
): boolean {
  switch (filter) {
    case "all":
      return true;
    case "unassigned":
      return (
        (task.assignee_id === null || task.assignee_id === undefined) &&
        (task.role_key === null || task.role_key === undefined || task.role_key.trim() === "")
      );
    case "blocked":
      return task.status === "blocked";
    case "risk":
      return (
        options.riskTaskIds.has(task.id) ||
        isTaskLate(task, Date.parse(`${options.today}T23:59:59Z`))
      );
    case "critical":
      return options.criticalPath.has(task.id) || task.is_critical === true;
    default:
      return true;
  }
}

export function ProjectPlanTable(props: PlanTableProps): React.JSX.Element {
  const {
    phases,
    tasks,
    people,
    deps,
    criticalPath,
    riskTaskIds,
    filter,
    expanded,
    selected,
    focusId,
    creating,
    createError,
    busy,
    narrow,
    weekdays,
    minutesPerDay,
    today,
    onTogglePhase,
    onToggleSelected,
    onTogglePhaseSelected,
    onToggleAll,
    onFocusRow,
    onOpenTask,
    onAddTask,
    onScalePhase,
    onSetDates,
    onCreatingChange,
    onCreateCommit,
    onCreateCancel,
  } = props;

  const columns = narrow ? PLAN_COLUMNS_NARROW : PLAN_COLUMNS;
  const phaseColumns = narrow ? PLAN_PHASE_COLUMNS_NARROW : PLAN_PHASE_COLUMNS;

  const visible = (task: PlanTask): boolean =>
    isVisible(task, filter, { deps, criticalPath, riskTaskIds, today });

  const dated = tasks.filter(
    (task) =>
      task.start_at !== null &&
      task.start_at !== undefined &&
      task.start_at !== "",
  );
  const undated = tasks.filter((task) => task.start_at === null || task.start_at === undefined || task.start_at === "");
  const unphasedDated = dated.filter(
    (task) => task.phase_id === null || task.phase_id === undefined || !phases.some((p) => p.id === task.phase_id),
  );
  const visibleDated = dated.filter(visible);
  const visibleUndated = undated.filter(visible);

  const taskRow = (task: PlanTask, undatedRow = false): ReactNode => {
    const phaseId = task.phase_id ?? null;
    const menuItems: MenuItem[] = [
      {
        label: "Open details",
        onSelect: () => onOpenTask(task),
      },
      {
        label: "Set start date…",
        onSelect: () => onOpenTask(task),
      },
      {
        label: "Add task in this phase",
        onSelect: () => onAddTask(phaseId),
      },
      {
        label: "Add dependency",
        disabled: true,
        onSelect: () => undefined,
      },
      {
        label: "Promote to a ticket",
        disabled: true,
        onSelect: () => undefined,
      },
    ];
    return (
      <PlanRow
        key={task.id}
        columns={columns}
        selected={selected.has(task.id)}
        current={focusId === task.id}
        tabIndex={0}
        aria-label={`${task.title}, ${task.status.replaceAll("_", " ")}`}
        onFocus={() => onFocusRow(task.id)}
        onClick={() => {
          onFocusRow(task.id);
          onOpenTask(task);
        }}
        onKeyDown={(event) => {
          if (event.target !== event.currentTarget) return;
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            onOpenTask(task);
          }
          if (event.key.toLowerCase() === "a") {
            event.preventDefault();
            onAddTask(phaseId);
          }
        }}
      >
        <PlanCheckCell>
          <Checkbox
            aria-label={`Select ${task.title}`}
            checked={selected.has(task.id)}
            onCheckedChange={(next) => onToggleSelected(task.id, next === true)}
            onClick={(event) => event.stopPropagation()}
          />
        </PlanCheckCell>
        <PlanCell>
          {task.is_milestone ? (
            <Diamond className="mx-[6px]" tone={task.status === "completed" ? "done" : "default"} />
          ) : (
            <WorkTypeMark type={task.work_type ?? "coordination"} />
          )}
          <span className="min-w-0 truncate font-medium">{task.title}</span>
          {task.is_critical === true ? (
            <span className="inline-flex shrink-0 items-center gap-1 rounded-[6px] border border-border px-1.5 py-[1px] text-[12px] text-muted-foreground" title="On the critical path">
              <Icon name="link" size={12} />
              Critical
            </span>
          ) : null}
        </PlanCell>
        <PlanCell>
          <TaskStatusPill task={task} today={today} weekdays={weekdays} minutesPerDay={minutesPerDay} />
        </PlanCell>
        <PlanCell>
          {task.is_milestone ? <span className="text-muted-foreground">–</span> : <WhoCell task={task} people={people} />}
        </PlanCell>
        <PlanCell>
          <span className="font-mono tabular-nums">{undatedRow ? "No date" : formatDay(task.start_at)}</span>
        </PlanCell>
        <PlanCell right>
          <span className="font-mono tabular-nums">
            {undatedRow ? "–" : (taskDays(task, minutesPerDay) ?? "–")}
          </span>
        </PlanCell>
        <PlanCell right>
          <span className="font-mono tabular-nums">{hoursLabel(task.effort_estimate_minutes)}</span>
        </PlanCell>
        {narrow ? null : (
          <PlanCell>{undatedRow ? <span className="text-muted-foreground">—</span> : <DependsCell task={task} deps={deps} />}</PlanCell>
        )}
        <PlanCheckCell onClick={(event) => event.stopPropagation()}>
          <Menu
            label={`Actions for ${task.title}`}
            align="end"
            trigger={
              <Button variant="ghost" size="sm" aria-label={`Actions for ${task.title}`} className="size-8 px-0">
                <Icon name="more" size={16} />
              </Button>
            }
            items={menuItems}
          />
        </PlanCheckCell>
      </PlanRow>
    );
  };

  const createRow = (phaseId: string | null): ReactNode => {
    if (creating === null || creating.phaseId !== phaseId) return null;
    const commit = () => {
      if (creating.title.trim() === "") return;
      onCreateCommit();
    };
    const key = (event: { key: string; preventDefault: () => void }) => {
      if (event.key === "Enter") {
        event.preventDefault();
        commit();
      }
      if (event.key === "Escape") {
        event.preventDefault();
        onCreateCancel();
      }
    };
    return (
      <div
        key="plan-create"
        role="row"
        style={{ gridTemplateColumns: columns }}
        className="relative grid min-h-row items-center border-b border-border bg-mc-act text-[length:var(--m-density-fs)] [&>*]:min-w-0 [&>*]:px-[var(--m-density-px)]"
      >
        <PlanCheckCell>
          <Icon name="plus" size={16} />
        </PlanCheckCell>
        <PlanCell>
          <Input
            aria-label="New task title"
            placeholder="What is the work?"
            autoFocus
            value={creating.title}
            onChange={(event) => onCreatingChange({ title: event.target.value })}
            onKeyDown={key}
            className="h-8"
          />
        </PlanCell>
        <PlanCell>
          <Input
            aria-label="Role this work needs"
            placeholder="Role"
            value={creating.role}
            onChange={(event) => onCreatingChange({ role: event.target.value })}
            onKeyDown={key}
            className="h-8"
          />
        </PlanCell>
        <PlanCell>
          <Input
            aria-label="Days"
            inputMode="decimal"
            placeholder="Days"
            value={creating.days}
            onChange={(event) => onCreatingChange({ days: event.target.value })}
            onKeyDown={key}
            className="h-8"
          />
        </PlanCell>
        <PlanCell right>
          <Input
            aria-label="Effort in hours"
            inputMode="decimal"
            placeholder={`${creating.suggestionHours} h`}
            value={creating.effortHours}
            onChange={(event) => onCreatingChange({ effortHours: event.target.value })}
            onKeyDown={key}
            className="h-8 text-right"
          />
        </PlanCell>
        <PlanCell>
          <span className="truncate text-[12px] text-muted-foreground">
            {createError ?? "Enter to add, Escape to cancel."}
          </span>
        </PlanCell>
        {narrow ? null : <PlanCell />}
        <PlanCheckCell>
          <Button size="sm" variant="primary" onClick={commit} disabled={busy || creating.title.trim() === ""}>
            Add
          </Button>
        </PlanCheckCell>
      </div>
    );
  };

  const allVisibleIds = [...visibleDated, ...visibleUndated].map((task) => task.id);
  const allSelected = allVisibleIds.length > 0 && allVisibleIds.every((id) => selected.has(id));

  return (
    <PlanTable
      aria-label="Work plan"
      aria-rowcount={dated.length + undated.length + phases.length}
      data-testid="phase-rail"
    >
      <PlanHeaderRow columns={columns}>
        <PlanCheckCell>
          <Checkbox
            aria-label={allSelected ? "Clear selection" : "Select all visible tasks"}
            checked={allSelected}
            onCheckedChange={(next) => onToggleAll(next === true)}
          />
        </PlanCheckCell>
        <PlanHeaderCell>Task</PlanHeaderCell>
        <PlanHeaderCell>Status</PlanHeaderCell>
        <PlanHeaderCell>Who</PlanHeaderCell>
        <PlanHeaderCell>Start</PlanHeaderCell>
        <PlanHeaderCell right>Days</PlanHeaderCell>
        <PlanHeaderCell right>Effort</PlanHeaderCell>
        {narrow ? null : <PlanHeaderCell>Depends on</PlanHeaderCell>}
        <PlanHeaderCell aria-label="Row actions" />
      </PlanHeaderRow>

      {unphasedDated.length === 0 ? createRow(null) : null}

      {phases.map((phase) => {
        const phaseTasks = tasks.filter((task) => task.phase_id === phase.id);
        const phaseDated = dated.filter((task) => task.phase_id === phase.id);
        const shownTasks = phaseDated.filter(visible);
        const phaseUndated = phaseTasks.filter(
          (task) => task.start_at === null || task.start_at === undefined,
        );
        const open = expanded.has(phase.id);
        const burn = effortOf(phaseTasks);
        const done = phaseTasks.filter((task) => task.status === "completed").length;
        // A phase often has no window of its own while its tasks do. Derive the
        // span from the work rather than telling a planner "No dates" about a
        // phase whose steps are plainly dated.
        const derivedStart = phaseDated.reduce<string | null>(
          (min, task) =>
            task.start_at && (min === null || task.start_at < min) ? task.start_at : min,
          null,
        );
        const derivedEnd = phaseDated.reduce<string | null>((max, task) => {
          const finish = taskEnd(task);
          if (finish === null) return max;
          return max === null || finish > max ? finish : max;
        }, null);
        const start = phase.start_date ?? derivedStart;
        const end = phase.end_date ?? derivedEnd;
        const allSelectedHere =
          phaseTasks.length > 0 && phaseTasks.every((task) => selected.has(task.id));
        const phaseMenu: MenuItem[] = [
          { label: "Add task in this phase", onSelect: () => onAddTask(phase.id) },
          { label: "Scale this phase…", onSelect: () => onScalePhase(phase) },
          {
            label: allSelectedHere ? "Clear this phase's selection" : "Select every task in this phase",
            onSelect: () => onTogglePhaseSelected(phase.id, !allSelectedHere),
          },
        ];
        return (
          <div key={phase.id} role="rowgroup">
            <PlanPhaseRow columns={phaseColumns} aria-expanded={open}>
              <PlanCheckCell align="start">
                <button
                  type="button"
                  aria-label={open ? `Collapse ${phase.name}` : `Expand ${phase.name}`}
                  aria-expanded={open}
                  onClick={() => onTogglePhase(phase.id)}
                  className="inline-flex size-6 cursor-pointer items-center justify-center rounded-[6px] border-0 bg-transparent text-muted-foreground hover:bg-muted hover:text-foreground"
                >
                  <Icon name={open ? "down" : "chev"} size={16} />
                </button>
              </PlanCheckCell>
              <PlanCell flushLeft>
                <button
                  type="button"
                  onClick={() => onTogglePhase(phase.id)}
                  className="min-w-0 cursor-pointer truncate border-0 bg-transparent p-0 text-left font-semibold text-foreground"
                >
                  {phase.name}
                </button>
                {phase.is_milestone ? (
                  <span className="inline-flex shrink-0 items-center gap-1 rounded-[6px] border border-border px-1.5 py-[1px] text-[12px] font-medium text-muted-foreground" title="This phase releases billing on completion">
                    <Diamond className="size-[9px]" />
                    Milestone
                  </span>
                ) : null}
                {phase.status === "completed" ? (
                  <Pill tone="success" icon={<Icon name="check" size={12} />}>
                    Complete
                  </Pill>
                ) : null}
                {phaseTasks.length > 0 ? (
                  <span className="ml-auto shrink-0 text-[12px] font-medium text-muted-foreground tabular-nums">
                    {done}/{phaseTasks.length} done
                  </span>
                ) : null}
              </PlanCell>
              <PlanCell right>
                <span className="font-mono text-[12px] font-medium tabular-nums text-muted-foreground">
                  {start === null && end === null
                    ? "No dates"
                    : formatDayRange(start, end)}
                </span>
              </PlanCell>
              <PlanCell>
                <BurnStrip
                  usedHours={Math.round(burn.loggedMinutes / 60)}
                  totalHours={Math.round(burn.estimateMinutes / 60)}
                  label={`${phase.name}: ${Math.round(burn.loggedMinutes / 60)} of ${Math.round(burn.estimateMinutes / 60)} hours logged`}
                />
              </PlanCell>
              <PlanCheckCell>
                <Menu
                  label={`Actions for ${phase.name}`}
                  align="end"
                  trigger={
                    <Button variant="ghost" size="sm" aria-label={`Actions for ${phase.name}`} className="size-8 px-0">
                      <Icon name="more" size={16} />
                    </Button>
                  }
                  items={phaseMenu}
                />
              </PlanCheckCell>
            </PlanPhaseRow>
            {open ? (
              <div role="rowgroup">
                {shownTasks.map((task) => taskRow(task))}
                {createRow(phase.id)}
                {phaseTasks.length === 0 ? (
                  <div className="border-b border-border px-[var(--m-density-px)] py-2 text-[13px] text-muted-foreground">
                    No tasks in this phase yet.{" "}
                    <button
                      type="button"
                      onClick={() => onAddTask(phase.id)}
                      className="cursor-pointer border-0 bg-transparent p-0 text-[13px] underline underline-offset-2"
                    >
                      Add one
                    </button>
                    .
                  </div>
                ) : shownTasks.length === 0 && phaseUndated.length > 0 ? (
                  <div className="border-b border-border px-[var(--m-density-px)] py-2 text-[13px] text-muted-foreground">
                    {phaseUndated.length} {phaseUndated.length === 1 ? "task waits" : "tasks wait"} in Not
                    dated yet.
                  </div>
                ) : null}
              </div>
            ) : null}
          </div>
        );
      })}

      {unphasedDated.filter(visible).length > 0 ? (
        <div role="rowgroup">
          <PlanPhaseRow columns={phaseColumns}>
            <PlanCheckCell />
            <PlanCell flushLeft>
              <span className="font-semibold">No phase</span>
              <span className="text-[12px] font-medium text-muted-foreground">
                {unphasedDated.filter(visible).length} tasks
              </span>
            </PlanCell>
            <PlanCell right>
              <span className="font-mono text-[12px] tabular-nums text-muted-foreground">
                {formatDayRange(
                  unphasedDated
                    .map((task) => task.start_at ?? "")
                    .filter(Boolean)
                    .sort()[0],
                  unphasedDated
                    .map((task) => taskEnd(task, weekdays, minutesPerDay))
                    .filter((value): value is string => value !== null)
                    .sort()
                    .at(-1),
                )}
              </span>
            </PlanCell>
            <PlanCell>
              <BurnStrip
                usedHours={Math.round(effortOf(unphasedDated).loggedMinutes / 60)}
                totalHours={Math.round(effortOf(unphasedDated).estimateMinutes / 60)}
                label="Tasks without a phase"
              />
            </PlanCell>
            <PlanCheckCell />
          </PlanPhaseRow>
          {unphasedDated.filter(visible).map((task) => taskRow(task))}
          {createRow(null)}
        </div>
      ) : null}

      {undated.length > 0 ? (
        <div role="rowgroup" data-testid="plan-undated">
          <PlanPhaseRow columns={phaseColumns} tone="muted">
            <PlanCheckCell />
            <PlanCell flushLeft>
              <span className="font-semibold">Not dated yet</span>
              <span className="text-[12px] font-medium text-muted-foreground">
                {visibleUndated.length} tasks, {hoursLabel(effortOf(visibleUndated).estimateMinutes)}
              </span>
            </PlanCell>
            <PlanCell right>
              <span className="truncate text-[12px] font-medium text-muted-foreground">
                Not on the timeline or in anyone&rsquo;s week
              </span>
            </PlanCell>
            <PlanCell right>
              <Button size="sm" variant="secondary" onClick={onSetDates}>
                <Icon name="calendar" size={14} />
                Set dates
              </Button>
            </PlanCell>
            <PlanCheckCell />
          </PlanPhaseRow>
          {visibleUndated.map((task) => taskRow(task, true))}
        </div>
      ) : null}
    </PlanTable>
  );
}
