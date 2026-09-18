// /sales/catalog — product/service catalog management (Wave 12 lane 2
// backends, migration 0060).
//
// Purpose: the org's sellable product/service catalog: filterable list,
// create + edit through the IF1–IF3 envelope (name, unit, price/cost in
// minor units, currency, category/group, taxability), and archive
// (inactive items stay queryable — doctrine #9 — and stop appearing as
// defaults in the quote builder).
//
// Non-purpose: no stock/inventory quantities here (catalog is pricing
// truth; inventory tracking lives with config items/assets).
"use client";

import { rejectionText } from "../../../lib/error-copy";
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
  Spinner,
  StatusTag,
  Tag,
  Table,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { money } from "../../../lib/format";

type CatalogItem = components["schemas"]["CatalogItemView"];

interface CatalogForm {
  name: string;
  unit: string;
  currency: string;
  unitPriceMajor: string;
  costMajor: string;
  category: string;
  groupName: string;
  sku: string;
  description: string;
  taxable: boolean;
}

const EMPTY_FORM: CatalogForm = {
  name: "",
  unit: "each",
  currency: "USD",
  unitPriceMajor: "",
  costMajor: "",
  category: "",
  groupName: "",
  sku: "",
  description: "",
  taxable: true,
};

function toMinor(major: string): number | undefined {
  if (major.trim() === "") return undefined;
  const v = Number(major);
  return Number.isNaN(v) ? undefined : Math.round(v * 100);
}

