// /settings/preferences — per-user preferences.
//
// Purpose: consolidates the account-level surfaces in one place: theme is
// a REAL registry setting (appearance.theme, user scope) written through
// /api/v1/settings/user AND applied locally via the Workbench theme hook;
// the email greeting / sign-off pair (email.greeting, email.signoff —
// user-scope registry settings) autosave debounced through the same
// endpoint; notification channel preferences deep-link to the existing
// dedicated screen rather than duplicating its components.
//
// Non-purpose: no org-scope editing here and no re-implementation of the
// notification preference table (owned by the notifications lane).
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import { Button, Card, FieldHint, Label, Textarea } from "@meridian/ui";
import { useTheme } from "@meridian/ui";
import type { Theme } from "@meridian/ui";
import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";

type GreetingKey = "email.greeting" | "email.signoff";

const GREETING_KEYS: readonly (readonly [GreetingKey, string])[] = [
  ["email.greeting", "Greeting"],
  ["email.signoff", "Sign-off"],
];

/**
 * Debounced autosave for one user-scope text setting — the instant-first
 * dominant path: the local state updates as the user types, the wire write
 * fires after the user stops (600ms), and a typed rejection surfaces
 * inline.
 */
function useDebouncedUserSetting(key: GreetingKey, authed: boolean): {
  value: string;
  onChange(next: string): void;
  state: "idle" | "saving" | "saved" | "error";
  error: string | null;
} {
  const [value, setValue] = useState("");
  const [state, setState] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const runId = useRef(0);

  useEffect(() => {
    if (!authed) return;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/settings/user");
      const row = (data ?? []).find((v) => v.key === key);
      if (row && typeof row.value === "string") setValue(row.value);
    })();
    return () => {
      if (timer.current !== null) clearTimeout(timer.current);
    };
  }, [key, authed]);

  function onChange(next: string): void {
    setValue(next);
    setState("saving");
    setError(null);
    if (timer.current !== null) clearTimeout(timer.current);
    const myRun = ++runId.current;
    timer.current = setTimeout(() => {
      void (async () => {
        const res = await api.api.PUT("/api/v1/settings/user", {
          body: { key, value: next },
        });
        if (runId.current !== myRun) return;
        if (res.error) {
          const detail =
            (res.error as { error?: { message?: string } }).error?.message ?? "rejected";
          setState("error");
          setError(detail);
          return;
        }
        setState("saved");
      })();
    }, 600);
  }

  return { value, onChange, state, error };
}

const GREETING_HINTS: Record<GreetingKey, string> = {
  "email.greeting": "Opens every outgoing ticket email — e.g. “Hi there,”.",
  "email.signoff": "Closes every outgoing ticket email — e.g. “Kind regards,\nThe team”.",
};

export default function UserPreferencesPage() {
  const auth = useRequireAuth();
  const { theme, setTheme } = useTheme();
  const authed = auth.status === "authed";
  // The registry's stored choice, when one exists.
  const [registryTheme, setRegistryTheme] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const greeting = useDebouncedUserSetting("email.greeting", authed);
  const signoff = useDebouncedUserSetting("email.signoff", authed);

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
      setError(friendlyMutationText(e));
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
        <h2 className="text-sm font-semibold">Email signature</h2>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Saved automatically as you type — no Save button. Applied to every
          outgoing ticket email.
        </p>
        {GREETING_KEYS.map(([settingKey, title]) => {
          const hook = settingKey === "email.greeting" ? greeting : signoff;
          return (
            <div key={settingKey} className="grid gap-1">
              <Label htmlFor={settingKey}>{title}</Label>
              <Textarea
                id={settingKey}
                value={hook.value}
                onChange={(e) => hook.onChange(e.target.value)}
                rows={2}
                data-testid={`pref-${settingKey.replaceAll(".", "-")}`}
              />
              <FieldHint>{GREETING_HINTS[settingKey]}</FieldHint>
              {hook.state === "error" && (
                <span
                  className="text-[12px]"
                  role="alert"
                  style={{ color: "var(--m-status-danger-fg)" }}
                  data-testid={`pref-${settingKey.replaceAll(".", "-")}-error`}
                >
                  {hook.error ?? "Save failed — keep editing, it retries on the next keystroke."}
                </span>
              )}
              {hook.state === "saving" && (
                <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }} role="status">
                  Saving…
                </span>
              )}
              {hook.state === "saved" && (
                <span
                  className="text-[12px]"
                  role="status"
                  style={{ color: "var(--m-status-success-fg)" }}
                >
                  Saved to your account.
                </span>
              )}
            </div>
          );
        })}
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
