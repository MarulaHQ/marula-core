"use client";
// The guided invite flow — the ONE component both invite surfaces render.
//
// Purpose: inviting a member is a role decision wearing an email form's
// clothes. This component walks it in three steps: what the person will do
// (intent chips, each mapped to a recommended builtin role), which role they
// get (preselected from the intent, overridable) beside a permission preview
// fed by lib/settings/roles.ts, and finally name + email for the send. Both
// /settings/team's dialog and the setup wizard's invite step render this, so
// the words about what a role can do can never drift between them.
//
// The POST itself stays with the caller: `send` receives the body with the
// flow's per-episode idempotency key (instant-first envelope — a retry can
// never double-invite) and returns a typed result, because the two surfaces
// diverge after success (the settings dialog swaps to the copy-once link
// panel; the wizard completes its org step). Everything before and including
// that call shape is shared here.
//
// Accessibility: the intent chips are a real radiogroup — buttons with
// role="radio", roving tabindex and arrow-key movement, matching the preset
// picker pattern in settings/appearance. Step headings take focus on step
// changes (tabIndex -1), and the permission preview is a labelled section
// whose one-line reading is announced politely when the role changes.

import { useEffect, useRef, useState } from "react";
import type { components } from "@marula-core/sdk";
import {
  Button,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@marula-core/ui";
import {
  BUILTIN_ROLES,
  INVITE_DEFAULT_ROLE,
  INVITE_INTENTS,
  ROLE_CATALOG,
  type BuiltinRole,
} from "../../lib/settings/roles";

export interface InviteMemberBody {
  display_name: string;
  email: string;
  role: BuiltinRole;
  /** Instant-first envelope: one key per send episode (UUID). The same key
   *  replaying is the same result; it is never regenerated mid-episode. */
  idempotency_key: string;
}

export type InviteSendResult =
  | { ok: true; data: components["schemas"]["OrgUserMutationResponse"] }
  | { ok: false; message: string };

export interface InviteMemberFlowProps {
  /** The caller's POST /api/v1/org/users wrapper. The flow generates the
   *  body, including the idempotency key; the caller keeps its own error
   *  copy and success wiring. */
  send: (body: InviteMemberBody) => Promise<InviteSendResult>;
  /** Called once the invite was created and the setup token is in hand
   *  (the caller has already verified `invite` is present). */
  onSent: (data: components["schemas"]["OrgUserMutationResponse"]) => void | Promise<void>;
  /** Optional leading footer action — the settings dialog's Cancel. */
  cancel?: { label: string; onClick: () => void };
  /** Deterministic hook points; the wizard passes its existing ids. */
  testIds?: Partial<{
    intent: string;
    role: string;
    name: string;
    email: string;
    send: string;
  }>;
  /** Two intent chips per row. Only for roomy surfaces (the settings
   * dialog); the wizard's 440px card keeps one column. */
  wide?: boolean;
}

/** Per-episode idempotency key, the report-authoring shape (UUID when the
 * platform has one, timestamped fallback when it does not). */
function newIdempotencyKey(): string {
  return typeof crypto !== "undefined" && crypto.randomUUID
    ? crypto.randomUUID()
    : `invite-${Date.now()}`;
}

/** One intent chip: a real radio button (role, aria-checked, roving
 * tabindex) styled as a chip — the appearance page's preset-card pattern. */
function IntentChip({
  intent,
  checked,
  onPick,
  onArrow,
  tabbable,
}: {
  intent: (typeof INVITE_INTENTS)[number];
  checked: boolean;
  onPick: () => void;
  onArrow: (dir: 1 | -1) => void;
  tabbable: boolean;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={checked}
      tabIndex={tabbable ? 0 : -1}
      onClick={onPick}
      onKeyDown={(e) => {
        const dir =
          e.key === "ArrowDown" || e.key === "ArrowRight"
            ? (1 as const)
            : e.key === "ArrowUp" || e.key === "ArrowLeft"
              ? (-1 as const)
              : 0;
        if (dir === 0) return;
        e.preventDefault();
        onArrow(dir);
      }}
      data-intent={intent.id}
      className="rounded-[var(--m-radius-control)] border px-3 py-2 text-left transition-[transform,opacity] duration-[120ms] hover:-translate-y-px focus-visible:outline focus-visible:outline-2 motion-reduce:transform-none motion-reduce:hover:translate-y-0 disabled:opacity-50"
      style={{
        borderColor: checked ? "var(--m-accent)" : "var(--m-border)",
        background: checked ? "var(--m-accent)" : "var(--m-surface)",
        outlineColor: "var(--m-accent)",
      }}
    >
      <span
        className="block text-[13px] font-semibold"
        style={{ color: checked ? "var(--m-accent-fg)" : "var(--m-text)" }}
      >
        {intent.label}
      </span>
      <span
        className="block text-xs"
        style={{ color: checked ? "var(--m-accent-fg)" : "var(--m-text-muted)" }}
      >
        {intent.description}
      </span>
    </button>
  );
}

/** The permission preview panel: the role's own words, from the catalog. */
function PermissionPreview({ role }: { role: BuiltinRole }) {
  const entry = ROLE_CATALOG[role];
  return (
    <section
      aria-label={`What the ${entry.label} role can do`}
      className="rounded-[var(--m-radius-control)] border p-3"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface-2)" }}
      data-testid="invite-permission-preview"
    >
      <div className="flex items-center gap-2">
        <h4 className="m-0 text-[13px] font-semibold">{entry.label}</h4>
        {/* Screen readers hear the role change through the polite line
            below; the badge is the sighted shorthand for the same fact. */}
        <span
          aria-hidden="true"
          className="rounded-full px-2 py-0.5 text-xs font-semibold"
          style={{ background: "var(--m-accent)", color: "var(--m-accent-fg)" }}
        >
          Recommended
        </span>
      </div>
      <p className="mt-1 mb-2 text-xs" style={{ color: "var(--m-text-secondary)" }}>
        {entry.does}
      </p>
      <ul className="m-0 grid list-none gap-1 p-0">
        {entry.summary.map((line) => (
          <li key={line} className="flex gap-2 text-xs" style={{ color: "var(--m-text)" }}>
            <span aria-hidden="true" style={{ color: "var(--m-accent-text)" }}>
              •
            </span>
            <span>{line}</span>
          </li>
        ))}
      </ul>
      {/* The line a screen reader hears when the role changes. Kept to one
          sentence so a role override doesn't re-read the whole list. */}
      <p className="sr-only" role="status" aria-live="polite">
        {`Previewing ${entry.label}: ${entry.does}`}
      </p>
    </section>
  );
}

