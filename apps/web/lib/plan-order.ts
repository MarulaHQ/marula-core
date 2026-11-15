/**
 * plan-order — the plan's order as data, so a drag is arithmetic instead of
 * DOM bookkeeping.
 *
 * The plan is two lists: the project's phases, and the project's tasks grouped
 * by the phase they belong to (with one group for work that has no phase yet).
 * A drag names a source row and a target row; these functions turn that into the
 * complete order the `/order` endpoint expects — which is also the whole point
 * of the endpoint: one request, no half-applied order.
 *
 * A task dropped on another phase's row changes phase. That is the decision this
 * module encodes: the group a task appears in IS its phase, so a drag can move
 * work between phases and never leaves a task in a group it does not belong to.
 *
 * Non-purpose: no fetching, no React. The table reports a drop, the surface
 * sends the result.
 */

export type OrderKind = "phase" | "task";

/** The tasks of one phase, in the order they should read. */
export interface PlanOrderGroup {
  phase_id: string | null;
  task_ids: string[];
}

/** The whole plan order, as a drop leaves it. */
export interface PlanOrder {
  phases: string[];
  tasks: PlanOrderGroup[];
}

/** Which half of a row a drop landed on. */
export type DropPosition = "before" | "after";

/** The row a drag started from. */
export interface OrderSource {
  kind: OrderKind;
  id: string;
}

/** The row a drag landed on. */
export interface OrderTarget {
  kind: OrderKind;
  id: string;
  position: DropPosition;
}

/** A phase as the order needs it: identity and the number that sorts it. */
export interface OrderPhase {
  id: string;
  sort_order?: number;
}

/** A task as the order needs it: identity, its phase and its number. */
export interface OrderTask {
  id: string;
  phase_id?: string | null;
  sort_order?: number;
}

const byOrder = (a: { sort_order?: number }, b: { sort_order?: number }): number =>
  (a.sort_order ?? 0) - (b.sort_order ?? 0);

/**
 * The plan as two ordered lists.
 *
 * The unphased group is included only when it has work, and it is always last:
 * "No phase" is a bucket, not a phase, and a plan that names it first would read
 * as if it came before the project's first phase.
 */
export function orderFromPlan(
  phases: readonly OrderPhase[],
  tasks: readonly OrderTask[],
): PlanOrder {
  const orderedPhases = [...phases].sort(byOrder).map((phase) => phase.id);
  const known = new Set(orderedPhases);
  const groups: PlanOrderGroup[] = orderedPhases.map((phaseId) => ({
    phase_id: phaseId,
    task_ids: [],
  }));
  const unphased: string[] = [];
  for (const task of [...tasks].sort(byOrder)) {
    const phaseId = task.phase_id ?? null;
    if (phaseId !== null && known.has(phaseId)) {
      groups.find((group) => group.phase_id === phaseId)?.task_ids.push(task.id);
    } else {
      unphased.push(task.id);
    }
  }
  if (unphased.length > 0) groups.push({ phase_id: null, task_ids: unphased });
  return { phases: orderedPhases, tasks: groups };
}

/** The group a task sits in, or undefined when the order does not name it. */
export function groupOf(order: PlanOrder, taskId: string): PlanOrderGroup | undefined {
  return order.tasks.find((group) => group.task_ids.includes(taskId));
}

/** A copy of the order with every task group holding a fresh array. */
function clone(order: PlanOrder): PlanOrder {
  return {
    phases: [...order.phases],
    tasks: order.tasks.map((group) => ({
      phase_id: group.phase_id,
      task_ids: [...group.task_ids],
    })),
  };
}

/** Insert `value` next to `anchor`, or append when the anchor is absent. */
function insertNear<T>(list: T[], value: T, anchor: T | undefined, position: DropPosition): void {
  const at = anchor === undefined ? list.length : list.indexOf(anchor);
  if (at < 0) {
    list.push(value);
    return;
  }
  list.splice(position === "before" ? at : at + 1, 0, value);
}

/**
 * Apply one drop.
 *
 * Returns the SAME object when the drop changes nothing (a row onto itself, or
 * one position away), so a caller can skip the request instead of writing back
 * what it already had.
 *
 * Dropping a task on a phase row places it at the start of that phase when the
 * pointer was over the row's top half and at the end when it was over the bottom
 * half — the half a person aimed at, not a fixed end.
 */
