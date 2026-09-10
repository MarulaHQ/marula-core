// Client-scoped asset list (Wave 11 lane 5, migration 0054): the full
// replacement for the flat asset table. Typed taxonomy rides
// `document_layouts` (asset-shaped system layouts are the type set);
// creates ride the instant-first envelope; rows link to the full asset
// detail page at /assets/[id].
"use client";

import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useAuth } from "../../../lib/api";
import { newIdempotencyKey } from "../../../lib/instant/instant-write";

type DeviceAsset = components["schemas"]["DeviceAsset"];
type DocumentLayout = components["schemas"]["DocumentLayout"];
type LayoutField = components["schemas"]["LayoutField"];

function fieldInputType(f: LayoutField): string {
  switch (f.type) {
    case "number":
      return "number";
    case "date":
      return "date";
    case "email":
      return "email";
    case "url":
      return "url";
    case "bool":
      return "checkbox";
    default:
      return "text";
  }
}

/** Renders one layout-schema custom-field input. */
function CustomFieldInput({
  field,
  value,
  onChange,
}: {
  field: LayoutField;
  value: string;
  onChange: (next: string) => void;
}) {
  const id = `asset-field-${field.key}`;
  if (field.type === "bool") {
    return (
      <div className="flex items-center gap-2">
        <input
          id={id}
          type="checkbox"
          checked={value === "true"}
          onChange={(e) => onChange(e.target.checked ? "true" : "false")}
          className="accent-[var(--m-accent)]"
        />
        <Label htmlFor={id} className="mb-0">
          {field.label}
        </Label>
      </div>
    );
  }
  if (field.type === "select" && (field.options ?? []).length > 0) {
    return (
      <div>
        <Label htmlFor={id}>{field.label}</Label>
        <Select value={value} onValueChange={onChange}>
          <SelectTrigger id={id} className="w-56">
            <SelectValue placeholder={field.label} />
          </SelectTrigger>
          <SelectContent>
            {(field.options ?? []).map((o) => (
              <SelectItem key={o} value={o}>
                {o}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
    );
  }
  return (
    <div>
      <Label htmlFor={id}>{field.label}</Label>
      <Input
        id={id}
        type={fieldInputType(field)}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={field.required ? "Required" : "Optional"}
      />
    </div>
  );
}

export default function AssetsTab({ clientId }: { clientId: string }) {
  const { me } = useAuth();
  const canCreate = me !== null;

  const [assets, setAssets] = useState<DeviceAsset[] | null>(null);
  const [layouts, setLayouts] = useState<DocumentLayout[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // Filters: free-text over tag/name/fields, and the typed type filter.
  const [q, setQ] = useState("");
  const [typeFilter, setTypeFilter] = useState<string>("all");

  // Create dialog state.
  const [createOpen, setCreateOpen] = useState(false);
  const [tag, setTag] = useState("");
  const [name, setName] = useState("");
  const [layoutId, setLayoutId] = useState<string>("");
  const [fields, setFields] = useState<Record<string, string>>({});

  const refresh = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/assets", {
      params: { query: { client_id: clientId } },
    });
    if (error !== undefined || data === undefined) {
      setLoadError("Assets are not available to your role.");
      setAssets([]);
      return;
    }
    setLoadError(null);
    setAssets(data);
  }, [clientId]);

  const refreshLayouts = useCallback(async () => {
    const { data } = await api.api.GET("/api/v1/documents/layouts");
    setLayouts(data ?? []);
  }, []);

  useEffect(() => {
    void refresh();
    void refreshLayouts();
  }, [refresh, refreshLayouts]);

  const layoutById = useMemo(() => {
    const map = new Map<string, DocumentLayout>();
    for (const l of layouts ?? []) map.set(l.id, l);
    return map;
  }, [layouts]);

  const filtered = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return (assets ?? []).filter((a) => {
      if (typeFilter !== "all" && a.layout_id !== typeFilter) return false;
      if (needle === "") return true;
      const haystack = [
        a.tag,
        a.name,
        a.kind,
        JSON.stringify(a.fields ?? {}),
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(needle);
    });
  }, [assets, q, typeFilter, layoutById]);

  const selectedLayout = layoutId !== "" ? layoutById.get(layoutId) : undefined;

  const resetForm = () => {
    setTag("");
    setName("");
    setLayoutId("");
    setFields({});
  };

  const runCreate = async () => {
    if (selectedLayout === undefined) return;
    setSaving(true);
    setFormError(null);
    // Build the fields object with light client coercion: numbers and
    // booleans as their JSON types; empty optionals omitted. The server
    // still validates the full schema — this only keeps honest inputs.
    const fieldValues: Record<string, unknown> = {};
    for (const f of selectedLayout.fields) {
      const raw = fields[f.key];
      if (raw === undefined || raw === "") continue;
      if (f.type === "number") {
        const n = Number(raw);
        if (Number.isNaN(n)) {
          setFormError(`Field "${f.label}" must be a number.`);
          setSaving(false);
          return;
        }
        fieldValues[f.key] = n;
      } else if (f.type === "bool") {
        fieldValues[f.key] = raw === "true";
      } else {
        fieldValues[f.key] = raw;
      }
    }
    const { data, error } = await api.api.POST("/api/v1/assets", {
      body: {
        client_id: clientId,
        tag: tag.trim(),
        name: name.trim(),
        layout_id: layoutId,
        fields: fieldValues,
        idempotency_key: newIdempotencyKey(),
      },
    });
    setSaving(false);
    if (error !== undefined || data === undefined) {
      setFormError("The server rejected this asset. Check the fields and retry.");
      return;
    }
    setCreateOpen(false);
    resetForm();
    void refresh();
  };

  if (loadError !== null) {
    return (
      <Card>
        <p role="alert" className="p-4 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {loadError}
        </p>
      </Card>
    );
  }

  return (
    <Card>
      <div
        className="flex flex-wrap items-center justify-between gap-3 p-4"
        style={{ borderBottom: "1px solid var(--m-border)" }}
      >
        <h2 className="text-sm font-semibold">Assets</h2>
        <div className="flex flex-wrap items-center gap-2">
          <div className="w-64">
            <Input
              aria-label="Search assets"
              placeholder="Search tag, name, or fields…"
              value={q}
              onChange={(e) => setQ(e.target.value)}
            />
          </div>
          <div className="w-48">
            <Select value={typeFilter} onValueChange={setTypeFilter}>
              <SelectTrigger aria-label="Filter by type" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All types</SelectItem>
                {(layouts ?? []).map((l) => (
                  <SelectItem key={l.id} value={l.id}>
                    {l.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <Dialog
            open={createOpen}
            onOpenChange={(open) => {
              setCreateOpen(open);
              if (!open) {
                setFormError(null);
                resetForm();
              }
            }}
          >
            <DialogTrigger asChild>
              <Button size="sm" disabled={!canCreate}>
                Add asset
              </Button>
            </DialogTrigger>
            <DialogContent
              title="Add asset"
              description="Pick the asset type; its fields follow the type's schema. Types are managed under Documents → layouts."
            >
              <div className="grid gap-3 p-4">
                <div>
                  <Label htmlFor="asset-tag">Asset tag</Label>
                  <Input
                    id="asset-tag"
                    value={tag}
                    onChange={(e) => setTag(e.target.value)}
                    placeholder="EXCH-01"
                  />
                </div>
                <div>
                  <Label htmlFor="asset-name">Name</Label>
                  <Input
                    id="asset-name"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder="Exchange server"
                  />
                </div>
                <div>
                  <Label htmlFor="asset-type">Type</Label>
                  <Select value={layoutId} onValueChange={setLayoutId}>
                    <SelectTrigger id="asset-type" className="w-64">
                      <SelectValue placeholder="Choose a type…" />
                    </SelectTrigger>
                    <SelectContent>
                      {(layouts ?? []).map((l) => (
                        <SelectItem key={l.id} value={l.id}>
                          {l.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <FieldHint>
                    Types are org-managed layouts. Editing them lives with Documents → layouts.
                  </FieldHint>
                </div>
                {selectedLayout !== undefined && selectedLayout.fields.length > 0 ? (
                  <div
                    className="grid gap-3 rounded-md p-3"
                    style={{ background: "var(--m-bg-raised)" }}
                  >
                    <p className="text-[12px] font-medium" style={{ color: "var(--m-text-secondary)" }}>
                      {selectedLayout.name} fields
                    </p>
                    {selectedLayout.fields.map((f) => (
                      <CustomFieldInput
                        key={f.key}
                        field={f}
                        value={fields[f.key] ?? ""}
                        onChange={(next) => setFields((prev) => ({ ...prev, [f.key]: next }))}
                      />
                    ))}
                  </div>
                ) : null}
                {formError !== null ? (
                  <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                    {formError}
                  </p>
                ) : null}
                <DialogActions>
                  <Button variant="ghost" onClick={() => setCreateOpen(false)}>
                    Cancel
                  </Button>
                  <Button
                    onClick={() => void runCreate()}
                    disabled={saving || tag.trim() === "" || name.trim() === "" || layoutId === ""}
                  >
                    {saving ? <Spinner /> : null} Save
                  </Button>
                </DialogActions>
              </div>
            </DialogContent>
          </Dialog>
        </div>
      </div>
      {assets === null ? (
        <div className="p-4">
          <Spinner />
        </div>
      ) : assets.length === 0 ? (
        <div className="p-4">
          <EmptyState message="No assets yet. Track the devices and infrastructure you manage for this client — pick a type and its fields follow." />
        </div>
      ) : filtered.length === 0 ? (
        <div className="p-4">
          <EmptyState message="No assets match this search or type filter. Clear the filters to see everything on this client." />
        </div>
      ) : (
        <Table>
          <thead>
            <tr>
              <Th>Tag</Th>
              <Th>Name</Th>
              <Th>Type</Th>
              <Th>Status</Th>
            </tr>
          </thead>
          <tbody>
            {filtered.map((asset) => (
              <Tr key={asset.id}>
                <Td className="tabular-nums">
                  <Link
                    href={`/assets/${asset.id}`}
                    className="underline decoration-[var(--m-border)] underline-offset-2 hover:decoration-[var(--m-text)]"
                  >
                    {asset.tag}
                  </Link>
                </Td>
                <Td>{asset.name}</Td>
                <Td>{asset.layout_id !== null && asset.layout_id !== undefined ? (layoutById.get(asset.layout_id)?.name ?? "—") : "—"}</Td>
                <Td>
                  <Tag>{asset.status}</Tag>
                </Td>
              </Tr>
            ))}
          </tbody>
        </Table>
      )}
    </Card>
  );
}
