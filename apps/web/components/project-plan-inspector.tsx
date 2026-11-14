"use client";
/**
 * ProjectPlanInspector — the five questions about one task (S3.11, S3.17).
 *
 * Purpose: selecting a row answers who and when, how much, what it waits on and
 * what waits on it, what it physically needs, and where the work happens, in
 * that order. Editing the start date happens here and always goes through the
 * surface's impact preview, never a direct save.
 *
 * At 1024 and below the same answers become a compact bottom sheet over the
 * table, four columns across and the actions on one line, so the selected row
 * stays visible above it.
 *
 * Two actions are present but disabled, with the reason stated in the panel:
 * promoting a task to a ticket and adding a dependency both need endpoints this
 * build does not carry (the task-ticket link and the dependency write with a
 * cycle refusal). Hiding them would read as "the product cannot do this"; a
 * greyed control beside its reason reads as "not in this build".
 *
 * Non-purpose: no fetching and no writes. The surface owns the reads.
 */
import {
  BottomSheet,
  DepChain,
  Eyebrow,
  Inspector,
  InspectorBody,
  InspectorFooter,
  InspectorHeader,
  InspectorTitle,
  Button,
  FieldHint,
  Input,
  KeyValueList,
  Label,
  Rule,
  WorkTypeMark,
  WORK_TYPES,
  type ChainState,
} from "@marula-core/ui";
import { Avatar, Icon, Pill } from "@marula-core/ui/kit";
import {
  dateInputValue,
  effortOf,
  formatDay,
  formatDayRange,
  hoursLabel,
  isTaskLate,
  roleLabel,
  taskDays,
  taskEnd,
  workingDaysBetween,
  type PlanPerson,
  type PlanTask,
} from "../lib/projects-plan-types";
import { TaskStatusPill } from "./project-plan-table";

export interface PlanInspectorProps {
  task: PlanTask;
  people: ReadonlyMap<string, PlanPerson>;
  deps: Readonly<Record<string, PlanTask[]>>;
  dependents: Readonly<Record<string, PlanTask[]>>;
  minutesPerDay: number;
  weekdays: number;
  today: string;
  /** True at 1024 and below: the panel is a bottom sheet. */
  narrow: boolean;
  onClose: () => void;
  onStartDateChange: (task: PlanTask, date: string) => void;
  onAddDependency: () => void;
  onPromote: () => void;
}

function WorkTypeWords({ task }: { task: PlanTask }) {
  const type = task.work_type === null || task.work_type === undefined ? "coordination" : task.work_type;
  return (
    <span className="flex items-center gap-2">
      <WorkTypeMark type={type} />
      <span>{WORK_TYPES[type].label}</span>
    </span>
  );
}

function PersonLine({ task, people }: { task: PlanTask; people: ReadonlyMap<string, PlanPerson> }) {
  const person = task.assignee_id === null || task.assignee_id === undefined ? null : people.get(task.assignee_id);
  if (person !== undefined && person !== null) {
    const initials = person.display_name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((word) => word[0]!.toUpperCase())
      .join("");
    return (
      <span className="flex min-w-0 items-center gap-2">
        <Avatar initials={initials} title={person.display_name} />
        <span className="min-w-0 truncate">{person.display_name}</span>
      </span>
    );
  }
  if (task.assignee_id !== null && task.assignee_id !== undefined) {
    return <span className="text-muted-foreground">Assigned to someone outside this directory</span>;
  }
  const role = roleLabel(task.role_key);
  return (
    <span className="flex min-w-0 items-center gap-2 text-muted-foreground">
      <Avatar unassigned aria-hidden="true" />
      <span className="min-w-0 truncate">{role === "" ? "No one yet" : `${role}, open`}</span>
    </span>
  );
}

function needsOf(task: PlanTask): string[] {
  const skills = task.required_skills ?? [];
  return [...skills, ...(task.external_owner === null || task.external_owner === undefined ? [] : [task.external_owner])];
}

