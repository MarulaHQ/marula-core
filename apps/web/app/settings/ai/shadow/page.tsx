// /settings/ai/shadow — what a surface would have said, and the run log.
//
// Purpose: states 11 and 12 of the settings pack, on their own page, because
// this is the ONE place in the product where shadow mode is visible. On every
// product surface a shadow run leaves no trace; here the admin can watch what
// it would have proposed, how it agreed with the person who actually decided,
// and decide whether to promote it.
//
// What is real and what is not:
//   - The surfaces, their mode and their run budget are real, read from the
//     decision-harness API.
//   - The per-ticket "what it would have said" list is NOT in the product: no
//     run log endpoint exists, so the list says so plainly and points at the
//     surface editor, which fires a real shadow decision and shows the real
//     answers. The drawn page shape — surface picker, the two tabs, the strip,
//     the four figures, the run table and the promote card — is kept, so the
//     place exists and reads right the day the log lands.
//   - Promotion is real: it flips the surface's mode to live through the
//     config API, and it is reversible.
//
// Non-purpose: no new backend, and no way to read a client's data from here.

"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Badge, Button, Spinner } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { api } from "../../../../lib/api";
import { resultErrorText } from "../../../../lib/error-copy";
import { SettingsHeader } from "../../../../components/settings/settings-shell";
import {
  Note,
  PromoteCard,
  ThresholdBar,
} from "../../../../components/settings/settings-forms";

type DecisionSurfaceOverview = components["schemas"]["DecisionSurfaceOverview"];

const THRESHOLD = 0.8;