const STEP_TITLES = [
  "What will this person do?",
  "Give them the right role",
  "Who are you inviting?",
] as const;

export function InviteMemberFlow({
  send,
  onSent,
  cancel,
  testIds,
  wide = false,
}: InviteMemberFlowProps) {
  const [step, setStep] = useState(0);
  const [intentId, setIntentId] = useState<string | null>(null);
  const [role, setRole] = useState<BuiltinRole>(INVITE_DEFAULT_ROLE);
  /** True once the person has picked a role by hand on the role step. A
   *  deliberate override must survive a round trip back to the intent
   *  step, so Continue only adopts a recommendation while it is untouched. */
  const [roleTouched, setRoleTouched] = useState(false);
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const headingRef = useRef<HTMLHeadingElement | null>(null);
  const mounted = useRef(false);

  useEffect(() => {
    if (!mounted.current) {
      mounted.current = true;
      return;
    }
    // Focus management between steps: the heading announces where the
    // dialog landed. tabIndex -1 keeps it out of the tab order itself.
    headingRef.current?.focus();
  }, [step]);

  const intent = INVITE_INTENTS.find((i) => i.id === intentId) ?? null;

  const moveFocus = (dir: 1 | -1, from: string) => {
    const idx = INVITE_INTENTS.findIndex((i) => i.id === from);
    if (idx < 0) return;
    const next = INVITE_INTENTS[(idx + dir + INVITE_INTENTS.length) % INVITE_INTENTS.length];
    if (next === undefined) return;
    // Picking with the keyboard follows focus, the way a native radio
    // group behaves; the browser moves focus to the now-checked chip.
    setIntentId(next.id);
    requestAnimationFrame(() => {
      document
        .querySelector<HTMLButtonElement>(`[data-intent="${next.id}"]`)
        ?.focus();
    });
  };

  const sendIt = async () => {
    setError(null);
    setBusy(true);
    try {
      const result = await send({
        display_name: name.trim(),
        email: email.trim(),
        role,
        idempotency_key: newIdempotencyKey(),
      });
      if (!result.ok) {
        setError(result.message);
        return;
      }
      // The person is invited. Clear the form before the caller's success
      // wiring runs, so a wiring failure can never leave a filled form
      // behind that would invite the same person twice.
      setName("");
      setEmail("");
      await onSent(result.data);
    } catch {
      setError("The invite could not be sent — check the connection and try again.");
    } finally {
      setBusy(false);
    }
  };

  const entry = ROLE_CATALOG[role];

  return (
    <div className="grid gap-3">
      <h3
        ref={headingRef}
        tabIndex={-1}
        className="m-0 text-sm font-semibold focus-visible:outline focus-visible:outline-2"
        style={{ outlineColor: "var(--m-accent)" }}
      >
        {STEP_TITLES[step]}
      </h3>

      {step === 0 ? (
        <div
          role="radiogroup"
          aria-label="What this person will do"
          className={wide ? "grid gap-2 sm:grid-cols-2" : "grid gap-2"}
          data-testid={testIds?.intent}
        >
          {INVITE_INTENTS.map((i) => (
            <IntentChip
              key={i.id}
              intent={i}
              checked={intentId === i.id}
              tabbable={intentId === i.id || intentId === null ? true : false}
              onPick={() => setIntentId(i.id)}
              onArrow={(dir) => moveFocus(dir, i.id)}
            />
          ))}
        </div>
      ) : null}

      {step === 1 ? (
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="invite-member-role">Org role</Label>
            <Select
              value={role}
              onValueChange={(v) => {
                setRole(v as BuiltinRole);
                setRoleTouched(true);
              }}
              disabled={busy}
            >
              <SelectTrigger id="invite-member-role" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent data-testid={testIds?.role}>
                {BUILTIN_ROLES.map((r) => (
                  <SelectItem key={r} value={r}>
                    {ROLE_CATALOG[r].label}
                    {intent?.role === r ? " — recommended" : ""}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="m-0 text-xs" style={{ color: "var(--m-text-muted)" }}>
              {intent !== null && intent.role === role
                ? `Chosen from "${intent.label}". You can pick a different role — the preview follows it.`
                : "Overridden from the recommendation. The preview shows the role as it will apply."}
            </p>
          </div>
          <PermissionPreview role={role} />
        </div>
      ) : null}

      {step === 2 ? (
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="invite-member-name">Display name</Label>
            <Input
              id="invite-member-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              disabled={busy}
              data-testid={testIds?.name}
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="invite-member-email">Email</Label>
            <Input
              id="invite-member-email"
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              disabled={busy}
              data-testid={testIds?.email}
            />
          </div>
          <p className="m-0 text-xs" style={{ color: "var(--m-text-muted)" }}>
            {entry.label} — {entry.does}
          </p>
        </div>
      ) : null}

      {error !== null ? (
        <p role="alert" className="m-0 text-[13px]" style={{ color: "var(--m-destructive)" }}>
          {error}
        </p>
      ) : null}

      <div className="flex flex-wrap items-center gap-2">
        {step === 0 && cancel ? (
          <Button variant="ghost" onClick={cancel.onClick}>
            {cancel.label}
          </Button>
        ) : null}
        {step > 0 ? (
          <Button
            variant="ghost"
            onClick={() => setStep(step - 1)}
            disabled={busy}
            aria-label="Go back to the previous invite step"
          >
            Back
          </Button>
        ) : null}
        <span className="flex-1" />
        {step < 2 ? (
          <Button
            variant="primary"
            disabled={step === 0 && intent === null}
            onClick={() => {
              // Landing on the role step: adopt the intent's recommendation
              // unless the role was picked by hand (an override stands).
              if (intent !== null && !roleTouched) setRole(intent.role);
              setStep(step + 1);
            }}
          >
            Continue
          </Button>
        ) : (
          <Button
            variant="primary"
            disabled={busy || name.trim() === "" || email.trim() === ""}
            onClick={() => void sendIt()}
            data-testid={testIds?.send}
          >
            {busy ? "Sending…" : "Send invite"}
          </Button>
        )}
      </div>
    </div>
  );
}
