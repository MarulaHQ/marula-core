"use client";
/**
 * /settings/appearance — Wave 24 theming system, extended by Wave 41 lane 2b.
 *
 * Purpose: the three appearance preferences, all of them per-user except the
 * org default. The theme-preset picker — org-wide default (admins) with a
 * per-user override riding the typed settings model — plus the installed
 * custom theme packs (ADR 0009 V1 distribution: approval of a staged `theme`
 * import mints the pack; this surface lists it, activates it, and uninstalls
 * it). The D7 density step lives here too, because density is how much room
 * the product takes and that is an appearance decision; its three
 * measurements are on the control, since "Compact" means nothing to a user
 * until they know it is 40px rows.
 *
 * Non-purpose: light/dark mode (Preferences page, appearance.mode) and pack
 * distribution itself (plugins staging + approval). The per-surface views,
 * columns, filters and export (D12) are not here either: they belong to the
 * table they describe, and they write their own key.
 */
import { useCallback, useEffect, useState } from "react";
import { Button, Card, DensitySwitch, FieldHint, Label, Tag } from "@marula-core/ui";
import { BUILTIN_PRESETS, DEFAULT_DENSITY, isBuiltinPreset, type ThemePreset } from "@marula-core/ui";
import { api, useRequireAuth } from "../../../lib/api";
import { SettingsHeader } from "../../../components/settings/settings-shell";
import { useDataSurfacePrefs } from "../../../lib/data-surface-prefs";

/** Swatch trio for one mode of a preset: canvas / surface / accent. */
function Swatch({ value, accent }: { value: string; accent: string }): React.ReactNode {
  return (
    <div className="flex h-8 overflow-hidden rounded-[var(--m-radius-control)] border" style={{ borderColor: "var(--m-border)" }}>
      <div className="w-10" style={{ background: value }} />
      <div className="w-10 border-l" style={{ background: "var(--m-paper)", borderColor: "var(--m-border)" }} />
      <div className="w-6 border-l" style={{ background: accent, borderColor: "var(--m-border)" }} />
    </div>
  );
}

function PresetCard({
  preset,
  dark,
  active,
  onSelect,
  disabled,
}: {
  preset: ThemePreset;
  dark: boolean;
  active: boolean;
  onSelect: () => void;
  disabled: boolean;
}) {
  const values = dark ? preset.dark : preset.light;
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      onClick={onSelect}
      disabled={disabled}
      data-testid={`appearance-preset-${preset.id}`}
      className="group rounded-[var(--m-radius-control)] border p-3 text-left transition-[transform,opacity] duration-[120ms] hover:-translate-y-px focus-visible:outline focus-visible:outline-2 motion-reduce:transform-none motion-reduce:hover:translate-y-0 disabled:opacity-50"
      style={{
        borderColor: active ? "var(--m-accent)" : "var(--m-border)",
        outlineColor: "var(--m-accent)",
      }}
    >
      <div className="grid gap-2">
        <Swatch value={values.background as string} accent={values.accent as string} />
        <span className="text-[13px] font-semibold">{preset.label}</span>
        <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
          {preset.description}
        </span>
      </div>
    </button>
  );
}

interface PackRow {
  slug: string;
  name: string;
  version: string;
  description: string;
  source_kind: string;
  active: boolean;
}

