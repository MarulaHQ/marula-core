// /reports — the report surface as a data-surface template (Wave 41 lane 11).
//
// Purpose: builder, saved reports and schedules over the R6 API, drawn in
// the TL frame: a 26px title row, ONE toolbar row (view switch left, filter
// chips middle, Columns + Export right), a bordered 16px-radius table, and
// the 52px footer with shown-of-total and the density control. Columns,
// sort and filters persist per user under the surface key
// `reports.saved` via the appearance.data_surfaces document (the lane 2b
// pattern). Export says what it will export and respects the view.
// Validation trouble is the status danger pair (`--m-status-danger-*`), never
// the destructive red — destructive red is reserved for things that destroy.
// A failed save is still an error message, not a warning that something is
// coming due.
//
// Wave 46 lane 6 adds the "describe it" entry: a dialog that borrows the
// authoring panel, so a described report is reviewed and then saved through
// the definition routes that already exist. It is not a second writer and not
// a second builder. The saved table also prints each definition's stored card
// (window, refresh, size) — the card migration 0180 stopped losing.
//
// Non-purpose: no report-spec editing outside the builder, no bulk actions.
"use client";

export const dynamic = "force-dynamic";

import {
  Badge,
  Button,
  Card,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  ExportControl,
  FieldHint,
  FilterChips,
  FilterTrigger,
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
  type FilterClause,
  type FilterField,
} from "@marula-core/ui";
import { LocalTime } from "../../components/local-time";
import type { components } from "@marula-core/sdk";
import { Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { api, useRequireAuth } from "../../lib/api";
import { friendlyMutationText } from "../../lib/error-copy";
import { ReportDeliveryHistory } from "../../components/report-delivery-history";
import { ReportBuilder } from "../../components/report-builder";
import { ScheduleProposal } from "../../components/reports/schedule-proposal";
import { DescribeEntry } from "../../components/reports/describe-entry";
import { CardContractMeta } from "../../components/dashboards/card-contract";
import { MONTH_DAYS, WEEKDAYS, type ScheduleCadenceFields } from "../../lib/schedule-phrase";
import { VIEW_LABELS } from "../../lib/report-builder";
import { cardReadout, isManagerPermissions } from "../../lib/dashboards";
import { decodeSpecFromQuery, encodeSpecToQuery } from "../../lib/report-spec-codec";
import type { ReportDraft } from "../../lib/report-authoring";
import {
  downloadCsv,
  toCsv,
  useSurfaceTableColumns,
  type InsightColumn,
  sortRows,
  toggleSort,
} from "../../lib/insight-table";
import { useDataSurfacePrefs } from "../../lib/data-surface-prefs";
import { SurfaceTabs } from "../../components/insight/surface-tabs";
import { InsightState } from "../../components/insight/insight-state";
import "../../components/insight/insight-table.css";

type DeepLinkSeed =
  | { spec: components["schemas"]["ReportSpec"]; from?: string; to?: string; nonce: number }
  | null;

type ReportDefinitionView = components["schemas"]["ReportDefinitionView"];
type ReportScheduleView = components["schemas"]["ReportScheduleView"];
type ScheduleCadence = components["schemas"]["ScheduleCadence"];

type Tab = "builder" | "saved" | "schedules";

/** The zone this browser can infer, or null when it cannot say. */
function browserTimezone(): string | null {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || null;
  } catch {
    return null;
  }
}

const SURFACE_KEY = "reports.saved";

/** A spec view outside the shipped label set renders as its raw id, never blank. */
function viewLabel(view: string): string {
  const known = VIEW_LABELS as Record<string, string>;
  return known[view] ?? view;
}

export default function ReportsPage() {
  return (
    <Suspense fallback={null}>
      <ReportsInner />
    </Suspense>
  );
}