export default function ShadowRunsPage(): React.JSX.Element {
  const [surfaces, setSurfaces] = useState<DecisionSurfaceOverview[]>([]);
  const [featuresEnabled, setFeaturesEnabled] = useState(false);
  const [selected, setSelected] = useState<string>("");
  const [tab, setTab] = useState<"list" | "log">("list");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    const res = await api.api.GET("/api/v1/settings/ai/decision");
    if (res.error !== undefined) {
      setError("The AI settings did not load.");
      setLoading(false);
      return;
    }
    const all = res.data?.surfaces ?? [];
    setFeaturesEnabled(res.data?.features_enabled ?? false);
    setSurfaces(all.filter((surface) => !surface.internal));
    setSelected((prev) =>
      prev !== "" && all.some((surface) => surface.surface_key === prev)
        ? prev
        : (all.find((surface) => !surface.internal && surface.configured)?.surface_key ?? ""),
    );
    setLoading(false);
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const surface = useMemo(
    () => surfaces.find((s) => s.surface_key === selected) ?? null,
    [surfaces, selected],
  );

  const setMode = async (mode: "shadow" | "live"): Promise<void> => {
    if (surface === null) return;
    setBusy(true);
    setError(null);
    const res = await api.api.PUT("/api/v1/settings/ai/decision/{surface_key}/config", {
      params: { path: { surface_key: surface.surface_key } },
      body: {
        idempotency_key: `ai-mode:${surface.surface_key}:${mode}:${Date.now()}`,
        mode,
      } as never,
    });
    setBusy(false);
    if (res.error !== undefined) {
      setError(resultErrorText(res, "The mode was refused."));
      return;
    }
    await load();
  };

  if (loading) {
    return (
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: 24 }}>
        <Spinner />
        <span style={{ fontSize: 13, color: "var(--m-text-secondary)" }}>
          Loading the decision surfaces…
        </span>
      </div>
    );
  }

  return (
    <div data-testid="ai-shadow-page">
      <SettingsHeader
        title="Shadow runs"
        description="What a surface would have said, recorded without anyone seeing it."
      >
        <label className="flex items-center gap-2">
          <span className="sr-only">Surface</span>
          <select
            aria-label="Surface"
            value={selected}
            onChange={(event) => setSelected(event.target.value)}
            data-testid="ai-shadow-surface"
            style={{
              height: 40,
              borderRadius: 10,
              border: "1px solid var(--m-input)",
              background: "var(--m-surface)",
              color: "var(--m-text)",
              padding: "0 12px",
              fontWeight: 600,
            }}
          >
            {surfaces.length === 0 ? <option value="">No surfaces</option> : null}
            {surfaces.map((s) => (
              <option key={s.surface_key} value={s.surface_key}>
                {s.label}
              </option>
            ))}
          </select>
        </label>
      </SettingsHeader>

      {!featuresEnabled ? (
        <Note testId="ai-shadow-master-off">
          <b>The master switch is off.</b> No surface can run while the organisation has AI
          disabled, so there is nothing to watch. Turn it on in{" "}
          <a href="/settings/ai" style={{ fontWeight: 600 }}>
            AI
          </a>
          .
        </Note>
      ) : surface === null || !surface.configured ? (
        <Note testId="ai-shadow-not-granted">
          <b>Not granted.</b> This surface has no configuration yet, so it has never run. Grant it
          in <a href="/settings/ai" style={{ fontWeight: 600 }}>AI</a> and it starts in shadow, with
          nothing shown to anyone.
        </Note>
      ) : null}

      <div
        role="group"
        aria-label="Views"
        data-testid="ai-shadow-tabs"
        style={{ display: "flex", gap: 4, margin: "0 0 16px" }}
      >
        {(
          [
            ["list", "What it would have said"],
            ["log", "Run log"],
          ] as const
        ).map(([key, label]) => (
          <button
            key={key}
            type="button"
            aria-pressed={tab === key}
            onClick={() => setTab(key)}
            data-testid={`ai-shadow-tab-${key}`}
            style={{
              height: 36,
              padding: "0 14px",
              borderRadius: 10,
              border: "none",
              background: tab === key ? "var(--m-surface-2)" : "transparent",
              color: tab === key ? "var(--m-text)" : "var(--m-text-secondary)",
              fontWeight: tab === key ? 600 : 500,
              cursor: "pointer",
            }}
          >
            {label}
          </button>
        ))}
      </div>

      {surface !== null && surface.configured && featuresEnabled ? (
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
            marginBottom: 14,
          }}
        >
          <ShieldIcon />
          <span>
            {surface.mode === "live" ? (
              <>
                <b>Live.</b> This surface now proposes above the threshold. A person still approves
                every write.
              </>
            ) : (
              <>
                <b>In shadow.</b> Nothing below was shown to anyone. This page is the only place it
                appears.
              </>
            )}
          </span>
        </div>
      ) : null}

      {error !== null ? (
        <p role="alert" style={{ fontSize: 13, color: "var(--m-status-due-fg)", margin: "0 0 12px" }}>
          {error}
        </p>
      ) : null}

      {tab === "list" ? (
        <section
          data-testid="ai-shadow-list"
          style={{
            border: "1px solid var(--m-border)",
            borderRadius: 16,
            background: "var(--m-surface)",
            padding: "18px 20px",
          }}
        >
          <h2 style={{ margin: "0 0 4px", fontSize: 15, fontWeight: 600, color: "var(--m-text)" }}>
            Nothing recorded yet
          </h2>
          <p
            style={{
              margin: 0,
              fontSize: 13,
              lineHeight: 1.5,
              color: "var(--m-text-secondary)",
              maxWidth: 560,
            }}
          >
            The product keeps no per-ticket list of what a shadow run would have said, so there is
            nothing to show here yet. The surface editor fires a real shadow decision on a real
            ticket and shows the answers, the full probability distribution and the confidence — the
            same evidence this page will show once the run log lands.
          </p>
          <div style={{ marginTop: 12 }}>
            <a href={`/settings/ai`} style={{ fontWeight: 600 }}>
              Open the surface editor
            </a>
          </div>
        </section>
      ) : (
        <>
          <div
            data-testid="ai-shadow-log"
            style={{
              border: "1px solid var(--m-border)",
              borderRadius: 16,
              background: "var(--m-surface)",
              padding: "18px 20px",
            }}
          >
            <h2 style={{ margin: "0 0 4px", fontSize: 15, fontWeight: 600, color: "var(--m-text)" }}>
              No run log to summarise
            </h2>
            <p
              style={{
                margin: 0,
                fontSize: 13,
                lineHeight: 1.5,
                color: "var(--m-text-secondary)",
                maxWidth: 560,
              }}
            >
              Agreement, disagreement and the count above the threshold are all read from a run log
              the product does not write yet, so the four figures and the table below are not
              invented here. Until it lands, the honest position is that there is no evidence to
              promote on, and this surface should stay in shadow.
            </p>
            <div style={{ display: "flex", gap: 12, marginTop: 14, alignItems: "center" }}>
              <span style={{ fontSize: 13, color: "var(--m-text-secondary)" }}>
                Threshold in force
              </span>
              <ThresholdBar value={THRESHOLD} threshold={THRESHOLD} width={120} />
              <b style={{ fontSize: 13, fontVariantNumeric: "tabular-nums" }}>{THRESHOLD.toFixed(2)}</b>
              <span style={{ fontSize: 13, color: "var(--m-text-secondary)" }}>
                — set per question in the surface drawer
              </span>
            </div>
          </div>

          {surface !== null && surface.configured ? (
            <PromoteCard
              promoted={surface.mode === "live"}
              title={surface.mode === "live" ? "Live" : "Ready to go live?"}
              actions={
                surface.mode === "live" ? (
                  <Button
                    variant="secondary"
                    disabled={busy}
                    onClick={() => void setMode("shadow")}
                    data-testid="ai-shadow-demote"
                  >
                    Return to shadow
                  </Button>
                ) : (
                  <>
                    <Button
                      variant="secondary"
                      disabled={busy}
                      onClick={() => void setMode("shadow")}
                      data-testid="ai-shadow-keep"
                    >
                      Keep in shadow
                    </Button>
                    <Button
                      variant="primary"
                      disabled={busy}
                      onClick={() => void setMode("live")}
                      data-testid="ai-shadow-promote"
                    >
                      Promote to live
                    </Button>
                  </>
                )
              }
            >
              {surface.mode === "live"
                ? "Proposals now appear above the threshold. You can return it to shadow at any time, and a person still approves every write."
                : "Promoting flips this surface to live, so it may propose above the threshold. A person still approves every write, and you can put it back in shadow at any time."}
            </PromoteCard>
          ) : null}
        </>
      )}

      {surface !== null ? (
        <p
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            marginTop: 14,
            fontSize: 13,
            color: "var(--m-text-secondary)",
          }}
        >
          <Badge tone={surface.mode === "live" ? "success" : "warning"}>
            {surface.mode === "live" ? "Live" : "Shadow"}
          </Badge>
          <span>
            {surface.model_provider !== null && surface.model_provider !== undefined
              ? `Pinned to ${surface.model_provider} · ${surface.model_id ?? "no model pinned"}`
              : "No decision model pinned yet — the surface cannot run until one is."}
          </span>
        </p>
      ) : null}
    </div>
  );
}

function ShieldIcon(): React.JSX.Element {
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
      <path d="M12 3 5 6v6c0 4 3 7 7 8 4-1 7-4 7-8V6l-7-3Z" />
      <path d="M9 12h6" />
    </svg>
  );
}
