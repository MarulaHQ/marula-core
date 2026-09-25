// LabelPreview — the label as the print target will draw it (Wave 32 lane 5,
// `p32-labels-scanning`).
//
// Renders the SERVER's resolved geometry (millimetres) as CSS millimetres,
// so what the operator sees is what the printer gets — the same model the
// ZPL renderer consumes. The QR + Code128 arrive as inline SVG from the
// core and are drawn as images (never injected as markup).
//
// A label is a print medium: its own white stock and black ink, at its real
// physical size — not a themed surface. The surrounding chrome follows the
// app theme.

"use client";

import type { LabelRender } from "../../lib/labels/model";
import { LABEL_PADDING_MM, qrPositionMm } from "../../lib/labels/model";
import { svgDataUri } from "../../lib/labels/print";

export function LabelPreview({
  render,
  showWarnings = true,
}: {
  render: LabelRender;
  showWarnings?: boolean;
}): React.ReactNode {
  const { label, qr_svg, code128_svg } = render;
  const qr = qrPositionMm(label);
  return (
    <div data-testid="label-preview" className="grid justify-start gap-1">
      <div
        data-testid="label-preview-surface"
        style={{
          position: "relative",
          width: `${label.width_mm}mm`,
          height: `${label.height_mm}mm`,
          background: "white",
          color: "black",
          border: "1px solid var(--m-border)",
          borderRadius: "2px",
          overflow: "hidden",
          fontFamily: "Inter, system-ui, sans-serif",
        }}
      >
        {label.lines.map((line) => (
          <div
            key={`${line.token}-${line.y_mm}`}
            data-testid={`label-line-${line.token}`}
            style={{
              position: "absolute",
              left: `${LABEL_PADDING_MM}mm`,
              right: `${LABEL_PADDING_MM}mm`,
              top: `${line.y_mm}mm`,
              height: `${line.height_mm}mm`,
              fontSize: `${line.font_mm.toFixed(2)}mm`,
              lineHeight: 1.05,
              fontWeight: 600,
              whiteSpace: "nowrap",
              overflow: "hidden",
            }}
          >
            {line.text}
          </div>
        ))}
        {label.barcode != null ? (
          <div
            data-testid="label-barcode"
            style={{
              position: "absolute",
              left: `${LABEL_PADDING_MM}mm`,
              width: `${label.barcode.width_mm}mm`,
              top: `${label.barcode.y_mm}mm`,
              height: `${label.barcode.height_mm}mm`,
            }}
          >
            {code128_svg != null ? (
              <img alt="" src={svgDataUri(code128_svg)} style={{ width: "100%", height: "100%", display: "block" }} />
            ) : (
              <span style={{ fontFamily: "var(--font-mono)", fontSize: "2.2mm" }}>
                {label.barcode.data}
              </span>
            )}
          </div>
        ) : null}
        {qr != null && label.qr != null ? (
          <div
            data-testid="label-qr"
            style={{
              position: "absolute",
              left: `${qr.xMm}mm`,
              top: `${qr.yMm}mm`,
              width: `${label.qr.size_mm}mm`,
              height: `${label.qr.size_mm}mm`,
            }}
          >
            {qr_svg != null ? (
              <img alt="" src={svgDataUri(qr_svg)} style={{ width: "100%", height: "100%", display: "block" }} />
            ) : (
              <span style={{ fontFamily: "var(--font-mono)", fontSize: "1.6mm" }}>{label.qr.data}</span>
            )}
          </div>
        ) : null}
      </div>
      <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
        {label.width_mm} × {label.height_mm} mm · {label.dpi} dpi ·{" "}
        {render.sample ? "sample values" : render.entity.display_name}
      </p>
      {showWarnings && label.lines.some((line) => line.truncated) ? (
        <p
          data-testid="label-truncation-warning"
          className="text-[11px]"
          style={{ color: "var(--m-status-warning-fg)" }}
        >
          {label.lines.filter((line) => line.truncated).length === 1
            ? "One line is too long for this size even at the smallest font — its tail will not print."
            : `${label.lines.filter((line) => line.truncated).length} lines are too long for this size even at the smallest font — their tails will not print.`}
        </p>
      ) : null}
      {showWarnings && label.dropped_lines > 0 ? (
        <p
          data-testid="label-overflow-warning"
          className="text-[11px]"
          style={{ color: "var(--m-status-warning-fg)" }}
        >
          {label.dropped_lines === 1
            ? "One line does not fit this size at the minimum height and will not print."
            : `${label.dropped_lines} lines do not fit this size at the minimum height and will not print.`}
        </p>
      ) : null}
    </div>
  );
}
