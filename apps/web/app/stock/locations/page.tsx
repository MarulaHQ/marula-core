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
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { rejectionText } from "../../../lib/error-copy";
import { LabelPrintButton } from "../../../components/labels/label-print-button";

type StockLocationView = components["schemas"]["StockLocationView"];

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
    const locationsRes = await api.api.GET("/api/v1/stock-locations", { params: { query: {} } });
    if (locationsRes.data === undefined) {
      setLoadError("Could not load stock locations — try reloading.");
      return;
    }
    setLocations(locationsRes.data);
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

  if (locations === null && loadError === null) return <Spinner />;
  if (loadError !== null) return <EmptyState message={loadError} />;

  const list = locations ?? [];
  const parentOptions = list.filter((location) => location.id !== editing?.id);

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
    <div className="mx-auto grid w-full max-w-350 gap-6 p-4 md:p-6">
      <header className="m-rise flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Stock locations
          </h1>
          <p className="mt-0.5 text-sm" style={{ color: "var(--m-text-secondary)" }}>
            Where stock lives. A van or truck can carry an assigned user.
          </p>
        </div>
        <Button onClick={openCreate} data-testid="stock-locations-new">
          New location
        </Button>
      </header>

      {directoryDenied && (
        <p className="text-xs" style={{ color: "var(--m-text-muted)" }} data-testid="stock-locations-directory-note">
          The staff directory needs a manager role. The assigned-user picker lists you only.
        </p>
      )}
      {actionError !== null && (
        <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="stock-locations-error">
          {actionError}
        </p>
      )}
      {notice !== null && (
        <p className="text-xs" role="status" style={{ color: "var(--m-status-success-fg)" }} data-testid="stock-locations-notice">
          {notice}
        </p>
      )}

      <div className="grid gap-3 sm:hidden" data-testid="stock-locations-cards">
        {list.map((location, index) => (
          <Card key={location.id} className="m-rise grid gap-2 p-4" style={{ animationDelay: `${Math.min(index, 6) * 40}ms` }} data-testid={`stock-location-card-${location.id}`}>
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-medium" style={{ color: "var(--m-text)" }}>
                {location.name}
              </span>
              <Tag tone="outline">{location.kind}</Tag>
              {!location.active && <StatusTag tone="neutral">archived</StatusTag>}
            </div>
            <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
              {location.code ?? "no code"} · parent {nameOf(location.parent_id)} · assigned {userLabel(location.assigned_user_id)}
            </p>
            {cardActions(location)}
          </Card>
        ))}
        {list.length === 0 && <EmptyState message="No locations yet. Create the first warehouse." />}
      </div>

      <Card className="m-rise hidden overflow-x-auto p-0 sm:block">
        <Table data-testid="stock-locations-table">
          <thead>
            <tr>
              <Th>Name</Th>
              <Th>Code</Th>
              <Th>Kind</Th>
              <Th>Parent</Th>
              <Th>Assigned user</Th>
              <Th>State</Th>
              <Th className="text-right">Actions</Th>
            </tr>
          </thead>
          <tbody>
            {list.map((location) => (
              <Tr key={location.id} className="row-hover" data-testid={`stock-location-row-${location.id}`}>
                <Td>
                  <span className="font-medium" style={{ color: "var(--m-text)" }}>
                    {location.name}
                  </span>
                </Td>
                <Td className="font-mono text-xs">{location.code ?? "—"}</Td>
                <Td>
                  <Tag tone="outline">{location.kind}</Tag>
                </Td>
                <Td className="text-xs">{nameOf(location.parent_id)}</Td>
                <Td className="text-xs">{userLabel(location.assigned_user_id)}</Td>
                <Td>
                  <StatusTag tone={location.active ? "success" : "neutral"}>
                    {location.active ? "active" : "archived"}
                  </StatusTag>
                </Td>
                <Td>
                  <div className="flex justify-end">{cardActions(location)}</div>
                </Td>
              </Tr>
            ))}
            {list.length === 0 && (
              <Tr>
                <Td>
                  <EmptyState message="No locations yet. Create the first warehouse." />
                </Td>
              </Tr>
            )}
          </tbody>
        </Table>
      </Card>

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
    </div>
  );
}
