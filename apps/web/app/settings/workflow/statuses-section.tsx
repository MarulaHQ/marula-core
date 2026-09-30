// The Workflow group's Statuses surface — state 02 to 08 of the settings pack.
//
// Purpose: the drawn list, drawn once, over the REAL board-status API:
//   02 the list with kind pills and staged toggles
//   03 selection replacing the header, plus the unsaved-changes bar
//   04 the New status drawer with submit-time validation
//   05 the same drawer when the save did not land, with Try again
//   06 the destructive confirm that names what happens
//   07 the import drawer, a partly valid file
//   08 the empty list
//
// Three departures from the drawn picture, all because the product has no such
// knob, and all recorded in the lane landing record rather than faked:
//   - The drawn "Tickets" count column is absent: the ticket list returns
//     rows, not a total, and no status-count endpoint exists.
//   - The drawn "Visible to client" switch is present but DISABLED, because
//     client visibility is not a per-status setting in this product. The
//     column is drawn so the place exists, and the note under the table says
//     so in words rather than leaving a dead control unexplained.
//   - The drawn Kind choice has three options. The product binds behaviour to
//     four semantic categories (in_progress has carried billing behaviour
//     since the billing wave), so the drawer offers four in the pack's order.
//     Dropping one would silently change what a status does.
//
// "Pauses SLA" IS wired, and to the real thing: SLA clocks pause for the
// waiting category, so the switch stages that category and the save bar
// applies it. This is the pack's staged-toggles rule: a table stages, a page
// of its own toggles applies at once.
//
// Non-purpose: transitions and the pre-close checklist (the page's other two
// editors), board selection, and permissions (the page decides `canEdit`).

"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button, EmptyState } from "@marula-core/ui";
import { api } from "../../../lib/api";
import { resultErrorText } from "../../../lib/error-copy";
import type { components } from "@marula-core/sdk";
import {
  BulkActionBar,
  BulkDestructiveAction,
  ConfirmRemap,
  DrawerFooter,
  FieldInput,
  FormField,
  ImportDropZone,
  ImportPrimary,
  ImportReview,
  Note,
  SaveFailedBanner,
  SegmentedChoice,
  SettingsDrawer,
  StagedSaveBar,
  ToggleRow,
  ToggleSwitch,
  type ImportRow,
} from "../../../components/settings/settings-forms";

type BoardStatus = components["schemas"]["BoardStatus"];
type TicketStatusCategory = components["schemas"]["TicketStatusCategory"];

const KIND_OPTIONS: readonly TicketStatusCategory[] = [
  "open",
  "in_progress",
  "waiting",
  "closed",
] as const;

const KIND_LABEL: Record<TicketStatusCategory, string> = {
  open: "Open",
  in_progress: "In progress",
  waiting: "Waiting",
  closed: "Done",
};

/** The drawn kind pill. Open work is the gold tint, waiting is sand, done is
 * green — and no row is ever coloured without its word beside it. */
function KindPill({ category }: { category: TicketStatusCategory }): React.JSX.Element {
  const tone =
    category === "closed"
      ? "var(--m-status-success-bg)"
      : category === "waiting"
        ? "var(--m-surface-2)"
        : "var(--m-status-warning-bg)";
  const fg =
    category === "closed"
      ? "var(--m-status-success-fg)"
      : category === "waiting"
        ? "var(--m-text-secondary)"
        : "var(--m-status-warning-fg)";
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        height: 24,
        padding: "0 10px",
        borderRadius: 999,
        background: tone,
        color: fg,
        fontSize: 13,
        fontWeight: 600,
        whiteSpace: "nowrap",
      }}
    >
      <span
        aria-hidden="true"
        style={{ width: 7, height: 7, borderRadius: 999, background: "currentColor" }}
      />
      {KIND_LABEL[category]}
    </span>
  );
}

interface Draft {
  name: string;
  kind: TicketStatusCategory;
  pause: boolean;
  isEntry: boolean;
  wipLimit: string;
}

const EMPTY_DRAFT: Draft = {
  name: "",
  kind: "open",
  pause: false,
  isEntry: true,
  wipLimit: "",
};

