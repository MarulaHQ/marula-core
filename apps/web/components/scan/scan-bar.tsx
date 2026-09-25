// ScanBar — THE shared keyboard-wedge scan affordance (Wave 32 lane 5,
// `p32-labels-scanning`).
//
// One component, every surface. It captures HID-scanner keystroke bursts
// (terminated by the suffix Enter) anywhere on the page, resolves the text
// through the ONE resolver (`POST /api/scan/resolve`), and hands the typed
// resolution to the surface through `onScan`. The surface decides what the
// scan means (fill a cost line, receive a PO line, pick a tool); this
// component never invents meaning beyond the shared contract.
//
// Focus law: a scan NEVER moves focus and NEVER hijacks a form. A burst
// that lands in a foreign field is detected, its injected characters are
// stripped back out of that field, its Enter is swallowed (no accidental
// form submit), and the scan routes — the user keeps typing where they
// were. Focus moves only when the operator clicks the scan button.
//
// Accessibility: the input is a real labelled text field (manual entry +
// paste work), the result is an aria-live region, candidates are real
// buttons. Motion stays transform/opacity, ≤120ms.

"use client";

import type { components } from "@meridian/sdk";
import { Button } from "@meridian/ui";
import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../../lib/api";
import type { ScanResolution } from "../../lib/labels/model";
import { accepts, kindLabel, rejectionCopy, scanSummary } from "../../lib/scan/routing";
import {
  burstLandedInForeignField,
  createWedgeState,
  feedWedge,
  stripBurstFromField,
  WEDGE_MAX_GAP_MS,
} from "../../lib/scan/wedge";

type ScanResolutionView = components["schemas"]["ScanResolutionView"];

interface ScanEvent {
  resolution: ScanResolutionView;
  source: "bar" | "wedge";
}

export interface ScanBarProps {
  /** Surface key in `SURFACE_ACCEPTS` — decides which kinds apply here. */
  surface: string;
  /** Called with an ACCEPTED resolution; the surface fills/queues its action. */
  onScan: (event: ScanEvent) => void;
  /** Optional: observe rejections (unknown text, wrong kind) for copy. */
  onRejected?: (message: string, resolution: ScanResolutionView) => void;
  hint?: string;
  disabled?: boolean;
  className?: string;
  /** Focus the input on mount — use only on surfaces whose whole job is scanning. */
  autoFocus?: boolean;
}

/**
 * Only the most recently mounted bar handles document-level bursts, so two
 * bars on one page can never double-route a scan.
 */
let activeBarId = 0;

function ScanGlyph() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden>
      <path d="M3 5v14M7 5v14M11 5v10M15 5v14M19 5v14" />
    </svg>
  );
}

