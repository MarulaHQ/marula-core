"use client";
/**
 * ThemePresetSync — Wave 24: applies the effective theme preset.
 *
 * Resolution (user→org precedence, mirroring the sticky ai_model_picks
 * shape): the account's appearance.theme_override (a preset id, or the
 * org_default sentinel) wins over the org's appearance.theme setting;
 * the org value may name an installed theme pack (`pack:<slug>`), in
 * which case the pack's coerced manifest is applied as inline styles.
 * Unknown values stay on the Tangerine baseline — the theme-token schema
 * can restyle surfaces and nothing else, so a failed sync is always safe.
 *
 * Non-purpose: light/dark MODE (the pre-paint bootstrap + ThemeToggle own
 * it; this only re-applies the ACCOUNT-saved mode at session start) and
 * pack distribution (theme packs over ADR 0009).
 */
import { useEffect } from "react";
import { applyPackStyles, setThemePreset } from "@meridian/ui";
import type { Theme } from "@meridian/ui";
import { applyTheme } from "@meridian/ui";
import { api, useAuth } from "../lib/api";

export function ThemePresetSync(): null {
  const auth = useAuth();
  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      try {
        const [userRes, orgRes] = await Promise.all([
          api.api.GET("/api/v1/settings/user"),
          api.api.GET("/api/v1/settings/org"),
        ]);
        const userValues = userRes.data ?? [];
        const orgValues = orgRes.data ?? [];
        const read = (rows: { key: string; value: unknown }[] | undefined, key: string): string | null => {
          const row = (rows ?? []).find((v) => v.key === key);
          return typeof row?.value === "string" ? row.value : null;
        };
        // Account-saved mode re-applies across devices at session start
        // (device toggle keeps instant local persistence).
        const savedMode = read(userValues, "appearance.mode");
        if (savedMode === "light" || savedMode === "dark") {
          applyTheme(savedMode as Theme);
        }
        // user → org precedence for the preset.
        const override = read(userValues, "appearance.theme_override");
        const orgTheme = read(orgValues, "appearance.theme");
        const preset = override && override !== "org_default" ? override : orgTheme;
        if (!preset || preset === "org_default") return setThemePreset(null);
        if (preset.startsWith("pack:")) {
          const slug = preset.slice("pack:".length);
          // Off-schema presets can never exist (typed enumeration), so the
          // else branch needs no validation; packs verify against the
          // INSTALLED list and fall back to the baseline otherwise.
          const packs = await api.api.GET("/api/v1/plugins/distribution/theme-packs", {});
          const pack = (packs.data?.packs ?? []).find((p) => p.slug === slug);
          if (!pack) return setThemePreset(null);
          const manifest = pack.manifest as {
            light?: Record<string, string>;
            dark?: Record<string, string>;
          };
          applyPackStyles({ slug: pack.slug, light: manifest.light ?? {}, dark: manifest.dark ?? {} });
          setThemePreset(pack.slug);
        } else {
          setThemePreset(preset);
        }
      } catch {
        // Sync failure never breaks rendering — Tangerine baseline stands.
      }
    })();
  }, [auth.status]);
  return null;
}