export interface StatusesSectionProps {
  boardId: string;
  canEdit: boolean;
  /** Called after every successful write so the page re-syncs its snapshots
   * (the transitions matrix is built from the SAVED status list). */
  onChanged: () => void;
}

export function StatusesSection({
  boardId,
  canEdit,
  onChanged,
}: StatusesSectionProps): React.JSX.Element {
  const [rows, setRows] = useState<BoardStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [staged, setStaged] = useState<Record<string, TicketStatusCategory>>({});
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const [drawer, setDrawer] = useState<null | { mode: "new" | "edit"; id: string | null }>(null);
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [drawerFailed, setDrawerFailed] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<BoardStatus | null>(null);
  const [confirmBusy, setConfirmBusy] = useState(false);
  const [importing, setImporting] = useState(false);
  const fileInput = useRef<HTMLInputElement | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    const res = await api.api.GET("/api/v1/boards/{id}/statuses", {
      params: { path: { id: boardId } },
    });
    if (res.error !== undefined) {
      setLoadError("The board's statuses did not load.");
      setRows([]);
    } else {
      setLoadError(null);
      setRows(res.data ?? []);
    }
    setStaged({});
    setSelected([]);
    setLoading(false);
  }, [boardId]);

  useEffect(() => {
    void load();
  }, [load]);

  /** The row as it stands: staged category if one is staged, else the server's. */
  const categoryOf = useCallback(
    (row: BoardStatus): TicketStatusCategory => staged[row.id] ?? row.category,
    [staged],
  );

  const stagedCount = useMemo(
    () =>
      rows.filter((row) => staged[row.id] !== undefined && staged[row.id] !== row.category).length,
    [rows, staged],
  );

  const stage = (ids: string[], category: TicketStatusCategory): void => {
    setSaved(false);
    setStaged((prev) => {
      const next = { ...prev };
      for (const id of ids) next[id] = category;
      return next;
    });
  };

  const save = async (): Promise<void> => {
    setSaving(true);
    setSaveError(null);
    const changed = rows.filter((row) => staged[row.id] !== undefined);
    try {
      for (const row of changed) {
        const category = staged[row.id];
        if (category === undefined || category === row.category) continue;
        const res = await api.api.PATCH("/api/v1/boards/{id}/statuses/{status_id}", {
          params: { path: { id: boardId, status_id: row.id } },
          body: { category },
        });
        if (res.error !== undefined) {
          throw new Error(resultErrorText(res, "The board refused that change."));
        }
      }
      await load();
      onChanged();
      setSaved(true);
    } catch (error) {
      setSaveError(error instanceof Error ? error.message : "The change was refused.");
    } finally {
      setSaving(false);
    }
  };

  /* --- the drawer --------------------------------------------------------- */

  const openNew = (): void => {
    setDraft(EMPTY_DRAFT);
    setFieldError(null);
    setDrawerFailed(null);
    setDrawer({ mode: "new", id: null });
  };

  const openEdit = (row: BoardStatus): void => {
    setDraft({
      name: row.name,
      kind: categoryOf(row),
      pause: categoryOf(row) === "waiting",
      isEntry: row.is_entry,
      wipLimit: row.wip_limit === null || row.wip_limit === undefined ? "" : String(row.wip_limit),
    });
    setFieldError(null);
    setDrawerFailed(null);
    setDrawer({ mode: "edit", id: row.id });
  };

  const submitDrawer = async (): Promise<void> => {
    const name = draft.name.trim();
    // Validation happens on SUBMIT, never on keystroke, and the primary is
    // never disabled to hint at it.
    if (name === "") {
      setFieldError("Give the status a name.");
      return;
    }
    const clash = rows.some(
      (row) => row.id !== drawer?.id && row.name.trim().toLowerCase() === name.toLowerCase(),
    );
    if (clash) {
      setFieldError(`A status called ${name} already exists.`);
      return;
    }
    setFieldError(null);
    const category: TicketStatusCategory = draft.pause && draft.kind !== "closed" ? "waiting" : draft.kind;
    const wip =
      draft.wipLimit.trim() === ""
        ? null
        : Number.parseInt(draft.wipLimit.trim(), 10);
    if (wip !== null && (!Number.isFinite(wip) || wip < 1 || wip > 1000)) {
      setFieldError("A work-in-progress cap is a whole number between 1 and 1000.");
      return;
    }
    try {
      if (drawer?.mode === "edit" && drawer.id !== null) {
        const res = await api.api.PATCH("/api/v1/boards/{id}/statuses/{status_id}", {
          params: { path: { id: boardId, status_id: drawer.id } },
          body: {
            name,
            category,
            is_entry: draft.isEntry,
            wip_limit: wip === null ? "Clear" : { Set: wip },
          },
        });
        if (res.error !== undefined) {
          throw new Error(resultErrorText(res, "The board refused that status."));
        }
      } else {
        const res = await api.api.POST("/api/v1/boards/{id}/statuses", {
          params: { path: { id: boardId } },
          body: {
            name,
            category,
            is_entry: draft.isEntry,
            sort_order: rows.length,
            ...(wip === null ? {} : { wip_limit: wip }),
          },
        });
        if (res.error !== undefined) {
          throw new Error(resultErrorText(res, "The board refused that status."));
        }
      }
      setDrawer(null);
      await load();
      onChanged();
    } catch (error) {
      // Nothing is lost: the drawer keeps the form and offers the retry.
      setDrawerFailed(
        error instanceof Error ? error.message : "The settings service did not answer.",
      );
    }
  };

  const confirmDelete = async (): Promise<void> => {
    if (confirm === null) return;
    setConfirmBusy(true);
    try {
      const res = await api.api.DELETE("/api/v1/boards/{id}/statuses/{status_id}", {
        params: { path: { id: boardId, status_id: confirm.id } },
      });
      if (res.error !== undefined) {
        throw new Error(
          resultErrorText(
            res,
            "The board still holds tickets on that status, so it stayed.",
          ),
        );
      }
      setConfirm(null);
      await load();
      onChanged();
    } catch (error) {
      setSaveError(
        error instanceof Error
          ? error.message
          : "The board still holds tickets on that status, so it stayed.",
      );
      setConfirm(null);
    } finally {
      setConfirmBusy(false);
    }
  };

  /* --- the import drawer --------------------------------------------------- */

  const [importRows, setImportRows] = useState<ImportRow[]>([]);
  const [importName, setImportName] = useState("");
  const [importBusy, setImportBusy] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);

  const parseCsv = (text: string, fileName: string): void => {
    const lines = text
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line.length > 0);
    if (lines.length === 0) {
      setImportRows([]);
      setImportName(fileName);
      setImportError("That file has no rows in it.");
      return;
    }
    const seen = new Set(rows.map((row) => row.name.trim().toLowerCase()));
    const parsed: ImportRow[] = lines.map((line, index) => {
      const [name = "", kindRaw = ""] = line.split(",").map((part) => part.trim());
      const label = KIND_OPTIONS.find(
        (option) => KIND_LABEL[option].toLowerCase() === kindRaw.toLowerCase(),
      );
      let ok = true;
      let reason: string | undefined;
      if (name === "") {
        ok = false;
        reason = "This row has no name.";
      } else if (label === undefined) {
        ok = false;
        reason = `A kind of ${kindRaw === "" ? "blank" : kindRaw} must be Open, In progress, Waiting or Done.`;
      } else if (seen.has(name.toLowerCase())) {
        ok = false;
        reason = `${name} already exists.`;
      }
      if (ok) seen.add(name.toLowerCase());
      return { key: `${index}-${name}`, name, kind: kindRaw, ok, reason };
    });
    setImportRows(parsed);
    setImportName(fileName);
    setImportError(null);
  };

  const runImport = async (): Promise<void> => {
    setImportBusy(true);
    const valid = importRows.filter((row) => row.ok);
    let created = 0;
    const failures: string[] = [];
    for (const row of valid) {
      const kind = KIND_OPTIONS.find(
        (option) => KIND_LABEL[option].toLowerCase() === row.kind.toLowerCase(),
      );
      if (kind === undefined) continue;
      const res = await api.api.POST("/api/v1/boards/{id}/statuses", {
        params: { path: { id: boardId } },
        body: {
          name: row.name,
          category: kind,
          is_entry: false,
          sort_order: rows.length + created,
        },
      });
      if (res.error !== undefined) failures.push(row.name);
      else created += 1;
    }
    setImportBusy(false);
    if (failures.length > 0) {
      setImportError(
        `The board refused ${failures.length} of the rows: ${failures.join(", ")}. They are still listed above.`,
      );
    }
    if (created > 0) {
      setImporting(false);
      setImportRows([]);
      setImportName("");
      await load();
      onChanged();
    }
  };

  /* --- render -------------------------------------------------------------- */

  const validImport = importRows.filter((row) => row.ok).length;
  const skippedImport = importRows.length - validImport;

  return (
    <section
      data-testid="settings-statuses"
      style={{
        border: "1px solid var(--m-border)",
        borderRadius: 16,
        background: "var(--m-surface)",
        overflow: "hidden",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "14px 16px 10px",
        }}
      >
        <h2 style={{ margin: 0, fontSize: 15, fontWeight: 600, color: "var(--m-text)" }}>
          Statuses
        </h2>
        <span style={{ flex: 1 }} />
        {canEdit ? (
          <>
            <Button
              variant="secondary"
              onClick={() => {
                setImporting(true);
                setImportRows([]);
                setImportName("");
                setImportError(null);
              }}
              data-testid="statuses-import"
            >
              Import
            </Button>
            <Button
              variant="primary"
              onClick={openNew}
              data-testid="statuses-new"
              style={{ display: "inline-flex", alignItems: "center", gap: 6 }}
            >
              <PlusIcon />
              New status
            </Button>
          </>
        ) : null}
      </div>

      {loadError !== null ? (
        <p role="alert" style={{ padding: "0 16px 12px", fontSize: 13, color: "var(--m-status-due-fg)" }}>
          {loadError}
        </p>
      ) : null}

      {loading ? (
        <p style={{ padding: "0 16px 16px", fontSize: 13, color: "var(--m-text-secondary)" }}>
          Loading the board's statuses…
        </p>
      ) : rows.length === 0 ? (
        <div data-testid="statuses-empty">
          <EmptyState
            message="No statuses yet. The board needs at least one Open status that new tickets can land in."
            action={
              canEdit ? (
                <Button variant="primary" onClick={openNew}>
                  New status
                </Button>
              ) : undefined
            }
          />
        </div>
      ) : (
        <div
          role="table"
          aria-label="Status list"
          style={{ borderTop: "1px solid var(--m-border)" }}
        >
          {selected.length > 0 ? (
            <BulkActionBar
              count={selected.length}
              onClear={() => setSelected([])}
              testId="statuses-bulk-bar"
            >
              <Button
                variant="secondary"
                disabled={!canEdit}
                onClick={() => {
                  stage(selected, "waiting");
                  setSelected([]);
                }}
                style={{
                  height: 32,
                  fontSize: 13,
                  background: "var(--m-surface)",
                  color: "var(--m-text)",
                }}
              >
                Turn on Pauses SLA
              </Button>
              <Button
                variant="secondary"
                disabled={!canEdit}
                onClick={() => {
                  stage(selected, "open");
                  setSelected([]);
                }}
                style={{
                  height: 32,
                  fontSize: 13,
                  background: "var(--m-surface)",
                  color: "var(--m-text)",
                }}
              >
                Move to Open
              </Button>
              {canEdit ? (
                <BulkDestructiveAction
                  testId="statuses-bulk-delete"
                  onClick={() => {
                    const first = rows.find((row) => row.id === selected[0]);
                    setSelected([]);
                    if (first !== undefined) setConfirm(first);
                  }}
                >
                  Delete
                </BulkDestructiveAction>
              ) : null}
            </BulkActionBar>
          ) : (
            <div
              role="row"
              style={{
                display: "grid",
                gridTemplateColumns: COLUMNS,
                alignItems: "center",
                minHeight: 40,
                background: "var(--m-canvas)",
                borderBottom: "1px solid var(--m-border)",
              }}
            >
              <span style={{ paddingLeft: 12 }}>
                <input
                  type="checkbox"
                  aria-label="Select every status"
                  checked={rows.length > 0 && selected.length === rows.length}
                  onChange={(event) =>
                    setSelected(event.target.checked ? rows.map((row) => row.id) : [])
                  }
                  style={{ width: 18, height: 18, accentColor: "var(--m-primary)" }}
                />
              </span>
              <span />
              {COLUMN_HEADERS.map((header) => (
                <span
                  key={header}
                  style={{
                    fontSize: 12,
                    letterSpacing: "0.06em",
                    textTransform: "uppercase",
                    fontWeight: 600,
                    color: "var(--m-text-secondary)",
                    paddingLeft: 8,
                  }}
                >
                  {header}
                </span>
              ))}
            </div>
          )}

          {rows.map((row) => {
            const category = categoryOf(row);
            const isSelected = selected.includes(row.id);
            return (
              <div
                key={row.id}
                role="row"
                data-testid={`status-row-${row.name.replace(/\s+/g, "-").toLowerCase()}`}
                style={{
                  display: "grid",
                  gridTemplateColumns: COLUMNS,
                  alignItems: "center",
                  minHeight: 56,
                  borderBottom: "1px solid var(--m-border)",
                  background: isSelected ? "var(--m-accent)" : "var(--m-surface)",
                  boxShadow: isSelected ? "inset 3px 0 0 0 var(--m-primary)" : undefined,
                }}
              >
                <span style={{ paddingLeft: 12 }}>
                  <input
                    type="checkbox"
                    aria-label={`Select ${row.name}`}
                    checked={isSelected}
                    onChange={(event) =>
                      setSelected((prev) =>
                        event.target.checked ? [...prev, row.id] : prev.filter((id) => id !== row.id),
                      )
                    }
                    style={{ width: 18, height: 18, accentColor: "var(--m-primary)" }}
                  />
                </span>
                <span aria-hidden="true" style={{ color: "var(--m-text-secondary)", display: "flex" }}>
                  <GripIcon />
                </span>
                <span
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 8,
                    fontWeight: 600,
                    color: "var(--m-text)",
                    paddingLeft: 8,
                    minWidth: 0,
                  }}
                >
                  <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {row.name}
                  </span>
                  {row.is_entry ? (
                    <span
                      title="New tickets can land in this status"
                      style={{
                        fontSize: 12,
                        color: "var(--m-text-secondary)",
                        background: "var(--m-surface-2)",
                        borderRadius: 999,
                        padding: "0 8px",
                        height: 20,
                        display: "inline-flex",
                        alignItems: "center",
                      }}
                    >
                      entry
                    </span>
                  ) : null}
                </span>
                <span style={{ paddingLeft: 8 }}>
                  <KindPill category={category} />
                </span>
                <span style={{ paddingLeft: 8, display: "flex" }}>
                  <ToggleSwitch
                    on={category === "waiting"}
                    disabled={!canEdit || category === "closed"}
                    label={`Pauses SLA for ${row.name}`}
                    testId={`status-pause-${row.id}`}
                    onChange={(next) => stage([row.id], next ? "waiting" : "open")}
                  />
                </span>
                <span style={{ paddingLeft: 8, display: "flex" }}>
                  <ToggleSwitch
                    on={false}
                    disabled
                    label={`Visible to clients: ${row.name}`}
                    testId={`status-client-visible-${row.id}`}
                  />
                </span>
                <span style={{ display: "flex", justifyContent: "flex-end" }}>
                  {canEdit ? (
                    <button
                      type="button"
                      aria-label={`More for ${row.name}`}
                      onClick={() => openEdit(row)}
                      style={{
                        width: 36,
                        height: 36,
                        borderRadius: 10,
                        border: "1px solid var(--m-border)",
                        background: "var(--m-surface)",
                        color: "var(--m-text)",
                        cursor: "pointer",
                      }}
                    >
                      <MoreIcon />
                    </button>
                  ) : null}
                </span>
              </div>
            );
          })}
        </div>
      )}

      {rows.length > 0 ? (
        <p
          style={{
            margin: 0,
            padding: "10px 16px 12px",
            fontSize: 13,
            color: "var(--m-text-secondary)",
          }}
        >
          Toggles apply when you save. Visible to client is not a per-status setting yet: the portal
          shows a status through its own mapping, so the switch is drawn and disabled rather than
          pretending to work.
        </p>
      ) : null}

      {canEdit && rows.length > 0 ? (
        <div style={{ padding: "0 16px 16px" }}>
          <StagedSaveBar
            count={stagedCount}
            saving={saving}
            onDiscard={() => setStaged({})}
            onSave={() => void save()}
          />
          {saveError !== null ? (
            <p
              role="alert"
              data-testid="statuses-save-error"
              style={{ margin: "10px 0 0", fontSize: 13, color: "var(--m-status-due-fg)" }}
            >
              {saveError}
            </p>
          ) : null}
          {saved && saveError === null ? (
            <p
              role="status"
              style={{ margin: "10px 0 0", fontSize: 13, color: "var(--m-status-success-fg)" }}
            >
              Saved. The statuses on this board changed for everyone.
            </p>
          ) : null}
        </div>
      ) : null}

      {/* --- the New / Edit status drawer --- */}
      {drawer !== null ? (
        <SettingsDrawer
          title={drawer.mode === "edit" ? "Edit status" : "New status"}
          description="Statuses appear in every ticket picker and on every board."
          onClose={() => {
            setDrawer(null);
            setDrawerFailed(null);
          }}
          footer={
            <DrawerFooter
              onCancel={() => {
                setDrawer(null);
                setDrawerFailed(null);
              }}
            >
              <Button variant="primary" onClick={() => void submitDrawer()}>
                {drawerFailed !== null
                  ? "Try again"
                  : drawer.mode === "edit"
                    ? "Save status"
                    : "Save status"}
              </Button>
            </DrawerFooter>
          }
          testId="status-drawer"
        >
          {drawerFailed !== null ? (
            <SaveFailedBanner testId="status-drawer-failed">
              <b>Couldn&apos;t save.</b> {drawerFailed} Your changes are still here.
            </SaveFailedBanner>
          ) : null}
          <FormField
            label="Name"
            htmlFor="status-name"
            hint="Up to 64 characters."
            error={fieldError}
            testId="status-name-field"
          >
            <FieldInput
              id="status-name"
              value={draft.name}
              invalid={fieldError !== null}
              placeholder="For example, Awaiting approval"
              testId="status-name-input"
              onChange={(next) => setDraft((prev) => ({ ...prev, name: next }))}
            />
          </FormField>
          <FormField
            label="Kind"
            hint="Kind decides how the ticket counts, on boards and in reports."
          >
            <SegmentedChoice
              label="Kind"
              testId="status-kind"
              value={draft.kind}
              options={KIND_OPTIONS}
              onChange={(next) =>
                setDraft((prev) => ({
                  ...prev,
                  kind: next,
                  pause: next === "closed" ? false : prev.pause,
                }))
              }
            />
          </FormField>
          <ToggleRow
            first
            title="Pauses the SLA clock"
            description={
              draft.kind === "closed"
                ? "A done ticket has no clock."
                : "Time here does not count against the target."
            }
          >
            <ToggleSwitch
              on={draft.pause && draft.kind !== "closed"}
              disabled={draft.kind === "closed"}
              label="Pauses the SLA clock"
              testId="status-drawer-pause"
              onChange={(next) => setDraft((prev) => ({ ...prev, pause: next }))}
            />
          </ToggleRow>
          <ToggleRow
            title="Tickets can enter this status"
            description="New tickets may be created directly in this status."
          >
            <ToggleSwitch
              on={draft.isEntry}
              label="Tickets can enter this status"
              testId="status-drawer-entry"
              onChange={(next) => setDraft((prev) => ({ ...prev, isEntry: next }))}
            />
          </ToggleRow>
          <FormField
            label="Work in progress cap"
            htmlFor="status-wip"
            hint="How many tickets may sit here at once. Leave it empty for no cap."
          >
            <FieldInput
              id="status-wip"
              value={draft.wipLimit}
              placeholder="No cap"
              testId="status-wip-input"
              onChange={(next) => setDraft((prev) => ({ ...prev, wipLimit: next }))}
            />
          </FormField>
          <Note>
            Anything a client can see, or that leaves the building, waits for a person.
          </Note>
        </SettingsDrawer>
      ) : null}

      {/* --- the delete confirm --- */}
      <ConfirmRemap
        open={confirm !== null}
        title={confirm === null ? "" : `Delete ${confirm.name}?`}
        footerNote="History keeps the old name."
        confirmLabel={confirm === null ? "Delete" : `Delete ${confirm.name}`}
        busy={confirmBusy}
        testId="status-delete-confirm"
        onCancel={() => setConfirm(null)}
        onConfirm={() => void confirmDelete()}
      >
        <p style={{ margin: 0, fontSize: 13, lineHeight: 1.5, color: "var(--m-text-secondary)" }}>
          Tickets that still sit in this status keep it until they are moved, so the board refuses
          the delete and names what is holding it. Nothing about the tickets or their history
          changes either way.
        </p>
      </ConfirmRemap>

      {/* --- the import drawer --- */}
      {importing ? (
        <SettingsDrawer
          title="Import statuses"
          description="A CSV with a name and a kind on each row."
          onClose={() => setImporting(false)}
          footer={
            <DrawerFooter onCancel={() => setImporting(false)}>
              {importRows.length > 0 ? (
                <ImportPrimary
                  validCount={validImport}
                  skipCount={skippedImport}
                  onConfirm={() => void runImport()}
                  testId="status-import-primary"
                />
              ) : (
                <Button variant="primary" disabled>
                  Import
                </Button>
              )}
            </DrawerFooter>
          }
          testId="status-import-drawer"
        >
          {importRows.length === 0 ? (
            <ImportDropZone
              onChoose={() => fileInput.current?.click()}
              onDownloadTemplate={() =>
                setImportError(
                  "A template row is a name and a kind, for example: Awaiting approval,Waiting",
                )
              }
            />
          ) : (
            <ImportReview
              fileName={importName}
              rowCount={importRows.length}
              rows={importRows}
              onChooseAnother={() => {
                setImportRows([]);
                setImportName("");
              }}
              skipNote={
                skippedImport === 0 ? (
                  <>
                    Every row in the file can be imported.
                  </>
                ) : (
                  <>
                    {skippedImport} row{skippedImport === 1 ? "" : "s"} will be skipped:{" "}
                    {importRows
                      .filter((row) => !row.ok)
                      .map((row, index) => (
                        <span key={row.key}>
                          {index > 0 ? ", and " : ""}
                          <b>{row.name}</b> {row.reason}
                        </span>
                      ))}
                  </>
                )
              }
            />
          )}
          {importError !== null ? (
            <p role="alert" style={{ margin: 0, fontSize: 13, color: "var(--m-status-due-fg)" }}>
              {importError}
            </p>
          ) : null}
          <input
            ref={fileInput}
            type="file"
            accept=".csv,text/csv"
            aria-label="Choose a CSV"
            data-testid="status-import-file"
            style={{ display: "none" }}
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file === undefined) return;
              const reader = new FileReader();
              reader.onload = () => parseCsv(String(reader.result ?? ""), file.name);
              reader.readAsText(file);
            }}
          />
        </SettingsDrawer>
      ) : null}
    </section>
  );
}

const COLUMNS = "44px 28px minmax(180px, 1.4fr) 120px 120px 150px 56px";

const COLUMN_HEADERS = ["Status", "Kind", "Pauses SLA", "Visible to client"];

function PlusIcon(): React.JSX.Element {
  return (
    <svg
      aria-hidden="true"
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
    >
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}

function GripIcon(): React.JSX.Element {
  return (
    <svg
      aria-hidden="true"
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="currentColor"
      stroke="none"
    >
      <circle cx="9" cy="6" r="1.6" />
      <circle cx="15" cy="6" r="1.6" />
      <circle cx="9" cy="12" r="1.6" />
      <circle cx="15" cy="12" r="1.6" />
      <circle cx="9" cy="18" r="1.6" />
      <circle cx="15" cy="18" r="1.6" />
    </svg>
  );
}

function MoreIcon(): React.JSX.Element {
  return (
    <svg
      aria-hidden="true"
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="currentColor"
      stroke="none"
    >
      <circle cx="5" cy="12" r="1.7" />
      <circle cx="12" cy="12" r="1.7" />
      <circle cx="19" cy="12" r="1.7" />
    </svg>
  );
}
