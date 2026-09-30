// Close/validation flow — the three QA steps plus the optional client email.
//
// Step 1 — merge: resolved (checked, non-N/A) tasks + the next-steps value
//   render into a formatted final-note preview the tech can edit before it
//   posts (the merge is a suggestion, never a silent mutation).
// Step 2 — time validation: the ticket's logged time entries are summed
//   against any running timer; a running timer on this ticket or zero
//   logged minutes surfaces a missing-time gap inline (surfaced, never
//   blocked silently — the tech confirms through it explicitly).
// Step 3 — next status: explicit confirmation from the board's status set
//   (semantic floor mirrored from core-domain workflow; the API stays
//   authoritative). Confirming posts the final note (chosen visibility,
//   client-visible by default), then applies the status change.
// Step 4 (optional) — client email: greeting + signature read from the
//   tech's ACCOUNT (Wave 42; they were device-local panel prefs welded into
//   the note text, which meant a second device signed mail differently and
//   neither matched what the client received). Each is overridable for this
//   message only, with "save as my default" (policy-checked) and a reset.
//   The requester's name is pulled from the ticket, the note posts
//   client-visible, and the backend's outbound path (2.4 MailTransport
//   fan-out) mails it with the resolved blocks around the generated body.
// Non-purpose: no checklist enforcement changes (the server refuses
// closes that violate required items — typed rejections surface inline).

"use client";

import type { components } from "@marula-core/sdk";
import { Button, FieldHint, Label, StatusTag, Tag, Textarea } from "@marula-core/ui";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api, apiBaseUrl, authHeaders } from "../../lib/api";
import { friendlyMutationText } from "../../lib/error-copy";
import { useDialogA11y } from "../../lib/a11y/use-dialog-a11y";
import { PaperclipIcon } from "./icons";
import type { NoteKindWire } from "./notes-composer";

type BoardStatus = components["schemas"]["BoardStatus"];
type BoardTransitionsResponse = components["schemas"]["BoardTransitionsResponse"];
type ChecklistItem = components["schemas"]["ChecklistItem"];
type TimeEntry = components["schemas"]["TimeEntry"];

/**
 * The caller's resolved email envelope (Wave 42), read from the ACCOUNT
 * rather than this device's panel state — the greeting and sign-off used to
 * live in local storage, which meant a second device signed mail differently
 * and neither matched what the client actually received. The endpoint is
 * pinned by this lane's OpenAPI regen; until the pin lands it is fetched
 * through the authed fetch helper rather than the generated client.
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
  tokens: { key: string; label: string; value: string | null }[];
}

const ENVELOPE_PATH = "/api/v1/settings/user/email-envelope";

async function loadEnvelope(): Promise<EmailEnvelope | null> {
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

/** Ledger wording for each rung of the chain, as the composer shows it. */
const SOURCE_LABEL: Record<string, string> = {
  message_override: "this message",
  user_default: "my default",
  board_identity: "the board",
  org_default: "the organization",
  product: "a fixed Marula line",
  unset: "nothing yet",
};

/** Client mirror of the server's transition verdict (core-domain workflow). */
function transitionAllowed(
  statuses: BoardStatus[],
  workflow: BoardTransitionsResponse,
  fromId: string,
  toId: string,
): boolean {
  if (fromId === toId) return false;
  if ((workflow.mode ?? "floor") === "explicit") {
    return (workflow.edges ?? []).some(
      (e) => e.from_status_id === fromId && e.to_status_id === toId,
    );
  }
  const catOf = new Map(statuses.map((s) => [s.id, s.category]));
  const fromCat = catOf.get(fromId);
  const toCat = catOf.get(toId);
  if (fromCat === undefined || toCat === undefined) return false;
  return !(fromCat === "closed" && toCat === "open");
}

