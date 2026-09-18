// /settings/general — org profile + registry-backed org settings.
//
// Purpose: one page for the tenant-level General section: identity fields
// (name, ticket prefix) via /api/v1/org and every UI-visible org-scope
// declaration rendered FROM the registry payload (kind → control), saved
// through /api/v1/settings/org. Defaults-first: values shown are the
// effective chain (declared default unless overridden).
//
// Non-purpose: user-scope preferences (Preferences page), notification
// channel matrix (dedicated screen), and any editor whose backend lands
// in a later wave.
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import {
  Card,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { SettingsHeader, SaveBar } from "../../../components/settings/settings-shell";

type Declaration = components["schemas"]["DeclarationView"];
type ValueView = components["schemas"]["ValueView"];

interface OrgProfile {
  id: string;
  name: string;
  slug: string;
  ticket_prefix: string;
  base_currency: string;
}

/** Renders one registry declaration as the right control. */
function SettingControl({
  decl,
  value,
  onChange,
}: {
  decl: Declaration;
  value: string | number | boolean;
  onChange: (next: string | number | boolean) => void;
}) {
  const kind = decl.kind;
  if (kind.type === "boolean") {
    return (
      <Select
        value={value ? "true" : "false"}
        onValueChange={(v) => onChange(v === "true")}
      >
        <SelectTrigger className="w-40">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="true">On</SelectItem>
          <SelectItem value="false">Off</SelectItem>
        </SelectContent>
      </Select>
    );
  }
  if (kind.type === "integer") {
    const options: number[] = [];
    for (let n = kind.value.min; n <= kind.value.max; n += 1) options.push(n);
    return (
      <Select value={String(value)} onValueChange={(v) => onChange(Number(v))}>
        <SelectTrigger className="w-40" aria-label={decl.label}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((n) => (
            <SelectItem key={n} value={String(n)}>
              {String(n).padStart(2, "0")}:00
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    );
  }
  if (kind.type === "enumeration") {
    return (
      <Select value={String(value)} onValueChange={(v) => onChange(v)}>
        <SelectTrigger className="w-60" aria-label={decl.label}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {decl.options.map((option) => (
            <SelectItem key={option} value={option}>
              {option}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    );
  }
  // text
  return (
    <Input
      value={String(value)}
      maxLength={kind.value.max_len}
      onChange={(e) => onChange(e.target.value)}
      aria-label={decl.label}
      className="max-w-xs"
    />
  );
}

/** Narrows the registry's JSONB payload to the control-supported types. */
function asControlValue(raw: unknown): string | number | boolean {
  return typeof raw === "string" ||
    typeof raw === "number" ||
    typeof raw === "boolean"
    ? raw
    : "";
}

export default function GeneralSettingsPage() {
  const auth = useRequireAuth();
  const [profile, setProfile] = useState<OrgProfile | null>(null);
  const [profileDraft, setProfileDraft] = useState<{
    name: string;
    ticket_prefix: string;
    base_currency: string;
  } | null>(null);
  const [declarations, setDeclarations] = useState<Declaration[]>([]);
  const [values, setValues] = useState<
    Record<string, string | number | boolean>
  >({});
  const [dirtyKeys, setDirtyKeys] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const [profileRes, registryRes, valuesRes] = await Promise.all([
        api.api.GET("/api/v1/org"),
        api.api.GET("/api/v1/settings/registry"),
        api.api.GET("/api/v1/settings/org"),
      ]);
      if (profileRes.data) {
        setProfile(profileRes.data);
        setProfileDraft({
          name: profileRes.data.name,
          ticket_prefix: profileRes.data.ticket_prefix,
          base_currency: profileRes.data.base_currency,
        });
      }
      if (registryRes.data && valuesRes.data) {
        const uiDecls = (registryRes.data.settings ?? []).filter(
          (d) =>
            d.scope === "org" &&
            d.visibility === "ui" &&
            d.section !== "onboarding",
        );
        setDeclarations(uiDecls);
        const map: Record<string, string | number | boolean> = {};
        // Defaults-first: an unoverridden key renders the shipped default so
        // no control starts blank when the value has never been saved.
        for (const d of uiDecls) {
          const dv = d.default;
          if (typeof dv === "string" || typeof dv === "number" || typeof dv === "boolean") {
            map[d.key] = dv;
          }
        }
        for (const v of valuesRes.data ?? [])
          map[v.key] = asControlValue(v.value);
        setValues(map);
      }
    })();
  }, [auth.status]);

  const profileDirty = useMemo(
    () =>
      profile !== null &&
      profileDraft !== null &&
      (profile.name !== profileDraft.name ||
        profile.ticket_prefix !== profileDraft.ticket_prefix ||
        profile.base_currency !== profileDraft.base_currency.trim().toUpperCase()),
    [profile, profileDraft],
  );

  const patchValue = (key: string, next: string | number | boolean) => {
    setValues((prev) => ({ ...prev, [key]: next }));
    setSaved(false);
    setDirtyKeys((prev) => (prev.includes(key) ? prev : [...prev, key]));
  };

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      if (profileDirty && profileDraft) {
        const res = await api.api.PATCH("/api/v1/org", {
          body: {
            name: profileDraft.name.trim(),
            ticket_prefix: profileDraft.ticket_prefix.trim(),
            base_currency: profileDraft.base_currency.trim().toUpperCase(),
          },
        });
        if (res.error) {
          const detail =
            (res.error as { error?: { message?: string }; response?: Response }).error?.message;
          // A base-currency change is governed (ADR 0002): a 409/typed
          // conflict means conversion snapshots exist — surface the reason.
          throw new Error(detail ?? "org profile was rejected");
        }
        if (res.data) {
          setProfile(res.data);
          setProfileDraft({
            name: res.data.name,
            ticket_prefix: res.data.ticket_prefix,
            base_currency: res.data.base_currency,
          });
        }
      }
      for (const key of dirtyKeys) {
        const res = await api.api.PUT("/api/v1/settings/org", {
          body: { key, value: values[key] },
        });
        if (res.error) {
          const detail =
            (res.error as { error?: { message?: string } }).error?.message ??
            "invalid value";
          throw new Error(`${key}: ${detail}`);
        }
      }
      setDirtyKeys([]);
      setSaved(true);
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setSaving(false);
    }
  };

  const canSave = !saving && (profileDirty || dirtyKeys.length > 0);

  return (
    <div className="grid gap-6 max-w-2xl">
      <SettingsHeader
        title="General"
        description="Tenant identity and organization-wide defaults. Changes apply immediately after saving and are recorded in the audit log."
      />

      {profileDraft && (
        <Card className="p-5 grid gap-3">
          <h2 className="text-sm font-semibold">Organization profile</h2>
          <label className="grid gap-1">
            <Label>Organization name</Label>
            <Input
              value={profileDraft.name}
              onChange={(e) => {
                setProfileDraft({ ...profileDraft, name: e.target.value });
                setSaved(false);
              }}
            />
          </label>
          <label className="grid gap-1">
            <Label>Ticket prefix</Label>
            <Input
              value={profileDraft.ticket_prefix}
              maxLength={8}
              onChange={(e) => {
                setProfileDraft({
                  ...profileDraft,
                  ticket_prefix: e.target.value.toUpperCase(),
                });
                setSaved(false);
              }}
              className="w-32"
            />
            <FieldHint>
              Short uppercase code prepended to ticket numbers.
            </FieldHint>
          </label>
          <label className="grid gap-1">
            <Label>Base currency</Label>
            <Input
              value={profileDraft.base_currency}
              maxLength={3}
              data-testid="org-base-currency"
              onChange={(e) => {
                setProfileDraft({
                  ...profileDraft,
                  base_currency: e.target.value.toUpperCase(),
                });
                setSaved(false);
              }}
              className="w-32"
            />
            <FieldHint>
              ISO 4217 code — the ONE reporting currency all money surfaces
              reconcile to. Changing it is refused while FX conversion
              snapshots exist; record rates first.
            </FieldHint>
          </label>
          <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            Workspace URL slug: {profile?.slug}
          </p>
        </Card>
      )}

      <Card className="p-5 grid gap-4">
        <h2 className="text-sm font-semibold">Org defaults</h2>
        {declarations.map((decl) => (
          <div key={decl.key} className="grid gap-1">
            <Label>{decl.label}</Label>
            <SettingControl
              decl={decl}
              value={asControlValue(values[decl.key])}
              onChange={(next) => patchValue(decl.key, next)}
            />
            <FieldHint>{decl.description}</FieldHint>
          </div>
        ))}
        {declarations.length === 0 && (
          <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            Loading registry…
          </p>
        )}
      </Card>

      <SaveBar dirty={canSave} saving={saving} saved={saved} error={error} onSave={() => void save()} />
    </div>
  );
}