export function moveItem(
  order: PlanOrder,
  source: OrderSource,
  target: OrderTarget,
): PlanOrder {
  if (source.kind === "phase" && target.kind === "phase") {
    return movePhase(order, source.id, target.id, target.position);
  }
  if (source.kind === "task" && target.kind === "task") {
    return moveTask(order, source.id, target.id, target.position);
  }
  if (source.kind === "task") {
    // A task dropped on a phase row joins that phase.
    return moveTaskToPhase(order, source.id, target.id, target.position);
  }
  // A phase dropped on a task row lands beside the phase that task is in.
  const group = groupOf(order, target.id);
  if (group === undefined || group.phase_id === null) return order;
  return movePhase(order, source.id, group.phase_id, target.position);
}

/** Move one phase next to another. */
export function movePhase(
  order: PlanOrder,
  phaseId: string,
  targetId: string,
  position: DropPosition,
): PlanOrder {
  if (phaseId === targetId) return order;
  const next = clone(order);
  const from = next.phases.indexOf(phaseId);
  if (from < 0) return order;
  next.phases.splice(from, 1);
  insertNear(next.phases, phaseId, targetId, position);
  return sameOrder(order, next) ? order : next;
}

/** Move one task next to another task, in whatever phase that task is in. */
export function moveTask(
  order: PlanOrder,
  taskId: string,
  targetTaskId: string,
  position: DropPosition,
): PlanOrder {
  if (taskId === targetTaskId) return order;
  const targetGroup = groupOf(order, targetTaskId);
  if (targetGroup === undefined) return order;
  const phaseId = targetGroup.phase_id;
  const next = clone(order);
  detachTask(next, taskId);
  const group = next.tasks.find((candidate) => candidate.phase_id === phaseId);
  if (group === undefined) {
    next.tasks.push({ phase_id: phaseId, task_ids: [taskId] });
  } else {
    insertNear(group.task_ids, taskId, targetTaskId, position);
  }
  return sameOrder(order, next) ? order : next;
}

/** Move one task into a phase, at the start or the end of it. */
export function moveTaskToPhase(
  order: PlanOrder,
  taskId: string,
  phaseId: string,
  position: DropPosition,
): PlanOrder {
  const next = clone(order);
  detachTask(next, taskId);
  let group = next.tasks.find((candidate) => candidate.phase_id === phaseId);
  if (group === undefined) {
    group = { phase_id: phaseId, task_ids: [] };
    next.tasks.push(group);
  }
  if (position === "before") group.task_ids.unshift(taskId);
  else group.task_ids.push(taskId);
  return sameOrder(order, next) ? order : next;
}

/** Remove a task from whichever group holds it. */
function detachTask(order: PlanOrder, taskId: string): void {
  for (const group of order.tasks) {
    const at = group.task_ids.indexOf(taskId);
    if (at >= 0) group.task_ids.splice(at, 1);
  }
}

/** True when two orders say the same thing. */
export function sameOrder(a: PlanOrder, b: PlanOrder): boolean {
  if (a.phases.length !== b.phases.length) return false;
  for (let i = 0; i < a.phases.length; i += 1) {
    if (a.phases[i] !== b.phases[i]) return false;
  }
  const groupsA = a.tasks.filter((group) => group.task_ids.length > 0);
  const groupsB = b.tasks.filter((group) => group.task_ids.length > 0);
  if (groupsA.length !== groupsB.length) return false;
  return groupsA.every((group, index) => {
    const other = groupsB[index];
    if (other === undefined || other.phase_id !== group.phase_id) return false;
    if (other.task_ids.length !== group.task_ids.length) return false;
    return group.task_ids.every((taskId, at) => other.task_ids[at] === taskId);
  });
}

/**
 * Move one entry of a list up or down, for the keyboard path.
 *
 * Returns the same object when the entry is already at the end it is moving
 * toward, so the caller can skip a write that would change nothing.
 */
export function stepOrder(
  order: PlanOrder,
  kind: OrderKind,
  id: string,
  direction: -1 | 1,
): PlanOrder {
  const list = kind === "phase" ? order.phases : (groupOf(order, id)?.task_ids ?? []);
  const at = list.indexOf(id);
  if (at < 0) return order;
  const to = at + direction;
  const neighbour = list[to];
  if (neighbour === undefined) return order;
  return moveItem(
    order,
    { kind, id },
    { kind, id: neighbour, position: direction < 0 ? "before" : "after" },
  );
}

/** Where a row sits in its list, for the words a notice says. */
export function positionOf(
  order: PlanOrder,
  kind: OrderKind,
  id: string,
): { index: number; total: number } {
  const list = kind === "phase" ? order.phases : (groupOf(order, id)?.task_ids ?? []);
  return { index: list.indexOf(id), total: list.length };
}