export default function AppearanceSettingsPage() {
  const auth = useRequireAuth();
  const authed = auth.status === "authed";
  const isAdmin = (auth.me?.permissions ?? []).includes("special:manage_plugins");
  const prefs = useDataSurfacePrefs();
  const [orgTheme, setOrgTheme] = useState<string>("marula");
  const [override, setOverride] = useState<string>("org_default");
  const [packs, setPacks] = useState<PackRow[]>([]);
  const [saving, setSaving] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!authed) return;
    try {
      const [orgRes, userRes, packRes] = await Promise.all([
        api.api.GET("/api/v1/settings/org"),
        api.api.GET("/api/v1/settings/user"),
        api.api.GET("/api/v1/plugins/distribution/theme-packs", {}),
      ]);
      const orgRow = (orgRes.data ?? []).find((v) => v.key === "appearance.theme");
      const userRow = (userRes.data ?? []).find((v) => v.key === "appearance.theme_override");
      setOrgTheme(
        typeof orgRow?.value === "string" && orgRow.value !== ""
          ? orgRow.value
          : "marula",
      );
      setOverride(
        typeof userRow?.value === "string" && userRow.value !== ""
          ? userRow.value.slice(0, 64)
          : "org_default",
      );
      setPacks(
        (packRes.data?.packs ?? []).map((p) => ({
          slug: p.slug,
          name: p.name,
          version: p.version,
          description: p.description,
          source_kind: p.source_kind,
          active: p.active,
        })),
      );
    } catch {
      // Values stay on defaults; the wire errors surface on the next write.
    }
  }, [authed]);

  useEffect(() => {
    void load();
  }, [load]);

  async function putOrgTheme(value: string) {
    setSaving("org");
    setNotice(null);
    try {
      const res = await api.api.PUT("/api/v1/settings/org", {
        body: { key: "appearance.theme", value },
      });
      if (res.error) {
        const detail = (res.error as { error?: { message?: string } }).error?.message ?? "rejected";
        setNotice(detail);
        return;
      }
      setOrgTheme(value);
      setNotice("Org theme saved.");
    } finally {
      setSaving(null);
    }
  }

  async function putOverride(value: string) {
    setSaving("override");
    setNotice(null);
    try {
      const res = await api.api.PUT("/api/v1/settings/user", {
        body: { key: "appearance.theme_override", value },
      });
      if (res.error) {
        const detail = (res.error as { error?: { message?: string } }).error?.message ?? "rejected";
        setNotice(detail);
        return;
      }
      setOverride(value);
      setNotice("Saved to your account.");
    } finally {
      setSaving(null);
    }
  }

  async function uninstallPack(slug: string) {
    setSaving(slug);
    setNotice(null);
    try {
      const res = await api.api.DELETE(
        "/api/v1/plugins/distribution/theme-packs/{slug}" as "/api/v1/plugins/distribution/theme-packs/{slug}",
        { params: { path: { slug } } },
      );
      if (res.error) {
        const detail = (res.error as { error?: { message?: string } }).error?.message ?? "rejected";
        setNotice(detail);
        return;
      }
      await load();
      setNotice("The theme pack is uninstalled.");
    } finally {
      setSaving(null);
    }
  }

  // The org setting only accepts built-ins or the pack:* sentinel shape;
  // the pack UI activates a pack via the same org setting.
  const activeOrgSlug = orgTheme.startsWith("pack:") ? orgTheme.slice("pack:".length) : null;
  const activeOrgPack = activeOrgSlug;

  return (
    <div className="grid gap-6 max-w-2xl">
      <SettingsHeader
        title="Appearance"
        description="Themes are restyle-only token sets — Marula by default, org-wide with a per-user override. Density is yours alone. Light / dark lives on your Preferences page."
      />

      <Card className="grid gap-4 p-5" data-testid="appearance-density">
        <div>
          <h2 className="text-sm font-semibold">Density</h2>
          <FieldHint>
            How much room the interface takes. Yours alone — nobody else&apos;s
            view changes, and it follows you to every device.
          </FieldHint>
        </div>
        <DensitySwitch
          value={prefs.density}
          onChange={(step) => void prefs.setDensity(step)}
          disabled={!prefs.ready}
        />
        <FieldHint>
          Tap targets stay at 44px on every step, including Dense, so the same
          control is no harder to hit on a touchscreen at any density.
        </FieldHint>
        {prefs.error ? (
          <p className="text-[12px]" style={{ color: "var(--m-destructive)" }} role="alert">
            {prefs.error}
          </p>
        ) : null}
      </Card>

      <Card className="p-5 grid gap-4">
        <div>
          <h2 className="text-sm font-semibold">Org theme</h2>
          <FieldHint>
            {isAdmin
              ? "The preset applied company-wide until a user overrides it. Admins choose."
              : "Set by your org admins; shown here for reference."}
          </FieldHint>
        </div>
        <div
          role="radiogroup"
          aria-label="Org theme"
          className="grid grid-cols-2 gap-3 sm:grid-cols-4"
        >
          {BUILTIN_PRESETS.map((preset) => (
            <PresetCard
              key={preset.id}
              preset={preset}
              dark={false}
              active={orgTheme === preset.id}
              onSelect={() => isAdmin && void putOrgTheme(preset.id)}
              disabled={!isAdmin || saving !== null || Boolean(activeOrgSlug)}
            />
          ))}
        </div>
        {packs.length > 0 && (
          <div className="grid gap-2">
            <Label>Installed theme packs</Label>
            {packs.map((pack) => (
              <div
                key={pack.slug}
                className="flex flex-wrap items-center gap-2 rounded-[var(--m-radius-control)] border px-3 py-2"
                style={{ borderColor: "var(--m-border)" }}
                data-testid={`appearance-pack-${pack.slug}`}
              >
                <div className="grid min-w-40 gap-0.5">
                  <span className="text-[13px] font-semibold">
                    {pack.name} <Tag tone="neutral">{`v${pack.version}`}</Tag>
                  </span>
                  {pack.description ? (
                    <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                      {pack.description}
                    </span>
                  ) : null}
                  <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    {pack.source_kind === "marketplace_registry"
                      ? "Marketplace"
                      : pack.source_kind === "github_repo"
                        ? "GitHub"
                        : "Local zip"}{" "}
                    pack — restyle-only (no capabilities)
                  </span>
                </div>
                <div className="ms-auto flex gap-2">
                  <Button
                    size="sm"
                    variant={activeOrgPack === pack.slug ? "primary" : "secondary"}
                    disabled={saving !== null}
                    onClick={() => isAdmin && void putOrgTheme(`pack:${pack.slug}`)}
                  >
                    {activeOrgPack === pack.slug ? "Active" : "Use org-wide"}
                  </Button>
                  {isAdmin && (
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={saving !== null}
                      onClick={() => void uninstallPack(pack.slug)}
                    >
                      Uninstall
                    </Button>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </Card>

      <Card className="p-5 grid gap-3">
        <h2 className="text-sm font-semibold">Per-user override</h2>
        <FieldHint>
          Choose a preset just for this account, or follow the org theme. The
          override wins over the org choice (the same precedence your AI model
          picks use).
        </FieldHint>
        <div
          role="radiogroup"
          aria-label="Theme override"
          className="grid grid-cols-2 gap-3 sm:grid-cols-4"
        >
          {[
            {
              id: "org_default",
              label: "Org default",
              description: "Follow whatever the org choose.",
            },
            ...BUILTIN_PRESETS,
          ].map((presetOption) => (
            <PresetCard
              key={presetOption.id}
              preset={
                isBuiltinPreset(presetOption.id)
                  ? (BUILTIN_PRESETS.find((p) => p.id === presetOption.id) as ThemePreset)
                  : OVERRIDE_ORG_CARD
              }
              dark={false}
              active={override === presetOption.id}
              onSelect={() => void putOverride(presetOption.id)}
              disabled={saving !== null}
            />
          ))}
        </div>
      </Card>

      {notice && (
        <span
          className="text-[12px]"
          role="status"
          style={{ color: "var(--m-status-success-fg)" }}
        >
          {notice}
        </span>
      )}
    </div>
  );
}

const OVERRIDE_ORG_CARD: ThemePreset = {
  id: "org_default",
  label: "Org default",
  description: "Follow the org theme.",
  light: { background: "var(--m-canvas)", accent: "var(--m-accent)" },
  dark: {},
};
