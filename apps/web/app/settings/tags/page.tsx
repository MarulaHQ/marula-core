// /settings/tags — the org-wide tags manager (Wave 11 lane 9 backend,
// migration 0056).
//
// Purpose: flat org tag CRUD — create, rename, recolor, delete — with the
// honest delete guard: the row carries `applied_count`, so deleting warns
// with exactly how many tickets carry the tag. Mutations ride the
// instant-first envelope (optimistic patch + authoritative reconcile) and
// are owner/admin actions (`special:manage_boards`); the list itself is
// readable by anyone with ticket view. Duplicate names are typed 409s,
// rendered inline.
// Non-purpose: no per-ticket application here (that lives on the ticket
// detail chips and the ticket-list filter).
"use client";

import { rejectionText } from "../../../lib/error-copy";
import {
  Badge,
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  Input,
  Label,
  Spinner,
  Table,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { SettingsHeader } from "../../../components/settings/settings-shell";
import { ErrorNote } from "../../../components/knowledge/ledger";

type TagView = components["schemas"]["TagView"];
type Tag = components["schemas"]["Tag"];

const MANAGE_BOARDS = "special:manage_boards";

/** Workbench-palette chip colors (hex, no off-palette hues). */
const TAG_COLORS: { hex: string; label: string }[] = [
  { hex: "#d92d20", label: "Red" },
  { hex: "#b54708", label: "Amber" },
  { hex: "#067647", label: "Green" },
  { hex: "#175cd3", label: "Blue" },
  { hex: "#0e7090", label: "Cyan" },
  { hex: "#6941c6", label: "Purple" },
  { hex: "#c11574", label: "Pink" },
];

function Chip({ tag }: { tag: Tag }) {
  const color = tag.color ?? "var(--m-text-muted)";
  return (
    <span
      className="inline-flex items-center gap-1 rounded-[var(--m-radius-container)] px-2 py-0.5 text-[11px] font-medium"
      style={{ border: `1px solid ${color}`, color, background: "transparent" }}
    >
      {tag.name}
    </span>
  );
}

export default function TagsSettingsPage() {
  const auth = useRequireAuth();
  const canManage = (auth.me?.permissions ?? []).includes(MANAGE_BOARDS) ?? false;

  const [tags, setTags] = useState<TagView[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Create / edit dialog state
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<Tag | null>(null);
  const [name, setName] = useState("");
  const [color, setColor] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [dialogError, setDialogError] = useState<string | null>(null);

  // Delete confirmation state
  const [deleting, setDeleting] = useState<TagView | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/tags");
    if (error !== undefined || data === undefined) {
      setLoadError("Failed to load tags.");
      return;
    }
    setLoadError(null);
    setTags(data);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const openCreate = () => {
    setEditing(null);
    setName("");
    setColor(null);
    setDialogError(null);
    setDialogOpen(true);
  };

  const openEdit = (tag: Tag) => {
    setEditing(tag);
    setName(tag.name);
    setColor(tag.color ?? null);
    setDialogError(null);
    setDialogOpen(true);
  };

  const save = async () => {
    setBusy(true);
    setDialogError(null);
    if (editing === null) {
      const result = await runEnvelopeMutation<
        components["schemas"]["CreateTagRequest"],
        components["schemas"]["TagMutationResponse"]
      >({
        describe: "Create tag",
        optimistic: () => undefined,
        revert: () => undefined,
        buildBody: (base) => ({ idempotency_key: base.idempotency_key, name: name.trim(), color }),
        request: (client, body) => client.POST("/api/v1/tags", { body }),
      });
      setBusy(false);
      if (!result.ok) {
        setDialogError(
          result.message?.includes("duplicate") || result.reason === "conflict"
            ? "A tag with that name already exists."
            : (rejectionText(result, "The tag was rejected.")),
        );
        return;
      }
    } else {
      const before = tags ?? [];
      const result = await runEnvelopeMutation<
        components["schemas"]["UpdateTagRequest"],
        components["schemas"]["TagMutationResponse"]
      >({
        describe: "Update tag",
        entityId: editing.id,
        optimistic: () =>
          setTags((prev) =>
            (prev ?? []).map((tv) =>
              tv.tag.id === editing.id ? { ...tv, tag: { ...tv.tag, name: name.trim(), color } } : tv,
            ),
          ),
        revert: () => setTags(before),
        buildBody: (base) => ({
          idempotency_key: base.idempotency_key,
          name: name.trim(),
          color: color ?? null,
        }),
        request: (client, body) =>
          client.PATCH("/api/v1/tags/{id}", { params: { path: { id: editing.id } }, body }),
        versions: (response) => [[response.tag.tag.id, response.mutation.version]],
      });
      setBusy(false);
      if (!result.ok) {
        setDialogError(
          result.message?.includes("duplicate")
            ? "A tag with that name already exists."
            : (rejectionText(result, "The change was rejected.")),
        );
        return;
      }
    }
    setDialogOpen(false);
    void refresh();
  };

  const doDelete = async () => {
    if (deleting === null) return;
    setDeleteBusy(true);
    setDeleteError(null);
    const before = tags ?? [];
    const result = await runEnvelopeMutation<
      components["schemas"]["MutationParams"],
      unknown
    >({
      describe: "Delete tag",
      entityId: deleting.tag.id,
      optimistic: () => setTags((prev) => (prev ?? []).filter((tv) => tv.tag.id !== deleting.tag.id)),
      revert: () => setTags(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key }),
      request: (client, body) =>
        client.DELETE("/api/v1/tags/{id}", {
          params: { path: { id: deleting.tag.id } },
        }) as Promise<{ data?: unknown; error?: unknown; response: Response }>,
    });
    setDeleteBusy(false);
    if (!result.ok) {
      setDeleteError(rejectionText(result, "The delete was rejected."));
      return;
    }
    setDeleting(null);
    void refresh();
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-4">
      <div className="flex items-start justify-between gap-4">
        <SettingsHeader
          title="Tags"
          description="Org-wide labels applied to tickets. The ticket list filters with AND semantics: a ticket must carry every selected tag."
        />
        {canManage ? (
          <Button onClick={openCreate} data-testid="create-tag">
            New tag
          </Button>
        ) : null}
      </div>

      {loadError !== null ? (
        <ErrorNote>{loadError}</ErrorNote>
      ) : tags === null ? (
        <Card>
          <div className="flex justify-center p-6">
            <Spinner />
          </div>
        </Card>
      ) : tags.length === 0 ? (
        <Card>
          <EmptyState message="No tags yet. Tags let the team label tickets (e.g. warranty, vip, needs-review) and filter the ticket list." />
        </Card>
      ) : (
        <Card className="overflow-x-auto">
          <Table>
            <thead>
              <tr>
                <Th>Tag</Th>
                <Th className="text-right">Applied to</Th>
                {canManage ? <Th className="text-right">Actions</Th> : null}
              </tr>
            </thead>
            <tbody>
              {tags.map((tv) => (
                <Tr key={tv.tag.id}>
                  <Td>
                    <Chip tag={tv.tag} />
                  </Td>
                  <Td className="text-right text-[13px] tabular-nums" data-testid={`tag-count-${tv.tag.name}`}>
                    {tv.applied_count} ticket{tv.applied_count === 1 ? "" : "s"}
                  </Td>
                  {canManage ? (
                    <Td className="text-right">
                      <div className="flex justify-end gap-2">
                        <Button variant="ghost" size="sm" onClick={() => openEdit(tv.tag)}>
                          Edit
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => {
                            setDeleting(tv);
                            setDeleteError(null);
                          }}
                        >
                          Delete
                        </Button>
                      </div>
                    </Td>
                  ) : null}
                </Tr>
              ))}
            </tbody>
          </Table>
          {!canManage ? (
            <p className="px-4 pb-3 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              Tag management is an owner/admin (boards) action.
            </p>
          ) : null}
        </Card>
      )}

      <Dialog open={dialogOpen} onOpenChange={(o) => { setDialogOpen(o); if (!o) setDialogError(null); }}>
        <DialogContent
          title={editing === null ? "New tag" : "Edit tag"}
          description={editing === null ? undefined : "Renaming updates the tag everywhere it is applied."}
        >
          <div className="grid gap-3 p-4">
            <div>
              <Label htmlFor="tag-name">Name</Label>
              <Input id="tag-name" value={name} onChange={(e) => setName(e.target.value)} />
            </div>
            <div>
              <Label>Color</Label>
              <div className="flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  aria-label="No color"
                  aria-pressed={color === null}
                  className="h-6 w-6 cursor-pointer rounded-[var(--m-radius-container)] border-2"
                  style={{ borderColor: color === null ? "var(--m-text)" : "var(--m-border)", background: "transparent" }}
                  onClick={() => setColor(null)}
                />
                {TAG_COLORS.map((c) => (
                  <button
                    key={c.hex}
                    type="button"
                    aria-label={c.label}
                    aria-pressed={color === c.hex}
                    className="h-6 w-6 cursor-pointer rounded-[var(--m-radius-container)] border-2"
                    style={{ background: c.hex, borderColor: color === c.hex ? "var(--m-text)" : "transparent" }}
                    onClick={() => setColor(c.hex)}
                  />
                ))}
                <Input
                  aria-label="Custom color"
                  className="w-24"
                  value={color ?? ""}
                  placeholder="#175cd3"
                  onChange={(e) => setColor(e.target.value === "" ? null : e.target.value)}
                />
              </div>
            </div>
            {dialogError !== null ? (
              <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {dialogError}
              </p>
            ) : null}
            <DialogActions>
              <Button variant="ghost" onClick={() => setDialogOpen(false)}>
                Cancel
              </Button>
              <Button variant="primary" onClick={() => void save()} disabled={busy || name.trim() === ""}>
                {busy ? <Spinner /> : null} {editing === null ? "Create" : "Save"}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={deleting !== null} onOpenChange={(o) => { if (!o) { setDeleting(null); setDeleteError(null); } }}>
        <DialogContent
          title="Delete tag"
          description="Deleting removes the tag from every ticket that carries it. This cannot be undone."
        >
          <div className="grid gap-3 p-4">
            <p className="text-[13px]">
              {deleting !== null ? (
                <>
                  “{deleting.tag.name}” is currently applied to{" "}
                  <strong>{deleting.applied_count}</strong> ticket{deleting.applied_count === 1 ? "" : "s"}.
                </>
              ) : null}
            </p>
            {deleteError !== null ? (
              <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {deleteError}
              </p>
            ) : null}
            <DialogActions>
              <Button variant="ghost" onClick={() => setDeleting(null)}>
                Cancel
              </Button>
              <Button onClick={() => void doDelete()} disabled={deleteBusy}>
                {deleteBusy ? <Spinner /> : null} Delete tag
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
