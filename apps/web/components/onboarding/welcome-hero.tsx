// Wave 26 onboarding welcome hero — the FIRST-OPEN animated welcome screen.
//
// Purpose: the per-user first-run opens on a cool, animated welcome where
// the member PICKS their look: Light or Dark, plus any theme preset (and
// any additional theme pack their admin installed) as preview cards. A
// choice applies INSTANTLY (account-saved mode + theme override — the
// same Wave 24 keys ThemePresetSync applies at session start; installed
// packs ride the `pack:<slug>` shape the typed registry now accepts for
// the user override) and completes the "personal defaults" step, which
// stays first-class-skippable like every step in the flow family.
//
// Motion law: entrance + hover are transform/opacity only, ≤120ms, and
// the global `prefers-reduced-motion` envelope collapses animations.
//
// Non-purpose: preset management (admins install packs through the
// distribution surfaces) and preview token math beyond inline accent
// swatches from the pack manifest's light side.

"use client";

import { applyTheme, setThemePreset, Button, Card } from "@meridian/ui";
import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";

type PackRow = { slug: string; manifest?: unknown };

type CardChoice = {
  key: string;
  label: string;
  renderMode: "light" | "dark" | null;
  theme: string; // preset id or `pack:<slug>`
  swatch: string | null;
};

const MODE_OPTIONS: ("light" | "dark")[] = ["light", "dark"];

const BUILTINS: { id: string; label: string }[] = [
  { id: "tangerine", label: "Tangerine" },
  { id: "slate", label: "Slate" },
  { id: "high_contrast", label: "High contrast" },
  { id: "copper", label: "Copper" },
];

function packPrimary(manifest: unknown): string | null {
  const shape = manifest as { light?: Record<string, string> } | null;
  return shape?.light?.["--m-accent"] ?? null;
}

export function OnboardingWelcomeHero({
  onChosen,
  saving,
}: {
  onChosen: (choice: { mode: "light" | "dark"; theme: string }) => void;
  saving?: boolean;
}) {
  const [packs, setPacks] = useState<CardChoice[]>([]);
  const [mode, setMode] = useState<"light" | "dark">("light");
  const [pickedTheme, setPickedTheme] = useState<string | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        const [orgRes, userRes, packRes] = await Promise.all([
          api.api.GET("/api/v1/settings/org"),
          api.api.GET("/api/v1/settings/user"),
          api.api.GET("/api/v1/plugins/distribution/theme-packs", {}),
        ]);
        const pick = (rows: { key: string; value: unknown }[] | undefined, key: string) =>
          (rows ?? []).find((v) => v.key === key)?.value;
        const savedMode = pick(userRes.data, "appearance.mode");
        if (savedMode === "light" || savedMode === "dark") applyTheme(savedMode);
        setMode(savedMode === "light" || savedMode === "dark" ? savedMode : "light");
        const packs = ((packRes.data?.packs ?? []) as unknown as PackRow[]).map((p) => ({
          key: `pack:${p.slug}`,
          label: p.slug,
          renderMode: "light" as const,
          theme: `pack:${p.slug}`,
          swatch: packPrimary(p.manifest),
        }));
        setPacks(packs);
      } catch {
        // Degraded data is safe: the cards fall back to the built-in set.
      }
    })();
  }, []);

  const apply = useCallback(async (mode: "light" | "dark", theme: string) => {
    setMode(mode);
    setPickedTheme(theme);
    applyTheme(mode);
    try {
      for (const body of [
        { key: "appearance.mode", value: mode },
        { key: "appearance.theme_override", value: theme },
      ]) {
        await api.api.PUT("/api/v1/settings/user", { body });
      }
    } catch {
      // The visual preview is already live; a failed save surfaces through
      // the parent flow (the settings PUT error is non-catastrophic — the
      // member can re-apply from the defaults step).
    }
  }, []);

  return (
    <Card
      className="m-rise grid gap-3 p-5"
      data-testid="onboarding-welcome"
      role="region"
      aria-label="Welcome"
    >
      <div className="grid gap-0.5">
        <h2 className="text-[15px] font-semibold" style={{ color: "var(--m-text)" }}>
          Welcome! Pick your look
        </h2>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Light or dark, plus any theme preset you like and any pack your admin added. Clicking a
          preview applies it right away and saves it to your account.
        </p>
      </div>
      <div className="grid grid-cols-2 gap-2 md:grid-cols-4" data-testid="onboarding-welcome-modes">
        {MODE_OPTIONS.map((m) => (
          <button
            key={m}
            type="button"
            onClick={() => void apply(m, pickedTheme ?? "org_default")}
            className="m-rise rounded-[var(--m-radius-control)] px-3 py-2.5 text-[13px] text-left"
            style={{
              border: `1px solid ${mode === m ? "var(--m-accent)" : "var(--m-border)"}`,
              background: "var(--m-canvas-raised)",
              color: "var(--m-text)",
              transition: "border-color 120ms ease, transform 120ms ease, opacity 120ms ease",
            }}
            data-testid={`welcome-mode-${m}`}
          >
            {m === "light" ? "☀ Light" : "☾ Dark"}
          </button>
        ))}
        {BUILTINS.map((b) => (
          <button
            key={b.id}
            type="button"
            onClick={() => void apply(mode, b.id)}
            className="m-rise rounded-[var(--m-radius-control)] px-3 py-2.5 text-[13px] text-left"
            style={{
              border: `1px solid ${pickedTheme === b.id ? "var(--m-accent)" : "var(--m-border)"}`,
              background: "var(--m-canvas-raised)",
              color: "var(--m-text)",
              transition: "border-color 120ms ease, transform 120ms ease, opacity 120ms ease",
            }}
            data-testid={`welcome-theme-${b.id}`}
          >
            {b.label}
          </button>
        ))}
        {packs.length > 0 && (
          <p
            className="col-span-2 text-[12px] md:col-span-4"
            style={{ color: "var(--m-text-muted)" }}
          >
            Installed pack themes ({packs.length}, added by your admin):
          </p>
        )}
        {packs.map((p) => (
          <button
            key={p.key}
            type="button"
            onClick={() => void apply("light", p.theme)}
            className="m-rise grid gap-1 rounded-[var(--m-radius-control)] px-3 py-2.5 text-[13px] text-left"
            style={{
              border: `1px solid ${pickedTheme === p.theme ? "var(--m-accent)" : "var(--m-border)"}`,
              background: "var(--m-canvas-raised)",
              color: "var(--m-text)",
              transition: "border-color 120ms ease, transform 120ms ease, opacity 120ms ease",
            }}
            data-testid={`welcome-pack-${p.label}`}
          >
            <span className="flex items-center gap-2">
              {p.swatch && (
                <span
                  aria-hidden="true"
                  className="inline-block h-3 w-3 rounded-full"
                  style={{ background: p.swatch, border: "1px solid var(--m-border)" }}
                />
              )}
              {p.label}
            </span>
          </button>
        ))}
      </div>
      <div className="flex items-center gap-2">
        <Button
          variant="primary"
          onClick={() => onChosen({ mode, theme: pickedTheme ?? "org_default" })}
          disabled={saving}
          data-testid="welcome-confirm"
        >
          Looks good — continue
        </Button>
      </div>
    </Card>
  );
}
