"use client";
/**
 * AppearanceSync — Wave 24, extended by Wave 41 lane 2b: applies the
 * account's saved appearance.
 *
 * Three settings ride one request, because they live in one row set and a
 * second fetch would be a second round trip for nothing:
 *
 * 1. `appearance.mode` — the account-saved light/dark mode.
 * 2. `appearance.theme_override` / `appearance.theme` — the effective preset,
 *    user→org precedence, mirroring the sticky ai_model_picks shape. The org
 *    value may name an installed theme pack (`pack:<slug>`), in which case the
 *    pack's coerced manifest is applied as inline styles.
 * 3. `appearance.density` — the D7 step, written to `data-density` so
 *    tokens.css moves every control that reads the density tokens.
 *
 * Non-purpose: light/dark MODE's instant toggle (the pre-paint bootstrap
 * applies the stored value before first paint; this re-applies the ACCOUNT
 * value at session start, after the device toggle has had its say), theme-pack
 * distribution (ADR 0009), and any component styling — tokens.css reacts to
 * the attributes on its own.
 *
 * Every path here is fallible and none of them is allowed to break
 * rendering. An unknown mode, preset or step leaves the Marula baseline in
 * place: the theme-token schema can restyle surfaces and nothing else, and a
 * density step the ladder does not define has no token block to fall back
 * from beyond `comfortable`.
 */
import { useEffect } from "react";
import { applyPackStyles, setThemePreset } from "@marula-core/ui";
import type { Theme } from "@marula-core/ui";
import { applyTheme, applyDensity, isDensity, DEFAULT_DENSITY } from "@marula-core/ui";
import { api, useAuth } from "../lib/api";

export function AppearanceSync(): null {
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
        // D7: the account's density step. An unknown value falls back to the
        // Marula default rather than being written through — a step the
        // ladder does not define would leave every control on whatever
        // tokens.css last resolved.
        const savedDensity = read(userValues, "appearance.density");
        applyDensity(isDensity(savedDensity) ? savedDensity : DEFAULT_DENSITY);
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
        // Sync failure never breaks rendering — the Marula baseline stands.
      }
    })();
  }, [auth.status]);
  return null;
}
