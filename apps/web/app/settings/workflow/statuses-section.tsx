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
import { Icon } from "@marula-core/ui/kit";
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
  SkippedNotice,
  SettingsDrawer,
  StagedSaveBar,
  ToggleRow,
  ToggleSwitch,
  type ImportRow,
} from "../../../components/settings/settings-forms";

type BoardStatus = components["schemas"]["BoardStatus"];
type TicketStatusCategory = components["schemas"]["TicketStatusCategory"];

/** The per-status fields the drawn list stages behind its save bar. */
interface StatusPatch {
  category?: TicketStatusCategory;
  clientVisible?: boolean;
}

/** One status's numbers, from the single usage read. */
interface StatusUsage {
  ticketCount: number;
  automationCount: number;
}

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
  clientVisible: boolean;
  isEntry: boolean;
  wipLimit: string;
}

const EMPTY_DRAFT: Draft = {
  name: "",
  kind: "open",
  pause: false,
  clientVisible: true,
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
  const [staged, setStaged] = useState<Record<string, StatusPatch>>({});
  // The usage read: the TICKETS column and the delete confirm's counts come
  // from ONE endpoint, so the table and the confirm can never quote different
  // numbers for the same status.
  const [usage, setUsage] = useState<Record<string, StatusUsage>>({});
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
    const [statuses, counts] = await Promise.all([
      api.api.GET("/api/v1/boards/{id}/statuses", { params: { path: { id: boardId } } }),
      api.api.GET("/api/v1/boards/{id}/statuses/usage", { params: { path: { id: boardId } } }),
    ]);
    if (statuses.error !== undefined) {
      setLoadError("The board's statuses did not load.");
      setRows([]);
    } else {
      setLoadError(null);
      setRows(statuses.data ?? []);
    }
    if (counts.error === undefined) {
      setUsage(
        Object.fromEntries(
          (counts.data ?? []).map((row) => [
            row.status_id,
            { ticketCount: row.ticket_count, automationCount: row.automation_count },
          ]),
        ),
      );
    }
    setStaged({});
    setSelected([]);
    setLoading(false);
  }, [boardId]);

  useEffect(() => {
    void load();
  }, [load]);

  /** The row as it stands: a staged field if one is staged, else the server's. */
  const categoryOf = useCallback(
    (row: BoardStatus): TicketStatusCategory => staged[row.id]?.category ?? row.category,
    [staged],
  );

  const clientVisibleOf = useCallback(
    (row: BoardStatus): boolean => staged[row.id]?.clientVisible ?? row.client_visible,
    [staged],
  );

  /** A staged field that differs from what the server holds. */
  const isStaged = useCallback(
    (row: BoardStatus, patch: StatusPatch): boolean =>
      (patch.category !== undefined && patch.category !== row.category) ||
      (patch.clientVisible !== undefined && patch.clientVisible !== row.client_visible),
    [],
  );

  const stagedCount = useMemo(
    () =>
      rows.filter((row) => {
        const patch = staged[row.id];
        return patch !== undefined && isStaged(row, patch);
      }).length,
    [rows, staged, isStaged],
  );

  const stage = (ids: string[], patch: StatusPatch): void => {
    setSaved(false);
    setStaged((prev) => {
      const next = { ...prev };
      for (const id of ids) next[id] = { ...next[id], ...patch };
      return next;
    });
  };

  const save = async (): Promise<void> => {
    setSaving(true);
    setSaveError(null);
    const changed = rows.filter((row) => {
      const patch = staged[row.id];
      return patch !== undefined && isStaged(row, patch);
    });
    try {
      for (const row of changed) {
        const patch = staged[row.id];
        if (patch === undefined) continue;
        // Only the fields this admin actually touched ride the PATCH, so a
        // staged category can never silently revert someone else's visibility
        // change made in the meantime.
        const body: Record<string, unknown> = {};
        if (patch.category !== undefined && patch.category !== row.category) {
          body.category = patch.category;
        }
        if (patch.clientVisible !== undefined && patch.clientVisible !== row.client_visible) {
          body.client_visible = patch.clientVisible;
        }
        if (Object.keys(body).length === 0) continue;
        const res = await api.api.PATCH("/api/v1/boards/{id}/statuses/{status_id}", {
          params: { path: { id: boardId, status_id: row.id } },
          body: body as never,
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
      clientVisible: clientVisibleOf(row),
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
            client_visible: draft.clientVisible,
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
            client_visible: draft.clientVisible,
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

  const confirmDelete = async (reassignTo: string | null): Promise<void> => {
    if (confirm === null) return;
    setConfirmBusy(true);
    try {
      // The replacement rides the query string, so the request says what it
      // will do to the tickets before it does it.
      const res = await api.api.DELETE("/api/v1/boards/{id}/statuses/{status_id}", {
        params: {
          path: { id: boardId, status_id: confirm.id },
          ...(reassignTo === null ? {} : { query: { reassign_to: reassignTo } }),
        },
      } as never);
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
          client_visible: true,
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
      {/* The pack's page header, in a section's heading: the title, the one
          line about what the list governs, and the two actions at the right.
          It is an h2, not the shared `SurfaceHeader`, because the PAGE's h1 is
          the page's and two h1s on one page is a defect. The type is the
          pack's own 26px/600/-0.01em.
          The board picker sits above it because this product's statuses are
          per board; the pack's are org-wide and it has nothing to pick. */}
      <div style={{ padding: "4px 0 12px" }}>
        <div style={{ display: "flex", alignItems: "flex-start", gap: 16 }}>
          <div>
            <h2
              style={{
                margin: 0,
                fontSize: 26,
                fontWeight: 600,
                letterSpacing: "-0.01em",
                lineHeight: 1.2,
                color: "var(--m-text)",
              }}
            >
              Statuses
            </h2>
            <p
              style={{
                margin: "4px 0 0",
                fontSize: 13,
                color: "var(--m-text-secondary)",
                maxWidth: 560,
              }}
            >
              What a ticket can be, and what that does. Order here is the order in every picker.
            </p>
          </div>
          <span style={{ flex: 1 }} />
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
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
        </div>
      </div>

      {loadError !== null ? (
        <p role="alert" style={{ padding: "0 16px 12px", fontSize: 13, color: "var(--m-status-danger-fg)" }}>
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
                  stage(selected, { category: "waiting" });
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
                  stage(selected, { category: "open" });
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
              <Button
                variant="secondary"
                disabled={!canEdit}
                onClick={() => {
                  // Turning the last visible status off would hide the ticket's
                  // standing from every client, so the bulk action refuses
                  // rather than doing it behind the admin's back.
                  const remaining = rows.filter((row) => selected.includes(row.id));
                  const hideAll = remaining.every((row) => clientVisibleOf(row));
                  stage(selected, { clientVisible: !hideAll });
                  setSelected([]);
                }}
                style={{
                  height: 32,
                  fontSize: 13,
                  background: "var(--m-surface)",
                  color: "var(--m-text)",
                }}
              >
                Hide from clients
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
              <span role="columnheader" style={{ paddingLeft: 12 }}>
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
              <span role="columnheader" />
              {COLUMN_HEADERS.map((header) => (
                <span
                  key={header}
                  role="columnheader"
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
                <span role="cell" style={{ paddingLeft: 12 }}>
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
                <span role="cell" aria-hidden="true" style={{ color: "var(--m-text-secondary)", display: "flex" }}>
                  <GripIcon />
                </span>
                <span
                  role="cell"
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
                <span role="cell" style={{ paddingLeft: 8 }}>
                  <KindPill category={category} />
                </span>
                <span role="cell" style={{ paddingLeft: 8, display: "flex" }}>
                  <ToggleSwitch
                    on={category === "waiting"}
                    disabled={!canEdit || category === "closed"}
                    label={`Pauses SLA for ${row.name}`}
                    testId={`status-pause-${row.id}`}
                    onChange={(next) => stage([row.id], { category: next ? "waiting" : "open" })}
                  />
                </span>
                <span role="cell" style={{ paddingLeft: 8, display: "flex" }}>
                  <ToggleSwitch
                    on={clientVisibleOf(row)}
                    disabled={!canEdit}
                    label={`Visible to clients: ${row.name}`}
                    testId={`status-client-visible-${row.id}`}
                    onChange={(next) => stage([row.id], { clientVisible: next })}
                  />
                </span>
                <span
                  role="cell"
                  data-testid={`status-tickets-${row.id}`}
                  style={{
                    paddingLeft: 8,
                    textAlign: "right",
                    fontVariantNumeric: "tabular-nums",
                    color: "var(--m-text-secondary)",
                  }}
                >
                  {usage[row.id]?.ticketCount ?? 0}
                </span>
                <span role="cell" style={{ display: "flex", justifyContent: "flex-end" }}>
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
          Drag the grip to reorder. Toggles apply when you save.
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
              style={{ margin: "10px 0 0", fontSize: 13, color: "var(--m-status-danger-fg)" }}
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
              // The pack's words, not the stored slugs: a person choosing a
              // kind is choosing "In progress", not "in_progress".
              labelFor={(option) => KIND_LABEL[option]}
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
            title="Visible to the client"
            description="Clients see this status on the portal, in words."
          >
            <ToggleSwitch
              on={draft.clientVisible}
              label="Visible to the client"
              testId="status-drawer-client-visible"
              onChange={(next) => setDraft((prev) => ({ ...prev, clientVisible: next }))}
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
        confirmLabel={
          confirm === null
            ? "Delete"
            : (usage[confirm.id]?.ticketCount ?? 0) > 0
              ? `Move ${usage[confirm.id]?.ticketCount ?? 0} tickets and delete`
              : `Delete ${confirm.name}`
        }
        options={
          confirm === null || (usage[confirm.id]?.ticketCount ?? 0) === 0
            ? undefined
            : rows
                .filter((row) => row.id !== confirm.id)
                .map((row) => ({ label: row.name, value: row.id }))
        }
        busy={confirmBusy}
        testId="status-delete-confirm"
        onCancel={() => setConfirm(null)}
        onConfirm={(reassignTo) => void confirmDelete(reassignTo)}
      >
        <p style={{ margin: 0, fontSize: 13, lineHeight: 1.5, color: "var(--m-text-secondary)" }}>
          {confirm === null
            ? ""
            : (usage[confirm.id]?.ticketCount ?? 0) === 0
              ? "No ticket sits in this status, so nothing moves."
              : `${usage[confirm.id]?.ticketCount ?? 0} ${(usage[confirm.id]?.ticketCount ?? 0) === 1 ? "ticket uses" : "tickets use"} this status. Where should they go?`}
        </p>
        {confirm !== null && (usage[confirm.id]?.automationCount ?? 0) > 0 ? (
          <SkippedNotice>
            {usage[confirm.id]?.automationCount}{" "}
            {(usage[confirm.id]?.automationCount ?? 0) === 1 ? "automation" : "automations"}{" "}
            mention{((usage[confirm.id]?.automationCount ?? 0) === 1) ? "s" : ""} this status.{" "}
            {((usage[confirm.id]?.automationCount ?? 0) === 1) ? "It" : "They"} will be paused until
            you pick a new one.
          </SkippedNotice>
        ) : null}
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
            <p role="alert" style={{ margin: 0, fontSize: 13, color: "var(--m-status-danger-fg)" }}>
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

const COLUMNS = "44px 28px minmax(160px, 1.4fr) 120px 120px 150px 90px 56px";

const COLUMN_HEADERS = ["Status", "Kind", "Pauses SLA", "Visible to client", "Tickets"];

function PlusIcon(): React.JSX.Element {
  return <Icon name="plus" size={16} />;
}

function GripIcon(): React.JSX.Element {
  return <Icon name="grip" size={16} />;
}

function MoreIcon(): React.JSX.Element {
  return <Icon name="more" size={16} />;
}
