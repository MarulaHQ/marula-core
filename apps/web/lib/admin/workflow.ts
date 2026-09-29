// Pure state logic for the /settings/workflow editors.
//
// Purpose: reducers + diff/payload helpers for statuses, the transition
// matrix, and the checklist template — dirty tracking and save-payload
// shaping only, fully unit-testable with no React or fetch involved.
// Payload types mirror @marula-core/sdk request schemas exactly.
//
// Non-purpose: no network I/O and no rendering; the page owns calls and
// error surfacing.
import type { components } from "@marula-core/sdk";

export type BoardStatus = components["schemas"]["BoardStatus"];
export type TicketStatusCategory = components["schemas"]["TicketStatusCategory"];
export type TransitionEdge = components["schemas"]["TransitionEdge"];
export type SetTransitionsRequest = components["schemas"]["SetTransitionsRequest"];
export type PutChecklistTemplateRequest = components["schemas"]["PutChecklistTemplateRequest"];
export type CreateStatusRequest = components["schemas"]["CreateStatusRequest"];
export type UpdateStatusRequest = components["schemas"]["UpdateStatusRequest"];

export const STATUS_CATEGORIES: readonly TicketStatusCategory[] = [
  "open",
  "in_progress",
  "waiting",
  "closed",
] as const;

/** Category → semantic tone used by Tag chips (theme.md §6). */
export function categoryTone(category: TicketStatusCategory): "success" | "warning" | "neutral" | "danger" {
  switch (category) {
    case "open":
      return "success";
    case "in_progress":
      return "warning";
    case "waiting":
      return "neutral";
    case "closed":
      return "danger";
  }
}

// --- statuses draft ----------------------------------------------------------

/**
 * One editable status row. Rows lacking `id` are new (created on save);
 * rows in `removedIds` vanish on save.
 */
export interface StatusRow {
  id: string | null;
  name: string;
  category: TicketStatusCategory;
  isEntry: boolean;
}

export interface StatusDraftState {
  /** Server-ordered working list of live rows (removals already filtered). */
  rows: StatusRow[];
  removedIds: string[];
}

export const emptyStatusDraft: StatusDraftState = { rows: [], removedIds: [] };

export function statusRowsFromServer(statuses: BoardStatus[]): StatusRow[] {
  return statuses.map((s) => ({
    id: s.id,
    name: s.name,
    category: s.category,
    isEntry: s.is_entry,
  }));
}

export type StatusDraftAction =
  /** Wholesale reset from server rows (board switch / post-save resync). */
  | { type: "replace"; rows: StatusRow[] }
  | { type: "add"; name: string }
  | { type: "update"; index: number; patch: Partial<Omit<StatusRow, "id">> }
  | { type: "remove"; index: number }
  | { type: "move"; index: number; direction: -1 | 1 };

export function statusDraftReducer(
  state: StatusDraftState,
  action: StatusDraftAction,
): StatusDraftState {
  switch (action.type) {
    case "replace":
      return { rows: [...action.rows], removedIds: [] };
    case "add": {
      const row: StatusRow = {
        id: null,
        name: action.name,
        category: "open",
        isEntry: true,
      };
      return { ...state, rows: [...state.rows, row] };
    }
    case "update": {
      const rows = state.rows.map((row, i) =>
        i === action.index ? { ...row, ...action.patch } : row,
      );
      return { ...state, rows };
    }
    case "remove": {
      const removed = state.rows[action.index];
      if (removed === undefined) return state;
      const rows = state.rows.filter((_, i) => i !== action.index);
      return {
        rows,
        removedIds:
          removed.id !== null ? [...state.removedIds, removed.id] : state.removedIds,
      };
    }
    case "move": {
      const target = action.index + action.direction;
      if (target < 0 || target >= state.rows.length) return state;
      const rows = [...state.rows];
      const [row] = rows.splice(action.index, 1);
      if (row === undefined) return state;
      rows.splice(target, 0, row);
      return { ...state, rows };
    }
  }
}

/** Stable dirty check: does any row differ from the server snapshot? */
export function statusDraftIsDirty(
  server: BoardStatus[],
  draft: StatusDraftState,
): boolean {
  if (draft.removedIds.length > 0) return true;
  if (draft.rows.length !== server.length) return true;
  return draft.rows.some((row, i) => {
    const s = server[i];
    return (
      s === undefined ||
      row.id !== s.id ||
      row.name !== s.name ||
      row.category !== s.category ||
      row.isEntry !== s.is_entry
    );
  });
}

/**
 * Ordered mutation plan to reconcile draft against server rows.
 * Deletes run first (frees names/slots), then creates top-to-bottom, then
 * updates; sort_order is the draft position.
 */
export interface StatusSavePlan {
  deletes: { statusId: string }[];
  creates: CreateStatusRequest[];
  updates: { statusId: string; body: UpdateStatusRequest }[];
}

export function planStatusSaves(
  server: BoardStatus[],
  draft: StatusDraftState,
): StatusSavePlan {
  const deletes = draft.removedIds.map((statusId) => ({ statusId }));
  const byId = new Map(server.map((s) => [s.id, s]));
  const creates: CreateStatusRequest[] = [];
  const updates: { statusId: string; body: UpdateStatusRequest }[] = [];
  draft.rows.forEach((row, index) => {
    if (row.id === null) {
      creates.push({
        name: row.name,
        category: row.category,
        is_entry: row.isEntry,
        sort_order: index,
      });
      return;
    }
    const original = byId.get(row.id);
    if (original === undefined) return;
    const changed =
      original.name !== row.name ||
      original.category !== row.category ||
      original.is_entry !== row.isEntry ||
      original.sort_order !== index;
    if (!changed) return;
    updates.push({
      statusId: row.id,
      body: {
        name: row.name,
        category: row.category,
        is_entry: row.isEntry,
        sort_order: index,
      },
    });
  });
  return { deletes, creates, updates };
}