function ReportsInner() {
  const auth = useRequireAuth();
  const canPublish =
    auth.status === "authed" && isManagerPermissions(auth.me?.permissions ?? []);
  const prefs = useDataSurfacePrefs();

  const [tab, setTab] = useState<Tab>("builder");

  // Deep link: /reports?spec=<encoded> lands on the Builder tab with the
  // linked spec seeded and auto-run once (Wave 23 lane 1, items 2+3).
  const router = useRouter();
  const searchParams = useSearchParams();
  const [seedSpec, setSeedSpec] = useState<DeepLinkSeed>(null);
  const landedRef = useRef<string | null>(null);
  useEffect(() => {
    const raw = searchParams.get("spec");
    if (raw === null || landedRef.current === raw) return;
    const decoded = decodeSpecFromQuery(raw);
    if (decoded === null) return;
    landedRef.current = raw;
    setSeedSpec({ ...decoded, nonce: Date.now() });
    setTab("builder");
    router.replace("/reports", { unstable_noScroll: true } as never);
  }, [router, searchParams]);

  // Catalog (R6 builder field lists — served from the Rust catalog).
  const [catalog, setCatalog] = useState<components["schemas"]["CatalogView"][] | null>(null);
  const [catalogError, setCatalogError] = useState<string | null>(null);

  // Saved reports
  const [definitions, setDefinitions] = useState<ReportDefinitionView[] | null>(null);
  const [loadingDefs, setLoadingDefs] = useState(false);
  const [defsError, setDefsError] = useState<string | null>(null);
  const [editingDef, setEditingDef] = useState<ReportDefinitionView | null>(null);
  const [runToken, setRunToken] = useState<{ def: ReportDefinitionView; nonce: number } | null>(null);
  const [rowBusyId, setRowBusyId] = useState<string | null>(null);
  const [rowError, setRowError] = useState<{ id: string; text: string } | null>(null);
  /** The outcome of an accepted "describe it" draft, in the saved table's own words. */
  const [savedNote, setSavedNote] = useState<string | null>(null);

  // Saved-table state: view switch (all/mine), filters, columns.
  const [view, setView] = useState<"all" | "mine">("all");
  const [filterOpen, setFilterOpen] = useState(false);
  const [clauses, setClauses] = useState<FilterClause[]>([]);

  const FILTER_FIELDS: FilterField[] = useMemo(
    () => [
      { key: "name", label: "Name", operators: ["contains"] },
      { key: "data", label: "Data", operators: ["contains"] },
      { key: "visibility", label: "Visibility", operators: ["eq"] },
    ],
    [],
  );

  type DefRow = {
    id: string;
    name: string;
    description: string | null;
    view: string;
    scope: "org" | "self";
    updated: string;
    /** The stored card, or null. Read back from the definition, never inferred. */
    card: ReportDefinitionView["card"];
  };

  const defColumns: readonly InsightColumn<DefRow>[] = useMemo(
    () => [
      {
        key: "name",
        label: "Report",
        sortValue: (r) => r.name.toLowerCase(),
        render: (r) => (
          <div className="min-w-0">
            <div className="truncate text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
              {r.name}
            </div>
            {r.description !== null && r.description !== "" && (
              <div className="truncate text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                {r.description}
              </div>
            )}
          </div>
        ),
      },
      {
        key: "data",
        label: "Data",
        sortValue: (r) => r.view,
        render: (r) => (
          <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            {viewLabel(r.view)}
          </span>
        ),
      },
      {
        key: "runs",
        label: "Runs as",
        // Sorted by window so the table answers "which of these looks further
        // back" without a second control; a definition with no card sorts last.
        sortValue: (r) => r.card?.window_days ?? -1,
        render: (r) => (
          <CardContractMeta
            card={r.card}
            variant="cell"
            testId={`definition-card-meta-${r.id}`}
          />
        ),
      },
      {
        key: "visibility",
        label: "Visibility",
        sortValue: (r) => r.scope,
        render: (r) =>
          r.scope === "org" ? (
            <Badge tone="neutral">Org-wide</Badge>
          ) : (
            <Badge tone="neutral">Private</Badge>
          ),
      },
      {
        key: "updated",
        label: "Updated",
        sortValue: (r) => r.updated,
        render: (r) => (
          <span className="whitespace-nowrap text-[13px] tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
            <LocalTime value={r.updated} kind="date" />
          </span>
        ),
      },
      {
        key: "actions",
        label: "",
        render: (r) => (
          <div className="flex flex-wrap justify-end gap-1">
            {rowBusyId === r.id ? <Spinner /> : null}
            <Button
              size="sm"
              variant="secondary"
              onClick={() => {
                const def = definitions?.find((d) => d.id === r.id);
                if (def === undefined) return;
                setEditingDef(def);
                setRunToken({ def, nonce: Date.now() });
                setTab("builder");
              }}
            >
              Run
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                const def = definitions?.find((d) => d.id === r.id);
                if (def === undefined) return;
                setEditingDef(def);
                setTab("builder");
              }}
            >
              Edit
            </Button>
            {canPublish && r.scope === "self" ? (
              <Button size="sm" variant="ghost" onClick={() => void handleShare(r, "org")}>
                Publish
              </Button>
            ) : null}
            {canPublish && r.scope === "org" ? (
              <Button size="sm" variant="ghost" onClick={() => void handleShare(r, "self")}>
                Unpublish
              </Button>
            ) : null}
            <Button size="sm" variant="ghost" onClick={() => void handleDeleteDefinition(r.id)}>
              Delete
            </Button>
          </div>
        ),
      },
    ],
    // eslint-disable-next-line react-hooks/exhaustive-deps -- handleShare/handleDelete are stableEnough for render use
    [canPublish, rowBusyId, definitions],
  );

  const defsTable = useSurfaceTableColumns<DefRow>(SURFACE_KEY, prefs, defColumns);

  const clauseMatches = useCallback(
    (r: DefRow) =>
      clauses.every((c) => {
        const hay =
          c.field === "name"
            ? r.name
            : c.field === "data"
              ? viewLabel(r.view)
              : r.scope === "org"
                ? "org-wide"
                : "private";
        return c.op === "contains" ? hay.toLowerCase().includes((c.value ?? "").toLowerCase()) : hay.toLowerCase() === (c.value ?? "").toLowerCase();
      }),
    [clauses],
  );

  const defRows: DefRow[] = useMemo(
    () =>
      (definitions ?? []).map((d) => ({
        id: d.id,
        name: d.name,
        description: d.description ?? null,
        view: d.spec.view,
        scope: d.scope,
        updated: d.updated_at,
        card: d.card ?? null,
      })),
    [definitions],
  );

  const viewFiltered = useMemo(() => {
    const base = view === "mine" ? defRows.filter((r) => r.scope === "self") : defRows;
    return clauses.length === 0 ? base : base.filter(clauseMatches);
  }, [view, defRows, clauses, clauseMatches]);

  const defSort = defsTable.saved.sort;
  const sortedRows = useMemo(
    () => sortRows(viewFiltered, "defs", defColumns, defSort),
    [viewFiltered, defColumns, defSort],
  );

  // Schedules state
  const [schedules, setSchedules] = useState<ReportScheduleView[] | null>(null);
  const [loadingSchedules, setLoadingSchedules] = useState(false);
  const [schedulesError, setSchedulesError] = useState<string | null>(null);

  // Create schedule modal
  const [scheduleModalOpen, setScheduleModalOpen] = useState(false);
  const [schedName, setSchedName] = useState("");
  const [schedDefId, setSchedDefId] = useState("");
  const [schedCadence, setSchedCadence] = useState<ScheduleCadence>("daily");
  const [schedTime, setSchedTime] = useState("08:00");
  const [schedDayOfWeek, setSchedDayOfWeek] = useState(0);
  const [schedDayOfMonth, setSchedDayOfMonth] = useState(1);
  const [schedTimezone, setSchedTimezone] = useState<string | null>(() => browserTimezone());
  const [schedRecipients, setSchedRecipients] = useState<string[]>([]);
  const [orgUsers, setOrgUsers] = useState<{ id: string; display_name: string; email: string }[] | null>(null);
  const [savingSched, setSavingSched] = useState(false);
  const [schedFormError, setSchedFormError] = useState<string | null>(null);

  const refreshDefinitions = useCallback(async () => {
    setLoadingDefs(true);
    setDefsError(null);
    const { data, error } = await api.api.GET("/api/v1/report-definitions");
    setLoadingDefs(false);
    if (error !== undefined || !data) {
      setDefsError(friendlyMutationText(error, "Report definitions could not be loaded."));
      return;
    }
    setDefinitions(data.definitions);
  }, []);

  const refreshSchedules = useCallback(async () => {
    setLoadingSchedules(true);
    setSchedulesError(null);
    const { data, error } = await api.api.GET("/api/v1/report-schedules");
    setLoadingSchedules(false);
    if (error !== undefined || !data) {
      setSchedulesError(friendlyMutationText(error, "Report schedules could not be loaded."));
      return;
    }
    setSchedules(data.schedules);
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refreshDefinitions();
    void refreshSchedules();
    setCatalogError(null);
    void api.api
      .GET("/api/v1/report-catalog")
      .then(({ data, error }) => {
        if (error !== undefined || !data) {
          setCatalogError("The report field catalog could not be loaded.");
          return;
        }
        // A resolving read can carry a payload without `views`. Setting
        // `undefined` here would leave the builder's `catalog?.find` throwing
        // and take the whole surface down; `null` is its "no catalog" state.
        setCatalog(Array.isArray(data.views) ? data.views : null);
      })
      .catch(() => setCatalogError("The report field catalog could not be loaded."));
  }, [auth.status, refreshDefinitions, refreshSchedules]);

  const handleDeleteDefinition = useCallback(async (id: string) => {
    setRowBusyId(id);
    setRowError(null);
    const { error } = await api.api.DELETE("/api/v1/report-definitions/{id}", {
      params: { path: { id } },
    });
    setRowBusyId(null);
    if (error !== undefined) {
      setRowError({ id, text: friendlyMutationText(error, "The report could not be deleted.") });
      return;
    }
    if (editingDef?.id === id) setEditingDef(null);
    void refreshDefinitions();
  }, [editingDef?.id, refreshDefinitions]);

  /** Share = flip the definition's visibility scope (full-replace PUT). */
  const handleShare = useCallback(
    async (row: DefRow, scope: "org" | "self") => {
      const def = definitions?.find((d) => d.id === row.id);
      if (def === undefined) return;
      setRowBusyId(def.id);
      setRowError(null);
      const idempotencyKey =
        typeof crypto !== "undefined" && crypto.randomUUID ? crypto.randomUUID() : `share-${Date.now()}`;
      const { error } = await api.api.PUT("/api/v1/report-definitions/{id}", {
        params: { path: { id: def.id } },
        body: {
          name: def.name,
          description: def.description ?? null,
          spec: def.spec,
          // Full replace: omitting the card would clear it, and publishing is
          // not an edit to what the card displays. Migration 0180 only keeps
          // the card if every writer sends it back.
          card: def.card ?? null,
          scope,
          idempotency_key: idempotencyKey,
        },
      });
      setRowBusyId(null);
      if (error !== undefined) {
        setRowError({
          id: def.id,
          text: friendlyMutationText(
            error,
            scope === "org"
              ? "This report could not be published org-wide."
              : "This report could not be made private.",
          ),
        });
        return;
      }
      void refreshDefinitions();
    },
    [definitions, refreshDefinitions],
  );

  const handleSaveSchedule = async () => {
    if (!schedName.trim() || !schedDefId) return;
    setSavingSched(true);
    setSchedFormError(null);
    const idempotencyKey =
      typeof crypto !== "undefined" && crypto.randomUUID ? crypto.randomUUID() : `rsched-${Date.now()}`;
    const { error } = await api.api.POST("/api/v1/report-schedules", {
      body: {
        name: schedName.trim(),
        report_definition_id: schedDefId,
        cadence: schedCadence,
        time_of_day: schedTime,
        // Migration 0038: a weekly row carries day_of_week and nothing else in
        // the day columns; monthly is the mirror.
        day_of_week: schedCadence === "weekly" ? schedDayOfWeek : null,
        day_of_month: schedCadence === "monthly" ? schedDayOfMonth : null,
        timezone: schedTimezone ?? "",
        recipients: schedRecipients,
        idempotency_key: idempotencyKey,
      },
    });
    setSavingSched(false);
    if (error !== undefined) {
      setSchedFormError(friendlyMutationText(error, "The schedule could not be created."));
      return;
    }
    setScheduleModalOpen(false);
    setSchedName("");
    setSchedRecipients([]);
    void refreshSchedules();
  };

  /**
   * Recipient staff, fetched once per page. Both ways into the dialog call it:
   * the "New schedule" trigger through `onOpenChange`, and the proposal through
   * its own handler — a programmatic `open` flip does not fire Radix's callback,
   * so the recipients would otherwise never load on that path.
   */
  const loadOrgUsers = useCallback(() => {
    if (orgUsers !== null) return;
    void api.api
      .GET("/api/v1/org/users")
      .then(({ data }) => {
        if (data) {
          setOrgUsers(
            data.users
              .filter((u) => u.is_active)
              .map((u) => ({ id: u.id, display_name: u.display_name, email: u.email })),
          );
        }
      })
      .catch(() => setOrgUsers([]));
  }, [orgUsers]);

  /**
   * The proposal fills THIS form's cadence fields and opens it. The save still
   * rides the existing POST, so the typed 422 stays the only authority on what
   * the server will accept.
   */
  const handleUseProposal = useCallback(
    (fields: ScheduleCadenceFields) => {
      setSchedCadence(fields.cadence);
      setSchedTime(fields.time_of_day);
      if (fields.day_of_week !== null) setSchedDayOfWeek(fields.day_of_week);
      if (fields.day_of_month !== null) setSchedDayOfMonth(fields.day_of_month);
      setSchedTimezone(fields.timezone);
      setSchedFormError(null);
      loadOrgUsers();
      setScheduleModalOpen(true);
    },
    [loadOrgUsers],
  );

  const handleDeleteSchedule = useCallback(async (id: string) => {
    const { error } = await api.api.DELETE("/api/v1/report-schedules/{id}", {
      params: { path: { id } },
    });
    if (error !== undefined) {
      setSchedulesError(friendlyMutationText(error, "The schedule could not be deleted."));
      return;
    }
    void refreshSchedules();
  }, [refreshSchedules]);

  const handleExport = useCallback(
    (opts: { format: string }) => {
      const csv = toCsv(
        defsTable.keys.filter((k) => k !== "actions"),
        defColumns,
        sortedRows,
        (col, row) => {
          if (col.key === "data") return viewLabel(row.view);
          if (col.key === "runs") return cardReadout(row.card)?.line ?? "No card stored";
          if (col.key === "visibility") return row.scope === "org" ? "Org-wide" : "Private";
          if (col.key === "updated") return new Date(row.updated).toISOString().slice(0, 10);
          return row.name;
        },
      );
      downloadCsv(`marula-reports.${opts.format === "xlsx" ? "xlsx" : "csv"}`, csv);
    },
    [defsTable.keys, defColumns, sortedRows],
  );

  /**
   * An accepted draft is already a definition: `acceptReportDraft` wrote it
   * through the create/update routes. The page's only job is to read the list
   * again and say where the row is.
   */
  const handleAuthored = useCallback(
    (result: { id: string; created: boolean; draft: ReportDraft }) => {
      setSavedNote(
        result.created
          ? "Saved as a new private report. It is in the list below."
          : "The saved report was updated. Its row is in the list below.",
      );
      setTab("saved");
      void refreshDefinitions();
    },
    [refreshDefinitions],
  );

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-5 p-6">
      <SurfaceHeader
        className="m-rise"
        title="Reports"
        meta={
          <span
            className="inline-flex h-6 items-center rounded-full px-2.5 text-[12px] font-semibold"
            style={{ background: "var(--m-surface-2)", color: "var(--m-text-secondary)" }}
          >
            Build, run and schedule
          </span>
        }
        actions={
          <DescribeEntry
            surface="reports"
            testId="reports-describe-entry"
            onEdit={(draft) =>
              router.push(`/reports?spec=${encodeSpecToQuery(draft.spec)}`)
            }
            onAccepted={handleAuthored}
            onOpenReports={() => setTab("saved")}
          />
        }
      />

      {/* Sub-navigation sits under the header row: the pack draws the active
          one as a raised surface card. */}
      <div className="flex flex-wrap items-center gap-3">
        <SurfaceTabs
          tabs={[
            { id: "builder", label: "Builder" },
            { id: "saved", label: "Saved reports" },
            { id: "schedules", label: "Schedules" },
          ]}
          activeId={tab}
          onChange={(id) => setTab(id as Tab)}
          label="Report sections"
        />
      </div>

      {/* A rejected appearance.data_surfaces write reverts the user's
          columns and sort; it says so in blush, never red. */}
      {prefs.error !== null && (
        <div
          role="alert"
          className="flex items-center gap-2 rounded-[var(--m-radius-control)] px-3 py-2 text-[13px] font-medium"
          style={{ background: "var(--m-status-danger-bg)", color: "var(--m-status-danger-fg)" }}
          data-testid="prefs-save-error"
        >
          {prefs.error}
        </div>
      )}

      {tab === "builder" && (
        <div className="grid gap-4">
          {catalogError !== null ? (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {catalogError}
            </p>
          ) : null}
          <ReportBuilder
            catalog={catalog}
            editingDef={editingDef}
            canPublish={canPublish}
            runToken={runToken}
            seedSpec={seedSpec}
            onSaved={() => {
              setEditingDef(null);
              void refreshDefinitions();
            }}
            onCancelEdit={() => {
              setEditingDef(null);
              setRunToken(null);
            }}
          />
        </div>
      )}

      {tab === "saved" && (
        <div className="grid gap-3">
          {/* The one toolbar row: views left, filters middle, columns + export right. */}
          <DataToolbar
            label="Saved reports controls"
            views={
              <SurfaceTabs
                tabs={[
                  { id: "all", label: "All reports", hint: String(defRows.length) },
                  { id: "mine", label: "Private to me", hint: String(defRows.filter((r) => r.scope === "self").length) },
                ]}
                activeId={view}
                onChange={(id) => setView(id as "all" | "mine")}
                label="Saved report views"
              />
            }
            chips={
              <FilterChips
                fields={FILTER_FIELDS}
                clauses={clauses}
                onChange={setClauses}
                label="report filters"
                emptyHint=""
                onAdd={() => setFilterOpen(true)}
              />
            }
            filter={
              <FilterTrigger
                fields={FILTER_FIELDS}
                clauses={clauses}
                onChange={setClauses}
                label="report filters"
                open={filterOpen}
                onOpenChange={setFilterOpen}
              />
            }
            columns={
              <ColumnChooser
                columns={defsTable.chooser.filter((c) => c.key !== "actions")}
                onChange={(next) =>
                  defsTable.onChooser([
                    ...next,
                    defsTable.chooser.find((c) => c.key === "actions") ?? {
                      key: "actions",
                      label: "",
                      visible: true,
                    },
                  ])
                }
                label="Saved report columns"
              />
            }
            export={
              <ExportControl
                columns={defsTable.keys.filter((k) => k !== "actions")}
                rowCount={sortedRows.length}
                sort={defSort}
                onExport={(req) => handleExport({ format: req.format })}
                label="Export"
              />
            }
          />

          {savedNote !== null ? (
            <div
              role="status"
              className="flex flex-wrap items-center gap-2 rounded-[var(--m-radius-control)] border px-3 py-2 text-[13px]"
              style={{
                borderColor: "var(--m-border)",
                background: "var(--m-surface-2)",
                color: "var(--m-text-secondary)",
              }}
              data-testid="reports-saved-note"
            >
              <span>{savedNote}</span>
              <Button variant="ghost" size="sm" onClick={() => setSavedNote(null)}>
                Dismiss
              </Button>
            </div>
          ) : null}

          {defsError !== null && (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {defsError}
            </p>
          )}

          <div
            className="overflow-x-auto rounded-[16px] border"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
          >
            {loadingDefs ? (
              <div className="flex justify-center p-6">
                <Spinner />
              </div>
            ) : sortedRows.length === 0 ? (
              <InsightState
                title={
                  clauses.length > 0
                    ? "No saved reports match these filters"
                    : "No saved reports yet"
                }
                hint={
                  clauses.length > 0
                    ? "Try removing one, or widen the match."
                    : "Build one in the Builder tab, then save it here."
                }
                primary={
                  clauses.length > 0 ? (
                    <Button variant="primary" size="sm" onClick={() => setClauses([])}>
                      Clear filters
                    </Button>
                  ) : (
                    <Button variant="primary" size="sm" onClick={() => setTab("builder")}>
                      Open the builder
                    </Button>
                  )
                }
              />
            ) : (
              <Table className="m-insight-table">
                <thead>
                  <tr>
                    {defsTable.keys.map((key) => {
                      const col = defColumns.find((c) => c.key === key);
                      if (col === undefined) return null;
                      const active = defSort?.field === key;
                      return (
                        <Th
                          key={key}
                          columnKey={key}
                          sortable={false}
                          resizable={false}
                          aria-sort={active ? (defSort?.dir === "asc" ? "ascending" : "descending") : undefined}
                          className="cursor-pointer select-none"
                          onClick={
                            col.sortValue !== undefined
                              ? () => defsTable.save(toggleSort(defsTable.saved, key))
                              : undefined
                          }
                        >
                          <span className="inline-flex items-center gap-1">
                            {col.label}
                            {active ? <span aria-hidden="true">{defSort?.dir === "asc" ? "↑" : "↓"}</span> : null}
                          </span>
                        </Th>
                      );
                    })}
                  </tr>
                </thead>
                <tbody>
                  {sortedRows.map((r) => (
                    <Tr key={r.id}>
                      {defsTable.keys.map((key) => {
                        const col = defColumns.find((c) => c.key === key);
                        if (col === undefined) return null;
                        return <Td key={key}>{col.render(r)}</Td>;
                      })}
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
          </div>
          <div className="m-insight-footer flex min-h-[52px] flex-wrap items-center gap-3">
            <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
                {sortedRows.length}
              </b>{" "}
              of{" "}
              <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
                {defRows.length}
              </b>{" "}
              saved reports
            </span>
            <span className="flex-1" />
            <span className="flex items-center gap-2">
              {/* The switch carries its own visible label — adding one here
                  double-prints "Density". */}
              <DensitySwitch value={prefs.density} onChange={(d) => void prefs.setDensity(d)} />
            </span>
          </div>

          {rowError !== null ? (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {rowError.text}
            </p>
          ) : null}
        </div>
      )}

      {tab === "schedules" && (
        <div className="grid gap-4">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h2 className="text-[16px] font-semibold" style={{ color: "var(--m-text)" }}>
              Delivery schedules
            </h2>
            <Dialog
              open={scheduleModalOpen}
              onOpenChange={(open) => {
                setScheduleModalOpen(open);
                if (open) loadOrgUsers();
              }}
            >
              <DialogTrigger asChild>
                <Button>New schedule</Button>
              </DialogTrigger>
              <DialogContent title="Create delivery schedule">
                <div className="grid gap-3 p-4">
                  <div>
                    <Label htmlFor="sch-name">Schedule name</Label>
                    <Input
                      id="sch-name"
                      placeholder="e.g. Daily executive digest"
                      value={schedName}
                      onChange={(e) => setSchedName(e.target.value)}
                      required
                    />
                  </div>

                  <div>
                    <Label htmlFor="sch-def">Report</Label>
                    <Select value={schedDefId} onValueChange={setSchedDefId}>
                      <SelectTrigger id="sch-def" className="w-full">
                        <SelectValue placeholder="Pick a saved report" />
                      </SelectTrigger>
                      <SelectContent>
                        {(definitions ?? []).map((d) => (
                          <SelectItem key={d.id} value={d.id}>
                            {d.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <FieldHint>Scheduled delivery emails the report as of each run.</FieldHint>
                  </div>

                  <div className="grid grid-cols-2 gap-2">
                    <div>
                      <Label htmlFor="sch-cadence">Cadence</Label>
                      <Select value={schedCadence} onValueChange={(v) => setSchedCadence(v as ScheduleCadence)}>
                        <SelectTrigger id="sch-cadence" className="w-full">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="daily">Daily</SelectItem>
                          <SelectItem value="weekly">Weekly</SelectItem>
                          <SelectItem value="monthly">Monthly</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>

                    <div>
                      <Label htmlFor="sch-time">Time of day</Label>
                      <Input
                        id="sch-time"
                        value={schedTime}
                        onChange={(e) => setSchedTime(e.target.value)}
                        placeholder="08:00"
                      />
                      <FieldHint>Wall clock in the time zone below, not UTC.</FieldHint>
                    </div>
                  </div>

                  <div className="grid grid-cols-2 gap-2">
                    {schedCadence === "weekly" ? (
                      <div>
                        <Label htmlFor="sch-weekday">Weekday</Label>
                        <Select
                          value={String(schedDayOfWeek)}
                          onValueChange={(v) => setSchedDayOfWeek(Number(v))}
                        >
                          <SelectTrigger id="sch-weekday" className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {WEEKDAYS.map((day, index) => (
                              <SelectItem key={day} value={String(index)}>
                                {day}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                    ) : schedCadence === "monthly" ? (
                      <div>
                        <Label htmlFor="sch-monthday">Day of the month</Label>
                        <Select
                          value={String(schedDayOfMonth)}
                          onValueChange={(v) => setSchedDayOfMonth(Number(v))}
                        >
                          <SelectTrigger id="sch-monthday" className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {MONTH_DAYS.map((day) => (
                              <SelectItem key={day} value={String(day)}>
                                {day}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                        <FieldHint>1–28, so every month has the day.</FieldHint>
                      </div>
                    ) : (
                      <div />
                    )}

                    <div>
                      <Label htmlFor="sch-timezone">Time zone</Label>
                      <Input
                        id="sch-timezone"
                        value={schedTimezone ?? ""}
                        onChange={(e) => setSchedTimezone(e.target.value)}
                        placeholder="Europe/London"
                      />
                    </div>
                  </div>

                  <div>
                    <Label htmlFor="sch-recip">Recipients</Label>
                    <div
                      className="grid max-h-40 gap-1 overflow-y-auto border p-2"
                      style={{ borderColor: "var(--m-border)" }}
                    >
                      {orgUsers === null ? (
                        <Spinner />
                      ) : orgUsers.length === 0 ? (
                        <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                          No staff users found.
                        </span>
                      ) : (
                        orgUsers.map((u) => (
                          <label key={u.id} className="flex items-center gap-2 text-[13px]">
                            <input
                              type="checkbox"
                              checked={schedRecipients.includes(u.id)}
                              onChange={(e) =>
                                setSchedRecipients((prev) =>
                                  e.target.checked ? [...prev, u.id] : prev.filter((id) => id !== u.id),
                                )
                              }
                            />
                            {u.display_name} <span style={{ color: "var(--m-text-muted)" }}>{u.email}</span>
                          </label>
                        ))
                      )}
                    </div>
                    <FieldHint>Delivery goes to org staff — pick at least one recipient.</FieldHint>
                  </div>

                  {schedFormError !== null && (
                    <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                      {schedFormError}
                    </p>
                  )}

                  <DialogActions>
                    <Button variant="ghost" onClick={() => setScheduleModalOpen(false)}>
                      Cancel
                    </Button>
                    <Button
                      onClick={() => void handleSaveSchedule()}
                      disabled={
                        savingSched ||
                        !schedName.trim() ||
                        schedRecipients.length === 0 ||
                        (schedTimezone ?? "").trim() === "" ||
                        (schedCadence === "weekly" && schedDayOfWeek === null) ||
                        (schedCadence === "monthly" && schedDayOfMonth === null)
                      }
                    >
                      {savingSched ? <Spinner /> : null} Save schedule
                    </Button>
                  </DialogActions>
                </div>
              </DialogContent>
            </Dialog>
          </div>

          <ScheduleProposal defaultTimezone={schedTimezone} onUse={handleUseProposal} />

          {schedulesError !== null && (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {schedulesError}
            </p>
          )}

          <div
            className="overflow-x-auto rounded-[16px] border"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
          >
            {loadingSchedules ? (
              <div className="flex justify-center p-6">
                <Spinner />
              </div>
            ) : schedules === null || schedules.length === 0 ? (
              <InsightState
                title="No delivery schedules yet"
                hint="Save a report first, then schedule it here."
                primary={
                  <Button variant="primary" size="sm" onClick={() => setTab("saved")}>
                    Go to saved reports
                  </Button>
                }
              />
            ) : (
              <Table className="m-insight-table">
                <thead>
                  <tr>
                    <Th sortable={false} resizable={false}>Name</Th>
                    <Th sortable={false} resizable={false}>Report</Th>
                    <Th sortable={false} resizable={false}>Cadence</Th>
                    <Th sortable={false} resizable={false}>Next run</Th>
                    <Th sortable={false} resizable={false}>Last run</Th>
                    <Th sortable={false} resizable={false}>Recipients</Th>
                    <Th sortable={false} resizable={false} className="text-right">Actions</Th>
                  </tr>
                </thead>
                <tbody>
                  {schedules.map((s) => (
                    <Tr key={s.id}>
                      <Td className="text-[13px] font-medium">{s.name}</Td>
                      <Td className="text-[13px]">{s.definition_name}</Td>
                      <Td>
                        <Badge tone="neutral">
                          {s.cadence} @ {s.time_of_day}
                        </Badge>
                      </Td>
                      <Td className="whitespace-nowrap text-[13px] tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                        {s.next_run_at
                          ? <LocalTime value={s.next_run_at} kind="dayTime" />
                          : "—"}
                      </Td>
                      <Td className="whitespace-nowrap text-[13px] tabular-nums">
                        {s.last_error !== null && s.last_error !== undefined && s.last_error !== "" ? (
                          <span title={s.last_error} style={{ color: "var(--m-status-danger-fg)" }}>
                            Failed · needs attention
                          </span>
                        ) : s.last_run_at ? (
                          <span style={{ color: "var(--m-text-secondary)" }}>
                            <LocalTime value={s.last_run_at} kind="dayTime" />
                          </span>
                        ) : (
                          "—"
                        )}
                      </Td>
                      <Td className="text-[13px]">
                        {s.recipients.length} recipient{s.recipients.length === 1 ? "" : "s"}
                      </Td>
                      <Td className="text-right">
                        <div className="flex justify-end gap-1">
                          <ReportDeliveryHistory scheduleId={s.id} scheduleName={s.name} />
                          <Button variant="ghost" size="sm" onClick={() => void handleDeleteSchedule(s.id)}>
                            Delete
                          </Button>
                        </div>
                      </Td>
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
          </div>
          <div className="m-insight-footer flex min-h-[52px] flex-wrap items-center gap-3">
            <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              <b className="tabular-nums" style={{ color: "var(--m-text)" }}>
                {schedules?.length ?? 0}
              </b>{" "}
              schedules
            </span>
            <span className="flex-1" />
            <span className="flex items-center gap-2">
              {/* The switch carries its own visible label — adding one here
                  double-prints "Density". */}
              <DensitySwitch value={prefs.density} onChange={(d) => void prefs.setDensity(d)} />
            </span>
          </div>
        </div>
      )}
    </div>
  );
}

