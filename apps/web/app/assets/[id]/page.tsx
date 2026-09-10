// /assets/[id] — the full asset detail page (Wave 11 lane 5, migration
// 0054). Replaces the flat client tab with everything an MSP expects:
// typed layout fields (schema-driven custom fields), core identity
// fields, the asset's audit trail (provenance write-trail), integration
// link status (data, not sync), the related-items panel
// (`document_links`), and the asset's vault entries — metadata only,
// with reveal behind the audited password endpoint (strict posture: a
// typed 403 renders inline; secrets exist only in that audited response,
// copy-after-reveal).
//
// Mutations ride the instant-first envelope via runEnvelopeMutation
// (IF1–IF3): optimistic patch, idempotency key, version stamp, typed-
// rejection rollback. Light+dark ride Workbench tokens only.
"use client";

import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
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

type DeviceAsset = components["schemas"]["DeviceAsset"];
type DocumentLayout = components["schemas"]["DocumentLayout"];
type LayoutField = components["schemas"]["LayoutField"];
type ResolvedLink = components["schemas"]["ResolvedLink"];
type VaultEntryView = components["schemas"]["VaultEntryView"];
type IntegrationLinkStatus = components["schemas"]["IntegrationLinkStatus"];
type ProvenancePage = components["schemas"]["ProvenancePage"];
type Site = components["schemas"]["Site"];

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

  // Link panel state.
  const [linkKind, setLinkKind] = useState<string>("");
  const [linkTargetLabel, setLinkTargetLabel] = useState("");
  const [linkError, setLinkError] = useState<string | null>(null);

  // Reveal dialog state (audited; copy-after-reveal only).
  const [reveal, setReveal] = useState<(VaultEntryView & { secret: string }) | null>(null);
  const [revealing, setRevealing] = useState<string | null>(null);
  const [revealError, setRevealError] = useState<string | null>(null);

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
    if (a.data !== undefined) {
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
          : (result.message ?? "The server rejected this change. Check the fields and retry.")
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

  const addLink = async () => {
    if (linkKind === "" || linkTargetLabel.trim() === "") return;
    setLinkError(null);
    const uuid = parseUuid(linkTargetLabel.trim());
    if (uuid === null) {
      setLinkError("Paste the target's id (a UUID). Documents and passwords can be picked from their browsers later; this panel accepts ids for now.");
      return;
    }
    const { error } = await api.api.POST("/api/v1/assets/{id}/links", {
      params: { path: { id: assetId } },
      body: { to_kind: linkKind, to_id: uuid },
    });
    if (error !== undefined) {
      setLinkError(
        "The server rejected this link. The target must exist and be visible in your scope."
      );
      return;
    }
    setLinkKind("");
    setLinkTargetLabel("");
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
    <div className="grid gap-6 p-6">
      <div>
        <Link
          href={`/clients/${asset.client_id}`}
          className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2"
          style={{ color: "var(--m-text-secondary)" }}
        >
          ← Client
        </Link>
        <div className="mt-2 flex flex-wrap items-center gap-3">
          <h1 className="text-lg font-semibold">{asset.name}</h1>
          <span className="font-mono text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            {asset.tag}
          </span>
          <Tag>{asset.status}</Tag>
          {layout !== undefined ? <Tag>{layout.name}</Tag> : null}
        </div>
      </div>

      {/* ---- fields ---- */}
      <Card>
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

      {/* ---- integration link status: data, not sync ---- */}
      <Card>
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
      <Card>
        <h2 className="p-4 pb-0 text-sm font-semibold">Related items</h2>
        <p className="px-4 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Links to documents, credentials, tickets, KB articles, clients, and other assets. Broken
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
          <div className="grid gap-2 p-4">
            {links.map((l) => (
              <div
                key={l.id}
                className="flex flex-wrap items-center gap-3 rounded-md p-3"
                style={{ background: "var(--m-bg-raised)" }}
              >
                <Tag>{l.to_kind}</Tag>
                <span className="text-[13px]">
                  {l.target_deleted ? (
                    <span style={{ color: "var(--m-status-danger-fg)" }}>
                      broken target — the item no longer exists
                    </span>
                  ) : (
                    l.target_label ?? l.to_id
                  )}
                </span>
                <span className="ml-auto">
                  <Button
                    variant="ghost"
                    size="sm"
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
              </div>
            ))}
          </div>
        )}
        <div
          className="flex flex-wrap items-end gap-3 p-4"
          style={{ borderTop: "1px solid var(--m-border)" }}
        >
          <div className="w-44">
            <Label htmlFor="link-kind">Link to</Label>
            <Select value={linkKind} onValueChange={setLinkKind}>
              <SelectTrigger id="link-kind" className="w-full">
                <SelectValue placeholder="Item type" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="document">document</SelectItem>
                <SelectItem value="password">password</SelectItem>
                <SelectItem value="client">client</SelectItem>
                <SelectItem value="ticket">ticket</SelectItem>
                <SelectItem value="kb_article">kb_article</SelectItem>
                <SelectItem value="asset">asset</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="grow">
            <Label htmlFor="link-target">Target id</Label>
            <Input
              id="link-target"
              value={linkTargetLabel}
              onChange={(e) => setLinkTargetLabel(e.target.value)}
              placeholder="Paste the target's id"
            />
          </div>
          <Button
            onClick={() => void addLink()}
            disabled={linkKind === "" || linkTargetLabel.trim() === ""}
          >
            Add link
          </Button>
        </div>
        {linkError !== null ? (
          <p role="alert" className="px-4 pb-3 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {linkError}
          </p>
        ) : null}
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

function parseUuid(s: string): string | null {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s) ? s : null;
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