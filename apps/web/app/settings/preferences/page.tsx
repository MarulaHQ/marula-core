// /settings/preferences — per-user preferences.
//
// Purpose: consolidates the account-level surfaces in one place: appearance
// mode is a REAL registry setting (appearance.mode, user scope — Wave 24
// supersede of the old flat appearance.theme key) written through
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
import {
  Button,
  Card,
  FieldHint,
  Input,
  Label,
  Spinner,
  Tag,
  Textarea,
} from "@marula-core/ui";
import { useTheme } from "@marula-core/ui";
import type { Theme } from "@marula-core/ui";
import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { api, apiBaseUrl, authHeaders, useRequireAuth } from "../../../lib/api";
import { useToursState } from "../../../lib/help/help-settings";
import { SettingsHeader } from "../../../components/settings/settings-shell";

/** The two per-user email keys this card owns, in editor order. */
const GREETING_HINTS: Record<string, string> = {
  "email.greeting": "Opens every outgoing ticket email — e\u2019Hi there,\u2019",
  "email.signoff": "Closes every outgoing ticket email — e.g. \u201cKind regards,\n{{user.name}}\u201d.",
};

/** One insertable placeholder, with the value it expands to for this member. */
interface SignatureToken {
  key: string;
  label: string;
  value: string | null;
}

/**
 * The caller's resolved envelope. The endpoint is pinned by this lane's
 * OpenAPI regen; until the pin lands the fetch is typed here rather than
 * through the generated client, which is the same escape the other
 * settings pages use for a shape the SDK does not carry yet.
 */
interface EmailEnvelope {
  policy: "allowed" | "org_locked";
  greeting: string | null;
  greeting_source: string;
  signature: string | null;
  signature_source: string;
  org_signature: string | null;
  user_greeting: string;
  user_signature: string;
  tokens: SignatureToken[];
}

const ENVELOPE_PATH = "/api/v1/settings/user/email-envelope";

async function fetchEnvelope(): Promise<EmailEnvelope | null> {
  // The API base may carry a trailing slash (it comes from
  // NEXT_PUBLIC_API_URL), so a rooted path has to join with exactly one
  // separator — `base + "/api/..."` on a base ending in `/` is `//api/...`,
  // and the router answers that with a 404 that looks like a missing
  // endpoint rather than a malformed URL.
  const response = await fetch(`${apiBaseUrl().replace(/\/$/, "")}${ENVELOPE_PATH}`, {
    headers: authHeaders(),
  });
  if (!response.ok) return null;
  return (await response.json()) as EmailEnvelope;
}

/** Reads a stored user setting straight from the registry projection. */
async function putUserSetting(key: string, value: string): Promise<string | null> {
  const res = await api.api.PUT("/api/v1/settings/user", { body: { key, value } });
  if (res.error) {
    return (
      (res.error as { error?: { message?: string } }).error?.message ?? "rejected"
    );
  }
  return null;
}

/** Renders the source a resolved block came from, in the member's own words. */
const SOURCE_LABEL: Record<string, string> = {
  message_override: "this message",
  user_default: "your default",
  board_identity: "the board",
  org_default: "the organization",
  product: "a fixed Marula line",
  unset: "nothing yet",
};

/**
 * The per-user greeting and signature (Wave 42).
 *
 * The signature is edited as PLACEHOLDERS and previewed as VALUES: the
 * picker inserts `{{user.name}}`, the preview shows what that becomes for
 * this member right now, and the send path expands the placeholder at send
 * time against the sending user's row. A preview that showed the rendered
 * text and saved it back would bake today's name into tomorrow's mail.
 */
