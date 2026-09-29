// LabelPrintButton — print a label for a REAL entity, from any surface
// (Wave 32 lane 5, `p32-labels-scanning`).
//
// Drop this on an entity surface with its purpose + id: it finds the org's
// default template for that purpose, asks the core to render the label
// (values resolved server-side, QR = the entity's deep link), and offers
// the two print targets — the browser dialog, or raw ZPL through the
// desktop shell when it is present. With no default template it says so
// and links to the designer instead of inventing a label.

"use client";

import type { components } from "@marula-core/sdk";
import { api } from "../../lib/api";
import { Button, Spinner } from "@marula-core/ui";
import Link from "next/link";
import { useState } from "react";

import type { LabelRender, LabelTemplate } from "../../lib/labels/model";
import { loadPrinterAddress, printLabelViaBrowser, sendZpl } from "../../lib/labels/print";
import { LabelPreview } from "./label-preview";

type LabelTemplateView = components["schemas"]["LabelTemplateView"];

export function LabelPrintButton({
  purpose,
  entityId,
  entityLabel,
  size = "sm",
}: {
  purpose: string;
  entityId: string;
  entityLabel?: string;
  size?: "sm" | "md";
}): React.ReactNode {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [render, setRender] = useState<LabelRender | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [printer, setPrinter] = useState("");

  const load = async () => {
    setBusy(true);
    setNote(null);
    setRender(null);
    setPrinter(loadPrinterAddress());
    const list = await api.api.GET("/api/v1/label-templates", {
      params: { query: { purpose, active_only: true } },
    });
    if (list.error !== undefined || list.data === undefined) {
      setBusy(false);
      setNote("The label templates could not be loaded.");
      return;
    }
    const templates = list.data as LabelTemplateView[];
    const template: LabelTemplateView | undefined =
      templates.find((candidate) => candidate.is_default) ?? undefined;
    if (template === undefined) {
      setBusy(false);
      setNote(
        `No default template for this kind of label yet. Create one under Settings → Labels & scanning and mark it default for “${purpose.replaceAll("_", " ")}”.`,
      );
      return;
    }
    const rendered = await api.api.POST("/api/v1/labels/render", {
      body: { template_id: template.id, entity_id: entityId },
    });
    setBusy(false);
    if (rendered.error !== undefined || rendered.data === undefined) {
      setNote("The label could not be rendered for this record.");
      return;
    }
    setRender(rendered.data as LabelRender);
  };

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next) void load();
  };

  return (
    <div className="grid gap-2">
      <Button
        type="button"
        variant="secondary"
        size={size}
        data-testid="label-print-button"
        onClick={toggle}
        aria-expanded={open}
      >
        {busy ? <Spinner /> : null} Print label
      </Button>
      {open ? (
        <div
          className="grid gap-2 border p-3"
          data-testid="label-print-panel"
          style={{ borderColor: "var(--m-border)", borderRadius: "var(--m-radius-control)" }}
        >
          {note !== null ? (
            <p className="text-xs" role="status" style={{ color: "var(--m-text-secondary)" }}>
              {note}{" "}
              {note.startsWith("No default template") ? (
                <Link href="/settings/labels" style={{ color: "var(--m-accent)" }}>
                  Open Labels &amp; scanning
                </Link>
              ) : null}
            </p>
          ) : null}
          {render !== null ? (
            <>
              <LabelPreview render={render} />
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  size="sm"
                  data-testid="label-browser-print"
                  onClick={() =>
                    printLabelViaBrowser({
                      label: render.label,
                      qrSvg: render.qr_svg ?? null,
                      code128Svg: render.code128_svg ?? null,
                      title: entityLabel ?? render.entity.display_name,
                    })
                  }
                >
                  Print (browser dialog)
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="secondary"
                  data-testid="label-zpl-print"
                  disabled={render.zpl == null}
                  onClick={async () => {
                    if (render.zpl == null) return;
                    const result = await sendZpl(render.zpl, printer);
                    if (result.kind === "sent") setNote("ZPL sent to the printer.");
                    else if (result.kind === "unsupported")
                      setNote(
                        "Direct ZPL printing needs the desktop app; here the browser dialog is the honest path.",
                      );
                    else setNote(result.message);
                  }}
                >
                  Send ZPL to Zebra
                </Button>
              </div>
              <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                Printer {printer === "" ? "not set — add the address under Settings → Labels & scanning" : printer} ·
                QR scans back to this record.
              </p>
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
