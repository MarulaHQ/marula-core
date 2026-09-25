// Label print targets (Wave 32 lane 5, `p32-labels-scanning`).
//
// Two honest targets, in doctrine order:
//   1. BROWSER PRINT — everywhere. The label is laid out in millimetres and
//      printed through the dialog at its real physical size (@page sized to
//      the label, zero margin).
//   2. RAW ZPL — only where the Tauri desktop shell is present. The core
//      rendered the exact ZPL bytes; the shell transports them to a
//      networked Zebra printer. On plain web the ZPL path reports
//      "unsupported" and the UI falls back to the print dialog — never a
//      pretend-sent label.
//
// The printer ADDRESS is workstation-local (localStorage): a Zebra printer
// is a device on one desk's network, not org configuration. Nothing here
// is sent anywhere except the printer.

import type { ResolvedLabel } from "./model";
import { LABEL_PADDING_MM, qrPositionMm } from "./model";

const PRINTER_KEY = "meridian.labels.printer";
const DEFAULT_PRINTER_PORT = 9100;

export interface LabelPrintInput {
  label: ResolvedLabel;
  qrSvg: string | null;
  code128Svg: string | null;
  /** Document title — the entity's display name. */
  title: string;
}

function escapeHtml(s: string): string {
  return s
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

/** Inline SVG → a data URI an <img> can carry (never innerHTML'd script). */
export function svgDataUri(svg: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

/**
 * The complete print document for one label. Pure + deterministic: same
 * input, same bytes — the print twin of the preview, both driven by the
 * server's millimetre geometry.
 */
export function labelPrintHtml(input: LabelPrintInput): string {
  const { label, qrSvg, code128Svg, title } = input;
  const qr = qrPositionMm(label);
  const parts: string[] = [];
  for (const line of label.lines) {
    parts.push(
      `<div class="m-line" style="top:${line.y_mm}mm;height:${line.height_mm}mm;font-size:${line.font_mm.toFixed(2)}mm">${escapeHtml(line.text)}</div>`,
    );
  }
  if (label.barcode != null) {
    const inner =
      code128Svg !== null
        ? `<img alt="" src="${svgDataUri(code128Svg)}" />`
        : // No server SVG (older response): print the human-readable data
          // rather than nothing, so the operator still sees what it encodes.
          `<div class="m-barcode-fallback">${escapeHtml(label.barcode.data)}</div>`;
    parts.push(
      `<div class="m-barcode" style="top:${label.barcode.y_mm}mm;height:${label.barcode.height_mm}mm;width:${label.barcode.width_mm}mm">${inner}</div>`,
    );
  }
  if (qr != null && label.qr != null) {
    const inner =
      qrSvg !== null
        ? `<img alt="" src="${svgDataUri(qrSvg)}" />`
        : `<div class="m-qr-fallback">${escapeHtml(label.qr.data)}</div>`;
    parts.push(
      `<div class="m-qr" style="left:${qr.xMm}mm;top:${qr.yMm}mm;width:${label.qr.size_mm}mm;height:${label.qr.size_mm}mm">${inner}</div>`,
    );
  }
  return `<!doctype html>
<html>
<head>
<meta charset="utf-8" />
<title>${escapeHtml(title)}</title>
<style>
  @page { size: ${label.width_mm}mm ${label.height_mm}mm; margin: 0; }
  html, body { margin: 0; padding: 0; background: white; }
  body { -webkit-print-color-adjust: exact; print-color-adjust: exact; }
  .m-label {
    position: relative;
    width: ${label.width_mm}mm;
    height: ${label.height_mm}mm;
    overflow: hidden;
    background: white;
    color: black;
    font-family: Inter, system-ui, sans-serif;
  }
  .m-line {
    position: absolute;
    left: ${LABEL_PADDING_MM}mm;
    right: ${LABEL_PADDING_MM}mm;
    line-height: 1.05;
    white-space: nowrap;
    overflow: hidden;
    font-weight: 600;
  }
  .m-barcode {
    position: absolute;
    left: ${LABEL_PADDING_MM}mm;
  }
  .m-barcode img { width: 100%; height: 100%; display: block; }
  .m-barcode-fallback { font-family: "JetBrains Mono", monospace; font-size: 2.2mm; }
  .m-qr { position: absolute; }
  .m-qr img { width: 100%; height: 100%; display: block; }
  .m-qr-fallback { font-family: "JetBrains Mono", monospace; font-size: 1.6mm; overflow: hidden; word-break: break-all; }
</style>
</head>
<body><div class="m-label">${parts.join("")}</div></body>
</html>`;
}

/**
 * Prints through the browser dialog: a hidden same-origin iframe carries the
 * print document, the dialog opens on it, and the iframe is removed after.
 */
export function printLabelViaBrowser(input: LabelPrintInput): void {
  if (typeof document === "undefined") return;
  const frame = document.createElement("iframe");
  frame.setAttribute("aria-hidden", "true");
  frame.style.position = "fixed";
  frame.style.right = "0";
  frame.style.bottom = "0";
  frame.style.width = "0";
  frame.style.height = "0";
  frame.style.border = "0";
  frame.srcdoc = labelPrintHtml(input);
  frame.addEventListener("load", () => {
    const view = frame.contentWindow;
    if (view != null) {
      view.focus();
      view.print();
    }
    // Give the dialog time to grab the document before cleaning up.
    window.setTimeout(() => frame.remove(), 30_000);
  });
  document.body.appendChild(frame);
}

// --- desktop raw-ZPL transport ---------------------------------------------

interface TauriGlobal {
  core?: {
    invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  };
}

/** True only inside the Tauri desktop shell (web falls back to the dialog). */
export function isDesktopShell(): boolean {
  if (typeof window === "undefined") return false;
  const w = window as unknown as { __TAURI__?: TauriGlobal };
  return typeof w.__TAURI__?.core?.invoke === "function";
}

/** `host` or `host:port` → parsed destination (default port 9100). */
export function parsePrinterAddress(raw: string): { host: string; port: number } | null {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  const [host, portPart, ...rest] = trimmed.split(":");
  if (host === undefined || host.trim() === "" || rest.length > 0) return null;
  if (portPart === undefined || portPart === "") {
    return { host: host.trim(), port: DEFAULT_PRINTER_PORT };
  }
  const port = Number.parseInt(portPart, 10);
  if (!Number.isInteger(port) || port < 1 || port > 65535) return null;
  return { host: host.trim(), port };
}

export function loadPrinterAddress(): string {
  if (typeof window === "undefined") return "";
  return window.localStorage.getItem(PRINTER_KEY) ?? "";
}

export function savePrinterAddress(raw: string): void {
  if (typeof window === "undefined") return;
  if (raw.trim() === "") window.localStorage.removeItem(PRINTER_KEY);
  else window.localStorage.setItem(PRINTER_KEY, raw.trim());
}

export type ZplSendResult =
  | { kind: "sent" }
  | { kind: "unsupported" }
  | { kind: "error"; message: string };

/** Hands the core-rendered ZPL to the desktop shell; honest on web. */
export async function sendZpl(zpl: string, address: string): Promise<ZplSendResult> {
  // Web first: raw ZPL is a desktop capability, so on plain web the honest
  // answer is "unsupported" no matter what address is typed.
  if (!isDesktopShell()) return { kind: "unsupported" };
  const parsed = parsePrinterAddress(address);
  if (parsed === null) {
    return { kind: "error", message: "Enter the printer as host or host:port (e.g. 10.0.0.9:9100)." };
  }
  const w = window as unknown as { __TAURI__?: TauriGlobal };
  const invoke = w.__TAURI__?.core?.invoke;
  if (invoke === undefined) return { kind: "unsupported" };
  try {
    await invoke("print_raw_zpl", { host: parsed.host, port: parsed.port, zpl });
    return { kind: "sent" };
  } catch (error) {
    return {
      kind: "error",
      message: error instanceof Error ? error.message : String(error),
    };
  }
}