export interface CloseFlowProps {
  open: boolean;
  onCloseDialog(): void;
  ticketId: string;
  ticketNumber: string;
  ticketTitle: string;
  userId: string;
  checklist: ChecklistItem[];
  nextSteps: string;
  clientName: string | null;
  contactName: string | null;
  statuses: BoardStatus[];
  workflow: BoardTransitionsResponse;
  statusId: string | null;
  /**
   * Posts a note through the page's instant-first mutate. The envelope is
   * the per-message greeting/signature (Wave 42): the SERVER assembles them
   * around the note when it mails the requester, so the note stays the note
   * rather than a letter with a salutation welded on.
   */
  onPostNote(
    body: string,
    kind: NoteKindWire,
    envelope?: { greeting?: string | null; signature?: string | null } | null,
  ): Promise<void>;
  /** Applies the status change through the page's instant-first mutate. */
  onChangeStatus(statusId: string): Promise<void>;
}

type Step = "merge" | "time" | "status" | "email" | "done";

export function CloseFlow(props: CloseFlowProps): ReactNode {
  if (!props.open) return null;
  return <CloseFlowInner {...props} />;
}

function CloseFlowInner({
  ticketId,
  ticketNumber,
  ticketTitle,
  userId,
  checklist,
  nextSteps,
  clientName,
  contactName,
  statuses,
  workflow,
  statusId,
  onPostNote,
  onChangeStatus,
  onCloseDialog,
}: CloseFlowProps): ReactNode {
  // Modal shell: trap + Escape + focus restore via the shared hook.
  const dialogRef = useDialogA11y<HTMLDivElement>(true, onCloseDialog);
  const [step, setStep] = useState<Step>("merge");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Step 1: the merged final note (editable).
  const resolved = checklist.filter((i) => i.checked);
  const [finalBody, setFinalBody] = useState("");
  const [finalKind, setFinalKind] = useState<NoteKindWire>("client_visible");

  useEffect(() => {
    if (step !== "merge") return;
    const lines: string[] = [];
    if (resolved.length > 0) {
      lines.push("Resolved tasks:");
      for (const item of resolved) lines.push(`- ${item.text}`);
    }
    if (nextSteps.trim() !== "") {
      lines.push("Next steps:");
      for (const line of nextSteps.trim().split("\n")) lines.push(line);
    }
    setFinalBody(lines.join("\n"));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step]);

  // Step 2: time validation — timer vs entries. A still-running timer on
  // this ticket or zero logged minutes surfaces the missing-time gap.
  const [entries, setEntries] = useState<TimeEntry[] | null>(null);
  const [runningTimer, setRunningTimer] = useState<components["schemas"]["TimerSession"] | null>(null);
  const [timeError, setTimeError] = useState<string | null>(null);
  useEffect(() => {
    if (step !== "time") return;
    let cancelled = false;
    void (async () => {
      const [time, timer] = await Promise.all([
        api.api.GET("/api/v1/tickets/{id}/time-entries", { params: { path: { id: ticketId } } }),
        api.api.GET("/api/v1/timers/active"),
      ]);
      if (cancelled) return;
      if (time.data === undefined) {
        setTimeError("Could not load time entries");
        setEntries([]);
      } else {
        setEntries(time.data ?? []);
      }
      const active = timer.data as components["schemas"]["TimerSession"] | null | undefined;
      setRunningTimer(active?.ticket_id === ticketId ? (active ?? null) : null);
    })();
    return () => {
      cancelled = true;
    };
  }, [step, ticketId]);

  const totalMinutes = useMemo(
    () => (entries ?? []).reduce((sum, e) => sum + e.minutes, 0),
    [entries],
  );

  // Step 3: next status.
  const [nextStatusId, setNextStatusId] = useState<string | null>(null);
  useEffect(() => {
    if (step !== "status" || statusId === null) return;
    const candidates = statuses.filter(
      (s) => s.category === "closed" && transitionAllowed(statuses, workflow, statusId, s.id),
    );
    setNextStatusId(candidates[0]?.id ?? null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step]);

  // Step 4: client email — greeting and signature (Wave 42).
  //
  // These were device-local panel prefs welded into the note text. They are
  // read from the ACCOUNT now, and travel to the server as a per-message
  // envelope, because the requester is mailed by the backend's own generated
  // body — not by a textarea here. The preview below is assembled the way
  // the send path assembles it, so what the member reads is what the
  // client receives.
  const recipientName = contactName ?? clientName;
  const [envelope, setEnvelope] = useState<EmailEnvelope | null>(null);
  const [greetingOverride, setGreetingOverride] = useState("");
  const [signatureOverride, setSignatureOverride] = useState("");
  const [touched, setTouched] = useState({ greeting: false, signature: false });
  const [saveState, setSaveState] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (step !== "email") return;
    let cancelled = false;
    void (async () => {
      const loaded = await loadEnvelope();
      if (cancelled || loaded === null) return;
      setEnvelope(loaded);
      setGreetingOverride(loaded.user_greeting);
      setSignatureOverride(loaded.user_signature);
    })();
    return () => {
      cancelled = true;
    };
  }, [step]);

  const locked = envelope?.policy === "org_locked";

  // The effective blocks: the override when one was written for this
  // message, otherwise the resolved default, labelled by its source so the
  // member can see which rung will answer.
  const greetingSource = touched.greeting
    ? "message_override"
    : (envelope?.greeting_source ?? "product");
  const effectiveGreeting =
    touched.greeting && greetingOverride.trim() !== ""
      ? greetingOverride
      : (envelope?.greeting ?? `Hi ${recipientName ?? "there"},`);
  const signatureSource = touched.signature
    ? "message_override"
    : (envelope?.signature_source ?? "unset");
  const effectiveSignature =
    touched.signature && signatureOverride.trim() !== ""
      ? signatureOverride
      : (envelope?.signature ?? null);

  /** Saves the edited block as this member's own default (policy-checked). */
  async function saveAsMyDefault(
    key: "email.greeting" | "email.signoff",
    value: string,
  ): Promise<void> {
    setSaveState("saving");
    setSaveError(null);
    const res = await api.api.PUT("/api/v1/settings/user", { body: { key, value } });
    if (res.error) {
      setSaveState("error");
      setSaveError(
        (res.error as { error?: { message?: string } }).error?.message ??
          friendlyMutationText(res.error, "Could not save your default"),
      );
      return;
    }
    setSaveState("saved");
    const refreshed = await loadEnvelope();
    if (refreshed !== null) setEnvelope(refreshed);
  }

  /** Puts the member's own default back after a per-message edit. */
  function resetToMyDefault(field: "greeting" | "signature"): void {
    if (envelope === null) return;
    setTouched((current) => ({ ...current, [field]: false }));
    if (field === "greeting") setGreetingOverride(envelope.user_greeting);
    else setSignatureOverride(envelope.user_signature);
  }

  function insertToken(key: string): void {
    if (locked) return;
    setSignatureOverride((current) =>
      current.length > 0 ? `${current}\n{{${key}}}` : `{{${key}}}`,
    );
    setTouched((current) => ({ ...current, signature: true }));
  }

  async function run(fn: () => Promise<void>): Promise<boolean> {
    setBusy(true);
    setError(null);
    try {
      await fn();
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : "Step failed");
      return false;
    } finally {
      setBusy(false);
    }
  }

  if (step === "done") {
    return <CloseFlowDone ticketNumber={ticketNumber} onCloseDialog={onCloseDialog} />;
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      style={{ background: "var(--m-scrim)" }}
      role="dialog"
      aria-modal="true"
      aria-label="Close validation"
      data-testid="close-flow"
    >
      <div
        ref={dialogRef}
        tabIndex={-1}
        className="grid max-h-[90vh] w-full max-w-xl gap-3 overflow-y-auto rounded-[var(--m-radius-container)] border p-4"
        style={{ background: "var(--m-overlay-surface)", borderColor: "var(--m-border)" }}
      >
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-semibold">
            Validate {ticketNumber} for close — {ticketTitle}
          </h2>
          <Button size="sm" variant="ghost" onClick={onCloseDialog} disabled={busy}>
            Cancel
          </Button>
        </div>
        <ol className="flex gap-1 text-[11px]" aria-label="Validation steps">
          {(
            [
              ["merge", "1 · Merge"],
              ["time", "2 · Time"],
              ["status", "3 · Status"],
              ["email", "4 · Email (optional)"],
            ] as const
          ).map(([value, label]) => (
            <li key={value}>
              <StatusTag tone={step === value ? "success" : "neutral"}>{label}</StatusTag>
            </li>
          ))}
        </ol>

        {step === "merge" && (
          <div className="grid gap-2" data-testid="close-step-merge">
            <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
              {resolved.length > 0
                ? `${resolved.length} resolved task${resolved.length === 1 ? "" : "s"} folded in.`
                : "No resolved tasks to fold in."}
              {nextSteps.trim() !== "" ? " Next steps merged." : " No next steps recorded."}
            </p>
            <div className="grid gap-1">
              <Label htmlFor="close-final-note">Final notes</Label>
              <Textarea
                id="close-final-note"
                rows={8}
                value={finalBody}
                onChange={(e) => setFinalBody(e.target.value)}
                data-testid="close-final-note"
              />
            </div>
            <div className="flex items-center gap-2" role="radiogroup" aria-label="Final note visibility">
              {(
                [
                  ["client_visible", "Client-visible"],
                  ["internal", "Internal"],
                ] as const
              ).map(([value, label]) => (
                <button
                  key={value}
                  type="button"
                  role="radio"
                  aria-checked={finalKind === value}
                  className="rounded-[var(--m-radius-container)] border px-2 py-0.5 text-[11px]"
                  style={{
                    borderColor: finalKind === value ? "var(--m-accent)" : "var(--m-border)",
                    color: finalKind === value ? "var(--m-accent)" : "var(--m-text-muted)",
                  }}
                  data-testid={`close-note-kind-${value}`}
                  onClick={() => setFinalKind(value)}
                >
                  {label}
                </button>
              ))}
            </div>
            <div className="flex justify-end gap-2">
              <Button size="sm" disabled={busy || finalBody.trim() === ""} onClick={() => setStep("time")} data-testid="close-next-time">
                Next: time validation
              </Button>
            </div>
          </div>
        )}

        {step === "time" && (
          <div className="grid gap-2" data-testid="close-step-time">
            {entries === null ? (
              <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>Checking logged time…</p>
            ) : (
              <>
                <p className="text-xs" style={{ color: "var(--m-text-secondary)" }} data-testid="close-time-summary">
                  {entries.length} time entr{entries.length === 1 ? "y" : "ies"} · {totalMinutes} min logged.
                </p>
                {(timeError !== null || entries.length === 0 || runningTimer !== null) && (
                  <p
                    className="rounded-[var(--m-radius-container)] border border-dashed px-2 py-1.5 text-xs"
                    style={{ borderColor: "var(--m-status-warning-fg)", color: "var(--m-status-warning-fg)" }}
                    data-testid="close-time-gap"
                  >
                    Missing-time gap:{" "}
                    {timeError !== null
                      ? timeError
                      : runningTimer !== null
                        ? "a timer is still running on this ticket — stop it and log the entry before closing."
                        : "no time has been logged on this ticket."}{" "}
                    Confirm through it explicitly if this is intended.
                  </p>
                )}
              </>
            )}
            <div className="flex justify-between gap-2">
              <Button size="sm" variant="ghost" onClick={() => setStep("merge")} disabled={busy}>Back</Button>
              <Button size="sm" onClick={() => setStep("status")} data-testid="close-next-status">Next: status</Button>
            </div>
          </div>
        )}

        {step === "status" && (
          <div className="grid gap-2" data-testid="close-step-status">
            <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>Confirm the next status from this board's set:</p>
            <div className="flex flex-wrap gap-1">
              {statuses.map((s) => {
                const current = s.id === statusId;
                const allowed = current || (statusId !== null && transitionAllowed(statuses, workflow, statusId, s.id));
                return (
                  <button
                    key={s.id}
                    type="button"
                    disabled={!allowed}
                    title={allowed ? undefined : "This board's workflow does not allow that transition"}
                    className="rounded-[var(--m-radius-container)] border px-2 py-1 text-xs"
                    style={{
                      borderColor: nextStatusId === s.id ? "var(--m-accent)" : "var(--m-border)",
                      color: nextStatusId === s.id ? "var(--m-accent)" : undefined,
                    }}
                    data-testid={`close-status-${s.id}`}
                    onClick={() => setNextStatusId(s.id)}
                  >
                    {s.name}
                    {current ? " (current)" : ""}
                  </button>
                );
              })}
            </div>
            <div className="flex justify-between gap-2">
              <Button size="sm" variant="ghost" onClick={() => setStep("time")} disabled={busy}>Back</Button>
              <Button
                size="sm"
                variant="primary"
                disabled={busy || nextStatusId === null}
                data-testid="close-confirm"
                onClick={() =>
                  void run(async () => {
                    await onPostNote(finalBody, finalKind);
                    if (nextStatusId !== null && nextStatusId !== statusId) {
                      await onChangeStatus(nextStatusId);
                    }
                    setStep("email");
                  })
                }
              >
                Post final note &amp; apply status
              </Button>
            </div>
          </div>
        )}

        {step === "email" && (
          <div className="grid gap-2" data-testid="close-step-email">
            <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
              Emails the client the summary
              {recipientName !== null ? ` (writing to ${recipientName})` : ""}. Your
              greeting and signature come from your account; change them here for this
              message only, or keep them. The note posts client-visible and the requester
              is emailed by the backend path with these blocks around it.
            </p>

            {locked ? (
              <div
                className="grid gap-2 rounded-md border p-3"
                data-testid="close-email-locked"
                style={{
                  borderColor: "var(--m-border)",
                  background: "var(--m-surface-muted)",
                }}
              >
                <div className="flex flex-wrap items-center gap-2">
                  <Tag tone="neutral">Locked to the organization</Tag>
                  <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                    This workspace sends one signature of its own.
                  </span>
                </div>
                {effectiveSignature !== null && (
                  <pre
                    className="m-0 whitespace-pre-wrap text-xs"
                    style={{ color: "var(--m-text)" }}
                    data-testid="close-email-locked-block"
                  >
                    {effectiveSignature}
                  </pre>
                )}
              </div>
            ) : (
              <>
                <div className="grid gap-1">
                  <div className="flex flex-wrap items-baseline justify-between gap-2">
                    <Label htmlFor="close-email-greeting">Greeting for this message</Label>
                    {touched.greeting && (
                      <span className="flex items-center gap-1.5">
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => resetToMyDefault("greeting")}
                          data-testid="close-email-greeting-reset"
                        >
                          Reset to my default
                        </Button>
                        <Button
                          size="sm"
                          variant="secondary"
                          onClick={() =>
                            void saveAsMyDefault("email.greeting", greetingOverride)
                          }
                          data-testid="close-email-greeting-save"
                        >
                          Save as my default
                        </Button>
                      </span>
                    )}
                  </div>
                  <Textarea
                    id="close-email-greeting"
                    rows={2}
                    value={greetingOverride}
                    placeholder="Leave as my default"
                    onChange={(e) => {
                      setGreetingOverride(e.target.value);
                      setTouched((current) => ({ ...current, greeting: true }));
                    }}
                    data-testid="close-email-greeting"
                  />
                  <FieldHint>
                    Empty sends your own default, and the workspace greeting if you have
                    none.
                  </FieldHint>
                </div>

                <div className="grid gap-1">
                  <div className="flex flex-wrap items-baseline justify-between gap-2">
                    <Label htmlFor="close-email-signoff">Signature for this message</Label>
                    {touched.signature && (
                      <span className="flex items-center gap-1.5">
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => resetToMyDefault("signature")}
                          data-testid="close-email-signoff-reset"
                        >
                          Reset to my default
                        </Button>
                        <Button
                          size="sm"
                          variant="secondary"
                          onClick={() =>
                            void saveAsMyDefault("email.signoff", signatureOverride)
                          }
                          data-testid="close-email-signoff-save"
                        >
                          Save as my default
                        </Button>
                      </span>
                    )}
                  </div>
                  <Textarea
                    id="close-email-signoff"
                    rows={2}
                    value={signatureOverride}
                    placeholder="Leave as my default"
                    onChange={(e) => {
                      setSignatureOverride(e.target.value);
                      setTouched((current) => ({ ...current, signature: true }));
                    }}
                    data-testid="close-email-signoff"
                  />
                  <FieldHint>
                    Empty sends your own default, and the workspace signature if you have
                    none. Placeholders fill in when the message sends.
                  </FieldHint>
                  {envelope !== null && envelope.tokens.length > 0 && (
                    <div className="flex flex-wrap items-center gap-1.5">
                      <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                        Insert
                      </span>
                      {envelope.tokens.map((token) => (
                        <Button
                          key={token.key}
                          size="sm"
                          variant="ghost"
                          onClick={() => insertToken(token.key)}
                          data-testid={`close-email-token-${token.key.replaceAll(".", "-")}`}
                        >
                          {token.label}
                        </Button>
                      ))}
                    </div>
                  )}
                </div>
              </>
            )}

            <div
              className="grid gap-1.5 rounded-md border p-3"
              data-testid="close-email-preview"
              style={{ borderColor: "var(--m-border)" }}
            >
              <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                How it goes out
              </span>
              <pre
                className="m-0 whitespace-pre-wrap text-xs"
                style={{ color: "var(--m-text)" }}
                data-testid="close-email-preview-body"
              >
                {[
                  effectiveGreeting,
                  "",
                  `New reply on ticket ${ticketNumber}:`,
                  "",
                  finalBody,
                  effectiveSignature === null ? null : "",
                  effectiveSignature,
                ]
                  .filter((line): line is string => line !== null)
                  .join("\n")
                  .trim()}
              </pre>
              <div className="flex flex-wrap items-center gap-1.5">
                <Tag tone="neutral">
                  Signature: {SOURCE_LABEL[signatureSource] ?? signatureSource}
                </Tag>
                <Tag tone="neutral">
                  Greeting: {SOURCE_LABEL[greetingSource] ?? greetingSource}
                </Tag>
              </div>
            </div>

            {saveState === "error" && (
              <span
                className="text-xs"
                role="alert"
                style={{ color: "var(--m-status-danger-fg)" }}
                data-testid="close-email-save-error"
              >
                {saveError ?? "Could not save that default."}
              </span>
            )}
            {saveState === "saved" && (
              <span
                className="text-xs"
                role="status"
                style={{ color: "var(--m-status-success-fg)" }}
                data-testid="close-email-save-saved"
              >
                Saved to your account.
              </span>
            )}

            <div className="flex justify-between gap-2">
              <Button size="sm" variant="ghost" onClick={onCloseDialog} disabled={busy}>
                Skip email
              </Button>
              <Button
                size="sm"
                variant="primary"
                disabled={busy || finalBody.trim() === ""}
                data-testid="close-email-send"
                onClick={() =>
                  void run(async () => {
                    await onPostNote(finalBody, "client_visible", {
                      greeting: touched.greeting ? greetingOverride : null,
                      signature: touched.signature ? signatureOverride : null,
                    });
                    setStep("done");
                  })
                }
              >
                Send &amp; finish
              </Button>
            </div>
          </div>
        )}

        {error !== null && (
          <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">{error}</p>
        )}
      </div>
    </div>
  );
}

// Done screen is its OWN component so the dialog hook binds to the element
// actually on screen (the step swap would otherwise strand the trap on a
// detached node). Focus restore returns to the close trigger on dismiss.
function CloseFlowDone({
  ticketNumber,
  onCloseDialog,
}: {
  ticketNumber: string;
  onCloseDialog: () => void;
}): ReactNode {
  const dialogRef = useDialogA11y<HTMLDivElement>(true, onCloseDialog);
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      style={{ background: "var(--m-scrim)" }}
      role="dialog"
      aria-modal="true"
      aria-label="Close validation complete"
      data-testid="close-flow"
    >
      <div
        ref={dialogRef}
        tabIndex={-1}
        className="grid w-full max-w-md gap-3 rounded-[var(--m-radius-container)] border p-4"
        style={{ background: "var(--m-overlay-surface)", borderColor: "var(--m-border)" }}
      >
        <h2 className="text-sm font-semibold">Ticket validated for close</h2>
        <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          {ticketNumber} — the final note is posted and the status is applied.
        </p>
        <div className="flex justify-end">
          <Button size="sm" onClick={onCloseDialog} data-testid="close-flow-finish">
            Done
          </Button>
        </div>
      </div>
    </div>
  );
}
