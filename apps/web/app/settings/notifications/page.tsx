// Notification preferences screen.
//
// Purpose: per-event-kind channel toggles (in-app / email) plus a
// quiet-hours window (`HH:MM` pair in an IANA timezone) per kind, saved as
// user override rows; each row shows where its current values came from
// (user override → org default → built-in) so defaults-first stays legible.
//
// Non-purpose: no org-default administration (that is an admin surface,
// later stage) and no live-preview of suppression — the worker applies the
// same chain at fan-out time.
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import {
  Button,
  Checkbox,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { SettingsHeader } from "../../../components/settings/settings-shell";
import { kindLabel } from "../../../lib/use-notifications";

type PreferenceRow = components["schemas"]["PreferenceView"];

const TIMEZONES = [
  "UTC",
  "America/Los_Angeles",
  "America/Denver",
  "America/Chicago",
  "America/New_York",
  "America/Sao_Paulo",
  "Europe/London",
  "Europe/Berlin",
  "Europe/Paris",
  "Europe/Warsaw",
  "Europe/Athens",
  "Africa/Cairo",
  "Africa/Johannesburg",
  "Asia/Dubai",
  "Asia/Karachi",
  "Asia/Kolkata",
  "Asia/Bangkok",
  "Asia/Singapore",
  "Asia/Shanghai",
  "Asia/Tokyo",
  "Australia/Perth",
  "Australia/Sydney",
  "Pacific/Auckland",
];

interface Draft {
  in_app: boolean;
  email: boolean;
  quiet_from: string;
  quiet_until: string;
  timezone: string;
}

function draftOf(p: PreferenceRow): Draft {
  return {
    in_app: p.in_app,
    email: p.email,
    quiet_from: p.quiet_from ?? "",
    quiet_until: p.quiet_until ?? "",
    timezone: p.timezone || "UTC",
  };
}

function dirty(p: PreferenceRow, d: Draft): boolean {
  const base = draftOf(p);
  return (
    base.in_app !== d.in_app ||
    base.email !== d.email ||
    base.quiet_from !== d.quiet_from ||
    base.quiet_until !== d.quiet_until ||
    base.timezone !== d.timezone
  );
}

const SOURCE_LABELS: Record<string, string> = {
  user: "your override",
  org_default: "org default",
  builtin: "built-in",
};

export default function NotificationPreferencesPage() {
  const auth = useRequireAuth();
  const [prefs, setPrefs] = useState<PreferenceRow[]>([]);
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);

  // Daily digest opt-in (Wave 34 lane 8): a user-scope typed setting pair
  // (`email.digest`, `email.digest_hour`), read through the settings
  // surface and sent at the chosen hour in this account's timezone.
  const [digest, setDigest] = useState<"off" | "daily">("off");
  const [digestHour, setDigestHour] = useState(7);
  const [digestSaving, setDigestSaving] = useState(false);
  const [digestSaved, setDigestSaved] = useState(false);
  const [digestError, setDigestError] = useState<string | null>(null);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/settings/user");
      if (!data) return;
      for (const row of data) {
        if (row.key === "email.digest" && typeof row.value === "string") {
          setDigest(row.value === "daily" ? "daily" : "off");
        }
        if (row.key === "email.digest_hour" && typeof row.value === "number") {
          setDigestHour(row.value);
        }
      }
    })();
  }, [auth.status]);

  const saveDigest = async () => {
    setDigestSaving(true);
    setDigestError(null);
    try {
      for (const [key, value] of [
        ["email.digest", digest],
        ["email.digest_hour", digestHour],
      ] as const) {
        const res = await api.api.PUT("/api/v1/settings/user", {
          body: { key, value },
        });
        if (res.error) {
          throw new Error(
            (res.error as { error?: { message?: string } }).error?.message ??
              "The digest setting was rejected.",
          );
        }
      }
      setDigestSaved(true);
    } catch (e) {
      setDigestError(friendlyMutationText(e));
    } finally {
      setDigestSaving(false);
    }
  };

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/notification-preferences");
      if (data) {
        setPrefs(data);
        setDrafts(Object.fromEntries(data.map((p) => [p.kind, draftOf(p)])));
      }
    })();
  }, [auth.status]);

  const changedKinds = useMemo(
    () =>
      prefs
        .filter((p) => {
          const d = drafts[p.kind];
          return d !== undefined && dirty(p, d);
        })
        .map((p) => p.kind),
    [prefs, drafts],
  );

  const patch = (kind: string, update: Partial<Draft>) => {
    setDrafts((prev) => {
      const current = prev[kind];
      if (!current) return prev;
      return { ...prev, [kind]: { ...current, ...update } };
    });
    setSavedAt(null);
  };

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      for (const kind of changedKinds) {
        const d = drafts[kind];
        if (!d) continue;
        // Quiet bounds are a pair: blanking one side blanks both.
        const quiet_from = d.quiet_from === "" ? null : d.quiet_from;
        const quiet_until = d.quiet_until === "" ? null : d.quiet_until;
        const res = await api.api.PUT("/api/v1/notification-preferences", {
          body: {
            kind: kind as never,
            in_app: d.in_app,
            email: d.email,
            quiet_from,
            quiet_until,
            timezone: d.timezone,
          },
        });
        if (res.error) {
          const message =
            (res.error as { error?: { message?: string } }).error?.message ??
            "one row was rejected";
          throw new Error(`${kindLabel(kind)}: ${message}`);
        }
      }
      const { data } = await api.api.GET("/api/v1/notification-preferences");
      if (data) {
        setPrefs(data);
        setDrafts(Object.fromEntries(data.map((p) => [p.kind, draftOf(p)])));
      }
      setSavedAt(Date.now());
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="grid gap-4">
      <SettingsHeader
        title="Notifications"
        description="Times shown in each row's timezone. Quiet hours hold non-critical email during the window; SLA breaches and escalations always break through. Rows without your override follow the org default or the built-in."
      />

      <div
        className="rounded-md border p-3 grid gap-3"
        style={{
          borderColor: "var(--m-border)",
          background: "var(--m-surface)",
        }}
      >
        <div>
          <div className="text-[13px] font-semibold">Daily digest email</div>
          <p
            className="mt-0.5 text-[12px]"
            style={{ color: "var(--m-text-secondary)" }}
          >
            One email a day summarizing your unread notifications, sent at the
            hour you pick in your account timezone. Because it is scheduled
            rather than interruptive, quiet hours do not hold it.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <Select
            value={digest}
            onValueChange={(v) => {
              setDigest(v === "daily" ? "daily" : "off");
              setDigestSaved(false);
            }}
          >
            <SelectTrigger className="w-[150px]" aria-label="Daily digest">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="off">Off</SelectItem>
              <SelectItem value="daily">Daily</SelectItem>
            </SelectContent>
          </Select>
          <label className="flex items-center gap-2 text-[12px]">
            <span style={{ color: "var(--m-text-secondary)" }}>Send at</span>
            <Select
              value={String(digestHour)}
              onValueChange={(v) => {
                setDigestHour(Number.parseInt(v, 10));
                setDigestSaved(false);
              }}
            >
              <SelectTrigger className="w-[110px]" aria-label="Digest send hour">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {Array.from({ length: 24 }, (_, hour) => (
                  <SelectItem key={hour} value={String(hour)}>
                    {String(hour).padStart(2, "0")}:00
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
          <Button
            variant="secondary"
            disabled={digestSaving}
            onClick={() => void saveDigest()}
          >
            {digestSaving ? "Saving…" : "Save digest"}
          </Button>
          {digestSaved && !digestError ? (
            <span
              className="text-[12px]"
              style={{ color: "var(--m-status-success-fg)" }}
            >
              Saved.
            </span>
          ) : null}
          {digestError ? (
            <span
              className="text-[12px]"
              role="alert"
              style={{ color: "var(--m-status-danger-fg)" }}
            >
              {digestError}
            </span>
          ) : null}
        </div>
      </div>

      <div
        className="rounded-md border overflow-hidden"
        style={{
          borderColor: "var(--m-border)",
          background: "var(--m-surface)",
        }}
      >
        <table
          className="w-full text-[13px]"
          style={{ color: "var(--m-text)" }}
        >
          <thead>
            <tr
              className="text-left text-[11px] uppercase tracking-wide"
              style={{
                color: "var(--m-text-muted)",
                background: "var(--m-canvas)",
              }}
            >
              <th className="px-3 py-2 font-medium">Event</th>
              <th className="px-3 py-2 font-medium">In-app</th>
              <th className="px-3 py-2 font-medium">Email</th>
              <th className="px-3 py-2 font-medium">Quiet from</th>
              <th className="px-3 py-2 font-medium">Until</th>
              <th className="px-3 py-2 font-medium">Timezone</th>
            </tr>
          </thead>
          <tbody>
            {prefs.map((p) => {
              const d = drafts[p.kind];
              if (!d) return null;
              return (
                <tr
                  key={p.kind}
                  className="border-t"
                  style={{ borderColor: "var(--m-border)" }}
                >
                  <td className="px-3 py-2.5">
                    <div>{kindLabel(p.kind)}</div>
                    <div
                      className="text-[11px]"
                      style={{ color: "var(--m-text-muted)" }}
                    >
                      {SOURCE_LABELS[p.source] ?? p.source}
                      {changedKinds.includes(p.kind) ? " · edited" : ""}
                    </div>
                  </td>
                  <td className="px-3 py-2.5">
                    <Checkbox
                      checked={d.in_app}
                      onCheckedChange={(v) =>
                        patch(p.kind, { in_app: v === true })
                      }
                      aria-label={`${kindLabel(p.kind)} in-app`}
                    />
                  </td>
                  <td className="px-3 py-2.5">
                    <Checkbox
                      checked={d.email}
                      onCheckedChange={(v) =>
                        patch(p.kind, { email: v === true })
                      }
                      aria-label={`${kindLabel(p.kind)} email`}
                    />
                  </td>
                  <td className="px-3 py-2.5">
                    <Input
                      type="time"
                      value={d.quiet_from}
                      onChange={(e) =>
                        patch(p.kind, { quiet_from: e.target.value })
                      }
                      aria-label={`${kindLabel(p.kind)} quiet from`}
                      className="w-[110px]"
                    />
                  </td>
                  <td className="px-3 py-2.5">
                    <Input
                      type="time"
                      value={d.quiet_until}
                      onChange={(e) =>
                        patch(p.kind, { quiet_until: e.target.value })
                      }
                      aria-label={`${kindLabel(p.kind)} quiet until`}
                      className="w-[110px]"
                    />
                  </td>
                  <td className="px-3 py-2.5">
                    <Select
                      value={d.timezone}
                      onValueChange={(v) => patch(p.kind, { timezone: v })}
                    >
                      <SelectTrigger
                        className="w-[180px]"
                        aria-label={`${kindLabel(p.kind)} timezone`}
                      >
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {TIMEZONES.map((tz) => (
                          <SelectItem key={tz} value={tz}>
                            {tz}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      <div className="flex items-center gap-3 mt-4">
        <Button
          variant="primary"
          disabled={changedKinds.length === 0 || saving}
          onClick={() => void save()}
        >
          {saving ? "Saving…" : "Save changes"}
        </Button>
        {savedAt && !error && (
          <span
            className="text-[12px]"
            style={{ color: "var(--m-status-neutral-fg)" }}
          >
            Saved.
          </span>
        )}
        {error && (
          <span
            className="text-[12px]"
            style={{ color: "var(--m-status-danger-fg)" }}
            role="alert"
          >
            {error}
          </span>
        )}
      </div>
    </div>
  );
}
