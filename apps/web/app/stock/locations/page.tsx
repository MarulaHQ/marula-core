// /stock/locations — the places stock lives (Wave 32 lane 4).
//
// Purpose: create and maintain warehouses, shelves, vans, and trucks. The
// assigned user is whose van/truck a location is; the directory read is
// manager-gated, so a 403 degrades the picker to the caller alone and the
// rest of the page keeps working.
//
// Non-purpose: no stock quantities (the board and item detail show those),
// no transfer flow (MovementDialog owns transfers), no site/address linking.
"use client";

import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  FieldError,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  StatusTag,
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { LedgerSurface, type LedgerColumn } from "../../../components/assets-stock/ledger-surface";
import { LedgerRow } from "../../../components/assets-stock/ledger-row";
import { csvFileName, downloadCsv } from "../../../components/assets-stock/csv";
import { NeutralMark, ReturnedMark } from "../../../components/assets-stock/ledger-marks";
import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { rejectionText } from "../../../lib/error-copy";
import { LabelPrintButton } from "../../../components/labels/label-print-button";
import type { FilterClause } from "@marula-core/ui";

const LOCATION_VIEWS = [
  { id: "locations", label: "Locations" },
  { id: "ledger", label: "Ledger" },
  { id: "tools", label: "Tools" },
];

const LOCATION_COLUMNS: LedgerColumn[] = [
  { key: "location", label: "Location", required: true },
  { key: "code", label: "Code" },
  { key: "kind", label: "Kind" },
  { key: "parent", label: "Parent" },
  { key: "assignee", label: "Assigned user" },
  { key: "items", label: "Items", align: "right", hiddenByDefault: true },
  { key: "state", label: "State" },
  { key: "actions", label: "Actions", align: "right" },
];

const LOCATION_FILTERS = [
  { key: "location", label: "Location" },
  { key: "code", label: "Code" },
  { key: "kind", label: "Kind" },
  { key: "state", label: "State" },
];

type StockLocationView = components["schemas"]["StockLocationView"];
type OnHandRow = components["schemas"]["OnHandRow"];

/**
 * Depth-first over the parent chain, so a shelf reads under its warehouse.
 * A location whose parent is absent (or that would close a cycle, which a
 * data bug can produce) is treated as a root: the row is still a place stock
 * lives, and hiding it would be worse than showing it at the wrong depth.
 */
function buildTree(locations: StockLocationView[]): { location: StockLocationView; depth: number }[] {
  const byId = new Map(locations.map((location) => [location.id, location]));
  const childrenOf = new Map<string | null, StockLocationView[]>();
  for (const location of locations) {
    const parent = byId.has(location.parent_id ?? "") ? (location.parent_id ?? null) : null;
    const bucket = childrenOf.get(parent) ?? [];
    bucket.push(location);
    childrenOf.set(parent, bucket);
  }
  for (const bucket of childrenOf.values()) {
    bucket.sort((a, b) => a.name.localeCompare(b.name));
  }
  const out: { location: StockLocationView; depth: number }[] = [];
  const seen = new Set<string>();
  const walk = (parent: string | null, depth: number): void => {
    for (const child of childrenOf.get(parent) ?? []) {
      if (seen.has(child.id)) continue;
      seen.add(child.id);
      out.push({ location: child, depth });
      walk(child.id, depth + 1);
    }
  };
  walk(null, 0);
  // Anything a cycle kept out of the walk still gets a row, at depth 0.
  for (const location of locations) {
    if (!seen.has(location.id)) out.push({ location, depth: 0 });
  }
  return out;
}

const KINDS = ["warehouse", "shelf", "van", "truck"] as const;

interface LocationForm {
  name: string;
  kind: string;
  code: string;
  parentId: string;
  assignedUserId: string;
}

const NONE = "__none";

function idPrefix(id: string): string {
  return `${id.slice(0, 8)}…`;
}

