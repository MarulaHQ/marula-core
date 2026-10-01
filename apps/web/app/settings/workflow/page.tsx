// /settings/workflow — per-board workflow editor (TASKLIST 2.6).
//
// Purpose: three editors over EXISTING backends for one picked board:
//   • statuses — custom statuses bound to semantic categories (behavior
//     reads the category, never the label), rendered with prominent
//     category chips;
//   • transitions — explicit allowed-matrix editor (checkbox grid) saved
//     wholesale via the transitions PUT; the automatic/floor-mode default
//     is displayed until a cell is touched, and can be restored;
//   • checklist template — ordered required-steps CRUD plus the close-time
//     enforcement toggle.
// Editing is politely disabled for users without board-admin / org
// manage-boards rights using real permission state (nav my_role +
// /api/v1/me permissions); read-only views still render fully.
//
// Non-purpose: no new backend endpoints, no board-member management, and
// no SLA/calendar concerns — those live on their own pages.
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import {
  Button,
  Card,
  Checkbox,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Tag,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useEffect, useMemo, useReducer, useState } from "react";
import { api, useAuth, useRequireAuth } from "../../../lib/api";
import { SaveBar, SettingsHeader } from "../../../components/settings/settings-shell";
import {
  checklistIsDirty,
  checklistPayload,
  checklistReducer,
  checklistTemplateFromServer,
  categoryTone,
  edgeKey,
  matrixPayload,
  matrixReducer,
  planStatusSaves,
  STATUS_CATEGORIES,
  statusDraftIsDirty,
  statusDraftReducer,
  statusRowsFromServer,
  validateStatusRow,
} from "../../../lib/admin/workflow";
import { canConfigureBoard } from "../../../lib/admin/board-access";
import { StatusesSection } from "./statuses-section";

type BoardNavEntry = components["schemas"]["BoardNavEntry"];
type BoardStatus = components["schemas"]["BoardStatus"];
type BoardTransitionsResponse = components["schemas"]["BoardTransitionsResponse"];

interface ChecklistView {
  items: { text: string; sort_order?: number }[];
  require_checklist_close: boolean;
  template: { id: string; name: string } | null | undefined;
}

const CATEGORY_LABELS: Record<string, string> = {
  open: "Open",
  in_progress: "In progress",
  waiting: "Waiting",
  closed: "Closed",
};

/** One-line behavior note per category — chips carry the semantics. */
const CATEGORY_HINTS: Record<string, string> = {
  open: "entry state; new tickets land here",
  in_progress: "billable work happens",
  waiting: "SLA clocks pause",
  closed: "terminal; billing stops",
};