function SignatureCard({ authed }: { authed: boolean }) {
  const [envelope, setEnvelope] = useState<EmailEnvelope | null>(null);
  const [fields, setFields] = useState<Record<string, string>>({});
  const [saveState, setSaveState] = useState<Record<string, "idle" | "saving" | "saved" | "error">>({});
  const [errors, setErrors] = useState<Record<string, string | null>>({});
  const [resetting, setResetting] = useState(false);
  const timers = useRef<Record<string, ReturnType<typeof setTimeout> | null>>({});
  const runIds = useRef<Record<string, number>>({});
  const signatureRef = useRef<HTMLTextAreaElement | null>(null);

  const locked = envelope?.policy === "org_locked";

  useEffect(() => {
    if (!authed) return;
    let cancelled = false;
    void (async () => {
      const loaded = await fetchEnvelope();
      if (cancelled || loaded === null) return;
      setEnvelope(loaded);
      setFields({
        "email.greeting": loaded.user_greeting,
        "email.signoff": loaded.user_signature,
      });
    })();
    return () => {
      cancelled = true;
      for (const timer of Object.values(timers.current)) {
        if (timer !== null && timer !== undefined) clearTimeout(timer);
      }
    };
  }, [authed]);

  /**
   * Debounced autosave per key — the instant-first dominant path, same as
   * the rest of this page: the field updates as you type, the wire write
   * fires once you stop, and a rejection lands on the field that caused it.
   * A newer keystroke wins: the run id is compared before any state is
   * written, so a slow save can never overwrite the field you are still
   * editing.
   */
  function edit(key: string, next: string): void {
    setFields((current) => ({ ...current, [key]: next }));
    setSaveState((current) => ({ ...current, [key]: "saving" }));
    setErrors((current) => ({ ...current, [key]: null }));
    const previous = timers.current[key];
    if (previous !== null && previous !== undefined) clearTimeout(previous);
    const myRun = (runIds.current[key] ?? 0) + 1;
    runIds.current[key] = myRun;
    timers.current[key] = setTimeout(() => {
      void (async () => {
        const rejection = await putUserSetting(key, next);
        if (runIds.current[key] !== myRun) return;
        if (rejection !== null) {
          setSaveState((current) => ({ ...current, [key]: "error" }));
          setErrors((current) => ({ ...current, [key]: rejection }));
          return;
        }
        setSaveState((current) => ({ ...current, [key]: "saved" }));
        const refreshed = await fetchEnvelope();
        if (runIds.current[key] === myRun && refreshed !== null) setEnvelope(refreshed);
      })();
    }, 600);
  }

  /** Inserts a placeholder at the caret, on its own line, and re-previews. */
  function insertToken(key: string): void {
    const field = signatureRef.current;
    if (field === null || locked) return;
    const at = field.selectionStart ?? fields["email.signoff"]?.length ?? 0;
    const current = fields["email.signoff"] ?? "";
    const before = current.slice(0, at);
    const after = current.slice(field.selectionEnd ?? at);
    const token = `{{${key}}}`;
    const spacer = before.length > 0 && !before.endsWith("\n") ? "\n" : "";
    edit("email.signoff", `${before}${spacer}${token}${after}`);
    const caret = before.length + spacer.length + token.length;
    requestAnimationFrame(() => {
      field.focus();
      field.setSelectionRange(caret, caret);
    });
  }

  /** Clears the stored default, so the next message falls back to the org's. */
  function resetToDefault(): void {
    if (envelope === null) return;
    setResetting(true);
    void (async () => {
      const rejection = await putUserSetting("email.signoff", "");
      setResetting(false);
      if (rejection !== null) {
        setSaveState((current) => ({ ...current, "email.signoff": "error" }));
        setErrors((current) => ({ ...current, "email.signoff": rejection }));
        return;
      }
      setFields((current) => ({ ...current, "email.signoff": "" }));
      setSaveState((current) => ({ ...current, "email.signoff": "saved" }));
      const refreshed = await fetchEnvelope();
      if (refreshed !== null) setEnvelope(refreshed);
    })();
  }

  // The preview is the server's own rendering of the SAVED block, resolved
  // against this member's row — so what is read here is what the next
  // message carries, and it moves the moment the row does. It is labelled
  // as the saved block because mid-edit the value that would send is
  // still the stored one, not the keystroke.
  const previewSignature = envelope?.signature ?? null;
  const previewGreeting = envelope?.greeting ?? null;

  return (
    <Card className="p-5 grid gap-2" data-testid="pref-signature-card">
      <h2 className="text-sm font-semibold">Email signature</h2>
      <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
        {locked
          ? "This workspace sends one signature of its own. An admin can allow personal ones."
          : "Saved automatically as you type — no Save button. Placeholders fill in when a message sends, so a new name or address reaches your mail without saving it again."}
      </p>

      {locked && (
        <div
          className="grid gap-2 rounded-lg border p-3"
          data-testid="pref-signature-locked"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface-muted)" }}
        >
          <div className="flex flex-wrap items-center gap-2">
            <Tag tone="neutral">Locked to the organization</Tag>
            <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              Your messages carry the workspace signature.
            </span>
          </div>
          {envelope?.org_signature != null && envelope.org_signature.length > 0 && (
            <pre
              className="m-0 whitespace-pre-wrap text-[13px]"
              style={{ color: "var(--m-text)" }}
              data-testid="pref-signature-locked-block"
            >
              {envelope.org_signature}
            </pre>
          )}
        </div>
      )}

      <div className="grid gap-1">
        <Label htmlFor="email.greeting">Greeting</Label>
        <Input
          id="email.greeting"
          value={fields["email.greeting"] ?? ""}
          onChange={(e) => edit("email.greeting", e.target.value)}
          data-testid="pref-email-greeting"
        />
        <FieldHint>{GREETING_HINTS["email.greeting"]}</FieldHint>
        <SaveNote
          state={saveState["email.greeting"]}
          error={errors["email.greeting"]}
          testId="pref-email-greeting"
        />
      </div>

      <div className="grid gap-1">
        <div className="flex flex-wrap items-baseline justify-between gap-2">
          <Label htmlFor="email.signoff">Signature</Label>
          {!locked && (fields["email.signoff"] ?? "").length > 0 && (
            <Button
              size="sm"
              variant="ghost"
              onClick={resetToDefault}
              disabled={resetting}
              data-testid="pref-signature-reset"
            >
              {resetting ? "Clearing…" : "Use the workspace signature"}
            </Button>
          )}
        </div>
        <Textarea
          id="email.signoff"
          ref={signatureRef}
          value={fields["email.signoff"] ?? ""}
          onChange={(e) => edit("email.signoff", e.target.value)}
          rows={3}
          disabled={locked}
          data-testid="pref-email-signoff"
        />
        <FieldHint>
          {locked
            ? "Locked: an admin set the workspace to a single signature."
            : GREETING_HINTS["email.signoff"]}
        </FieldHint>
        <SaveNote
          state={saveState["email.signoff"]}
          error={errors["email.signoff"]}
          testId="pref-email-signoff"
        />
      </div>

      {envelope !== null && envelope.tokens.length > 0 && (
        <div className="grid gap-1" data-testid="pref-signature-tokens">
          <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
            Insert a field
          </span>
          <div className="flex flex-wrap gap-1.5">
            {envelope.tokens.map((token) => (
              <Button
                key={token.key}
                variant="secondary"
                onClick={() => insertToken(token.key)}
                disabled={locked}
                data-testid={`pref-signature-token-${token.key.replaceAll(".", "-")}`}
              >
                {token.label}
                {token.value !== null && (
                  <>
                    <span aria-hidden="true" style={{ color: "var(--m-text-muted)" }}>
                      {" · "}
                    </span>
                    <span style={{ color: "var(--m-text-muted)" }}>{token.value}</span>
                  </>
                )}
              </Button>
            ))}
          </div>
        </div>
      )}

      {(previewSignature !== null || previewGreeting !== null) && (
        <div className="grid gap-1" data-testid="pref-signature-preview">
          <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
            Preview — the saved block, as the next message carries it
          </span>
          <div
            className="grid gap-2 rounded-lg border p-3"
            style={{ borderColor: "var(--m-border)" }}
          >
            {previewGreeting !== null && (
              <p className="m-0 text-[13px]" style={{ color: "var(--m-text)" }}>
                {previewGreeting}
              </p>
            )}
            <pre
              className="m-0 whitespace-pre-wrap text-[13px]"
              style={{ color: "var(--m-text)" }}
            >
              {previewSignature ?? "No signature configured — messages go out unsigned."}
            </pre>
            <div className="flex flex-wrap items-center gap-1.5">
              <Tag tone="neutral">
                Signature: {SOURCE_LABEL[envelope?.signature_source ?? "unset"] ?? "unset"}
              </Tag>
              <Tag tone="neutral">
                Greeting: {SOURCE_LABEL[envelope?.greeting_source ?? "unset"] ?? "unset"}
              </Tag>
            </div>
          </div>
        </div>
      )}

      {envelope === null && (
        <span
          className="flex items-center gap-2 text-xs"
          style={{ color: "var(--m-text-muted)" }}
        >
          <Spinner label="Loading your signature" /> Loading your signature…
        </span>
      )}
    </Card>
  );
}