/** Row-level validation mirroring server constraints; '' keys are valid rows. */
export function validateStatusRow(row: StatusRow): string | null {
  if (row.name.trim().length === 0) return "Name is required.";
  if (row.name.length > 64) return "Name must be at most 64 characters.";
  return null;
}

// --- transition matrix -------------------------------------------------------

/** Edge key format shared by UI grid state and payload building. */
export function edgeKey(from: string, to: string): string {
  return `${from}>${to}`;
}

export function parseEdgeKey(key: string): { from: string; to: string } | null {
  const parts = key.split(">");
  const from = parts[0];
  const to = parts[1];
  if (from === undefined || to === undefined || parts.length !== 2) return null;
  if (from.length === 0 || to.length === 0) return null;
  return { from, to };
}

/**
 * The floor-mode default implied matrix: any category may flow to any other
 * except closed→open (matches core_domain policy semantics).
 */
export function floorModeEdges(statuses: BoardStatus[]): Set<string> {
  const edges = new Set<string>();
  for (const from of statuses) {
    for (const to of statuses) {
      if (from.id === to.id) continue;
      if (from.category === "closed" && to.category === "open") continue;
      edges.add(edgeKey(from.id, to.id));
    }
  }
  return edges;
}

export interface MatrixDraftState {
  /** Checked cell keys; presence means allowed. */
  checked: Set<string>;
  /** False once the user touches a cell in floor mode (implicit upgrade). */
  pristineFloor: boolean;
}

export type MatrixDraftAction =
  | { type: "init"; mode: string; explicitEdges: TransitionEdge[]; statuses: BoardStatus[] }
  | { type: "toggle"; key: string };

export function matrixReducer(
  state: MatrixDraftState,
  action: MatrixDraftAction,
): MatrixDraftState {
  switch (action.type) {
    case "init": {
      if (action.mode === "explicit") {
        return {
          checked: new Set(action.explicitEdges.map((e) => edgeKey(e.from_status_id, e.to_status_id))),
          pristineFloor: false,
        };
      }
      return { checked: floorModeEdges(action.statuses), pristineFloor: true };
    }
    case "toggle": {
      const checked = new Set(state.checked);
      if (checked.has(action.key)) checked.delete(action.key);
      else checked.add(action.key);
      // Any manual touch upgrades floor → explicit governance.
      return { checked, pristineFloor: false };
    }
  }
}

/** Save payload: whole matrix replaces server state; shape = SDK PUT body. */
export function matrixPayload(state: MatrixDraftState): SetTransitionsRequest {
  const edges: TransitionEdge[] = [...state.checked].flatMap((key) => {
    const parsed = parseEdgeKey(key);
    return parsed ? [{ from_status_id: parsed.from, to_status_id: parsed.to }] : [];
  });
  return { edges };
}

// --- checklist template editor ----------------------------------------------

export interface ChecklistDraftState {
  name: string;
  requireClose: boolean;
  items: string[];
}

export type ChecklistDraftAction =
  | {
      type: "init";
      name: string;
      requireClose: boolean;
      /** Server-ordered item texts. */
      items: string[];
    }
  | { type: "set-name"; value: string }
  | { type: "set-require-close"; value: boolean }
  | { type: "add-item"; text: string }
  | { type: "update-item"; index: number; text: string }
  | { type: "remove-item"; index: number }
  | { type: "move-item"; index: number; direction: -1 | 1 };

export function checklistTemplateFromServer(view: {
  items: { text: string }[];
  require_checklist_close: boolean;
  template: { name: string } | null | undefined;
}): ChecklistDraftState {
  return {
    name: view.template?.name ?? "",
    requireClose: view.require_checklist_close,
    items: view.items.map((i) => i.text),
  };
}

export function checklistReducer(
  state: ChecklistDraftState,
  action: ChecklistDraftAction,
): ChecklistDraftState {
  switch (action.type) {
    case "init":
      return {
        name: action.name,
        requireClose: action.requireClose,
        items: [...action.items],
      };
    case "set-name":
      return { ...state, name: action.value };
    case "set-require-close":
      return { ...state, requireClose: action.value };
    case "add-item":
      return { ...state, items: [...state.items, action.text.trim()] };
    case "update-item":
      return {
        ...state,
        items: state.items.map((t, i) => (i === action.index ? action.text : t)),
      };
    case "remove-item":
      return { ...state, items: state.items.filter((_, i) => i !== action.index) };
    case "move-item": {
      const target = action.index + action.direction;
      if (target < 0 || target >= state.items.length) return state;
      const items = [...state.items];
      const [item] = items.splice(action.index, 1);
      if (item === undefined) return state;
      items.splice(target, 0, item);
      return { ...state, items };
    }
  }
}

export function checklistPayload(state: ChecklistDraftState): PutChecklistTemplateRequest {
  return {
    name: state.name.trim() || "Board checklist",
    items: state.items.map((t) => t.trim()).filter((t) => t.length > 0),
    require_checklist_close: state.requireClose,
  };
}

/** Whole-template dirty check against the last-loaded server view. */
export function checklistIsDirty(
  server: ReturnType<typeof checklistTemplateFromServer>,
  draft: ChecklistDraftState,
): boolean {
  return (
    server.name !== draft.name ||
    server.requireClose !== draft.requireClose ||
    server.items.join("\u0000") !== draft.items.join("\u0000")
  );
}
