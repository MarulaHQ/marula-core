// LabelDesigner — template CRUD + live preview + print test (Wave 32 lane 5,
// `p32-labels-scanning`).
//
// The designer edits a template and shows the SERVER's render of it against
// deterministic sample values (debounced). One renderer means the preview,
// the browser print and the raw ZPL can never disagree; this component owns
// no layout math at all.
//
// Print targets in doctrine order: the browser dialog everywhere, raw ZPL
// through the Tauri shell when it is present. On plain web the ZPL button
// says so instead of pretending.

"use client";

import type { components } from "@marula-core/sdk";
import {
  Button,
  Card,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
} from "@marula-core/ui";
import { useEffect, useMemo, useRef, useState } from "react";

import { api } from "../../lib/api";
import { newIdempotencyKey } from "../../lib/instant/instant-write";
import {
  ALLOWED_TOKENS,
  BARCODE_TOKENS,
  DPI_OPTIONS,
  SIZE_PRESETS,
  type LabelRender,
  type LabelTemplate,
  tokenLabel,
} from "../../lib/labels/model";
import { loadPrinterAddress, printLabelViaBrowser, savePrinterAddress, sendZpl } from "../../lib/labels/print";
import { LabelPreview } from "./label-preview";

type PreviewLabelRequest = components["schemas"]["PreviewLabelRequest"];

/** Pulls the server's validation sentence out of an openapi-fetch error. */
function errorMessage(error: unknown, fallback: string): string {
  if (error !== null && typeof error === "object") {
    const body = error as { message?: unknown; error?: { message?: unknown } };
    if (typeof body.error?.message === "string") return body.error.message;
    if (typeof body.message === "string") return body.message;
  }
  return fallback;
}

interface Draft {
  name: string;
  purpose: string;
  size_preset: string;
  widthMm: string;
  heightMm: string;
  dpi: number;
  barcodeToken: string; // "" = none
  includeQr: boolean;
  fields: string[];
  isDefault: boolean;
}

function draftFrom(template: LabelTemplate | null): Draft {
  if (template === null) {
    return {
      name: "",
      purpose: "stock_item",
      size_preset: "zebra_2x1",
      widthMm: "60",
      heightMm: "30",
      dpi: 203,
      barcodeToken: "sku",
      includeQr: true,
      fields: ["name", "sku"],
      isDefault: false,
    };
  }
  return {
    name: template.name,
    purpose: template.purpose,
    size_preset: template.size_preset,
    widthMm: String(template.width_mm),
    heightMm: String(template.height_mm),
    dpi: template.dpi,
    barcodeToken: template.barcode_token ?? "",
    includeQr: template.include_qr,
    fields: template.fields,
    isDefault: template.is_default,
  };
}

function previewBody(draft: Draft): PreviewLabelRequest {
  const custom = draft.size_preset === "custom";
  return {
    name: draft.name.trim() === "" ? "Preview" : draft.name.trim(),
    purpose: draft.purpose,
    size_preset: draft.size_preset,
    width_mm: custom ? Number(draft.widthMm) : undefined,
    height_mm: custom ? Number(draft.heightMm) : undefined,
    dpi: draft.dpi,
    barcode_token: draft.barcodeToken === "" ? null : draft.barcodeToken,
    include_qr: draft.includeQr,
    fields: draft.fields,
  };
}