export function ProjectPlanInspector(props: PlanInspectorProps): React.JSX.Element {
  const { task, people, deps, dependents, minutesPerDay, today, narrow, onClose } = props;
  const effort = effortOf([task]);
  const end = taskEnd(task, props.weekdays, minutesPerDay);
  const days = taskDays(task, minutesPerDay);
  const blockers = deps[task.id] ?? [];
  const successors = dependents[task.id] ?? [];
  const needs = needsOf(task);
  const hasDates = task.start_at !== null && task.start_at !== undefined;

  const chain = [
    ...blockers.map((blocker) => ({
      label: blocker.title,
      state: (blocker.status === "completed" ? "done" : "todo") as ChainState,
      trailing: <span className="font-mono text-[12px] tabular-nums">{formatDay(taskEnd(blocker, props.weekdays, minutesPerDay))}</span>,
    })),
    {
      label: <b className="font-bold">This task</b>,
      state: "current" as ChainState,
      trailing: (
        <span className="font-mono text-[12px] tabular-nums">
          {hasDates ? `${formatDay(task.start_at)}${end === null ? "" : ` to ${formatDay(end)}`}` : "No dates"}
        </span>
      ),
    },
    ...successors.map((successor) => ({
      label: successor.title,
      state: "todo" as ChainState,
      trailing: <span className="font-mono text-[12px] tabular-nums">{formatDay(successor.start_at)}</span>,
    })),
  ];

  const dateField = (
    <div className="flex flex-col gap-1">
      <Label htmlFor={`task-start-${task.id}`}>Start date</Label>
      <Input
        id={`task-start-${task.id}`}
        type="date"
        value={dateInputValue(task.start_at)}
        onChange={(event) => {
          if (event.target.value !== "") props.onStartDateChange(task, event.target.value);
        }}
        className="h-9 w-[172px] font-mono tabular-nums"
      />
      <FieldHint>
        Changing this previews what moves before anything is saved.
      </FieldHint>
    </div>
  );

  const disabledActions = (
    <>
      <Button
        size="sm"
        variant="secondary"
        disabled
        title="Adding a dependency needs the cycle-refusing dependency write, which is not in this build yet."
        onClick={props.onAddDependency}
      >
        <Icon name="link" size={14} />
        Add dependency
      </Button>
      <Button
        size="sm"
        variant="secondary"
        disabled
        title="Promoting a task needs the task-ticket link API, which is not in this build yet."
        onClick={props.onPromote}
      >
        <Icon name="ticket" size={14} />
        Promote
      </Button>
    </>
  );

  if (narrow) {
    return (
      <BottomSheet label={`Task: ${task.title}`}>
        <div className="flex items-center gap-2">
          <WorkTypeMark type={task.work_type ?? "coordination"} />
          <b className="min-w-0 flex-1 truncate text-[15px]">{task.title}</b>
          <TaskStatusPill task={task} today={today} weekdays={props.weekdays} minutesPerDay={minutesPerDay} />
          <Button variant="ghost" size="sm" aria-label="Close task details" className="size-8 px-0" onClick={onClose}>
            <Icon name="x" size={18} />
          </Button>
        </div>
        <div className="grid grid-cols-2 gap-3 text-[13px] md:grid-cols-4">
          <div>
            <div className="text-muted-foreground">Who</div>
            <PersonLine task={task} people={people} />
          </div>
          <div>
            <div className="text-muted-foreground">When</div>
            <span className="font-mono tabular-nums">
              {hasDates
                ? `${formatDayRange(task.start_at, end)}${days === null ? "" : `, ${days} days`}`
                : "No dates"}
            </span>
          </div>
          <div>
            <div className="text-muted-foreground">Effort</div>
            <span className="font-mono tabular-nums">
              {hoursLabel(effort.estimateMinutes)} planned, {hoursLabel(effort.remainingMinutes)} left
            </span>
          </div>
          <div>
            <div className="text-muted-foreground">Waits on</div>
            {blockers.length === 0 ? (
              <span className="text-muted-foreground">Nothing</span>
            ) : (
              <span className="line-clamp-2">{blockers.map((blocker) => blocker.title).join(", ")}</span>
            )}
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {needs.map((need) => (
            <span key={need} className="inline-flex items-center gap-1.5 rounded-pill border border-border px-2.5 py-[3px] text-[12px] text-muted-foreground">
              <Icon name="shield" size={13} />
              {need}
            </span>
          ))}
          <span className="flex-1" />
          {disabledActions}
          {dateField}
        </div>
      </BottomSheet>
    );
  }

  return (
    <Inspector aria-label={`Task: ${task.title}`} className="max-h-full">
      <InspectorHeader>
        <WorkTypeMark type={task.work_type ?? "coordination"} />
        <div className="min-w-0 flex-1">
          <InspectorTitle>{task.title}</InspectorTitle>
          <div className="mt-[6px] flex flex-wrap items-center gap-2">
            <TaskStatusPill task={task} today={today} weekdays={props.weekdays} minutesPerDay={minutesPerDay} />
            {task.is_critical === true ? (
              <span className="inline-flex items-center gap-1 rounded-[6px] border border-border px-1.5 py-[1px] text-[12px] text-muted-foreground">
                <Icon name="link" size={12} />
                Critical
              </span>
            ) : null}
            {task.is_milestone ? <Pill tone="neutral">Milestone</Pill> : null}
            <span className="font-mono text-[12px] tabular-nums text-muted-foreground">
              AV-{task.id.slice(0, 4).toUpperCase()}
            </span>
          </div>
        </div>
        <Button variant="ghost" size="sm" aria-label="Close task details" className="size-8 px-0" onClick={onClose}>
          <Icon name="x" size={18} />
        </Button>
      </InspectorHeader>
      <InspectorBody>
        <KeyValueList>
          <dt>Who</dt>
          <dd>
            <PersonLine task={task} people={people} />
          </dd>
          <dt>When</dt>
          <dd>
            <span className="font-mono tabular-nums">
              {hasDates ? formatDayRange(task.start_at, end) : "No dates"}
            </span>{" "}
            <span className="text-muted-foreground">
              {days === null
                ? ""
                : `${days} ${days === 1 ? "day" : "days"} · ${workingDaysBetween(task.start_at, end, props.weekdays)} working days`}
            </span>
          </dd>
          <dt>Effort</dt>
          <dd>
            <span className="font-mono tabular-nums">{hoursLabel(effort.estimateMinutes)}</span>{" "}
            <span className="text-muted-foreground">planned,</span>{" "}
            <span className="font-mono tabular-nums">{hoursLabel(effort.remainingMinutes)}</span>{" "}
            <span className="text-muted-foreground">left</span>
          </dd>
          <dt>Work type</dt>
          <dd>
            <WorkTypeWords task={task} />
          </dd>
          <dt>Needs</dt>
          <dd>
            {needs.length === 0 ? (
              <span className="text-muted-foreground">Nothing recorded</span>
            ) : (
              <span className="flex flex-wrap gap-1.5">
                {needs.map((need) => (
                  <span key={need} className="inline-flex items-center gap-1.5 rounded-pill border border-border px-2.5 py-[3px] text-[12px]">
                    <Icon name="shield" size={13} />
                    {need}
                  </span>
                ))}
              </span>
            )}
          </dd>
        </KeyValueList>

        <Rule />

        <div className="flex flex-col gap-2">
          <Eyebrow>Dependency chain</Eyebrow>
          {chain.length === 1 ? (
            <p className="m-0 text-[13px] text-muted-foreground">
              {blockers.length === 0 && successors.length === 0
                ? "Nothing waits on this task, and it waits on nothing."
                : "Only this task."}
            </p>
          ) : (
            <DepChain nodes={chain} />
          )}
        </div>

        <Rule />

        <div className="flex flex-col gap-2">
          <Eyebrow>Where the work happens</Eyebrow>
          <p className="m-0 text-[13px]">
            {WORK_TYPES[task.work_type ?? "coordination"].label}
            {task.external_owner !== null && task.external_owner !== undefined
              ? ` · ${task.external_owner} owns this step from outside the company`
              : task.assignee_id === null || task.assignee_id === undefined
                ? ` · ${roleLabel(task.role_key) === "" ? "nobody assigned yet" : `${roleLabel(task.role_key)}, nobody assigned yet`}`
                : ""}
          </p>
          {isTaskLate(task, Date.parse(`${today}T23:59:59Z`), props.weekdays, minutesPerDay) ? (
            <p className="m-0 text-[13px] text-[var(--m-status-danger-fg)]">
              The planned end has passed and the task is not complete.
            </p>
          ) : null}
        </div>

        <Rule />

        {dateField}

        <p className="m-0 text-[12px] leading-[1.5] text-muted-foreground">
          Promoting a task to a ticket and editing dependencies need APIs this build does not
          carry yet, so both controls are present and disabled rather than hidden.
        </p>
      </InspectorBody>
      <InspectorFooter>
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          {disabledActions}
        </div>
      </InspectorFooter>
    </Inspector>
  );
}
