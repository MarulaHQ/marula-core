// /settings/export — org data export admin (TASKLIST D1).
//
// Purpose: the dedicated admin surface over the existing backend seam
// (POST /api/v1/org/exports + the signed no-JWT download link). Owner/admin
// builds the versioned JSON bundle into the object store and gets ONE
// self-contained signed download URL — the page renders exactly what the
// server returned: bundle version, size, sha256, and the 15-minute expiry
// it computes. No client-side bundle guessing, no stored export history
// (the pins carry a single POST; creating the ledger is a later wave).
//
// RBAC: the server rejects non-owner/admin inside the handler (policy §5);
// the page mirrors that with the same client proxy the rest of the app
// uses (special:manage_users ≡ owner/admin) and still surfaces a typed
// 403 verbatim if a non-admin POSTs.
//
// Non-purpose: no migrations, no new endpoints; the file is UI-only over
// the regenerated typed SDK.
"use client";

import { friendlyMutationText, resultErrorText } from "../../../lib/error-copy";
import { Button, Card, FieldHint, Spinner } from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useState } from "react";
import { api, apiBaseUrl, useRequireAuth } from "../../../lib/api";
import { SettingsHeader, NotAvailable } from "../../../components/settings/settings-shell";

type ExportCreated = components["schemas"]["ExportCreatedResponse"];

/** Formats bytes as a stable human string (KiB/MiB) without bloat. */
function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function downloadHref(downloadUrl: string): string {
  return `${apiBaseUrl()}${downloadUrl}`;
}

export default function ExportSettingsPage() {
  const auth = useRequireAuth();
  const isOwnerOrAdmin =
    auth.me?.permissions.includes("special:manage_users") ?? false;
  const [building, setBuilding] = useState(false);
  const [exported, setExported] = useState<ExportCreated | null>(null);
  const [error, setError] = useState<string | null>(null);

  const createExport = async () => {
    setBuilding(true);
    setError(null);
    try {
      const res = await api.api.POST("/api/v1/org/exports");
      // The pin declares no error responses, so `error` is typed away —
      // any non-2xx still carries the IF3 envelope body at runtime.
      if (res.error !== undefined) {
        setExported(null);
        setError(resultErrorText(res, "export was rejected"));
        return;
      }
      if (res.data !== undefined) setExported(res.data);
    } catch (e) {
      setExported(null);
      setError(friendlyMutationText(e));
    } finally {
      setBuilding(false);
    }
  };

  const canCreate = auth.status === "authed" && isOwnerOrAdmin;

  return (
    <div className="grid gap-6 max-w-2xl">
      <SettingsHeader
        title="Export"
        description="Download a complete, versioned bundle of this organization's data as JSON. Exports are owner/admin-only and audited."
      />

      {exported !== null ? (
        <Card className="p-5 grid gap-3" data-testid="export-result">
          <div className="flex items-center justify-between gap-3">
            <h2 className="text-sm font-semibold">Export ready</h2>
            <span
              className="text-[11px] font-mono px-2 py-0.5 rounded-full"
              style={{
                color: "var(--m-text-secondary)",
                border: "1px solid var(--m-border)",
              }}
            >
              bundle v{exported.bundle_version}
            </span>
          </div>
          <dl className="grid gap-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            <div className="flex justify-between gap-2">
              <dt>Size</dt>
              <dd className="font-mono" style={{ color: "var(--m-text)" }}>
                {Number.isFinite(exported.size_bytes) ? formatSize(exported.size_bytes) : "—"}
              </dd>
            </div>
            <div className="flex justify-between gap-2">
              <dt>SHA-256</dt>
              <dd
                className="font-mono text-[12px] max-w-[20ch] truncate"
                style={{ color: "var(--m-text)" }}
                title={exported.sha256}
              >
                {exported.sha256 || "—"}
              </dd>
            </div>
            <div className="flex justify-between gap-2">
              <dt>Link expires</dt>
              <dd className="font-mono" style={{ color: "var(--m-text)" }}>
                {Number.isNaN(new Date(exported.expires_at).getTime())
                  ? "—"
                  : new Date(exported.expires_at).toLocaleTimeString()}
              </dd>
            </div>
          </dl>
          <a
            href={downloadHref(exported.download_url)}
            data-testid="export-download"
            className="inline-block"
            // The signed URL authorizes exactly one (org, export, expiry) —
            // download through the anchor so the browser saves the file.
            download
          >
            <Button variant="primary">Download bundle</Button>
          </a>
          <FieldHint>
            The signed link works standalone for {15} minutes — after that,
            build the bundle again. Old links expire on their own; nothing
            else needs revocation.
          </FieldHint>
        </Card>
      ) : (
        <Card className="p-5 grid gap-3">
          <h2 className="text-sm font-semibold">Build export bundle</h2>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Gathering the org's tickets, work, finance, taxonomy, and settings
            into one signed download. This runs server-wide for the org and
            appears in the audit log.
          </p>
          {canCreate ? (
            <div>
              <Button
                variant="primary"
                data-testid="export-create"
                disabled={building}
                onClick={() => void createExport()}
              >
                {building ? "Building…" : "Generate export"}
              </Button>
              {building ? (
                <span className="ml-3 inline-flex items-center gap-2 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  <Spinner /> Collecting org data…
                </span>
              ) : null}
            </div>
          ) : (
            <NotAvailable
              what={`Export for your role (${auth.me?.display_name ?? "non-admin"})`}
              hint="org data export is limited to org admins"
            />
          )}
          {error !== null && (
            <p
              role="alert"
              data-testid="export-error"
              className="text-[12px]"
              style={{ color: "var(--m-status-danger-fg)" }}
            >
              {error}
            </p>
          )}
        </Card>
      )}
    </div>
  );
}