export default function CatalogPage(): ReactNode {
  const auth = useRequireAuth();
  const [items, setItems] = useState<CatalogItem[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  const [editOpen, setEditOpen] = useState(false);
  const [editing, setEditing] = useState<CatalogItem | null>(null);
  const [form, setForm] = useState<CatalogForm>(EMPTY_FORM);
  const [formError, setFormError] = useState<string | null>(null);
  const [formBusy, setFormBusy] = useState(false);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const { data, error } = await api.api.GET("/api/v1/catalog-items");
      if (data === undefined) {
        setLoadError("Could not load the catalog — try reloading.");
        return;
      }
      setItems(data ?? []);
    })();
  }, [auth.status]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (items ?? []).filter(
      (i) =>
        q === "" ||
        i.name.toLowerCase().includes(q) ||
        (i.sku ?? "").toLowerCase().includes(q) ||
        (i.category ?? "").toLowerCase().includes(q),
    );
  }, [items, query]);

  function openCreate(): void {
    setEditing(null);
    setForm(EMPTY_FORM);
    setFormError(null);
    setEditOpen(true);
  }

  function openEdit(item: CatalogItem): void {
    setEditing(item);
    setForm({
      name: item.name,
      unit: item.unit,
      currency: item.currency,
      unitPriceMajor: (item.unit_price_minor / 100).toFixed(2),
      costMajor: item.cost_minor == null || item.cost_minor === 0 ? "" : (item.cost_minor / 100).toFixed(2),
      category: item.category ?? "",
      groupName: item.group_name ?? "",
      sku: item.sku ?? "",
      description: item.description ?? "",
      taxable: item.taxable,
    });
    setFormError(null);
    setEditOpen(true);
  }

  async function save(): Promise<void> {
    setFormError(null);
    const name = form.name.trim();
    if (name === "") {
      setFormError("Name is required.");
      return;
    }
    const unitPriceMinor = toMinor(form.unitPriceMajor) ?? 0;
    const costMinor = toMinor(form.costMajor) ?? 0;
    if (editing === null) {
      const tempId = `pending-${Date.now()}`;
      const result = await runEnvelopeMutation({
        describe: "Create catalog item",
        buildBody: (base) => ({
          name,
          unit: form.unit.trim() || "each",
          currency: form.currency.trim().toUpperCase() || "USD",
          unit_price_minor: unitPriceMinor,
          cost_minor: costMinor,
          taxable: form.taxable,
          category: form.category.trim() === "" ? null : form.category,
          group_name: form.groupName.trim() === "" ? null : form.groupName,
          sku: form.sku.trim() === "" ? null : form.sku,
          description: form.description.trim() === "" ? null : form.description,
          ...base,
        }),
        optimistic: () => {
          setItems((cur) => [
            ...(cur ?? []),
            {
              id: tempId,
              org_id: "",
              name,
              unit: form.unit.trim() || "each",
              currency: form.currency.trim().toUpperCase() || "USD",
              unit_price_minor: unitPriceMinor,
              cost_minor: costMinor,
              taxable: form.taxable,
              active: true,
              category: form.category.trim() === "" ? null : form.category,
              group_name: form.groupName.trim() === "" ? null : form.groupName,
              sku: form.sku.trim() === "" ? null : form.sku,
              description: form.description.trim() === "" ? null : form.description,
              created_at: new Date().toISOString(),
              updated_at: new Date().toISOString(),
              created_by: null,
            },
          ]);
        },
        revert: () => {
          setItems((cur) => (cur ?? []).filter((i) => i.id !== tempId));
        },
        request: (client, body) =>
          client.POST("/api/v1/catalog-items", {
            body: body as components["schemas"]["CreateCatalogItemRequest"],
          }),
      });
      if (result.ok === false) {
        setFormError(rejectionText(result, "Could not create the catalog item."));
        return;
      }
    } else {
      const result = await runEnvelopeMutation({
        describe: "Update catalog item",
        entityId: editing.id,
        buildBody: (base) => ({
          name,
          unit: form.unit.trim() || "each",
          currency: form.currency.trim().toUpperCase() || "USD",
          unit_price_minor: unitPriceMinor,
          cost_minor: costMinor,
          taxable: form.taxable,
          category: form.category.trim() === "" ? null : form.category,
          group_name: form.groupName.trim() === "" ? null : form.groupName,
          sku: form.sku.trim() === "" ? null : form.sku,
          description: form.description.trim() === "" ? null : form.description,
          ...base,
        }),
        optimistic: () => {
          setItems((cur) =>
            (cur ?? []).map((i) =>
              i.id === editing.id
                ? {
                    ...i,
                    name,
                    unit: form.unit.trim() || "each",
                    currency: form.currency.trim().toUpperCase() || "USD",
                    unit_price_minor: unitPriceMinor,
                    cost_minor: costMinor,
                    taxable: form.taxable,
                    category: form.category.trim() === "" ? null : form.category,
                    group_name: form.groupName.trim() === "" ? null : form.groupName,
                    sku: form.sku.trim() === "" ? null : form.sku,
                    description: form.description.trim() === "" ? null : form.description,
                    updated_at: new Date().toISOString(),
                  }
                : i,
            ),
          );
        },
        revert: () => {
          setItems((cur) =>
            (cur ?? []).map((i) => (i.id === editing.id ? editing : i)),
          );
        },
        request: (client, body) =>
          client.PATCH("/api/v1/catalog-items/{id}", {
            params: { path: { id: editing.id } },
            body: body as components["schemas"]["UpdateCatalogItemRequest"],
          }),
      });
      if (result.ok === false) {
        setFormError(rejectionText(result, "Could not save the catalog item."));
        return;
      }
    }
    setEditOpen(false);
    void (async () => {
      const { data } = await api.api.GET("/api/v1/catalog-items");
      setItems(data ?? []);
    })();
  }

  async function archive(item: CatalogItem): Promise<void> {
    setActionError(null);
    setBusyId(item.id);
    const result = await runEnvelopeMutation({
      describe: "Archive catalog item",
      entityId: item.id,
      buildBody: (base) => ({ ...base }),
      optimistic: () => {
        setItems((cur) =>
          (cur ?? []).map((i) => (i.id === item.id ? { ...i, active: false } : i)),
        );
      },
      revert: () => {
        setItems((cur) =>
          (cur ?? []).map((i) => (i.id === item.id ? { ...i, active: item.active } : i)),
        );
      },
      request: (client, body) =>
        client.POST("/api/v1/catalog-items/{id}/archive", {
          params: { path: { id: item.id } },
          body: body as components["schemas"]["ArchiveCatalogItemRequest"],
        }),
    });
    setBusyId(null);
    if (result.ok === false) {
      setActionError(`Could not archive ${item.name}: ${rejectionText(result, "rejected.")}`);
    }
  }

  if (items === null && loadError === null) {
    return <Spinner />;
  }
  if (loadError !== null) {
    return <EmptyState message={loadError} />;
  }

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
      <header className="m-rise flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Catalog
          </h1>
          <p className="mt-0.5 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Products and services with their pricing; quote lines pick from here.
          </p>
        </div>
        <Button onClick={openCreate} data-testid="catalog-new">
          New catalog item
        </Button>
      </header>

      <div data-testid="catalog-stat-strip" className="grid gap-3 sm:grid-cols-3">
        {(() => {
          const list = items ?? [];
          const active = list.filter((i) => i.active);
          const cats = [...new Set(list.map((i) => i.category).filter((c): c is string => c != null))];
          return (
            [
              { label: "Items", value: list.length, hint: `${active.length} active`, testId: "catalog-stat-items" },
              { label: "Categories", value: cats.length, hint: "free-form groups", testId: "catalog-stat-cats" },
              { label: "Taxable", value: list.filter((i) => i.taxable).length, hint: "tax-defaulted items", testId: "catalog-stat-taxable" },
            ] as const
          ).map((t, i) => (
            <Card key={t.label} className="m-rise p-4" style={{ animationDelay: `${Math.min(i, 6) * 40}ms` }}>
              <p className="text-[11px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                {t.label}
              </p>
              <p className="mt-1 font-mono text-xl font-semibold tabular-nums" style={{ color: "var(--m-text)" }} data-testid={t.testId}>
                {t.value}
              </p>
              <p className="mt-0.5 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                {t.hint}
              </p>
            </Card>
          ));
        })()}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search name, SKU, category…"
          aria-label="Search catalog"
          data-testid="catalog-search"
          style={{ maxWidth: 320 }}
        />
      </div>

      {actionError !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="catalog-action-error">
          {actionError}
        </p>
      )}

      <Card className="m-rise overflow-x-auto p-0">
        <Table data-testid="catalog-table">
          <thead>
            <tr>
              <Th>Name</Th>
              <Th>SKU</Th>
              <Th>Unit</Th>
              <Th className="text-right">Price</Th>
              <Th className="text-right">Cost</Th>
              <Th>Tax</Th>
              <Th>State</Th>
              <Th className="text-right">Actions</Th>
            </tr>
          </thead>
          <tbody>
            {filtered.map((item, i) => (
              <Tr key={item.id} className="row-hover" data-testid={`catalog-row-${item.id}`}>
                <Td>
                  <span className="font-medium">{item.name}</span>
                  {item.category !== null && item.category !== undefined && (
                    <Tag tone="outline" className="ml-1.5 text-[10px]">
                      {item.category}
                    </Tag>
                  )}
                  {item.group_name != null && item.group_name !== "" ? (
                    <span className="ml-1.5 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                      {item.group_name}
                    </span>
                  ) : null}
                </Td>
                <Td className="font-mono text-[12px]">{item.sku ?? "—"}</Td>
                <Td>{item.unit}</Td>
                <Td className="text-right font-mono text-[13px] tabular-nums">{money(item.unit_price_minor, item.currency)}</Td>
                <Td className="text-right font-mono text-[12px] tabular-nums">
                  {item.cost_minor === null || item.cost_minor === undefined ? (
                    <span style={{ color: "var(--m-text-muted)" }}>—</span>
                  ) : (
                    money(item.cost_minor, item.currency)
                  )}
                </Td>
                <Td>
                  {item.taxable ? (
                    <Tag tone="outline">taxable</Tag>
                  ) : (
                    <Tag tone="outline" style={{ color: "var(--m-text-muted)" }}>
                      exempt
                    </Tag>
                  )}
                </Td>
                <Td>
                  <StatusTag tone={item.active ? "success" : "neutral"}>
                    {item.active ? "active" : "archived"}
                  </StatusTag>
                </Td>
                <Td>
                  <div className="flex justify-end gap-1">
                    <Button size="sm" variant="secondary" onClick={() => openEdit(item)}>
                      Edit
                    </Button>
                    {item.active && (
                      <Button size="sm" variant="ghost" disabled={busyId === item.id} onClick={() => void archive(item)}>
                        Archive
                      </Button>
                    )}
                  </div>
                </Td>
              </Tr>
            ))}
            {filtered.length === 0 && (
              <Tr>
                <Td>
                  <EmptyState message="No catalog items match yet." />
                </Td>
              </Tr>
            )}
          </tbody>
        </Table>
      </Card>

      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent title={editing === null ? "New catalog item" : `Edit ${editing.name}`}>
          <Card className="grid max-h-[80vh] gap-3 overflow-auto p-4">
            <div className="grid gap-1">
              <Label htmlFor="catalog-name">Name</Label>
              <Input id="catalog-name" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} data-testid="catalog-name" />
            </div>
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
              <div className="grid gap-1">
                <Label htmlFor="catalog-unit">Unit</Label>
                <Input id="catalog-unit" value={form.unit} onChange={(e) => setForm({ ...form, unit: e.target.value })} />
                <FieldHint>Default “each”.</FieldHint>
              </div>
              <div className="grid gap-1">
                <Label htmlFor="catalog-currency">Currency</Label>
                <Input
                  id="catalog-currency"
                  value={form.currency}
                  maxLength={3}
                  onChange={(e) => setForm({ ...form, currency: e.target.value.toUpperCase() })}
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="catalog-price">Unit price (major)</Label>
                <Input
                  id="catalog-price"
                  inputMode="decimal"
                  value={form.unitPriceMajor}
                  onChange={(e) => setForm({ ...form, unitPriceMajor: e.target.value })}
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="catalog-cost">Cost (major)</Label>
                <Input
                  id="catalog-cost"
                  inputMode="decimal"
                  value={form.costMajor}
                  onChange={(e) => setForm({ ...form, costMajor: e.target.value })}
                />
              </div>
            </div>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
              <div className="grid gap-1">
                <Label htmlFor="catalog-sku">SKU</Label>
                <Input id="catalog-sku" value={form.sku} onChange={(e) => setForm({ ...form, sku: e.target.value })} />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="catalog-category">Category</Label>
                <Input id="catalog-category" value={form.category} onChange={(e) => setForm({ ...form, category: e.target.value })} />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="catalog-group">Group</Label>
                <Input id="catalog-group" value={form.groupName} onChange={(e) => setForm({ ...form, groupName: e.target.value })} />
              </div>
            </div>
            <div className="grid gap-1">
              <Label htmlFor="catalog-description">Description</Label>
              <Input id="catalog-description" value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} />
            </div>
            <label className="flex items-center gap-2 text-xs" style={{ color: "var(--m-text-secondary)" }}>
              <input
                type="checkbox"
                checked={form.taxable}
                onChange={(e) => setForm({ ...form, taxable: e.target.checked })}
              />
              Taxable by default in quote lines
            </label>
            {formError !== null && (
              <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="catalog-form-error">
                {formError}
              </p>
            )}
            <DialogActions>
              <Button variant="secondary" onClick={() => setEditOpen(false)}>
                Cancel
              </Button>
              <Button disabled={formBusy} onClick={() => void save()} data-testid="catalog-save">
                {editing === null ? "Create" : "Save"}
              </Button>
            </DialogActions>
          </Card>
        </DialogContent>
      </Dialog>
    </div>
  );
}
