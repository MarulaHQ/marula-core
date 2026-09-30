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
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { money } from "../../../lib/format";
import { useDataSurfacePrefs } from "../../../lib/data-surface-prefs";
import { DensitySwitch, applyDensity } from "@marula-core/ui";
import { MoneyCard, MoneyHeader, MoneyPillTag, MoneyStatPill } from "../../../components/money";

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

  const prefs = useDataSurfacePrefs();

  return (
    <div className="m-money-page mx-auto w-full max-w-[1400px] p-4 md:p-6">
      {/* The price book: a card per line, the amount the first thing the eye
          lands on, because a catalogue is read by price (pack §9). */}
      <MoneyHeader
        title="Catalogue"
        stats={
          <div style={{ display: "flex", gap: 8, alignItems: "center" }} data-testid="catalog-stat-strip">
            <MoneyStatPill
              label="Items"
              count={(items ?? []).length}
              tone="gold"
              testId="catalog-stat-items"
            />
            <MoneyStatPill
              label="Active"
              count={(items ?? []).filter((i) => i.active).length}
              tone="bark"
              testId="catalog-stat-cats"
            />
            <MoneyStatPill
              label="Taxable"
              count={(items ?? []).filter((i) => i.taxable).length}
              tone="bark"
              testId="catalog-stat-taxable"
            />
          </div>
        }
        actions={
          <Button onClick={openCreate} data-testid="catalog-new">
            New catalog item
          </Button>
        }
      />

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

      <MoneyCard testId="catalog-grid">
        <div className="m-money-bar">
          <span>
            <b>{filtered.length}</b> items · <b>{(items ?? []).filter((i) => i.active).length}</b>{" "}
            active
          </span>
        </div>
        {filtered.length === 0 ? (
          <div style={{ padding: 16 }}>
            <EmptyState message="No catalog items match yet." />
          </div>
        ) : (
        <div className="m-money-grid-cards" data-testid="catalog-table" style={{ padding: 16 }}>
            {filtered.map((item) => (
              <article
                key={item.id}
                className="m-money-price row-hover"
                data-testid={`catalog-row-${item.id}`}
              >
                <div style={{ display: "flex", alignItems: "baseline", gap: 8, flexWrap: "wrap" }}>
                  <span style={{ fontWeight: 600 }}>{item.name}</span>
                  {item.category != null && item.category !== "" ? (
                    <span style={{ fontSize: 11, color: "var(--m-text-muted)" }}>{item.category}</span>
                  ) : null}
                </div>
                <span className="m-money-price-amount">
                  {money(item.unit_price_minor, item.currency)}{" "}
                  <span style={{ fontSize: 12, fontWeight: 400, color: "var(--m-text-muted)" }}>
                    / {item.unit}
                  </span>
                </span>
                <div
                  style={{
                    display: "flex",
                    gap: 8,
                    alignItems: "center",
                    flexWrap: "wrap",
                    fontSize: 12,
                    color: "var(--m-text-secondary)",
                  }}
                >
                  <MoneyPillTag
                    pill={item.active ? { label: "active", family: "green" } : { label: "archived", family: "neutral" }}
                  />
                  <span className="font-mono">{item.sku ?? "—"}</span>
                  <span>{item.taxable ? "taxable" : "exempt"}</span>
                  {item.group_name != null && item.group_name !== "" ? (
                    <span style={{ color: "var(--m-text-muted)" }}>{item.group_name}</span>
                  ) : null}
                </div>
                <div
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    alignItems: "center",
                    gap: 8,
                    marginTop: "auto",
                    paddingTop: 6,
                  }}
                >
                  <span
                    className="font-mono"
                    style={{ fontSize: 12, color: "var(--m-text-muted)" }}
                  >
                    {item.cost_minor == null
                      ? "cost —"
                      : `cost ${money(item.cost_minor, item.currency)}`}
                  </span>
                  <span style={{ display: "flex", gap: 6 }}>
                    <Button size="sm" variant="secondary" onClick={() => openEdit(item)}>
                      Edit
                    </Button>
                    {item.active ? (
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={busyId === item.id}
                        onClick={() => void archive(item)}
                      >
                        Archive
                      </Button>
                    ) : null}
                  </span>
                </div>
              </article>
            ))}
        </div>
        )}
      </MoneyCard>

      <footer className="m-money-foot">
        <span className="m-money-foot-sp" />
        <DensitySwitch
          value={prefs.density}
          onChange={(step) => {
            applyDensity(step);
            void prefs.setDensity(step);
          }}
        />
      </footer>

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
