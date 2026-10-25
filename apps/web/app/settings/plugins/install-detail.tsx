// /settings/plugins — per-install detail (ADR 0009 lifecycle wiring).
//
// Purpose: the expanded surface for one installed plugin: manifest
// JSON-Schema-rendered settings (GET/PATCH /plugins/installed/{id}/settings
// — settings-less installs get the consent/enable/disable-only posture
// with the typed 409), the update/rollback re-consent flow (propose →
// pending keeps the old version running → consent commit flips →
// discard clears), terminal removal with data-survival copy, and the
// append-only provenance chain read (survives uninstall).
//
// Non-purpose: no per-plugin custom settings pages (the schema renders
// the form — typed-registry doctrine) and no install authoring here.
"use client";

import type { components } from "@marula-core/sdk";
import {
  Badge,
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
} from "@marula-core/ui";
import { LocalTime } from "../../../components/local-time";
import { useCallback, useEffect, useState } from "react";
import { api } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { envelopeOf, reasonCopy } from "../../../lib/error-copy";
import { ConsentSummary, SettingsSchemaBlock } from "./consent";

type InstallView = components["schemas"]["InstallView"];
type InstallSettingsView = components["schemas"]["InstallSettingsView"];
type UpdateProposalView = components["schemas"]["UpdateProposalView"];
type ProvenanceView = components["schemas"]["ProvenanceView"];

interface SchemaLeaf {
  type?: string;
  title?: string;
  description?: string;
  enum?: (string | number | boolean)[];
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
}
interface SettingsSchema {
  type?: unknown;
  properties?: Record<string, SchemaLeaf>;
  required?: string[];
}

function readSchema(value: unknown): SettingsSchema | null {
  if (value === null || typeof value !== "object") return null;
  const schema = value as SettingsSchema;
  if (schema.type !== "object" || schema.properties === null || typeof schema.properties !== "object") {
    return null;
  }
  return schema;
}

/** The manifest-JSON-Schema-rendered settings form (0079 subset: flat
 * string|number|integer|boolean leaves + enum/bounds). Settings-less
 * installs never render inputs — the posture banner stands in, and any
 * PATCH would ride the typed 409. */