export function LabelDesigner({
  template,
  onSaved,
  onArchived,
  onDirtyChange,
}: {
  template: LabelTemplate | null;
  onSaved: (template: LabelTemplate) => void;
  onArchived: (template: LabelTemplate) => void;
  onDirtyChange?: (dirty: boolean) => void;
}): React.ReactNode {
  const [draft, setDraft] = useState<Draft>(() => draftFrom(template));
  const [render, setRender] = useState<LabelRender | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [printer, setPrinter] = useState("");
  const previewSeq = useRef(0);

  useEffect(() => {
    setDraft(draftFrom(template));
    setStatus(null);
  }, [template]);

  useEffect(() => {
    setPrinter(loadPrinterAddress());
  }, []);

  const allowed = ALLOWED_TOKENS[draft.purpose] ?? [];
  const barcodeChoices = BARCODE_TOKENS[draft.purpose] ?? [];
  const unusedTokens = useMemo(
    () => allowed.filter((token) => !draft.fields.includes(token)),
    [allowed, draft.fields],
  );

  // Live preview: debounced, last-write-wins (stale responses are dropped).
  useEffect(() => {
    const seq = ++previewSeq.current;
    const handle = window.setTimeout(async () => {
      const { data, error } = await api.api.POST("/api/v1/label-templates/preview", {
        body: previewBody(draft),
      });
      if (seq !== previewSeq.current) return;
      if (error !== undefined || data === undefined) {
        setRender(null);
        setPreviewError(
          errorMessage(error, "This draft cannot be rendered — check the field tokens and size."),
        );
        return;
      }
      setPreviewError(null);
      setRender(data as LabelRender);
    }, 250);
    return () => window.clearTimeout(handle);
  }, [draft]);

  const dirty = useMemo(() => {
    const base = draftFrom(template);
    return JSON.stringify(base) !== JSON.stringify(draft);
  }, [draft, template]);

  useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);

  const setFieldToken = (index: number, token: string) => {
    setDraft((prev) => {
      const fields = [...prev.fields];
      fields[index] = token;
      return { ...prev, fields };
    });
  };

  const moveField = (index: number, delta: number) => {
    setDraft((prev) => {
      const target = index + delta;
      if (target < 0 || target >= prev.fields.length) return prev;
      const fields = [...prev.fields];
      const [item] = fields.splice(index, 1);
      if (item === undefined) return prev;
      fields.splice(target, 0, item);
      return { ...prev, fields };
    });
  };

  const save = async () => {
    setBusy(true);
    setStatus(null);
    const custom = draft.size_preset === "custom";
    const body = {
      name: draft.name.trim(),
      purpose: draft.purpose,
      size_preset: draft.size_preset,
      width_mm: custom ? Number(draft.widthMm) : undefined,
      height_mm: custom ? Number(draft.heightMm) : undefined,
      dpi: draft.dpi,
      barcode_token: draft.barcodeToken === "" ? null : draft.barcodeToken,
      include_qr: draft.includeQr,
      fields: draft.fields,
      is_default: draft.isDefault,
      idempotency_key: newIdempotencyKey(),
    };
    const result =
      template === null
        ? await api.api.POST("/api/v1/label-templates", { body })
        : await api.api.PATCH("/api/v1/label-templates/{id}", {
            params: { path: { id: template.id } },
            body,
          });
    setBusy(false);
    if (result.error !== undefined || result.data === undefined) {
      setStatus(errorMessage(result.error, "The template was not saved."));
      return;
    }
    const saved = result.data as LabelTemplate;
    setStatus(`Saved “${saved.name}”.`);
    onSaved(saved);
  };

  const archive = async () => {
    if (template === null) return;
    setBusy(true);
    const { data, error } = await api.api.POST("/api/v1/label-templates/{id}/archive", {
      params: { path: { id: template.id } },
    });
    setBusy(false);
    if (error !== undefined || data === undefined) {
      setStatus("The template was not archived.");
      return;
    }
    onArchived(data as LabelTemplate);
  };

  const printTest = () => {
    if (render === null) return;
    printLabelViaBrowser({
      label: render.label,
      qrSvg: render.qr_svg ?? null,
      code128Svg: render.code128_svg ?? null,
      title: `${draft.name.trim() === "" ? "Label" : draft.name.trim()} (test)`,
    });
  };

  const sendTestZpl = async () => {
    if (render?.zpl == null) return;
    const result = await sendZpl(render.zpl, printer);
    if (result.kind === "sent") setStatus("ZPL sent to the printer.");
    else if (result.kind === "unsupported")
      setStatus("This browser cannot reach a Zebra printer directly — the desktop app can. The print dialog path still works here.");
    else setStatus(result.message);
  };

  const custom = draft.size_preset === "custom";

  return (
    <div className="grid min-w-0 gap-3 lg:grid-cols-[minmax(0,1fr)_auto]">
      <Card className="grid min-w-0 content-start gap-3 p-4">
        <div className="grid min-w-0 gap-3 md:grid-cols-2">
          <div className="min-w-0">
            <Label htmlFor="lt-name">Template name</Label>
            <Input
              id="lt-name"
              data-testid="label-name"
              value={draft.name}
              onChange={(event) => setDraft({ ...draft, name: event.target.value })}
              placeholder="Serialized tool — 2×1"
            />
          </div>
          <div className="min-w-0">
            <Label htmlFor="lt-purpose">Purpose</Label>
            <Select
              value={draft.purpose}
              onValueChange={(purpose) => {
                const nextAllowed = ALLOWED_TOKENS[purpose] ?? [];
                const nextBarcode = BARCODE_TOKENS[purpose] ?? [];
                setDraft((prev) => ({
                  ...prev,
                  purpose,
                  fields: prev.fields.filter((token) => nextAllowed.includes(token)),
                  barcodeToken: nextBarcode.includes(prev.barcodeToken) ? prev.barcodeToken : "",
                }));
              }}
            >
              <SelectTrigger id="lt-purpose" data-testid="label-purpose" className="w-full min-w-0 overflow-hidden [&>span]:truncate">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="stock_item">Stock item</SelectItem>
                <SelectItem value="stock_location">Location bin</SelectItem>
                <SelectItem value="serialized_unit">Serialized tool / unit</SelectItem>
                <SelectItem value="device_asset">Loan device</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="min-w-0">
            <Label htmlFor="lt-size">Label stock</Label>
            <Select
              value={draft.size_preset}
              onValueChange={(size) => setDraft({ ...draft, size_preset: size })}
            >
              <SelectTrigger id="lt-size" data-testid="label-size" className="w-full min-w-0 overflow-hidden [&>span]:truncate">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {SIZE_PRESETS.map((preset) => (
                  <SelectItem key={preset.id} value={preset.id}>
                    {preset.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="min-w-0">
            <Label htmlFor="lt-dpi">Printer resolution</Label>
            <Select
              value={String(draft.dpi)}
              onValueChange={(dpi) => setDraft({ ...draft, dpi: Number(dpi) })}
            >
              <SelectTrigger id="lt-dpi" className="w-full min-w-0 overflow-hidden [&>span]:truncate">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {DPI_OPTIONS.map((dpi) => (
                  <SelectItem key={dpi} value={String(dpi)}>
                    {dpi} dpi
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          {custom ? (
            <>
              <div className="min-w-0">
                <Label htmlFor="lt-width">Width (mm)</Label>
                <Input
                  id="lt-width"
                  type="number"
                  min="10"
                  max="300"
                  step="0.1"
                  value={draft.widthMm}
                  onChange={(event) => setDraft({ ...draft, widthMm: event.target.value })}
                />
              </div>
              <div className="min-w-0">
                <Label htmlFor="lt-height">Height (mm)</Label>
                <Input
                  id="lt-height"
                  type="number"
                  min="10"
                  max="300"
                  step="0.1"
                  value={draft.heightMm}
                  onChange={(event) => setDraft({ ...draft, heightMm: event.target.value })}
                />
              </div>
            </>
          ) : null}
          <div className="min-w-0">
            <Label htmlFor="lt-barcode">Code128 encodes</Label>
            <Select
              value={draft.barcodeToken === "" ? "__none" : draft.barcodeToken}
              onValueChange={(value) =>
                setDraft({ ...draft, barcodeToken: value === "__none" ? "" : value })
              }
            >
              <SelectTrigger id="lt-barcode" data-testid="label-barcode-token" className="w-full min-w-0 overflow-hidden [&>span]:truncate">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none">No barcode</SelectItem>
                {barcodeChoices.map((token) => (
                  <SelectItem key={token} value={token}>
                    {tokenLabel(token)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <label className="flex items-center gap-2 pt-5 text-xs">
            <input
              type="checkbox"
              data-testid="label-qr-toggle"
              checked={draft.includeQr}
              onChange={(event) => setDraft({ ...draft, includeQr: event.target.checked })}
            />
            QR code with the entity deep link
          </label>
          <label className="flex items-center gap-2 pt-5 text-xs">
            <input
              type="checkbox"
              data-testid="label-default-toggle"
              checked={draft.isDefault}
              onChange={(event) => setDraft({ ...draft, isDefault: event.target.checked })}
            />
            Default for this purpose
          </label>
        </div>

        <div>
          <Label>Printed lines (top to bottom, max 6)</Label>
          <ul className="grid gap-1" data-testid="label-fields">
            {draft.fields.map((token, index) => (
              <li key={`${token}-${index}`} className="flex min-w-0 items-center gap-2">
                <div className="min-w-0 flex-1">
                  <Select value={token} onValueChange={(value) => setFieldToken(index, value)}>
                    <SelectTrigger className="w-full min-w-0 overflow-hidden [&>span]:truncate">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {allowed.map((option) => (
                        <SelectItem key={option} value={option} disabled={draft.fields.includes(option) && option !== token}>
                          {tokenLabel(option)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <Button variant="ghost" size="sm" className="shrink-0" onClick={() => moveField(index, -1)} aria-label="Move line up">
                  ↑
                </Button>
                <Button variant="ghost" size="sm" className="shrink-0" onClick={() => moveField(index, 1)} aria-label="Move line down">
                  ↓
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  className="shrink-0"
                  aria-label={`Remove ${tokenLabel(token)}`}
                  onClick={() =>
                    setDraft((prev) => ({
                      ...prev,
                      fields: prev.fields.filter((_, i) => i !== index),
                    }))
                  }
                >
                  Remove
                </Button>
              </li>
            ))}
          </ul>
          {unusedTokens.length > 0 && draft.fields.length < 6 ? (
            <Button
              variant="secondary"
              size="sm"
              className="mt-1"
              data-testid="label-add-line"
              onClick={() => {
                const next = unusedTokens[0];
                if (next !== undefined) setDraft((prev) => ({ ...prev, fields: [...prev.fields, next] }));
              }}
            >
              Add line
            </Button>
          ) : null}
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="button"
            data-testid="label-save"
            disabled={busy || draft.name.trim() === ""}
            onClick={() => void save()}
          >
            {busy ? <Spinner /> : null} {template === null ? "Create template" : "Save changes"}
          </Button>
          <Button type="button" variant="secondary" data-testid="label-print-test" disabled={render === null} onClick={printTest}>
            Print test label
          </Button>
          {template !== null && template.active ? (
            <Button type="button" variant="ghost" onClick={() => void archive()}>
              Archive
            </Button>
          ) : null}
          {status !== null ? (
            <span className="text-xs" data-testid="label-status" role="status" style={{ color: "var(--m-text-secondary)" }}>
              {status}
            </span>
          ) : null}
        </div>

        <div className="grid gap-1 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
          <Label htmlFor="lt-printer">Zebra printer (this workstation)</Label>
          <div className="flex flex-wrap items-center gap-2">
            <Input
              id="lt-printer"
              data-testid="label-printer"
              className="max-w-56"
              placeholder="10.0.0.9:9100"
              value={printer}
              onChange={(event) => setPrinter(event.target.value)}
              onBlur={() => savePrinterAddress(printer)}
            />
            <Button type="button" variant="secondary" data-testid="label-send-zpl" disabled={render?.zpl == null} onClick={() => void sendTestZpl()}>
              Send ZPL to printer
            </Button>
          </div>
          <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            The address is stored on this workstation only — a printer is desk hardware, not org configuration.
          </p>
        </div>
      </Card>

      <Card className="grid content-start gap-2 p-4" data-testid="label-preview-panel">
        <h3 className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
          Live preview (sample data)
        </h3>
        {previewError !== null ? (
          <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
            {previewError}
          </p>
        ) : render === null ? (
          <Spinner />
        ) : (
          <LabelPreview render={render} />
        )}
      </Card>
    </div>
  );
}
