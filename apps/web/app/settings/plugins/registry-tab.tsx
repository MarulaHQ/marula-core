// /settings/plugins — registry browse + install (ADR 0009 sources).
//
// Purpose: the public marketplace-registry client: catalog browse
// (name/publisher/latest version/description, revoked entries pinned as
// not installable) with direct install for owner/admins, and the
// "stage from a registry URL" path any member can use — which routes the
// import through the staged-approvals queue with full consent review.
//
// Non-purpose: no re-distribution/publishing UI (catalog publish stays a
// core-api credential flow) and no ratings/attribution beyond the pin.
"use client";

import type { components } from "@marula-core/sdk";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  Input,
  Label,
  Spinner,
} from "@marula-core/ui";
import { useState } from "react";
import { api } from "../../../lib/api";
import { envelopeOf, reasonCopy } from "../../../lib/error-copy";

type CatalogEntryView = components["schemas"]["CatalogEntryView"];
type InstallView = components["schemas"]["InstallView"];

export function RegistryTab({
  catalog,
  installs,
  canManage,
  onChanged,
}: {
  catalog: CatalogEntryView[] | null;
  installs: InstallView[] | null;
  canManage: boolean;
  onChanged: () => void;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [registryUrl, setRegistryUrl] = useState("");
  const [installed, setInstalled] = useState<InstallView[] | null>(installs);

  const installedSlugs = new Set((installed ?? installs ?? []).map((i) => i.slug));

  const installEntry = async (entry: CatalogEntryView) => {
    setBusy(`install-${entry.slug}`);
    setError(null);
    setInstalled(installed ?? installs);
    const { data, error } = await api.api.POST("/api/v1/plugins/installed", {
      body: { slug: entry.slug, version: entry.latest_version ?? null },
    });
    setBusy(null);
    if (error !== undefined || data === undefined) {
      setError(reasonCopy(envelopeOf(error).reason ?? "conflict", envelopeOf(error).details));
      return;
    }
    // Optimistic row then authoritative reconcile (the response IS the
    // created install, created DISABLED until consent+enable).
    setInstalled((prev) => [...(prev ?? []), data]);
    setNotice(`${entry.name} installed — it starts DISABLED; enable it from the Installed tab.`);
    onChanged();
  };

  const stageFromRegistry = async () => {
    setBusy("stage-registry");
    setError(null);
    const { error } = await api.api.POST("/api/v1/plugins/distribution/staged/registry", {
      body: { url: registryUrl.trim() },
    });
    setBusy(null);
    if (error !== undefined) {
      const envelope = envelopeOf(error);
      setError(reasonCopy(envelope.reason ?? "validation_failed", envelope.details));
      return;
    }
    setRegistryUrl("");
    setNotice("Staged from the registry URL — waiting in the Approvals queue.");
    onChanged();
  };

  return (
    <div className="grid gap-3">
      <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
        The public marketplace registry client. Every artifact is digest-pinned and
        code-signed at a git tag; installs created here start DISABLED until consented.
      </p>

      {error !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }} data-testid="registry-error">
          {error}
        </p>
      ) : null}
      {notice !== null ? (
        <p role="status" className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {notice}
        </p>
      ) : null}

      <Card>
        <h3 className="p-4 pb-2 text-sm font-semibold">Marketplace catalog</h3>
        {catalog === null ? (
          <Spinner />
        ) : catalog.length === 0 ? (
          <div className="p-4">
            <EmptyState message="The registry catalog is empty — nothing to browse yet. The connector fixtures ride the relay/cosmos work." />
          </div>
        ) : (
          <div className="grid gap-3 p-4 pt-0" data-testid="registry-catalog">
            {catalog.map((entry) => (
              <div
                key={entry.id}
                className="flex flex-wrap items-center gap-2 rounded-[var(--m-radius-container)] border p-3"
                style={{ borderColor: "var(--m-border)" }}
              >
                <span className="font-mono text-[12px]">{entry.slug}</span>
                <span className="text-[13px] font-semibold">{entry.name}</span>
                {entry.latest_version !== null && entry.latest_version !== undefined ? (
                  <Badge tone="neutral">v{entry.latest_version}</Badge>
                ) : null}
                <Badge tone={entry.revoked ? "danger" : "success"}>{entry.revoked ? "revoked" : "available"}</Badge>
                <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  by {entry.publisher}
                </span>
                <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  {entry.description}
                </span>
                {installedSlugs.has(entry.slug) ? (
                  <Badge tone="neutral" className="ml-auto">
                    installed
                  </Badge>
                ) : entry.revoked || !canManage ? (
                  <Badge tone="neutral" className="ml-auto">
                    {entry.revoked ? "cannot install" : "install needs admin"}
                  </Badge>
                ) : (
                  <Button
                    size="sm"
                    variant="primary"
                    className="ml-auto"
                    disabled={busy !== null}
                    onClick={() => void installEntry(entry)}
                  >
                    {busy === `install-${entry.slug}` ? <Spinner /> : null} Install
                  </Button>
                )}
              </div>
            ))}
          </div>
        )}
      </Card>

      <Card>
        <h3 className="p-4 pb-2 text-sm font-semibold">Stage from a registry URL</h3>
        <div className="grid gap-3 p-4 pt-0" data-testid="registry-stage-form">
          <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            Paste a pinned registry document URL (a BUILD-PROMPT v1 catalog entry served from a
            git tag). Imports staged this way go through the same consent review as any upload.
          </p>
          <div>
            <Label htmlFor="registry-url">Registry document URL</Label>
            <Input id="registry-url" placeholder="https://…/collector/v1.0.0.json" value={registryUrl} onChange={(e) => setRegistryUrl(e.target.value)} />
          </div>
          <Button variant="secondary" disabled={busy !== null || registryUrl.trim() === ""} onClick={() => void stageFromRegistry()}>
            {busy === "stage-registry" ? <Spinner /> : null} Stage import
          </Button>
        </div>
      </Card>
    </div>
  );
}