export function InstallDetail({
  install,
  canManage,
  onOptimisticStatus,
  onChanged,
}: {
  install: InstallView;
  canManage: boolean;
  /** Optimistic status flip in the parent list (instant-first envelope). */
  onOptimisticStatus: (next: "enabled" | "disabled") => void;
  onChanged: () => void;
}) {
  const [settings, setSettings] = useState<InstallSettingsView | null>(null);
  const [settingsLoad, setSettingsLoad] = useState<"loading" | "ready">("loading");
  const [provenance, setProvenance] = useState<ProvenanceView[] | null>(null);
  const [proposal, setProposal] = useState<UpdateProposalView | null>(null);
  const [config, setConfig] = useState<Record<string, unknown>>({});
  const [saveState, setSaveState] = useState<string | null>(null);
  const [proposeVersion, setProposeVersion] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);

  const refreshDetail = useCallback(async () => {
    const [s, p, u] = await Promise.all([
      api.api.GET("/api/v1/plugins/installed/{id}/settings", { params: { path: { id: install.id } } }),
      api.api.GET("/api/v1/plugins/installed/{id}/provenance", { params: { path: { id: install.id } } }),
      api.api.GET("/api/v1/plugins/installed/{id}/update", { params: { path: { id: install.id } } }),
    ]);
    if (s.data !== undefined && s.data !== null) {
      setSettings(s.data);
      setConfig((s.data.config ?? {}) as Record<string, unknown>);
    }
    setProvenance(p.data ?? []);
    setProposal(u.data ?? null);
    setSettingsLoad("ready");
  }, [install.id]);

  useEffect(() => {
    void refreshDetail();
  }, [refreshDetail]);

  const toggleStatus = async () => {
    const next = install.status === "enabled" ? "disabled" : "enabled";
    setBusy("toggle");
    setSaveState(null);
    const result = await runEnvelopeMutation({
      describe: `${next} plugin install ${install.slug}`,
      entityId: install.id,
      buildBody: ({ idempotency_key }) => ({
        idempotency_key,
        status: next,
      }),
      optimistic: () => { onOptimisticStatus(next); },
      revert: () => { onOptimisticStatus(next === "enabled" ? "disabled" : "enabled"); },
      request: (client, body) =>
        client.PATCH("/api/v1/plugins/installed/{id}", {
          params: { path: { id: install.id } },
          body: body as { status: string },
        }),
    });
    setBusy(null);
    setSaveState(result.ok ? (next === "enabled" ? "Install enabled." : "Install disabled.") : (result.friendly ?? null));
    onChanged();
  };

  const saveSettings = async () => {
    const previous = config;
    setBusy("settings");
    setSaveState(null);
    setConfig({ ...config });
    const { error } = await api.api.PATCH("/api/v1/plugins/installed/{id}/settings", {
      params: { path: { id: install.id } },
      body: { config },
    });
    setBusy(null);
    if (error !== undefined) {
      setConfig(previous);
      const envelope = envelopeOf(error);
      setSaveState(reasonCopy(envelope.reason ?? "validation_failed", envelope.details));
      return;
    }
    setSaveState("Settings saved — validated against the pinned manifest schema.");
    void refreshDetail();
  };

  const proposeUpdate = async () => {
    const version = proposeVersion.trim();
    if (version === "") return;
    setBusy("propose");
    setSaveState(null);
    const { data, error } = await api.api.POST("/api/v1/plugins/installed/{id}/update", {
      params: { path: { id: install.id } },
      body: { version },
    });
    setBusy(null);
    if (error !== undefined || data === undefined) {
      const envelope = envelopeOf(error);
      setSaveState(reasonCopy(envelope.reason ?? "validation_failed", envelope.details));
      return;
    }
    // The proposal response IS the re-consent diff; the old version keeps
    // running until consented (pending columns pin the candidate).
    setProposal(data);
    setProposeVersion("");
    setSaveState(`Update proposed — ${data.current_version} keeps running until you consent.`);
    onChanged();
  };

  const consentUpdate = async () => {
    setBusy("consent");
    const result = await runEnvelopeMutation({
      describe: `consent update ${install.slug}`,
      entityId: install.id,
      buildBody: ({ idempotency_key }) => ({ idempotency_key }),
      optimistic: () => onChanged(),
      revert: () => onChanged(),
      request: (client) => client.POST("/api/v1/plugins/installed/{id}/consent", { params: { path: { id: install.id } } }),
    });
    setBusy(null);
    setSaveState(result.ok ? "Consent committed — the pending version is now running." : (result.friendly ?? null));
    void refreshDetail();
    onChanged();
  };

  const discardUpdate = async () => {
    setBusy("discard");
    const { error } = await api.api.POST("/api/v1/plugins/installed/{id}/discard-update", {
      params: { path: { id: install.id } },
    });
    setBusy(null);
    if (error !== undefined) {
      const envelope = envelopeOf(error);
      setSaveState(reasonCopy(envelope.reason ?? "validation_failed", envelope.details));
      return;
    }
    setSaveState("Pending update discarded — nothing changed.");
    void refreshDetail();
    onChanged();
  };

  const removeInstall = async () => {
    setBusy("remove");
    const { error } = await api.api.DELETE("/api/v1/plugins/installed/{id}", {
      params: { path: { id: install.id } },
    });
    setBusy(null);
    setConfirmRemove(false);
    if (error !== undefined) {
      const envelope = envelopeOf(error);
      setSaveState(reasonCopy(envelope.reason ?? "validation_failed", envelope.details));
      return;
    }
    onChanged();
  };

  const schema = readSchema(settings?.settings_schema);
  const value = (key: string): unknown => config[key];
  const setValue = (key: string, next: unknown) => setConfig((prev) => ({ ...prev, [key]: next }));

  return (
    <div className="grid gap-5 text-[13px]" data-testid="install-detail-root">
      {/* Lifecycle posture: enabled/disabled toggling stays explicit. */}
      <Card>
        <div className="flex flex-wrap items-center gap-2 p-4">
          <h3 className="text-sm font-semibold">Lifecycle</h3>
          <Badge tone={install.status === "enabled" ? "success" : "warning"}>{install.status}</Badge>
          {canManage ? (
            <Button
              size="sm"
              variant={install.status === "enabled" ? "secondary" : "primary"}
              onClick={() => void toggleStatus()}
              disabled={busy !== null}
              data-testid="install-toggle"
            >
              {busy === "toggle" ? <Spinner /> : null}
              {install.status === "enabled" ? "Disable" : "Enable"}
            </Button>
          ) : (
            <span style={{ color: "var(--m-text-muted)" }}>Enable/disable needs owner/admin.</span>
          )}
        </div>
      </Card>

      {/* Update / rollback re-consent */}
      <Card>
        <h3 className="p-4 pb-1 text-sm font-semibold">Version · update / rollback</h3>
        <div className="p-4 pt-2">
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Running version <span className="font-mono">{install.version}</span>. An update pins
            the pending version while the old one keeps running — consent is what flips it.
            Rolling back is the same re-consent path aimed at an older version.
          </p>
          {proposal !== null && proposal.pending_version !== null ? (
            <div className="mt-3" data-testid="pending-update">
              <p className="font-semibold">
                Pending update: <span className="font-mono">{proposal.pending_version}</span>
              </p>
              <div className="mt-2">
                <ConsentSummary artifact={consentArtifactFromProposal(proposal)} diff={proposal.consent_diff} />
              </div>
              {canManage ? (
                <div className="mt-3 flex flex-wrap gap-2">
                  <Button size="sm" variant="primary" onClick={() => void consentUpdate()} disabled={busy !== null}>
                    Consent to {proposal.pending_version}
                  </Button>
                  <Button size="sm" variant="secondary" onClick={() => void discardUpdate()} disabled={busy !== null}>
                    Discard
                  </Button>
                </div>
              ) : (
                <p className="mt-2" style={{ color: "var(--m-text-muted)" }}>
                  Owner/admin consent required for the swap.
                </p>
              )}
            </div>
          ) : canManage ? (
            <div className="mt-3 flex flex-wrap items-end gap-2">
              <div className="w-36">
                <Label htmlFor={`propose-${install.id}`}>Target version</Label>
                <Input
                  id={`propose-${install.id}`}
                  placeholder="1.2.0"
                  value={proposeVersion}
                  onChange={(e) => setProposeVersion(e.target.value)}
                />
              </div>
              <Button size="sm" variant="secondary" onClick={() => void proposeUpdate()} disabled={busy !== null || proposeVersion.trim() === ""}>
                {busy === "propose" ? <Spinner /> : null} Propose update
              </Button>
            </div>
          ) : null}
        </div>
      </Card>

      {/* Settings — manifest-rendered */}
      <Card>
        <div className="flex items-center justify-between p-4 pb-1">
          <h3 className="text-sm font-semibold">Settings</h3>
          {schema === null && settingsLoad === "ready" ? (
            <Badge tone="neutral" data-testid="settings-less-posture">
              consent/enable/disable only
            </Badge>
          ) : null}
        </div>
        <div className="p-4 pt-2">
          {settingsLoad === "loading" ? (
            <Spinner />
          ) : schema === null ? (
            <p style={{ color: "var(--m-text-secondary)" }} data-testid="settings-less-copy">
              This install declares no settings in its manifest — nothing to
              configure here. The lifecycle posture is consent, enable and
              disable only. Per the contract, editing settings on a
              settings-less install is refused with a typed 409.
            </p>
          ) : (
            <div className="grid gap-3">
              <SettingsSchemaBlock schema={settings?.settings_schema} />
              {Object.entries(schema.properties ?? {}).map(([key, leaf]) => (
                <div key={key}>
                  <Label htmlFor={`cfg-${install.id}-${key}`}>
                    {leaf.title ?? key}
                    {(schema.required ?? []).includes(key) ? " (required)" : ""}
                  </Label>
                  {leaf.enum !== undefined && leaf.enum !== null ? (
                    <Select
                      value={typeof value(key) === "string" ? (value(key) as string) : ""}
                      onValueChange={(v) => setValue(key, v)}
                    >
                      <SelectTrigger id={`cfg-${install.id}-${key}`} className="w-full" aria-label={leaf.title ?? key}>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {leaf.enum.filter((option) => option !== undefined).map((option) => (
                          <SelectItem key={String(option)} value={String(option)}>
                            {String(option)}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  ) : leaf.type === "boolean" ? (
                    <label className="flex items-center gap-2">
                      <input
                        id={`cfg-${install.id}-${key}`}
                        type="checkbox"
                        checked={value(key) === true}
                        onChange={(e) => setValue(key, e.target.checked)}
                      />
                      <span>{leaf.description ?? key}</span>
                    </label>
                  ) : (
                    <Input
                      id={`cfg-${install.id}-${key}`}
                      type={leaf.type === "number" || leaf.type === "integer" ? "number" : "text"}
                      value={value(key) === undefined || value(key) === null ? "" : String(value(key))}
                      onChange={(e) =>
                        setValue(
                          key,
                          leaf.type === "number" || leaf.type === "integer"
                            ? e.target.value === "" ? "" : Number(e.target.value)
                            : e.target.value,
                        )
                      }
                    />
                  )}
                  {leaf.description !== undefined && leaf.type !== "boolean" ? (
                    <p className="mt-0.5 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                      {leaf.description}
                    </p>
                  ) : null}
                </div>
              ))}
              {canManage ? (
                <Button size="sm" variant="primary" onClick={() => void saveSettings()} disabled={busy !== null}>
                  {busy === "settings" ? <Spinner /> : null} Save settings
                </Button>
              ) : (
                <p style={{ color: "var(--m-text-muted)" }}>Owner/admin permission required to change settings.</p>
              )}
            </div>
          )}
        </div>
      </Card>

      {/* Provenance chain */}
      <Card>
        <h3 className="p-4 pb-1 text-sm font-semibold">Provenance chain</h3>
        <div className="p-4 pt-2">
          {provenance === null ? (
            <Spinner />
          ) : provenance.length === 0 ? (
            <EmptyState message="No provenance recorded for this install." />
          ) : (
            <ol className="grid gap-2" data-testid="provenance-chain">
              {provenance.map((entry) => (
                <li key={entry.id} className="flex flex-wrap items-baseline gap-x-2 text-[12px]">
                  <Badge tone="neutral">{entry.action}</Badge>
                  <span style={{ color: "var(--m-text-secondary)" }}>{entry.source_kind}</span>
                  {entry.fetched_ref !== null && entry.fetched_ref !== undefined ? (
                    <span className="font-mono" style={{ color: "var(--m-text-muted)" }}>
                      {entry.fetched_ref}
                    </span>
                  ) : null}
                  <span className="font-mono" style={{ color: "var(--m-text-muted)" }}>
                    {entry.artifact_digest.slice(0, 12)}…
                  </span>
                  <span style={{ color: "var(--m-text-muted)" }}><LocalTime value={entry.recorded_at} kind="datetime" /></span>
                </li>
              ))}
            </ol>
          )}
        </div>
      </Card>

      {saveState !== null ? (
        <p role="status" className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {saveState}
        </p>
      ) : null}

      <Card>
        <h3 className="p-4 pb-1 text-sm font-semibold">Removal</h3>
        <div className="p-4 pt-2">
          <p style={{ color: "var(--m-text-secondary)" }}>
            Uninstall is terminal — a removed install cannot be re-registered into its
            lifecycle. Data the plugin wrote lives in your primary data and survives
            uninstall, and the provenance chain stays readable afterwards.
          </p>
          {canManage ? (
            <Button size="sm" variant="destructive" onClick={() => setConfirmRemove(true)}>
              Uninstall {install.name}
            </Button>
          ) : (
            <p className="mt-1" style={{ color: "var(--m-text-muted)" }}>
              Owner/admin permission required to uninstall.
            </p>
          )}
        </div>
      </Card>

      <Dialog open={confirmRemove} onOpenChange={setConfirmRemove}>
        <DialogContent
          title={`Uninstall ${install.name}?`}
          description="Uninstall is terminal: the install record cannot come back. Written data lives in primary data and is not deleted with the plugin; the provenance chain remains readable post-uninstall."
        >
          <div className="p-4">
            <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              Running version <span className="font-mono">{install.version}</span> — it stops
              running immediately. This cannot be reversed from this surface.
            </p>
            <DialogActions>
              <Button variant="secondary" onClick={() => setConfirmRemove(false)}>
                Keep running
              </Button>
              <Button variant="destructive" onClick={() => void removeInstall()} disabled={busy === "remove"}>
                {busy === "remove" ? <Spinner /> : null} Uninstall permanently
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** Shapes an UpdateProposalView into the shared consent artifact view so
 * the same ConsentSummary renders for re-consent. */
function consentArtifactFromProposal(proposal: UpdateProposalView) {
  return {
    artifact_kind: "plugin",
    artifact_digest: proposal.pending_digest ?? proposal.slug,
    declared_capabilities: proposal.current_granted_capabilities,
    fetched_ref: null,
    // An update proposal arrives through the consent commit, not the registry
    // adapter, so no signature the trust set verified covers this manifest.
    // The weaker claim is the honest one, and `pending-update` is not a signed
    // source, so the consent screen shows no coverage badge.
    manifest_authenticated: false,
    source_kind: "pending-update",
    source_url: proposal.slug,
  };
}
