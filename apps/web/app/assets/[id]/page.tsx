// /assets/[id] — the full asset detail page (Wave 11 lane 5, 0054;
// UX-excellence pass, Wave 17 lane 2). Two-pane CRM layout on wide
// viewports: identity fields + lifecycle on the left, an
// at-a-glance/related rail on the right. Everything the record carries:
// typed layout fields (schema-driven custom fields), core identity
// fields, the asset's audit trail (provenance write-trail), integration
// link status (data, not sync), the related-items panel
// (`document_links`) with the CROSS-ENTITY TARGET PICKER (browse/search
// documents, tickets, KB articles, assets, clients, legacy passwords —
// never paste-a-UUID), and the asset's vault entries — metadata only,
// with reveal behind the audited password endpoint (strict posture: a
// typed 403 renders inline; secrets exist only in that audited response,
// copy-after-reveal).
//
// Mutations ride the instant-first envelope via runEnvelopeMutation
// (IF1–IF3): optimistic patch, idempotency key, version stamp, typed-
// rejection rollback. Light+dark ride Workbench tokens only.
"use client";

import { rejectionText } from "../../../lib/error-copy";
import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
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
  Tag,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useAuth, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { EntityPicker } from "../../../components/entities/entity-picker";
import type { LinkTarget } from "../../../lib/entity-links";

type DeviceAsset = components["schemas"]["DeviceAsset"];
type DocumentLayout = components["schemas"]["DocumentLayout"];
type LayoutField = components["schemas"]["LayoutField"];
type ResolvedLink = components["schemas"]["ResolvedLink"];
type VaultEntryView = components["schemas"]["VaultEntryView"];
type IntegrationLinkStatus = components["schemas"]["IntegrationLinkStatus"];
type ProvenancePage = components["schemas"]["ProvenancePage"];
type Site = components["schemas"]["Site"];

/** Editable inventory lifecycle fields (rental tracking + depreciation). */
interface AssetLifecycleDraft {
  ownership: string;
  purchase_cost_major: string;
  purchase_currency: string;
  purchase_date: string;
  rental_vendor: string;
  rental_cost_major: string;
  rental_currency: string;
  rental_cycle: string;
  rental_start: string;
  rental_end: string;
  depreciation_method: string;
  depreciation_rate_bps: string;
  useful_life_months: string;
  salvage_major: string;
}

/** Major-unit string → minor-unit integer; "" and NaN → undefined (keep). */
function majorToMinor(v: string): number | undefined {
  if (v.trim() === "") return undefined;
  const n = Number(v);
  return Number.isNaN(n) ? undefined : Math.round(n * 100);
}

/** "" or "none" → null (absent patch semantics keep the value). */
function orNull(v: string): string | null | undefined {
  return v.trim() === "" ? null : v.trim();
}

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
    default:
      return "text";
  }
}

/** Bool layout fields render as an honest checkbox, not a text input. */
function BoolField({
  id,
  label,
  value,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (next: string) => void;
}) {
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
        {label}
      </Label>
    </div>
  );
}