export function ScanBar({
  surface,
  onScan,
  onRejected,
  hint,
  disabled = false,
  className,
  autoFocus = false,
}: ScanBarProps): React.ReactNode {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [last, setLast] = useState<ScanEvent | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [candidates, setCandidates] = useState<ScanResolutionView["candidates"]>([]);

  const inputRef = useRef<HTMLInputElement | null>(null);
  const barIdRef = useRef<number>(0);
  const wedgeRef = useRef(createWedgeState());

  const applyResolution = useCallback(
    async (text: string, source: "bar" | "wedge") => {
      const trimmed = text.trim();
      if (trimmed === "") return;
      setBusy(true);
      setNotice(null);
      setCandidates([]);
      const { data, error } = await api.api.POST("/api/v1/scan/resolve", {
        body: { text: trimmed },
      });
      setBusy(false);
      if (error !== undefined || data === undefined) {
        const message = "The scan could not be resolved — check the connection and try again.";
        setNotice(message);
        return;
      }
      const resolution = data as ScanResolutionView;
      const found = resolution.candidates ?? [];
      if (accepts(surface, resolution) && found.length === 0) {
        setLast({ resolution, source });
        onScan({ resolution, source });
        return;
      }
      if (found.length > 0) {
        setCandidates(found);
        setNotice(rejectionCopy(surface, resolution));
        onRejected?.(rejectionCopy(surface, resolution), resolution);
        return;
      }
      const message = rejectionCopy(surface, resolution);
      setNotice(message);
      onRejected?.(message, resolution);
    },
    [onRejected, onScan, surface],
  );

  // Document-level burst capture (capture phase so Enter never reaches a
  // form first). Only the active bar processes.
  useEffect(() => {
    if (disabled) return;
    barIdRef.current = ++activeBarId;
    const onKeyDown = (event: KeyboardEvent) => {
      if (barIdRef.current !== activeBarId) return;
      const { next, result } = feedWedge(wedgeRef.current, {
        key: event.key,
        at: event.timeStamp,
        ctrlKey: event.ctrlKey,
        metaKey: event.metaKey,
        altKey: event.altKey,
      });
      wedgeRef.current = next;
      if (result.kind === "scan") {
        // The burst is a scanner payload: swallow its Enter so no form
        // submits, restore any field it leaked into, then route.
        event.preventDefault();
        event.stopPropagation();
        const target = event.target;
        if (burstLandedInForeignField(target, inputRef.current)) {
          stripBurstFromField(target, result.text);
        }
        if (target === inputRef.current) setValue("");
        void applyResolution(result.text, "wedge");
      } else if (result.kind === "reset" && event.key === "Enter") {
        // Enter without a burst: let forms behave normally.
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [applyResolution, disabled]);

  useEffect(() => {
    if (autoFocus) inputRef.current?.focus();
  }, [autoFocus]);

  return (
    <div className={className} data-testid="scan-bar" data-surface={surface}>
      <div className="flex items-center gap-2">
        <div
          className="flex h-8 flex-1 items-center gap-2 border px-2"
          style={{
            borderColor: "var(--m-border)",
            borderRadius: "var(--m-radius-control)",
            background: "var(--m-surface)",
            outlineOffset: "2px",
          }}
        >
          <span aria-hidden style={{ color: "var(--m-text-muted)" }}>
            <ScanGlyph />
          </span>
          <input
            ref={inputRef}
            data-testid="scan-input"
            className="min-w-0 flex-1 bg-transparent text-xs outline-none"
            style={{ fontFamily: "var(--font-mono)" }}
            value={value}
            disabled={disabled}
            placeholder={disabled ? "Scanning unavailable" : "Scan or type a code, then press Enter"}
            aria-label="Scan barcode or QR code"
            autoComplete="off"
            spellCheck={false}
            onChange={(event) => setValue(event.target.value)}
            onKeyDown={(event) => {
              // Manual entry / paste: the burst detector did not claim this
              // Enter (it only claims realistic scanner bursts).
              if (event.key === "Enter" && value.trim() !== "") {
                event.preventDefault();
                const text = value;
                setValue("");
                void applyResolution(text, "bar");
              }
            }}
          />
          {busy ? (
            <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              Resolving…
            </span>
          ) : null}
          <button
            type="button"
            className="text-[11px]"
            style={{ color: "var(--m-text-muted)" }}
            onClick={() => inputRef.current?.focus()}
            aria-label="Focus the scan field"
          >
            Focus
          </button>
        </div>
        {last !== null ? (
          <Button
            variant="ghost"
            size="sm"
            data-testid="scan-clear"
            onClick={() => {
              setLast(null);
              setNotice(null);
              setCandidates([]);
            }}
          >
            Clear
          </Button>
        ) : null}
      </div>
      <p
        className="mt-1 text-[11px]"
        style={{ color: "var(--m-text-muted)" }}
        data-testid="scan-hint"
      >
        {hint ?? `Scanner-ready: a burst ending in Enter routes to this surface (gap ≤ ${WEDGE_MAX_GAP_MS} ms).`}
      </p>
      <div aria-live="polite" className="min-h-[18px]">
        {last !== null ? (
          <span
            data-testid="scan-chip"
            className="inline-flex items-center gap-2 text-[11px] transition-[transform,opacity] duration-100 motion-reduce:transition-none"
            style={{ color: "var(--m-status-success-fg)" }}
          >
            <span
              className="rounded-full px-2 py-[1px] text-[10px] font-bold"
              style={{ background: "var(--m-status-success-bg)", color: "var(--m-status-success-fg)" }}
            >
              {kindLabel(last.resolution.kind)}
            </span>
            <span data-testid="scan-chip-summary">{scanSummary(last.resolution)}</span>
            <span style={{ color: "var(--m-text-muted)" }}>
              {last.source === "wedge" ? "keyboard wedge" : "entered"}
            </span>
          </span>
        ) : null}
        {notice !== null ? (
          <span
            data-testid="scan-notice"
            className="text-[11px]"
            style={{ color: "var(--m-status-warning-fg)" }}
          >
            {notice}
          </span>
        ) : null}
      </div>
      {(candidates ?? []).length > 0 ? (
        <ul className="mt-1 grid gap-1" data-testid="scan-candidates">
          {(candidates ?? []).map((candidate) => (
            <li key={candidate.entity_id}>
              <button
                type="button"
                data-testid="scan-candidate"
                className="w-full border px-2 py-1 text-left text-[11px]"
                style={{
                  borderColor: "var(--m-border)",
                  borderRadius: "var(--m-radius-control)",
                  background: "var(--m-surface)",
                }}
                onClick={() => {
                  const resolution = {
                    input: "",
                    kind: candidate.kind,
                    entity_id: candidate.entity_id,
                    display_name: candidate.display_name,
                    secondary: candidate.secondary ?? null,
                    candidates: [],
                    deep_link: null,
                    is_tool: false,
                    item_kind: null,
                    serial_status: null,
                    stock_item_id: null,
                    serial_id: null,
                    location_id: null,
                    device_asset_id: null,
                  } as ScanResolutionView;
                  setCandidates([]);
                  setNotice(null);
                  if (accepts(surface, resolution)) {
                    setLast({ resolution, source: "bar" });
                    onScan({ resolution, source: "bar" });
                  } else {
                    const message = rejectionCopy(surface, resolution);
                    setNotice(message);
                    onRejected?.(message, resolution);
                  }
                }}
              >
                {candidate.display_name}
                {candidate.secondary != null ? ` · ${candidate.secondary}` : ""}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
