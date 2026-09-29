// /settings/labels — label templates + the shared scan contract (Wave 32
// lane 5, `p32-labels-scanning`).
//
// Purpose: the org's label library. Templates are typed, org-scoped print
// recipes per purpose (stock item, location bin, serialized unit, loan
// device); the designer previews through the core's ONE renderer and marks
// one default per purpose. Template CRUD lives here — in the settings IA —
// with its own endpoints, because the settings registry carries scalars by
// design and a template is an ordered token list (see the landing record).
//
// The scan bar at the top is the same shared component every operational
// surface uses: it resolves a scanned code into a typed entity and shows
// what the scan MEANS, which is also how you test a freshly printed label
// without leaving the designer.
//
// Non-purpose: no per-entity printing here (entity surfaces carry the
// print button); no printer address registry (the address is workstation
// hardware and stays in local storage).

"use client";

import { Button, Card, EmptyState, Spinner, StatusTag } from "@marula-core/ui";
import { useCallback, useEffect, useState } from "react";

import { LabelDesigner } from "../../../components/labels/label-designer";
import { ScanBar } from "../../../components/scan/scan-bar";
import { api, useRequireAuth } from "../../../lib/api";
import type { LabelTemplate } from "../../../lib/labels/model";
import { purposeLabel, tokenLabel } from "../../../lib/labels/model";
import { canManageOnboarding } from "../../../lib/onboarding";
import { scanSummary } from "../../../lib/scan/routing";
import { SettingsHeader } from "../../../components/settings/settings-shell";

export default function LabelsSettingsPage(): React.ReactNode {
  const auth = useRequireAuth();
  const [templates, setTemplates] = useState<LabelTemplate[] | null>(null);
  const [selected, setSelected] = useState<LabelTemplate | null>(null);
  const [creating, setCreating] = useState(false);
  const [scanLine, setScanLine] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const { data, error: err } = await api.api.GET("/api/v1/label-templates", {
      params: { query: { active_only: true } },
    });
    if (err !== undefined || data === undefined) {
      setError("The label templates could not be loaded.");
      setTemplates([]);
      return;
    }
    setTemplates(data as LabelTemplate[]);
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
  }, [auth.status, refresh]);

  const onSaved = (saved: LabelTemplate) => {
    setCreating(false);
    void refresh().then(() => setSelected(saved));
  };
  const onArchived = () => {
    setSelected(null);
    void refresh();
  };

  const isAdmin = canManageOnboarding(auth.me);

  return (
    <div>
      <SettingsHeader
        title="Labels & scanning"
        description="Typed label templates per purpose, printed through the browser dialog everywhere and as raw ZPL to networked Zebra printers from the desktop app. A printed QR scans back to the exact record."
      />

      <div className="grid gap-3">
        <Card className="p-4">
          <ScanBar
            surface="stock-locations"
            hint="Test a label: scan its QR or type a SKU, serial, location code or asset tag, then press Enter."
            onScan={({ resolution }) => setScanLine(scanSummary(resolution))}
            onRejected={(message) => setScanLine(message)}
          />
          {scanLine !== null ? (
            <p className="mt-2 text-xs" data-testid="labels-scan-result" style={{ color: "var(--m-text-secondary)" }}>
              {scanLine}
            </p>
          ) : null}
        </Card>

        {error !== null ? (
          <p role="alert" className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>
            {error}
          </p>
        ) : null}

        {templates === null ? (
          <div className="flex justify-center p-6">
            <Spinner />
          </div>
        ) : (
          <div className="grid gap-3 lg:grid-cols-[320px_minmax(0,1fr)]">
            <Card className="content-start p-3">
              <div className="mb-2 flex items-center justify-between">
                <h2 className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                  Templates
                </h2>
                {isAdmin ? (
                  <Button
                    size="sm"
                    data-testid="label-new"
                    onClick={() => {
                      setCreating(true);
                      setSelected(null);
                    }}
                  >
                    New template
                  </Button>
                ) : null}
              </div>
              {templates.length === 0 ? (
                <EmptyState message="No label templates yet. Create one for each kind of label you print: stock items, bins, tools, loan devices." />
              ) : (
                <ul className="grid gap-1" data-testid="label-template-list">
                  {templates.map((template) => (
                    <li key={template.id}>
                      <button
                        type="button"
                        data-testid="label-template-row"
                        aria-current={selected?.id === template.id}
                        className="w-full border px-2 py-1.5 text-left"
                        style={{
                          borderColor: "var(--m-border)",
                          borderRadius: "var(--m-radius-control)",
                          background: selected?.id === template.id ? "var(--m-accent-soft, var(--m-surface))" : "var(--m-surface)",
                          outline: selected?.id === template.id ? "2px solid var(--m-accent)" : "none",
                          outlineOffset: "1px",
                        }}
                        onClick={() => {
                          setCreating(false);
                          setSelected(template);
                        }}
                      >
                        <span className="flex items-center justify-between gap-2">
                          <span className="text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
                            {template.name}
                          </span>
                          {template.is_default ? <StatusTag tone="success">default</StatusTag> : null}
                        </span>
                        <span className="mt-0.5 block text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                          {purposeLabel(template.purpose)} · {template.width_mm} × {template.height_mm} mm ·{" "}
                          {template.fields.map(tokenLabel).join(" / ")}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </Card>

            {!isAdmin ? (
              <Card className="p-4">
                <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                  Label templates are edited by owners and admins. You can print labels from the records themselves.
                </p>
              </Card>
            ) : creating || selected !== null ? (
              <LabelDesigner
                key={creating ? "new" : (selected?.id ?? "none")}
                template={creating ? null : selected}
                onSaved={onSaved}
                onArchived={onArchived}
              />
            ) : (
              <Card className="p-4">
                <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                  Pick a template to edit, or create a new one. The preview uses sample values, and the QR always
                  encodes the record&rsquo;s deep link — never free text.
                </p>
              </Card>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