export default function StockLocationsPage(): ReactNode {
  const auth = useRequireAuth();
  const me = auth.me;
  const [locations, setLocations] = useState<StockLocationView[] | null>(null);
  const [onHand, setOnHand] = useState<OnHandRow[]>([]);
  const [clauses, setClauses] = useState<FilterClause[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const router = useRouter();
  const [directory, setDirectory] = useState<{ id: string; label: string }[]>([]);
  const [directoryDenied, setDirectoryDenied] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<StockLocationView | null>(null);
  const [form, setForm] = useState<LocationForm>({
    name: "",
    kind: "warehouse",
    code: "",
    parentId: "",
    assignedUserId: "",
  });
  const [formError, setFormError] = useState<string | null>(null);
  const [formBusy, setFormBusy] = useState(false);

  async function refresh(): Promise<void> {
    const [locationsRes, onHandRes] = await Promise.all([
      api.api.GET("/api/v1/stock-locations", { params: { query: {} } }),
      api.api.GET("/api/v1/stock/on-hand", { params: { query: { limit: 1000 } } }),
    ]);
    if (locationsRes.data === undefined) {
      setLoadError("Could not load stock locations — try reloading.");
      return;
    }
    setLocations(locationsRes.data);
    setOnHand(onHandRes.data ?? []);
    const usersRes = await api.api.GET("/api/v1/org/users", { params: { query: { limit: 200 } } });
    if (usersRes.data !== undefined) {
      setDirectory(usersRes.data.users.map((user) => ({ id: user.id, label: user.display_name })));
      setDirectoryDenied(false);
    } else {
      // The staff directory is manager+ only; the caller stays the one
      // assignable user, which is exactly the self-service case (my van).
      setDirectoryDenied(true);
      setDirectory(me === null ? [] : [{ id: me.user_id, label: me.display_name }]);
    }
  }

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
  }, [auth.status, me?.user_id]);

  const nameOf = useMemo(() => {
    const map = new Map((locations ?? []).map((location) => [location.id, location.name]));
    return (id: string | null | undefined): string => (id == null ? "—" : (map.get(id) ?? idPrefix(id)));
  }, [locations]);

  const userLabel = useMemo(() => {
    const map = new Map(directory.map((user) => [user.id, user.label]));
    return (id: string | null | undefined): string => (id == null ? "—" : (map.get(id) ?? idPrefix(id)));
  }, [directory]);

  function openCreate(): void {
    setEditing(null);
    setForm({
      name: "",
      kind: "warehouse",
      code: "",
      parentId: "",
      assignedUserId: me?.user_id ?? "",
    });
    setFormError(null);
    setDialogOpen(true);
  }

  function openEdit(location: StockLocationView): void {
    setEditing(location);
    setForm({
      name: location.name,
      kind: location.kind,
      code: location.code ?? "",
      parentId: location.parent_id ?? "",
      assignedUserId: location.assigned_user_id ?? "",
    });
    setFormError(null);
    setDialogOpen(true);
  }

  async function save(): Promise<void> {
    setFormError(null);
    const name = form.name.trim();
    if (name === "") {
      setFormError("Name is required.");
      return;
    }
    const code = form.code.trim() === "" ? null : form.code.trim();
    const parentId = form.parentId === "" ? null : form.parentId;
    const assignedUserId = form.assignedUserId === "" ? null : form.assignedUserId;
    setFormBusy(true);
    if (editing === null) {
      const tempId = `pending-${Date.now()}`;
      const result = await runEnvelopeMutation({
        describe: "Create stock location",
        buildBody: (base) => ({
          name,
          kind: form.kind,
          code,
          parent_id: parentId,
          assigned_user_id: assignedUserId,
          ...base,
        }),
        optimistic: () =>
          setLocations((cur) => [
            ...(cur ?? []),
            {
              id: tempId,
              org_id: "",
              name,
              kind: form.kind,
              code,
              parent_id: parentId,
              assigned_user_id: assignedUserId,
              active: true,
              created_at: new Date().toISOString(),
              updated_at: new Date().toISOString(),
            },
          ]),
        revert: () => setLocations((cur) => (cur ?? []).filter((location) => location.id !== tempId)),
        request: (client, body) =>
          client.POST("/api/v1/stock-locations", {
            body: body as components["schemas"]["CreateStockLocationRequest"],
          }),
        versions: (data) => [[data.id, data.mutation.version]],
      });
      setFormBusy(false);
      if (result.ok === false) {
        setFormError(rejectionText(result, "Could not create the location."));
        return;
      }
      setNotice(`${name} added.`);
    } else {
      const before = editing;
      const result = await runEnvelopeMutation({
        describe: "Update stock location",
        entityId: before.id,
        buildBody: (base) => ({
          name,
          kind: form.kind,
          code,
          assigned_user_id: assignedUserId,
          ...base,
        }),
        optimistic: () =>
          setLocations((cur) =>
            (cur ?? []).map((location) =>
              location.id === before.id
                ? {
                    ...location,
                    name,
                    kind: form.kind,
                    code,
                    assigned_user_id: assignedUserId,
                    updated_at: new Date().toISOString(),
                  }
                : location,
            ),
          ),
        revert: () => setLocations((cur) => (cur ?? []).map((location) => (location.id === before.id ? before : location))),
        request: (client, body) =>
          client.PATCH("/api/v1/stock-locations/{id}", {
            params: { path: { id: before.id } },
            body: body as components["schemas"]["UpdateStockLocationRequest"],
          }),
        versions: (data) => [[data.id, data.mutation.version]],
      });
      setFormBusy(false);
      if (result.ok === false) {
        setFormError(rejectionText(result, "Could not save the location."));
        return;
      }
      setNotice(`${name} saved.`);
    }
    setDialogOpen(false);
    await refresh();
  }

  async function setActive(location: StockLocationView, active: boolean): Promise<void> {
    setActionError(null);
    setBusyId(location.id);
    const result = await runEnvelopeMutation({
      describe: active ? "Reactivate stock location" : "Archive stock location",
      entityId: location.id,
      buildBody: (base) => ({ active, ...base }),
      optimistic: () =>
        setLocations((cur) => (cur ?? []).map((row) => (row.id === location.id ? { ...row, active } : row))),
      revert: () =>
        setLocations((cur) =>
          (cur ?? []).map((row) => (row.id === location.id ? { ...row, active: location.active } : row)),
        ),
      request: (client, body) =>
        client.PATCH("/api/v1/stock-locations/{id}", {
          params: { path: { id: location.id } },
          body: body as components["schemas"]["UpdateStockLocationRequest"],
        }),
      versions: (data) => [[data.id, data.mutation.version]],
    });
    setBusyId(null);
    if (result.ok === false) {
      setActionError(
        `Could not ${active ? "reactivate" : "archive"} ${location.name}: ${rejectionText(result, "refused.")}`,
      );
    }
  }

  if (loadError !== null) return <EmptyState message={loadError} />;

  const list = locations ?? [];
  const parentOptions = list.filter((location) => location.id !== editing?.id);

  // The location TREE: depth-first over the parent chain so a shelf reads
  // under its warehouse. A parent that is missing (or archived out from under
  // a child) is treated as a root rather than dropping the child, because a
  // hidden location is a data problem, not a reason to lose a row.
  const tree = useMemo(() => buildTree(list), [list]);

  // How many distinct items sit in each location, read from the on-hand rows
  // this page already loaded.
  const itemCounts = useMemo(() => {
    const counts = new Map<string, Set<string>>();
    for (const row of onHand) {
      const set = counts.get(row.stock_location_id) ?? new Set<string>();
      set.add(row.stock_item_id);
      counts.set(row.stock_location_id, set);
    }
    return new Map([...counts].map(([id, set]) => [id, set.size]));
  }, [onHand]);

  const rows = tree
    .filter(({ location }) => {
      for (const clause of clauses) {
        const value = (
          {
            location: location.name,
            code: location.code ?? "",
            kind: location.kind,
            state: location.active ? "active" : "archived",
          }[clause.field] ?? ""
        ).toLowerCase();
        const want = (clause.value ?? "").toLowerCase();
        if (clause.op === "is_set" && value === "") return false;
        if (clause.op === "is_not_set" && value !== "") return false;
        if (clause.op === "contains" && !value.includes(want)) return false;
        if (clause.op === "eq" && value !== want) return false;
        if (clause.op === "ne" && value === want) return false;
      }
      return true;
    })
    .map((entry) => ({ ...entry, itemCount: itemCounts.get(entry.location.id) ?? 0 }));

  // Every hook above runs on every render, so the loading guard belongs HERE,
  // after them — putting it first made the hook count change between the
  // loading render and the loaded one, which React rejects outright (#310).
  if (locations === null && loadError === null) return <Spinner />;

  function cardActions(location: StockLocationView): ReactNode {
    return (
      <div className="flex flex-wrap items-start gap-1">
        <LabelPrintButton purpose="stock_location" entityId={location.id} entityLabel={location.name} />
        <Button size="sm" variant="secondary" onClick={() => openEdit(location)} data-testid={`stock-location-edit-${location.id}`}>
          Edit
        </Button>
        {location.active ? (
          <Button
            size="sm"
            variant="ghost"
            disabled={busyId === location.id}
            onClick={() => void setActive(location, false)}
            data-testid={`stock-location-archive-${location.id}`}
          >
            Archive
          </Button>
        ) : (
          <Button
            size="sm"
            variant="ghost"
            disabled={busyId === location.id}
            onClick={() => void setActive(location, true)}
            data-testid={`stock-location-reactivate-${location.id}`}
          >
            Reactivate
          </Button>
        )}
      </div>
    );
  }

  return (
    <>
    <LedgerSurface
      surface="stock-locations"
      testId="locations-ledger"
      title="Stock locations"
      subtitle="Where stock lives, as a tree. A van or truck can carry an assigned user."
      headerAction={
        <Button onClick={openCreate} data-testid="stock-locations-new">
          New location
        </Button>
      }
      views={LOCATION_VIEWS}
      activeView="locations"
      onViewChange={(id) => {
        if (id === "ledger") router.push("/stock");
        if (id === "tools") router.push("/stock/tools");
      }}
      columns={LOCATION_COLUMNS}
      filterFields={LOCATION_FILTERS}
      filterClauses={clauses}
      onFilterChange={setClauses}
      rowCount={rows.length}
      rowsShown={rows.length}
      onExport={(request) => {
        const stamp = new Date().toISOString().slice(0, 10);
        downloadCsv(
          csvFileName("stock-locations", stamp),
          request.columns,
          rows.map((entry) => ({
            location: entry.location.name,
            code: entry.location.code ?? "",
            kind: entry.location.kind,
            parent: entry.location.parent_id == null ? "" : nameOf(entry.location.parent_id),
            assignee: entry.location.assigned_user_id == null ? "" : userLabel(entry.location.assigned_user_id),
            state: entry.location.active ? "active" : "archived",
            items: String(entry.itemCount),
          })),
        );
      }}
      selectedIds={selected}
      onToggleRow={(id, isSelected) =>
        setSelected((cur) => (isSelected ? [...cur, id] : cur.filter((row) => row !== id)))
      }
      onToggleAll={(isSelected) => setSelected(isSelected ? rows.map((r) => r.location.id) : [])}
      allSelected={rows.length > 0 && selected.length === rows.length}
      someSelected={selected.length > 0 && selected.length < rows.length}
      bulkActions={[
        {
          key: "edit",
          label: "Edit",
          disabled: selected.length !== 1,
          title: "Edit the selected location",
          onClick: () => {
            const only = rows.find((entry) => entry.location.id === selected[0]);
            if (only != null) openEdit(only.location);
          },
        },
      ]}
      onClearSelection={() => setSelected([])}
      emptyMessage="No locations yet. Create the first warehouse."
      emptyAction={
        <Button variant="secondary" onClick={openCreate} data-testid="locations-empty-new">
          New location
        </Button>
      }
      errorMessage={actionError}
    >
      {(columns) =>
        rows.map(({ location, depth, itemCount }) => (
          <LedgerRow
            key={location.id}
            id={location.id}
            selected={selected.includes(location.id)}
            selectable
            onToggle={(rowId, isSelected) =>
              setSelected((cur) => (isSelected ? [...cur, rowId] : cur.filter((id) => id !== rowId)))
            }
            columns={columns}
            selectLabel={`Select ${location.name}`}
            testId={`stock-location-row-${location.id}`}
            cells={{
              location: (
                <span className="m-ledger-tree-row">
                  {"\u00a0".repeat(depth * 4)}
                  {depth > 0 ? <span className="m-ledger-tree-glyph">└</span> : null}
                  <span style={{ color: "var(--m-text)" }}>{location.name}</span>
                </span>
              ),
              code: <span className="text-xs">{location.code ?? "—"}</span>,
              kind: <Tag tone="outline">{location.kind}</Tag>,
              parent: <span>{nameOf(location.parent_id)}</span>,
              assignee: <span>{userLabel(location.assigned_user_id)}</span>,
              items: <span className="tabular-nums">{itemCount}</span>,
              state:
                location.active ? (
                  <ReturnedMark label="Active" testId={`stock-location-state-${location.id}`} />
                ) : (
                  <NeutralMark label="Archived" />
                ),
              actions: <span className="flex justify-end gap-1">{cardActions(location)}</span>,
            }}
          />
        ))
      }
    </LedgerSurface>


      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent title={editing === null ? "New stock location" : `Edit ${editing.name}`} data-testid="stock-location-dialog">
          <div className="grid gap-3 overflow-auto p-1" style={{ maxHeight: "80vh" }}>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
              <div className="grid gap-1">
                <Label htmlFor="location-name">Name</Label>
                <Input
                  id="location-name"
                  value={form.name}
                  onChange={(e) => setForm({ ...form, name: e.target.value })}
                  data-testid="stock-location-name"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="location-kind">Kind</Label>
                <Select value={form.kind} onValueChange={(value) => setForm({ ...form, kind: value })}>
                  <SelectTrigger id="location-kind" data-testid="stock-location-kind">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {KINDS.map((kind) => (
                      <SelectItem key={kind} value={kind}>
                        {kind}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-1">
                <Label htmlFor="location-code">Code</Label>
                <Input
                  id="location-code"
                  value={form.code}
                  onChange={(e) => setForm({ ...form, code: e.target.value })}
                  data-testid="stock-location-code"
                />
              </div>
            </div>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <div className="grid gap-1">
                <Label htmlFor="location-parent">Parent location (optional)</Label>
                <Select
                  value={form.parentId === "" ? NONE : form.parentId}
                  onValueChange={(value) => setForm({ ...form, parentId: value === NONE ? "" : value })}
                  disabled={editing !== null}
                >
                  <SelectTrigger id="location-parent" data-testid="stock-location-parent">
                    <SelectValue placeholder="No parent" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={NONE}>No parent</SelectItem>
                    {parentOptions.map((location) => (
                      <SelectItem key={location.id} value={location.id}>
                        {location.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {editing !== null && <FieldHint>The parent is set on create and cannot be re-pointed yet.</FieldHint>}
              </div>
              <div className="grid gap-1">
                <Label htmlFor="location-user">Assigned user (optional)</Label>
                <Select
                  value={form.assignedUserId === "" ? NONE : form.assignedUserId}
                  onValueChange={(value) => setForm({ ...form, assignedUserId: value === NONE ? "" : value })}
                >
                  <SelectTrigger id="location-user" data-testid="stock-location-user">
                    <SelectValue placeholder="Nobody in particular" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={NONE}>Nobody in particular</SelectItem>
                    {directory.map((user) => (
                      <SelectItem key={user.id} value={user.id}>
                        {user.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FieldHint>For a van or truck, this is whose it is.</FieldHint>
              </div>
            </div>
            {formError !== null && <FieldError data-testid="stock-location-error">{formError}</FieldError>}
            <DialogActions>
              <Button variant="secondary" onClick={() => setDialogOpen(false)}>
                Cancel
              </Button>
              <Button disabled={formBusy} onClick={() => void save()} data-testid="stock-location-save">
                {formBusy ? "Saving…" : editing === null ? "Create location" : "Save"}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
