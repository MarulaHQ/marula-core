// /settings/preferences — per-user preferences.
//
// Purpose: consolidates the account-level surfaces in one place: theme is
// a REAL registry setting (appearance.theme, user scope) written through
// /api/v1/settings/user AND applied locally via the Workbench theme hook;
// notification channel preferences deep-link to the existing dedicated
// screen rather than duplicating its components.
//
// Non-purpose: no org-scope editing here and no re-implementation of the
// notification preference table (owned by the notifications lane).
"use client";

import { Button, Card, FieldHint, Label } from "@meridian/ui";
import { useTheme } from "@meridian/ui";
import type { Theme } from "@meridian/ui";
import Link from "next/link";
import { useEffect, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";

export default function UserPreferencesPage() {
  const auth = useRequireAuth();
  const { theme, setTheme } = useTheme();
  // The registry's stored choice, when one exists.
  const [registryTheme, setRegistryTheme] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/settings/user");
      if (!data) return;
      const row = (data ?? []).find((v) => v.key === "appearance.theme");
      if (row && typeof row.value === "string") setRegistryTheme(row.value);
    })();
  }, [auth.status]);

  const chooseTheme = async (next: Theme) => {
    setTheme(next); // applies + persists locally for instant feedback
    setSaving(true);
    setError(null);
    setSaved(false);
    try {
      const res = await api.api.PUT("/api/v1/settings/user", {
        body: { key: "appearance.theme", value: next },
      });
      if (res.error) {
        const detail =
          (res.error as { error?: { message?: string } }).error?.message ??
          "rejected";
        throw new Error(detail);
      }
      setRegistryTheme(next);
      setSaved(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="grid gap-6 max-w-2xl">
      <header>
        <h1
          className="text-lg font-semibold"
          style={{ color: "var(--m-text)" }}
        >
          Preferences
        </h1>
        <p
          className="mt-1 text-[13px]"
          style={{ color: "var(--m-text-secondary)" }}
        >
          Settings that travel with your account. Until you save a choice,
          appearance follows your system.
        </p>
      </header>

      <Card className="p-5 grid gap-3">
        <h2 className="text-sm font-semibold">Appearance</h2>
        <Label>Theme</Label>
        <div className="flex gap-2" role="radiogroup" aria-label="Theme">
          {(["light", "dark"] as Theme[]).map((option) => (
            <Button
              key={option}
              variant={theme === option ? "primary" : "secondary"}
              onClick={() => void chooseTheme(option)}
              disabled={saving}
              aria-pressed={theme === option}
            >
              {option === "light" ? "Light" : "Dark"}
            </Button>
          ))}
        </div>
        <FieldHint>
          {registryTheme === null
            ? "No saved choice yet — the app follows your system preference."
            : `Saved choice: ${registryTheme}.`}
        </FieldHint>
        {(saved || error) && (
          <span
            className="text-[12px]"
            role={error ? "alert" : "status"}
            style={{
              color: error
                ? "var(--m-status-danger-fg)"
                : "var(--m-status-success-fg)",
            }}
          >
            {error ?? "Preference saved to your account."}
          </span>
        )}
      </Card>

      <Card className="p-5 grid gap-2">
        <h2 className="text-sm font-semibold">Notifications</h2>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Per-event channel toggles and quiet hours live on their own page —
          every kind shows where its current value comes from (your override →
          org default → built-in).
        </p>
        <Link
          href="/settings/notifications"
          className="text-[13px] underline underline-offset-2"
          style={{ color: "var(--m-accent)" }}
        >
          Open notification settings →
        </Link>
      </Card>
    </div>
  );
}
