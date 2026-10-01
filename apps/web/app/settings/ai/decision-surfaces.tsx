// The AI governance table and its surface drawer — states 10 to 12 of the
// settings pack, in one place, because this is the ONLY place the AI is
// configured and the ONLY place shadow mode is visible.
//
// Purpose: the drawn shape over the real decision-harness API:
//   - the seven decision surfaces as a table: Access, Mode, Enabled, and a
//     Configure or Grant action per row
//   - the strip that explains shadow and links to the shadow runs
//   - the 480px configure drawer, carrying the existing
//     `DecisionSurfaceEditor` (questions, thresholds, pinned model, versions)
//     rather than a second editor
//   - the boundary line, in the drawer and on the page: anything a client can
//     see, or that leaves the building, waits for a person
//
// Two rules this file exists to hold:
//   1. **No grant means no trace.** A surface that is not configured shows no
//      affordance anywhere in the product, and this table says so in words
//      instead of pretending a switch exists.
//   2. **Shadow is invisible on the surface and visible only here.** The
//      drawer links to the shadow runs page; no product surface carries the
//      link, and the shadow page is the only place the word appears.
//
// Non-purpose: the availability matrix, caps, credentials, the model catalog
// and usage cards, which the page below keeps exactly as they were.

"use client";

import { useState } from "react";
import { Badge, Button } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { api } from "../../../lib/api";
import { resultErrorText } from "../../../lib/error-copy";
import { DecisionSurfaceEditor } from "../../../components/settings/decision-surface-editor";
import {
  DrawerFooter,
  Note,
  SettingsDrawer,
  ToggleRow,
  ToggleSwitch,
} from "../../../components/settings/settings-forms";

type DecisionSurfaceOverview = components["schemas"]["DecisionSurfaceOverview"];

export interface DecisionSurfacesProps {
  surfaces: DecisionSurfaceOverview[];
  featuresEnabled: boolean;
  onChanged: (key: string, next: DecisionSurfaceOverview) => void;
}

const COLUMNS = "minmax(200px, 1.5fr) 130px 120px 90px 120px";