export default function AssetDetailPage() {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const assetId = params.id;

  const [asset, setAsset] = useState<DeviceAsset | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [layouts, setLayouts] = useState<DocumentLayout[] | null>(null);
  const [sites, setSites] = useState<Site[] | null>(null);
  const [links, setLinks] = useState<ResolvedLink[] | null>(null);
  const [vault, setVault] = useState<VaultEntryView[] | null>(null);
  const [vaultDenied, setVaultDenied] = useState(false);
  const [integrations, setIntegrations] = useState<IntegrationLinkStatus[] | null>(null);
  const [provenance, setProvenance] = useState<ProvenancePage | null>(null);
  const [provenanceDenied, setProvenanceDenied] = useState(false);

  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // Draft edits for the fields card.
  const [nameDraft, setNameDraft] = useState("");
  const [statusDraft, setStatusDraft] = useState("");
  const [siteDraft, setSiteDraft] = useState<string>("");
  const [layoutDraft, setLayoutDraft] = useState<string>("");
  const [fieldDrafts, setFieldDrafts] = useState<Record<string, string>>({});

  // Link panel state (cross-entity target picker — no paste-a-UUID).
  const [linksError, setLinksError] = useState<string | null>(null);
  const [linkBusy, setLinkBusy] = useState(false);
  const [linkPickerOpen, setLinkPickerOpen] = useState(false);

  // Reveal dialog state (audited; copy-after-reveal only).
  const [reveal, setReveal] = useState<(VaultEntryView & { secret: string }) | null>(null);
  const [revealing, setRevealing] = useState<string | null>(null);
  const [revealError, setRevealError] = useState<string | null>(null);

  // Inventory lifecycle drafts (rental tracking + depreciation, 0061):
  // money in major-unit strings, persisted as minor units on save.
  const [lifecycleDraft, setLifecycleDraft] = useState<AssetLifecycleDraft>({
    ownership: "",
    purchase_cost_major: "",
    purchase_currency: "",
    purchase_date: "",
    rental_vendor: "",
    rental_cost_major: "",
    rental_currency: "",
    rental_cycle: "",
    rental_start: "",
    rental_end: "",
    depreciation_method: "",
    depreciation_rate_bps: "",
    useful_life_months: "",
    salvage_major: "",
  });

  const layoutById = useMemo(() => {
    const map = new Map<string, DocumentLayout>();
    for (const l of layouts ?? []) map.set(l.id, l);
    return map;
  }, [layouts]);
  const layout = asset?.layout_id !== null && asset?.layout_id !== undefined
    ? layoutById.get(asset.layout_id)
    : undefined;

  const refreshAsset = useCallback(async () => {
    const { data, error, response } = await api.api.GET("/api/v1/assets/{id}", {
      params: { path: { id: assetId } },
    });
    if (data !== undefined) {
      setAsset(data);
      setLoadError(null);
    } else {
      setLoadError(
        response.status === 403
          ? "Your role doesn't allow viewing this asset."
          : "This asset doesn't exist or is no longer in your scope."
      );
    }
  }, [assetId]);

  const refreshContext = useCallback(async () => {
    void api.api.GET("/api/v1/documents/layouts").then(({ data }) => setLayouts(data ?? []));
    const a = await api.api.GET("/api/v1/assets/{id}", { params: { path: { id: assetId } } });
    if (a.data !== undefined && a.data.client_id != null) {
      void api.api
        .GET("/api/v1/clients/{id}/sites", { params: { path: { id: a.data.client_id } } })
        .then(({ data }) => setSites(data ?? []));
    }
  }, [assetId]);

  const refreshLinks = useCallback(async () => {
    const { data } = await api.api.GET("/api/v1/assets/{id}/links", {
      params: { path: { id: assetId } },
    });
    setLinks(data ?? []);
  }, [assetId]);

  const refreshVault = useCallback(async () => {
    const { data, error, response } = await api.api.GET("/api/v1/assets/{id}/vault", {
      params: { path: { id: assetId } },
    });
    if (response.status === 403) {
      setVaultDenied(true);
      setVault([]);
      return;
    }
    setVaultDenied(false);
    setVault(data ?? []);
    void error;
  }, [assetId]);

  const refreshIntegrations = useCallback(async () => {
    const { data } = await api.api.GET("/api/v1/assets/integration-links");
    setIntegrations(data ?? []);
  }, []);

  const refreshProvenance = useCallback(async () => {
    const { data, error, response } = await api.api.GET("/api/v1/provenance", {
      params: { query: { entity: "asset", entity_id: assetId } },
    });
    if (response.status === 403) {
      setProvenanceDenied(true);
      setProvenance(null);
      return;
    }
    setProvenanceDenied(false);
    setProvenance(data ?? null);
  }, [assetId]);

  // Fetch only once authenticated — the api client carries the bearer
  // token from the auth provider, and effects fire before it lands.
  useEffect(() => {
    if (auth.status !== "authed") return;
    void refreshAsset();
    void refreshContext();
    void refreshLinks();
    void refreshVault();
    void refreshIntegrations();
    void refreshProvenance();
  }, [auth.status, refreshAsset, refreshContext, refreshLinks, refreshVault, refreshIntegrations, refreshProvenance]);

  useEffect(() => {
    if (asset !== null) {
      setNameDraft(asset.name);
      setStatusDraft(asset.status);
      setSiteDraft(asset.site_id ?? "");
      setLayoutDraft(asset.layout_id ?? "");
      setLifecycleDraft({
        ownership: asset.ownership ?? "",
        purchase_cost_major:
          asset.purchase_cost_minor === null || asset.purchase_cost_minor === undefined
            ? ""
            : (asset.purchase_cost_minor / 100).toString(),
        purchase_currency: asset.purchase_currency ?? "",
        purchase_date: asset.purchase_date ?? "",
        rental_vendor: asset.rental_vendor ?? "",
        rental_cost_major:
          asset.rental_cost_minor === null || asset.rental_cost_minor === undefined
            ? ""
            : (asset.rental_cost_minor / 100).toString(),
        rental_currency: asset.rental_currency ?? "",
        rental_cycle: asset.rental_cycle ?? "",
        rental_start: asset.rental_start ?? "",
        rental_end: asset.rental_end ?? "",
        depreciation_method: asset.depreciation_method ?? "",
        depreciation_rate_bps:
          asset.depreciation_rate_bps === null || asset.depreciation_rate_bps === undefined
            ? ""
            : String(asset.depreciation_rate_bps),
        useful_life_months:
          asset.useful_life_months === null || asset.useful_life_months === undefined
            ? ""
            : String(asset.useful_life_months),
        salvage_major:
          asset.salvage_value_minor === null || asset.salvage_value_minor === undefined
            ? ""
            : (asset.salvage_value_minor / 100).toString(),
      });
    }
  }, [asset]);

  const saveEdits = async () => {
    if (asset === null) return;
    setSaving(true);
    setFormError(null);
    // Layout change re-validates kept fields server-side against the NEW
    // schema; build the fields object with the same coercion as create.
    const fieldValues: Record<string, unknown> = {};
    const effectiveLayout =
      layoutDraft !== "" && layoutDraft !== asset.layout_id
        ? layoutById.get(layoutDraft)
        : layout;
    if (effectiveLayout !== undefined) {
      for (const f of effectiveLayout.fields) {
        const raw = fieldDraftOf(f, asset, fieldDrafts);
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
    }
    const result = await runEnvelopeMutation({
      describe: "asset update",
      entityId: asset.id,
      buildBody(base) {
        return {
          name: nameDraft.trim(),
          status: statusDraft.trim(),
          site_id: siteDraft === "" ? null : siteDraft,
          layout_id: layoutDraft === "" ? null : layoutDraft,
          fields: fieldValues,
          lifecycle: {
            ownership: orNull(lifecycleDraft.ownership),
            purchase_cost_minor: majorToMinor(lifecycleDraft.purchase_cost_major),
            purchase_currency: orNull(lifecycleDraft.purchase_currency),
            purchase_date: orNull(lifecycleDraft.purchase_date),
            rental_vendor: orNull(lifecycleDraft.rental_vendor),
            rental_cost_minor: majorToMinor(lifecycleDraft.rental_cost_major),
            rental_currency: orNull(lifecycleDraft.rental_currency),
            rental_cycle: orNull(lifecycleDraft.rental_cycle),
            rental_start: orNull(lifecycleDraft.rental_start),
            rental_end: orNull(lifecycleDraft.rental_end),
            depreciation_method: orNull(lifecycleDraft.depreciation_method),
            depreciation_rate_bps:
              lifecycleDraft.depreciation_rate_bps.trim() === "" || Number.isNaN(Number(lifecycleDraft.depreciation_rate_bps))
                ? null
                : Math.max(1, Math.min(9999, Math.round(Number(lifecycleDraft.depreciation_rate_bps)))),
            useful_life_months:
              lifecycleDraft.useful_life_months.trim() === "" || Number.isNaN(Number(lifecycleDraft.useful_life_months))
                ? null
                : Math.max(1, Math.round(Number(lifecycleDraft.useful_life_months))),
            salvage_value_minor: majorToMinor(lifecycleDraft.salvage_major),
          },
          idempotency_key: base.idempotency_key,
          version: base.version,
        };
      },
      optimistic() {},
      revert() {},
      request(client, body) {
        return client.PATCH("/api/v1/assets/{id}", {
          params: { path: { id: asset.id } },
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          body: body as any,
        });
      },
    });
    setSaving(false);
    if (!result.ok) {
      setFormError(
        result.reason === "version_conflict"
          ? "Someone else saved this asset first. Reload and retry."
          : (rejectionText(result, "The server rejected this change. Check the fields and retry."))
      );
      void refreshAsset();
      return;
    }
    setFormError(null);
    void refreshAsset();
    void refreshProvenance();
  };

  const runReveal = async (entry: VaultEntryView) => {
    setRevealing(entry.id);
    setRevealError(null);
    const { data, error } = await api.api.POST("/api/v1/passwords/{id}/reveal", {
      params: { path: { id: entry.id } },
    });
    setRevealing(null);
    if (error !== undefined || data === undefined) {
      setRevealError(
        "Reveal is not allowed for your role. Reveal requires vault edit rights and is audited."
      );
      return;
    }
    setRevealError(null);
    setReveal({ ...entry, secret: (data as { secret?: string }).secret ?? "" });
  };

  const addLink = async (target: LinkTarget) => {
    setLinksError(null);
    setLinkBusy(true);
    const { error } = await api.api.POST("/api/v1/assets/{id}/links", {
      params: { path: { id: assetId } },
      body: { to_kind: target.kind, to_id: target.id },
    });
    setLinkBusy(false);
    if (error !== undefined) {
      setLinksError(
        "The server rejected this link. The target must exist and be visible in your scope."
      );
      return;
    }
    void refreshLinks();
  };

  const removeLink = async (linkId: string) => {
    // Unlink rides the shared docs-platform endpoint; broken targets are
    // removable, targets are never cascade-deleted.
    const { error } = await api.api.DELETE("/api/v1/documents/links/{id}", {
      params: { path: { id: linkId } },
    });
    if (error === undefined) void refreshLinks();
  };

  if (auth.status !== "authed") return null;

  if (loadError !== null) {
    return (
      <div className="grid gap-4 p-6">
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {loadError}
        </p>
        <Link href="/clients" className="text-[13px] underline underline-offset-2">
          Back to clients
        </Link>
      </div>
    );
  }

  if (asset === null) {
    return (
      <div className="p-6">
        <Spinner />
      </div>
    );
  }

  return (
    <div className="mx-auto grid w-full grid-cols-1 max-w-[1400px] gap-6 p-4 md:p-6">
      <div className="m-entity-rise">
        {asset.client_id != null ? (
          <Link
            href={`/clients/${asset.client_id}`}
            className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2"
            style={{ color: "var(--m-text-secondary)" }}
          >
            ← Client
          </Link>
        ) : (
          <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Internal org asset
          </span>
        )}
        <div className="mt-2 flex flex-wrap items-center gap-3">
          <h1 className="text-lg font-semibold">{asset.name}</h1>
          <span className="font-mono text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            {asset.tag}
          </span>
          <Tag>{asset.status}</Tag>
          {layout !== undefined ? <Tag tone="outline">{layout.name}</Tag> : null}
        </div>
      </div>

      <div className="grid grid-cols-1 gap-6 xl:grid-cols-[minmax(0,1fr)_360px]">
        <div className="grid content-start gap-6">
      {/* ---- fields ---- */}
      <Card className="m-entity-rise" style={{ "--stagger-i": 0 } as React.CSSProperties}>
        <div
          className="flex flex-wrap items-center gap-3 p-4"
          style={{ borderBottom: "1px solid var(--m-border)" }}
        >
          <h2 className="text-sm font-semibold">Fields</h2>
        </div>
        <div className="grid gap-3 p-4 md:grid-cols-2">
          <div>
            <Label htmlFor="asset-name-edit">Name</Label>
            <Input id="asset-name-edit" value={nameDraft} onChange={(e) => setNameDraft(e.target.value)} />
          </div>
          <div>
            <Label htmlFor="asset-status-edit">Status</Label>
            <Input id="asset-status-edit" value={statusDraft} onChange={(e) => setStatusDraft(e.target.value)} />
          </div>
          <div>
            <Label htmlFor="asset-site-edit">Site</Label>
            <Select value={siteDraft} onValueChange={setSiteDraft}>
              <SelectTrigger id="asset-site-edit" className="w-64">
                <SelectValue placeholder="No site" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="">No site</SelectItem>
                {(sites ?? []).map((s) => (
                  <SelectItem key={s.id} value={s.id}>
                    {s.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label htmlFor="asset-type-edit">Type</Label>
            <Select value={layoutDraft} onValueChange={setLayoutDraft}>
              <SelectTrigger id="asset-type-edit" className="w-64">
                <SelectValue placeholder="Untyped" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="">Untyped</SelectItem>
                {(layouts ?? []).map((l) => (
                  <SelectItem key={l.id} value={l.id}>
                    {l.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          {(effectiveFields(layout, layoutDraft, layoutById, asset)).map((f) =>
            f.type === "bool" ? (
              <BoolField
                key={f.key}
                id={`af-${f.key}`}
                label={f.label}
                value={fieldDraftOf(f, asset, fieldDrafts)}
                onChange={(next) =>
                  setFieldDrafts((prev) => ({ ...prev, [f.key]: next }))
                }
              />
            ) : (
              <div key={f.key}>
                <Label htmlFor={`af-${f.key}`}>{f.label}</Label>
                <Input
                  id={`af-${f.key}`}
                  type={fieldInputType(f)}
                  value={fieldDraftOf(f, asset, fieldDrafts)}
                  onChange={(e) =>
                    setFieldDrafts((prev) => ({ ...prev, [f.key]: e.target.value }))
                  }
                />
              </div>
            )
          )}
        </div>
        {formError !== null ? (
          <p role="alert" className="px-4 pb-2 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {formError}
          </p>
        ) : null}
        <div className="flex justify-end gap-2 px-4 pb-4">
          <Button onClick={() => void saveEdits()} disabled={saving}>
            {saving ? <Spinner /> : null} Save fields
          </Button>
        </div>
      </Card>

      {/* ---- inventory lifecycle (rental tracking + depreciation, 0061) ---- */}
      <Card>
        <div
          className="flex flex-wrap items-center gap-3 p-4"
          style={{ borderBottom: "1px solid var(--m-border)" }}
        >
          <h2 className="text-sm font-semibold">Lifecycle</h2>
          <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            Rental tracking and depreciation — saves with the fields above.
          </span>
        </div>
        <div className="grid gap-3 p-4 md:grid-cols-2">
          <div className="grid gap-1">
            <Label htmlFor="asset-ownership">Ownership</Label>
            <select
              id="asset-ownership"
              className="rounded-[var(--m-radius-container)] border bg-transparent px-2 py-1 text-[13px]"
              style={{ borderColor: "var(--m-border)", color: "var(--m-text)" }}
              value={lifecycleDraft.ownership}
              onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, ownership: e.target.value })}
              data-testid="asset-ownership"
            >
              <option value="">—</option>
              <option value="owned">owned</option>
              <option value="leased">leased</option>
              <option value="rented">rented</option>
            </select>
          </div>
          <div className="grid gap-1">
            <Label htmlFor="asset-dep-method">Depreciation method</Label>
            <select
              id="asset-dep-method"
              className="rounded-[var(--m-radius-container)] border bg-transparent text-[13px]"
              style={{ borderColor: "var(--m-border)", color: "var(--m-text)" }}
              value={lifecycleDraft.depreciation_method}
              onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, depreciation_method: e.target.value })}
              data-testid="asset-dep-method"
            >
              <option value="">—</option>
              <option value="none">none</option>
              <option value="straight_line">straight line</option>
              <option value="declining_balance">declining balance</option>
            </select>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <div className="grid gap-1">
              <Label htmlFor="asset-purchase-cost">Purchase cost (major)</Label>
              <Input
                id="asset-purchase-cost"
                inputMode="decimal"
                value={lifecycleDraft.purchase_cost_major}
                onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, purchase_cost_major: e.target.value })}
              />
            </div>
            <div className="grid gap-1">
              <Label htmlFor="asset-purchase-currency">Currency</Label>
              <Input
                id="asset-purchase-currency"
                maxLength={3}
                value={lifecycleDraft.purchase_currency}
                onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, purchase_currency: e.target.value.toUpperCase() })}
              />
            </div>
            <div className="grid gap-1">
              <Label htmlFor="asset-purchase-date">Purchase date</Label>
              <Input
                id="asset-purchase-date"
                type="date"
                value={lifecycleDraft.purchase_date}
                onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, purchase_date: e.target.value })}
              />
            </div>
            <div className="grid gap-1">
              <Label htmlFor="asset-salvage">Salvage value (major)</Label>
              <Input
                id="asset-salvage"
                inputMode="decimal"
                value={lifecycleDraft.salvage_major}
                onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, salvage_major: e.target.value })}
              />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <div className="grid gap-1">
              <Label htmlFor="asset-dep-rate">Declining-balance rate (bps)</Label>
              <Input
                id="asset-dep-rate"
                inputMode="numeric"
                value={lifecycleDraft.depreciation_rate_bps}
                onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, depreciation_rate_bps: e.target.value })}
              />
              <FieldHint>1–9999; declining balance only.</FieldHint>
            </div>
            <div className="grid gap-1">
              <Label htmlFor="asset-useful-life">Useful life (months)</Label>
              <Input
                id="asset-useful-life"
                inputMode="numeric"
                value={lifecycleDraft.useful_life_months}
                onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, useful_life_months: e.target.value })}
              />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-2 md:col-span-2">
            <div className="grid gap-1">
              <Label htmlFor="asset-rental-vendor">Rental vendor</Label>
              <Input
                id="asset-rental-vendor"
                value={lifecycleDraft.rental_vendor}
                onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, rental_vendor: e.target.value })}
              />
            </div>
            <div className="grid gap-1">
              <Label htmlFor="asset-rental-cost">Rental cost (major)</Label>
              <Input
                id="asset-rental-cost"
                inputMode="decimal"
                value={lifecycleDraft.rental_cost_major}
                onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, rental_cost_major: e.target.value })}
              />
            </div>
            <div className="grid gap-1">
              <Label htmlFor="asset-rental-currency">Rental currency</Label>
              <Input
                id="asset-rental-currency"
                maxLength={3}
                value={lifecycleDraft.rental_currency}
                onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, rental_currency: e.target.value.toUpperCase() })}
              />
            </div>
            <div className="grid gap-1">
              <Label htmlFor="asset-rental-cycle">Rental cycle</Label>
              <select
                id="asset-rental-cycle"
                className="rounded-[var(--m-radius-container)] border bg-transparent text-[13px]"
                style={{ borderColor: "var(--m-border)", color: "var(--m-text)" }}
                value={lifecycleDraft.rental_cycle}
                onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, rental_cycle: e.target.value })}
                data-testid="asset-rental-cycle"
              >
                <option value="">—</option>
                <option value="monthly">monthly</option>
                <option value="quarterly">quarterly</option>
                <option value="annual">annual</option>
              </select>
            </div>
            <div className="grid gap-1">
              <Label htmlFor="asset-rental-start">Rental start</Label>
              <Input
                id="asset-rental-start"
                type="date"
                value={lifecycleDraft.rental_start}
                onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, rental_start: e.target.value })}
              />
            </div>
            <div className="grid gap-1">
              <Label htmlFor="asset-rental-end">Rental end</Label>
              <Input
                id="asset-rental-end"
                type="date"
                value={lifecycleDraft.rental_end}
                onChange={(e) => setLifecycleDraft({ ...lifecycleDraft, rental_end: e.target.value })}
              />
            </div>
          </div>
        </div>
        {lifecycleDraft.ownership !== "rented" && lifecycleDraft.ownership !== "leased" ? null : (
          <p className="px-4 pb-4 text-[11px]" style={{ color: "var(--m-text-muted)" }} data-testid="asset-rental-note">
            Rented/leased asset — rental period {lifecycleDraft.rental_start || "?"} → {lifecycleDraft.rental_end || "?"}
            {lifecycleDraft.rental_cost_major !== ""
              ? ` · ${lifecycleDraft.rental_cost_major} ${lifecycleDraft.rental_currency || ""} per ${lifecycleDraft.rental_cycle || "cycle"}`
              : ""}
          </p>
        )}
      </Card>

        </div>

        <div className="grid content-start gap-6" data-testid="asset-glance-rail">
          {/* ---- at a glance ---- */}
          <Card className="m-entity-rise" style={{ "--stagger-i": 2 } as React.CSSProperties}>
            <h2 className="p-4 pb-2 text-sm font-semibold">At a glance</h2>
            <dl className="grid gap-1 px-4 pb-4 text-[13px]">
              <GlanceRow label="Client">
                {asset.client_id != null ? (
                  <Link href={`/clients/${asset.client_id}`} className="underline decoration-[var(--m-border)] underline-offset-2">
                    {asset.client_id.slice(0, 8)}…
                  </Link>
                ) : (
                  "Internal org"
                )}
              </GlanceRow>
              <GlanceRow label="Site">
                {asset.site_id != null
                  ? (sites ?? []).find((s) => s.id === asset.site_id)?.name ?? "site"
                  : "—"}
              </GlanceRow>
              <GlanceRow label="Tag">
                <span className="font-mono">{asset.tag}</span>
              </GlanceRow>
              <GlanceRow label="Status">{asset.status}</GlanceRow>
              <GlanceRow label="Type">{layout?.name ?? "untyped"}</GlanceRow>
            </dl>
          </Card>

      {/* ---- integration link status: data, not sync ---- */}
      <Card className="m-entity-rise" style={{ "--stagger-i": 3 } as React.CSSProperties}>
        <h2 className="p-4 pb-0 text-sm font-semibold">Integrations</h2>
        <p className="px-4 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Per-connector link state. Link state is shown as data; no live sync runs from this page.
        </p>
        {integrations === null ? (
          <div className="p-4">
            <Spinner />
          </div>
        ) : integrations.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No connectors are linked for this org yet. Install one to mirror assets from your tooling." />
          </div>
        ) : (
          <div className="grid gap-2 p-4">
            {integrations.map((i) => (
              <div
                key={i.connector_type}
                className="flex flex-wrap items-center gap-3 rounded-md p-3"
                style={{ background: "var(--m-bg-raised)" }}
              >
                <span className="text-[13px] font-medium">{i.connector_type}</span>
                <Tag>linked</Tag>
                <span className="ml-auto text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                  linked {new Date(i.linked_at).toLocaleString()} · updated{" "}
                  {new Date(i.updated_at).toLocaleString()}
                </span>
              </div>
            ))}
          </div>
        )}
      </Card>

      {/* ---- related items ---- */}
      <Card className="m-entity-rise" style={{ "--stagger-i": 4 } as React.CSSProperties}>
        <h2 className="p-4 pb-0 text-sm font-semibold">Related items</h2>
        <p className="px-4 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Links to documents, tickets, credentials, clients, KB articles, and other assets. Broken
          targets stay visible so you can clean them up; removing a link never removes the target.
        </p>
        {links === null ? (
          <div className="p-4">
            <Spinner />
          </div>
        ) : links.length === 0 ? (
          <div className="p-4">
            <EmptyState message="Nothing linked yet. Connect the documents and credentials that describe this asset." />
          </div>
        ) : (
          <ul className="grid gap-2 p-4">
            {links.map((l, i) => (
              <li
                key={l.id}
                className="m-entity-row m-entity-rise flex flex-wrap items-center gap-3 rounded-[0.5rem] border p-3"
                style={{ borderColor: "var(--m-border)", "--stagger-i": Math.min(i, 8) } as React.CSSProperties}
              >
                <Tag tone="outline">{l.to_kind}</Tag>
                <span className="min-w-0 flex-1 truncate text-[13px]">
                  {l.target_deleted ? (
                    <span style={{ color: "var(--m-status-danger-fg)" }}>
                      broken target — the item no longer exists
                    </span>
                  ) : (
                    l.target_label ?? l.to_id
                  )}
                </span>
                <span className="shrink-0">
                  <Button
                    variant="ghost"
                    size="sm"
                    aria-label={`Remove link to ${l.target_label ?? l.to_id}`}
                    onClick={() =>
                      void api.api
                        .DELETE("/api/v1/documents/links/{id}", {
                          params: { path: { id: l.id } },
                        })
                        .then(() => refreshLinks())
                    }
                  >
                    Remove link
                  </Button>
                </span>
              </li>
            ))}
          </ul>
        )}
        <div
          className="flex flex-wrap items-end gap-3 p-4"
          style={{ borderTop: "1px solid var(--m-border)" }}
        >
          <Button
            variant="secondary"
            onClick={() => setLinkPickerOpen(true)}
            disabled={linkBusy}
          >
            Add link
          </Button>
          <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            Browse and search documents, tickets, KB articles, other assets, clients, and legacy
            passwords — the picker lists only targets your role can see.
          </span>
        </div>
        {linksError !== null ? (
          <p role="alert" className="px-4 pb-3 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {linksError}
          </p>
        ) : null}
        <EntityPicker
          open={linkPickerOpen}
          clientId={asset.client_id ?? undefined}
          onPick={(t) => void addLink(t)}
          onClose={() => setLinkPickerOpen(false)}
        />
      </Card>

      {/* ---- vault entries: strict posture ---- */}
      <Card>
        <h2 className="p-4 pb-0 text-sm font-semibold">Vault entries</h2>
        <p className="px-4 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Credentials linked to this asset. Metadata only — the secret exists solely behind the
          audited reveal action, which records who looked and when.
        </p>
        {vaultDenied ? (
          <p role="alert" className="p-4 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
            Your role doesn't allow reading vault metadata. That gate is the Passwords permission
            matrix — ask an admin if you need access.
          </p>
        ) : vault === null ? (
          <div className="p-4">
            <Spinner />
          </div>
        ) : vault.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No credentials linked to this asset. Link a vault entry from the vault, or add a password→asset link here." />
          </div>
        ) : (
          <div className="grid gap-2 p-4">
            {vault.map((e) => (
              <div
                key={e.id}
                className="flex flex-wrap items-center gap-3 rounded-md p-3"
                style={{ background: "var(--m-bg-raised)" }}
              >
                <div className="grid">
                  <span className="text-[13px] font-medium">{e.title}</span>
                  {(e.username ?? "") !== "" ? (
                    <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                      {e.username}
                    </span>
                  ) : null}
                </div>
                <span className="ml-auto">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => void runReveal(e)}
                    disabled={revealing === e.id}
                  >
                    {revealing === e.id ? <Spinner /> : null} Reveal
                  </Button>
                </span>
              </div>
            ))}
          </div>
        )}
        {revealError !== null ? (
          <p role="alert" className="px-4 pb-3 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {revealError}
          </p>
        ) : null}
      </Card>

      {/* ---- audit trail ---- */}
      <Card>
        <h2 className="p-4 pb-0 text-sm font-semibold">Audit trail</h2>
        <p className="px-4 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Every write to this asset, from the provenance write-trail: who, what, before → after.
          The trail is append-only; undo rides it rather than rewriting it.
        </p>
        {provenanceDenied ? (
          <p role="alert" className="p-4 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
            The provenance query is restricted to your role.
          </p>
        ) : provenance === null ? (
          <div className="p-4">
            <Spinner />
          </div>
        ) : provenance.events.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No recorded writes yet for this asset." />
          </div>
        ) : (
          <div className="grid gap-2 p-4">
            {provenance.events.map((e) => (
              <div key={e.write_id} className="rounded-md p-3" style={{ background: "var(--m-bg-raised)" }}>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-[13px] font-medium">{e.op}</span>
                  <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                    {e.field}
                  </span>
                  <Tag>{e.actor_kind}</Tag>
                  {e.undo ? <Tag>undo</Tag> : null}
                  <span className="ml-auto text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                    {new Date(e.occurred_at).toLocaleString()}
                  </span>
                </div>
                <p className="mt-1 break-all font-mono text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                  {JSON.stringify(e.before ?? null)} → {JSON.stringify(e.after ?? null)}
                </p>
              </div>
            ))}
          </div>
        )}
      </Card>
        </div>
      </div>

      {/* ---- audited reveal dialog ---- */}
      <Dialog open={reveal !== null} onOpenChange={(o) => !o && setReveal(null)}>
        <DialogContent
          title="Credential revealed"
          description="This reveal was recorded in the audit trail. Close the dialog when you've copied what you need — the plaintext is not stored anywhere else."
        >
          {reveal !== null ? (
            <div className="grid gap-3 p-4">
              <div>
                <Label htmlFor="reveal-secret">Secret</Label>
                <Input id="reveal-secret" readOnly value={reveal.secret} className="font-mono" />
              </div>
              <DialogActions>
                <Button
                  variant="secondary"
                  onClick={() => void navigator.clipboard.writeText(reveal.secret)}
                >
                  Copy
                </Button>
                <Button variant="ghost" onClick={() => setReveal(null)}>
                  Close
                </Button>
              </DialogActions>
            </div>
          ) : null}
        </DialogContent>
      </Dialog>
    </div>
  );
}

// ---- draft helpers (kept below the component for readability) -------------

type FieldDrafts = Record<string, string>;

function GlanceRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="shrink-0" style={{ color: "var(--m-text-secondary)" }}>
        {label}
      </dt>
      <dd className="min-w-0 truncate text-right">{children}</dd>
    </div>
  );
}

function effectiveFields(
  current: DocumentLayout | undefined,
  layoutDraft: string,
  layoutById: Map<string, DocumentLayout>,
  asset: DeviceAsset | null
): LayoutField[] {
  if (layoutDraft !== "" && layoutDraft !== asset?.layout_id) {
    return layoutById.get(layoutDraft)?.fields ?? [];
  }
  return current?.fields ?? [];
}

function fieldDraftOf(f: LayoutField, asset: DeviceAsset, drafts: FieldDrafts): string {
  const existing = drafts[f.key];
  if (existing !== undefined) return existing;
  const v = (asset.fields as Record<string, unknown> | null)?.[f.key];
  if (v === undefined || v === null) return "";
  return String(v);
}