export default function WorkflowSettingsPage() {
  const auth = useRequireAuth();
  const { me } = useAuth();
  const [boards, setBoards] = useState<BoardNavEntry[] | null>(null);
  const [boardId, setBoardId] = useState<string>("");

  // Server snapshots.
  const [statuses, setStatuses] = useState<BoardStatus[]>([]);
  const [transitions, setTransitions] = useState<BoardTransitionsResponse | null>(null);
  const [checklistServer, setChecklistServer] = useState<ReturnType<
    typeof checklistTemplateFromServer
  > | null>(null);

  // Drafts.
  const [statusDraft, dispatchStatus] = useReducer(statusDraftReducer, {
    rows: [],
    removedIds: [],
  });
  const [matrix, dispatchMatrix] = useReducer(matrixReducer, {
    checked: new Set<string>(),
    pristineFloor: false,
  });
  const [checklistDraft, dispatchChecklist] = useReducer(checklistReducer, {
    name: "",
    requireClose: false,
    items: [],
  });
  const [newStatusName, setNewStatusName] = useState("");
  const [newItemText, setNewItemText] = useState("");

  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  type SectionKey = "statuses" | "matrix" | "checklist";
  const [savingSection, setSavingSection] = useState<SectionKey | null>(null);
  const [sectionError, setSectionError] = useState<Partial<Record<SectionKey, string>>>({});
  const [savedSection, setSavedSection] = useState<string | null>(null);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const { data, error } = await api.api.GET("/api/v1/boards/nav");
      if (error !== undefined || !data) {
        setLoadError("Failed to load boards.");
        return;
      }
      setBoards(data);
      const first = data[0];
        if (first !== undefined && data.length > 0) setBoardId((current) => current || first.id);
    })();
  }, [auth.status]);

  useEffect(() => {
    if (boardId === "") return;
    setLoading(true);
    setLoadError(null);
    setSavedSection(null);
    setSectionError({});
    void (async () => {
      const [statusRes, transRes, checkRes] = await Promise.all([
        api.api.GET("/api/v1/boards/{id}/statuses", { params: { path: { id: boardId } } }),
        api.api.GET("/api/v1/boards/{id}/transitions", { params: { path: { id: boardId } } }),
        api.api.GET("/api/v1/boards/{id}/checklist-template", { params: { path: { id: boardId } } }),
      ]);
      if (statusRes.data) {
        setStatuses(statusRes.data);
        dispatchMatrix({
          type: "init",
          mode: transRes.data?.mode ?? "floor",
          explicitEdges: transRes.data?.edges ?? [],
          statuses: statusRes.data,
        });
        dispatchStatus({ type: "replace", rows: statusRowsFromServer(statusRes.data) });
      }
      if (transRes.data) setTransitions(transRes.data);
      if (checkRes.data) {
        const view: ChecklistView = {
          items: checkRes.data.items.map((i) => ({ text: i.text, sort_order: i.sort_order })),
          require_checklist_close: checkRes.data.require_checklist_close,
          template: checkRes.data.template ?? null,
        };
        const snapshot = checklistTemplateFromServer(view);
        setChecklistServer(snapshot);
        dispatchChecklist({
          type: "init",
          name: snapshot.name,
          requireClose: snapshot.requireClose,
          items: snapshot.items,
        });
      }
      setLoading(false);
    })();
    return () => {
      setChecklistServer(null);
      setTransitions(null);
      setStatuses([]);
    };
  }, [boardId]);

  const selectedBoard = boards?.find((b) => b.id === boardId) ?? null;
  const canEdit = selectedBoard !== null && me !== null && canConfigureBoard(selectedBoard, me.permissions);

  const statusDirty = statusDraftIsDirty(statuses, statusDraft);
  const matrixDirty = !matrix.pristineFloor;
  const checklistDirty =
    checklistServer !== null &&
    checklistIsDirty(
      checklistServer,
      checklistDraft,
    );

  const rowErrors = useMemo(
    () => statusDraft.rows.map((row) => validateStatusRow(row)),
    [statusDraft.rows],
  );
  const anyRowError = rowErrors.some((e) => e !== null);

  // Matrix grid axes follow the SAVED status list: the transitions PUT
  // requires real board-status ids, so unsaved draft rows are not offered
  // until "Save statuses" confirms them.
  const gridStatuses = statuses;

  const saveStatuses = async () => {
    if (!canEdit) return;
    setSavingSection("statuses");
    setSectionError((prev) => ({ ...prev, statuses: undefined }));
    const plan = planStatusSaves(statuses, statusDraft);
    try {
      for (const del of plan.deletes) {
        const res = await api.api.DELETE("/api/v1/boards/{id}/statuses/{status_id}", {
          params: { path: { id: boardId, status_id: del.statusId } },
        });
        if (res.error) throw new Error("a referenced or protected status blocked deletion");
      }
      for (const create of plan.creates) {
        const res = await api.api.POST("/api/v1/boards/{id}/statuses", {
          params: { path: { id: boardId } },
          body: create,
        });
        if (res.error || !res.data) throw new Error("adding a status was rejected");
      }
      for (const upd of plan.updates) {
        const res = await api.api.PATCH("/api/v1/boards/{id}/statuses/{status_id}", {
          params: { path: { id: boardId, status_id: upd.statusId } },
          body: upd.body,
        });
        if (res.error || !res.data) throw new Error("a status update was rejected");
      }
      // Re-sync server snapshots + drafts from confirmed state.
      const [refreshed, transRefreshed] = await Promise.all([
        api.api.GET("/api/v1/boards/{id}/statuses", { params: { path: { id: boardId } } }),
        api.api.GET("/api/v1/boards/{id}/transitions", { params: { path: { id: boardId } } }),
      ]);
      if (refreshed.data) {
        setStatuses(refreshed.data);
        dispatchStatus({ type: "replace", rows: statusRowsFromServer(refreshed.data) });
      }
      if (transRefreshed.data) {
        setTransitions(transRefreshed.data);
        dispatchMatrix({
          type: "init",
          mode: transRefreshed.data.mode,
          explicitEdges: transRefreshed.data.edges,
          statuses: refreshed.data ?? [],
        });
      }
      setSavedSection("statuses");
    } catch (e) {
      setSectionError((prev) => ({ ...prev, statuses: friendlyMutationText(e) }));
    } finally {
      setSavingSection(null);
    }
  };

  const saveMatrix = async () => {
    if (!canEdit) return;
    setSavingSection("matrix");
    setSectionError((prev) => ({ ...prev, matrix: undefined }));
    try {
      const res = await api.api.PUT("/api/v1/boards/{id}/transitions", {
        params: { path: { id: boardId } },
        body: matrixPayload(matrix),
      });
      if (res.error || !res.data) throw new Error("the transition matrix was rejected");
      setTransitions(res.data);
      setSavedSection("matrix");
    } catch (e) {
      setSectionError((prev) => ({ ...prev, matrix: friendlyMutationText(e) }));
    } finally {
      setSavingSection(null);
    }
  };

  const saveChecklist = async () => {
    if (!canEdit) return;
    setSavingSection("checklist");
    setSectionError((prev) => ({ ...prev, checklist: undefined }));
    try {
      const res = await api.api.PUT("/api/v1/boards/{id}/checklist-template", {
        params: { path: { id: boardId } },
        body: checklistPayload(checklistDraft),
      });
      if (res.error || !res.data) throw new Error("the checklist template was rejected");
      const view: ChecklistView = {
        items: res.data.items.map((i) => ({ text: i.text, sort_order: i.sort_order })),
        require_checklist_close: res.data.require_checklist_close,
        template: res.data.template ?? null,
      };
      setChecklistServer(checklistTemplateFromServer(view));
      dispatchChecklist({ type: "set-name", value: view.template?.name ?? "" });
      dispatchChecklist({ type: "set-require-close", value: view.require_checklist_close });
      setSavedSection("checklist");
    } catch (e) {
      setSectionError((prev) => ({ ...prev, checklist: friendlyMutationText(e) }));
    } finally {
      setSavingSection(null);
    }
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-6 max-w-3xl">
      <SettingsHeader
        title="Workflow"
        description="Per-board states, allowed moves, and the pre-close checklist. Behavior binds to a status's semantic category, never its label."
      />

      <Card className="p-4">
        <Label htmlFor="wf-board">Board</Label>
        <Select
          value={boardId}
          onValueChange={(v) => setBoardId(v)}
          disabled={boards === null || boards.length === 0}
        >
          <SelectTrigger className="w-72 mt-1" aria-label="Pick board">
            <SelectValue placeholder={boards === null ? "Loading boards…" : "Pick a board"} />
          </SelectTrigger>
          <SelectContent>
            {(boards ?? []).map((b) => (
              <SelectItem key={b.id} value={b.id}>
                {b.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {selectedBoard && (
          <div className="mt-2 flex items-center gap-2">
            {!canEdit && (
              <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                Read-only — editing needs board-admin (or org manage-boards) rights.
              </span>
            )}
            {canEdit && (
              <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                You can edit this workflow.
              </span>
            )}
          </div>
        )}
      </Card>

      {boards !== null && boards.length === 0 && (
        <EmptyState message="No boards yet — create a board first." />
      )}
      {loadError !== null && (
        <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {loadError}
        </p>
      )}

      {selectedBoard && (
        <>
          {/* --- Statuses ------------------------------------------------ */}
          {/* The list itself is the pack's drawn surface (states 02-08); the
              transitions matrix and the checklist below still ride this
              page's own snapshot, so a save here re-syncs both. */}
          <StatusesSection
            boardId={boardId}
            canEdit={canEdit}
            onChanged={() => void saveStatuses()}
          />

          {/* --- Transition matrix --------------------------------------- */}
          <Card className="p-5 grid gap-3">
            <h2 className="text-sm font-semibold">Allowed transitions</h2>
            <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Rows are the source state, columns the destination. Automatic mode allows every move
              except Closed → Open; touching any cell switches this board to an explicit matrix.
            </p>
            <div className="overflow-x-auto">
              <table className="text-[12px]" style={{ borderCollapse: "collapse" }}>
                <thead>
                  <tr>
                    <th scope="col" className="p-1 pr-3"></th>
                    {gridStatuses.map((s) => (
                      <th key={`head-${s.id}`} scope="col" className="p-1 px-2 font-medium" style={{ color: "var(--m-text-secondary)" }}>
                        {s.name}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {gridStatuses.map((from) => (
                    <tr key={`row-${from.id}`}>
                      <th scope="row" className="p-1 pr-3 text-left font-medium whitespace-nowrap">
                        <Tag tone={categoryTone(from.category)}>{from.name}</Tag>
                      </th>
                      {gridStatuses.map((to) => {
                        const key = edgeKey(from.id, to.id);
                        const selfTransition = from.id === to.id;
                        const disabled = !canEdit || selfTransition;
                        const checked = selfTransition ? true : matrix.checked.has(key);
                        return (
                          <td key={`cell-${key}`} className="p-1 text-center" style={{ border: "1px solid var(--m-border)" }}>
                            {selfTransition ? (
                              <span style={{ color: "var(--m-text-muted)" }}>—</span>
                            ) : (
                              <Checkbox
                                checked={checked}
                                disabled={disabled}
                                aria-label={`Allow ${from.name} → ${to.name}`}
                                onCheckedChange={() => dispatchMatrix({ type: "toggle", key })}
                                className="mx-auto"
                              />
                            )}
                          </td>
                        );
                      })}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              Current mode:{" "}
              {transitions?.mode === "explicit" && matrix.pristineFloor
                ? "explicit (saved)"
                : matrix.pristineFloor
                  ? "automatic"
                  : "custom"}
              . Self-moves are never offered; Closed→Open stays blocked until you explicitly allow it.
            </p>
            {canEdit && (
              <SaveBar
                dirty={matrixDirty}
                saving={savingSection === "matrix"}
                saved={savedSection === "matrix" && !sectionError["matrix"]}
                error={sectionError["matrix"] ?? null}
                onSave={() => void saveMatrix()}
                onRevert={
                  !matrix.pristineFloor
                    ? () =>
                        dispatchMatrix({
                          type: "init",
                          mode: "floor",
                          explicitEdges: [],
                          statuses: statuses,
                        })
                    : undefined
                }
                saveLabel="Save transitions"
              />
            )}
          </Card>

          {/* --- Checklist template -------------------------------------- */}
          <Card className="p-5 grid gap-3">
            <h2 className="text-sm font-semibold">Pre-close checklist</h2>
            <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Required steps every ticket completes before closing on this board.
            </p>
            <label className="grid gap-1 max-w-sm">
              <Label htmlFor="cl-name">Template name</Label>
              <Input
                id="cl-name"
                value={checklistDraft.name}
                disabled={!canEdit}
                onChange={(e) => dispatchChecklist({ type: "set-name", value: e.target.value })}
              />
            </label>
            <ol className="grid gap-2 pl-0" style={{ listStyle: "none" }}>
              {checklistDraft.items.map((item, index) => (
                <li key={`item-${index}`} className="flex items-center gap-2">
                  <span className="text-[12px] tabular-nums w-5 text-right" style={{ color: "var(--m-text-muted)" }}>
                    {index + 1}.
                  </span>
                  <Input
                    aria-label={`Step ${index + 1}`}
                    value={item}
                    disabled={!canEdit}
                    onChange={(e) =>
                      dispatchChecklist({ type: "update-item", index, text: e.target.value })
                    }
                    className="max-w-md"
                  />
                  {canEdit && (
                    <span className="flex gap-1">
                      <Button
                        variant="ghost"
                        aria-label={`Move step ${index + 1} up`}
                        disabled={index === 0}
                        onClick={() => dispatchChecklist({ type: "move-item", index, direction: -1 })}
                      >
                        ↑
                      </Button>
                      <Button
                        variant="ghost"
                        aria-label={`Move step ${index + 1} down`}
                        disabled={index === checklistDraft.items.length - 1}
                        onClick={() => dispatchChecklist({ type: "move-item", index, direction: 1 })}
                      >
                        ↓
                      </Button>
                      <Button
                        variant="ghost"
                        aria-label={`Remove step ${index + 1}`}
                        onClick={() => dispatchChecklist({ type: "remove-item", index })}
                      >
                        ✕
                      </Button>
                    </span>
                  )}
                </li>
              ))}
            </ol>
            {checklistDraft.items.length === 0 && (
              <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                No steps yet — add the first required action below.
              </p>
            )}
            {canEdit && (
              <form
                className="flex items-center gap-2"
                onSubmit={(e) => {
                  e.preventDefault();
                  const text = newItemText.trim();
                  if (text.length === 0) return;
                  dispatchChecklist({ type: "add-item", text });
                  setNewItemText("");
                }}
              >
                <Input
                  aria-label="New checklist step"
                  placeholder="Add required step…"
                  value={newItemText}
                  onChange={(e) => setNewItemText(e.target.value)}
                  className="w-80"
                />
                <Button type="submit" variant="secondary" disabled={newItemText.trim().length === 0}>
                  Add step
                </Button>
              </form>
            )}
            <label className="inline-flex items-center gap-2 text-[13px]">
              <Checkbox
                checked={checklistDraft.requireClose}
                disabled={!canEdit}
                onCheckedChange={(v) =>
                  dispatchChecklist({ type: "set-require-close", value: v === true })
                }
                aria-label="Require completed checklist before close"
              />
              Block closing while steps are incomplete
            </label>
            {canEdit && (
              <SaveBar
                dirty={checklistDirty}
                saving={savingSection === "checklist"}
                saved={savedSection === "checklist" && !sectionError["checklist"]}
                error={sectionError["checklist"] ?? null}
                onSave={() => void saveChecklist()}
                onRevert={
                  checklistDirty && checklistServer !== null
                    ? () =>
                        dispatchChecklist({
                          type: "init",
                          name: checklistServer.name,
                          requireClose: checklistServer.requireClose,
                          items: checklistServer.items,
                        })
                    : undefined
                }
                saveLabel="Save checklist"
              />
            )}
          </Card>
        </>
      )}
    </div>
  );
}