export function DecisionSurfaces({
  surfaces,
  featuresEnabled,
  onChanged,
}: DecisionSurfacesProps): React.JSX.Element | null {
  const [openKey, setOpenKey] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const open = surfaces.find((surface) => surface.surface_key === openKey) ?? null;

  if (surfaces.length === 0) return null;

  const setMode = async (
    surface: DecisionSurfaceOverview,
    body: Record<string, unknown>,
  ): Promise<void> => {
    setBusyKey(surface.surface_key);
    setError(null);
    const res = await api.api.PUT("/api/v1/settings/ai/decision/{surface_key}/config", {
      params: { path: { surface_key: surface.surface_key } },
      body: body as never,
    });
    setBusyKey(null);
    if (res.error !== undefined) {
      setError(resultErrorText(res, "That change was refused."));
      return;
    }
    // The overview is the source of truth after a write; re-read it rather
    // than guessing the row the server settled on.
    const overview = await api.api.GET("/api/v1/settings/ai/decision");
    const next = overview.data?.surfaces.find((s) => s.surface_key === surface.surface_key);
    if (next !== undefined) onChanged(surface.surface_key, next);
  };

  return (
    <>
      <section
        data-testid="ai-decision-surfaces"
        style={{
          border: "1px solid var(--m-border)",
          borderRadius: 16,
          background: "var(--m-surface)",
          overflow: "hidden",
          marginTop: 16,
        }}
      >
        {/* The label sits inside the bordered card, so it needs its own
            inset: without it the cap-height of the uppercase text is clipped
            by the card's overflow. */}
        <div
          style={{
            fontSize: 12,
            lineHeight: 1.4,
            letterSpacing: "0.08em",
            textTransform: "uppercase",
            fontWeight: 600,
            color: "var(--m-text-secondary)",
            margin: 0,
            padding: "14px 16px 10px",
          }}
        >
          Decision surfaces
        </div>
        <div
          role="table"
          aria-label="Decision surfaces"
          style={{ display: "flex", flexDirection: "column" }}
        >
          <div
            role="row"
            style={{
              display: "grid",
              gridTemplateColumns: COLUMNS,
              alignItems: "center",
              minHeight: 40,
              background: "var(--m-canvas)",
              borderBottom: "1px solid var(--m-border)",
            }}
          >
            {["Surface", "Access", "Mode", "Enabled", ""].map((header, index) => (
              <span
                key={header + String(index)}
                style={{
                  fontSize: 12,
                  letterSpacing: "0.06em",
                  textTransform: "uppercase",
                  fontWeight: 600,
                  color: "var(--m-text-secondary)",
                  paddingLeft: index === 0 ? 16 : 8,
                }}
              >
                {header}
              </span>
            ))}
          </div>

          {surfaces.map((surface) => {
            const internal = surface.internal;
            const granted = surface.configured;
            const busy = busyKey === surface.surface_key;
            return (
              <div
                key={surface.surface_key}
                role="row"
                data-testid={`ai-surface-row-${surface.surface_key}`}
                style={{
                  display: "grid",
                  gridTemplateColumns: COLUMNS,
                  alignItems: "center",
                  minHeight: 56,
                  borderBottom: "1px solid var(--m-border)",
                }}
              >
                <span
                  style={{
                    paddingLeft: 16,
                    paddingRight: 8,
                    minWidth: 0,
                    display: "flex",
                    flexDirection: "column",
                    gap: 2,
                  }}
                >
                  <b style={{ fontWeight: 600, color: "var(--m-text)" }}>{surface.label}</b>
                  <span
                    style={{
                      fontSize: 12,
                      color: "var(--m-text-secondary)",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {surface.description}
                  </span>
                </span>
                <span style={{ paddingLeft: 8 }}>
                  {internal ? (
                    <Badge tone="neutral">Internal</Badge>
                  ) : !featuresEnabled ? (
                    <Badge tone="neutral">Off</Badge>
                  ) : granted ? (
                    <Badge tone="success">Granted</Badge>
                  ) : (
                    <Badge tone="neutral">Not granted</Badge>
                  )}
                </span>
                <span style={{ paddingLeft: 8 }}>
                  {internal ? (
                    <span style={{ color: "var(--m-text-secondary)", fontSize: 13 }}>Not set</span>
                  ) : !featuresEnabled || !granted ? (
                    <span style={{ color: "var(--m-text-secondary)", fontSize: 13 }}>Not set</span>
                  ) : surface.mode === "live" ? (
                    <Badge tone="success">Live</Badge>
                  ) : (
                    <Badge tone="warning">Shadow</Badge>
                  )}
                </span>
                <span style={{ paddingLeft: 8, display: "flex" }}>
                  {featuresEnabled && granted && !internal ? (
                    <ToggleSwitch
                      on={surface.enabled}
                      disabled={busy}
                      label={`Enabled: ${surface.label}`}
                      testId={`ai-enabled-${surface.surface_key}`}
                      onChange={(next) =>
                        void setMode(surface, {
                          idempotency_key: `ai-enabled:${surface.surface_key}:${Date.now()}`,
                          enabled: next,
                        })
                      }
                    />
                  ) : null}
                </span>
                <span style={{ display: "flex", justifyContent: "flex-end", paddingRight: 12 }}>
                  {internal ? null : !featuresEnabled ? (
                    <span
                      style={{ fontSize: 12, color: "var(--m-text-secondary)" }}
                      title="Turn the master switch on to grant a surface"
                    >
                      Master off
                    </span>
                  ) : granted ? (
                    <Button
                      variant="secondary"
                      onClick={() => setOpenKey(surface.surface_key)}
                      data-testid={`ai-configure-${surface.surface_key}`}
                      style={{ height: 34, fontSize: 13 }}
                    >
                      Configure
                    </Button>
                  ) : (
                    <Button
                      variant="secondary"
                      disabled={busy}
                      onClick={() =>
                        void setMode(surface, {
                          idempotency_key: `ai-grant:${surface.surface_key}:${Date.now()}`,
                          enabled: true,
                          mode: "shadow",
                        })
                      }
                      data-testid={`ai-grant-${surface.surface_key}`}
                      style={{ height: 34, fontSize: 13 }}
                    >
                      Grant
                    </Button>
                  )}
                </span>
              </div>
            );
          })}
        </div>
      </section>

      <div
        data-testid="ai-shadow-strip"
        style={{
          display: "flex",
          alignItems: "center",
          gap: 12,
          padding: "12px 16px",
          borderRadius: 14,
          background: "var(--m-surface-2)",
          fontSize: 13,
          lineHeight: 1.5,
          marginTop: 14,
        }}
      >
        <InfoIcon />
        <span>
          <b>Shadow</b> means the surface runs and scores but proposes nothing, so nobody sees it.
          Watch it on{" "}
          <a href="/settings/ai/shadow" style={{ fontWeight: 600 }}>
            Shadow runs
          </a>
          , then promote it when the numbers convince you.
        </span>
      </div>

      <p
        style={{
          margin: "8px 0 0",
          fontSize: 13,
          color: "var(--m-text-secondary)",
          maxWidth: 720,
        }}
      >
        Whatever is set here, anything a client can see or that leaves the building waits for a
        person to approve.
      </p>

      {error !== null ? (
        <p role="alert" style={{ margin: "8px 0 0", fontSize: 13, color: "var(--m-status-due-fg)" }}>
          {error}
        </p>
      ) : null}

      {open !== null ? (
        <SettingsDrawer
          title={open.label}
          description={
            open.configured
              ? `Granted. ${open.mode === "live" ? "Live" : "In shadow"}, so ${
                  open.mode === "live"
                    ? "proposals are shown, and a person approves each one"
                    : "nothing is shown to anyone"
                }.`
              : "Not granted yet. Nothing about this surface appears in the product until it is."
          }
          onClose={() => setOpenKey(null)}
          footer={
            <DrawerFooter onCancel={() => setOpenKey(null)}>
              <Button variant="primary" onClick={() => setOpenKey(null)}>
                Done
              </Button>
            </DrawerFooter>
          }
          testId="ai-surface-drawer"
        >
          {open.configured ? (
            <>
              <ToggleRow
                first
                title="Enabled"
                description="Turn this one surface off without removing its grant."
              >
                <ToggleSwitch
                  on={open.enabled}
                  label={`Enabled: ${open.label}`}
                  testId="ai-drawer-enabled"
                  onChange={(next) =>
                    void setMode(open, {
                      idempotency_key: `ai-drawer:${open.surface_key}:${Date.now()}`,
                      enabled: next,
                    })
                  }
                />
              </ToggleRow>
              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <span style={{ fontSize: 14, fontWeight: 600 }}>Mode</span>
                <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  {open.mode === "live" ? (
                    <Badge tone="success">Live</Badge>
                  ) : (
                    <Badge tone="warning">Shadow</Badge>
                  )}
                  <a
                    href="/settings/ai/shadow"
                    style={{ fontSize: 13, fontWeight: 600, color: "var(--m-text)" }}
                  >
                    {open.mode === "live" ? "See how it is doing" : "Watch shadow runs, then promote"}
                  </a>
                </div>
              </div>
            </>
          ) : null}
          <DecisionSurfaceEditor
            surface={open}
            onSaved={(next) => onChanged(open.surface_key, next)}
          />
          <Note>Anything a client can see, or that leaves the building, waits for a person.</Note>
        </SettingsDrawer>
      ) : null}
    </>
  );
}

function InfoIcon(): React.JSX.Element {
  return (
    <svg
      aria-hidden="true"
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      style={{ flexShrink: 0, color: "var(--m-text-secondary)" }}
    >
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v5" />
      <path d="M12 8h.01" />
    </svg>
  );
}