/** The autosave state under one field: saving, saved, or a refusal. */
function SaveNote({
  state,
  error,
  testId,
}: {
  state: "idle" | "saving" | "saved" | "error" | undefined;
  error: string | null | undefined;
  testId: string;
}) {
  if (state === "error") {
    return (
      <span
        className="text-xs"
        role="alert"
        style={{ color: "var(--m-status-danger-fg)" }}
        data-testid={`${testId}-error`}
      >
        {error ?? "Save failed — keep editing, it retries on the next keystroke."}
      </span>
    );
  }
  if (state === "saving") {
    return (
      <span className="text-xs" style={{ color: "var(--m-text-muted)" }} role="status">
        Saving…
      </span>
    );
  }
  if (state === "saved") {
    return (
      <span
        className="text-xs"
        role="status"
        style={{ color: "var(--m-status-success-fg)" }}
        data-testid={`${testId}-saved`}
      >
        Saved to your account.
      </span>
    );
  }
  return null;
}

export default function UserPreferencesPage() {
  const auth = useRequireAuth();
  const { theme, setTheme } = useTheme();
  const authed = auth.status === "authed";
  // The registry's stored choice, when one exists.
  const [registryTheme, setRegistryTheme] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const tours = useToursState(authed);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/settings/user");
      if (!data) return;
      const row = (data ?? []).find((v) => v.key === "appearance.mode");
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
        body: { key: "appearance.mode", value: next },
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
      <SettingsHeader
        title="Preferences"
        description="Settings that travel with your account. Until you save a choice, appearance follows your system."
      />

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

      <SignatureCard authed={authed} />

      <Card className="p-5 grid gap-2" data-testid="pref-tours-card">
        <h2 className="text-sm font-semibold">Feature tours</h2>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Short first-visit highlights (the work bar, the merge workbench).
          Guides from the ? help panel always stay available.
        </p>
        <div className="mt-1 flex items-center justify-between gap-3">
          <Label htmlFor="pref-tours-disabled">Don&apos;t show me tours</Label>
          <input
            id="pref-tours-disabled"
            type="checkbox"
            data-testid="pref-tours-disabled"
            checked={tours.disabled}
            onChange={(e) => void tours.setDisabled(e.target.checked)}
            className="h-4 w-4 cursor-pointer"
            disabled={!tours.loaded}
          />
        </div>
        <FieldHint>
          {tours.orgDisabled
            ? "An admin has disabled tours org-wide; this switch stays available if that changes."
            : "Off means tours start on surfaces you have not seen yet — once each."}
        </FieldHint>
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
