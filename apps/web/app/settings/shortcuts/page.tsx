// /settings/shortcuts — keyboard-shortcut profile + the full registry.
//
// Purpose: the settings face of the Wave 29 keyboard seam. The profile is
// a per-user choice on the typed `shortcuts.profile` setting ('default' or
// 'all_off'; per-command custom is reserved for later in this same view).
// The registry table below renders from lib/commands/registry — the SAME
// single source the hold-Alt overlay and the ?-cheat sheet use, so this
// page can never drift from what actually dispatches.

/**
 * Per-user keyboard-shortcut profile + the full registry (Wave 29 lane 1).
 * The profile rides the typed `shortcuts.profile` user setting ('default'
 * or 'all_off'; per-command custom is reserved for later). The table
 * renders from lib/commands/registry — the same single source the
 * hold-Alt overlay and the ?-cheat sheet use.
 */
"use client";

import { useEffect, useState } from "react";
import { Button, Card, FieldHint, Label } from "@marula-core/ui";
import { api, useRequireAuth } from "../../../lib/api";
import { friendlyMutationText } from "../../../lib/error-copy";
import { SettingsHeader } from "../../../components/settings/settings-shell";
import { registryGroups } from "../../../lib/commands/registry";
import { displayChord } from "../../../lib/commands/chords";
import type { ShortcutProfile } from "../../../lib/commands/types";

const PROFILES: ShortcutProfile[] = ["default", "all_off"];

/** Machine chord → OS-native label for the search palette row. */
function kbdLabel(spec: string): string {
  const isMac = typeof navigator !== "undefined" && /mac/i.test(navigator.platform ?? "");
  return displayChord(spec).replace("Mod", isMac ? "⌘" : "Ctrl");
}

const PROFILE_HINTS: Record<ShortcutProfile, string> = {
  default: "Chords and the hold-Alt overlay are active.",
  all_off:
    "Chords and the hold-Alt overlay are off. Press ? any time to read the full cheat sheet (it is reference only).",
};

export default function ShortcutSettingsPage() {
  const auth = useRequireAuth();
  const authed = auth.status === "authed";
  const [profile, setProfile] = useState<ShortcutProfile | null>(null);
  const [busy, setBusy] = useState<ShortcutProfile | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<ShortcutProfile | null>(null);
  const groups = registryGroups();

  useEffect(() => {
    if (!authed) return;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/settings/user");
      if (!data) return;
      const row = (data ?? []).find((v) => v.key === "shortcuts.profile");
      if (row && (row.value === "default" || row.value === "all_off")) {
        setProfile(row.value);
      } else {
        setProfile("default");
      }
    })();
  }, [authed]);

  const chooseProfile = async (next: ShortcutProfile) => {
    const previous = profile ?? "default";
    setProfile(next);
    setBusy(next);
    setError(null);
    setSaved(null);
    try {
      const res = await api.api.PUT("/api/v1/settings/user", {
        body: { key: "shortcuts.profile", value: next },
      });
      if (res.error) {
        const detail =
          (res.error as { error?: { message?: string } }).error?.message ?? "rejected";
        throw new Error(detail);
      }
      setSaved(next);
    } catch (e) {
      setError(friendlyMutationText(e));
      setProfile(previous);
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="grid gap-6 max-w-3xl">
      <SettingsHeader
        title="Keyboard shortcuts"
        description="How keyboard commands behave for your account. Hold Alt anywhere to reveal the shortcuts for the surface you are on; press ? for the full cheat sheet."
      />

      <Card className="grid gap-3 p-5" data-testid="shortcuts-profile">
        <h2 className="text-sm font-semibold">Shortcut profile</h2>
        <Label>Profile</Label>
        <div className="flex gap-2" role="radiogroup" aria-label="Keyboard shortcuts profile" data-testid="shortcut-profile-group">
          {PROFILES.map((option) => (
            <Button
              key={option}
              variant={profile === option ? "primary" : "secondary"}
              onClick={() => void chooseProfile(option)}
              disabled={busy !== null}
              aria-pressed={profile === option}
              data-testid={`shortcut-profile-${option}`}
            >
              {option === "default" ? "Default" : "All off"}
            </Button>
          ))}
        </div>
        <FieldHint>
          {PROFILE_HINTS[profile ?? "default"]}
        </FieldHint>
        {(saved !== null || error !== null) && (
          <span
            className="text-xs"
            role={error ? "alert" : "status"}
            style={{ color: error ? "var(--m-status-danger-fg)" : "var(--m-status-ok-fg)" }}
            data-testid="shortcuts-save-state"
          >
            {error ? error : "Saved."}
          </span>
        )}
      </Card>

      <Card className="grid gap-4 p-5" data-testid="shortcuts-registry">
        <div>
          <h2 className="text-sm font-semibold">The full registry</h2>
          <FieldHint>
            The same commands, rendered by the hold-Alt overlay per surface and the ?-cheat sheet. Chords never
            collide and never touch OS/browser-reserved combos — a lint check in this repository fails the tests
            if they would.
          </FieldHint>
        </div>
        {groups.map((group) => (
          <section key={group.title} aria-label={group.title}>
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              {group.title}
            </h3>
            <ul className="grid gap-2" data-testid="shortcuts-registry-rows">
              {group.commands.map((command) => (
                <li key={command.id} className="flex items-start justify-between gap-6">
                  <span className="flex min-w-0 flex-col">
                    <span className="text-[13px] font-medium">{command.label}</span>
                    <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                      {command.description}
                    </span>
                  </span>
                  <kbd
                    className="shrink-0 rounded-md border px-1.5 py-0.5 font-mono text-xs leading-none"
                    style={{ borderColor: "var(--m-border)" }}
                  >
                    {kbdLabel(command.chord)}
                  </kbd>
                </li>
              ))}
            </ul>
          </section>
        ))}
        <FieldHint>
          Per-command remapping ("Custom") arrives later on this same page.
        </FieldHint>
      </Card>
    </div>
  );
}
