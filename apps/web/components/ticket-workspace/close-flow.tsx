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
// Step 4 (optional) — client email: greeting + sign-off pre-filled from
//   the tech's per-user preferences (panel-state store — the settings
//   registry has no greeting/sign-off declarations; follow-up recorded),
//   client/contact names pulled intelligently from the ticket, posting a
//   client-visible note so the backend's outbound email path (2.4
//   MailTransport fan-out to the requester) delivers it.
// Non-purpose: no checklist enforcement changes (the server refuses
// closes that violate required items — typed rejections surface inline).

"use client";

import type { components } from "@meridian/sdk";
import { Button, Label, StatusTag, Textarea } from "@meridian/ui";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api } from "../../lib/api";
import { LocalStoragePanelStateStore } from "@meridian/layout";
import { PaperclipIcon } from "./icons";
import type { NoteKindWire } from "./notes-composer";

type BoardStatus = components["schemas"]["BoardStatus"];
type BoardTransitionsResponse = components["schemas"]["BoardTransitionsResponse"];
type ChecklistItem = components["schemas"]["ChecklistItem"];
type TimeEntry = components["schemas"]["TimeEntry"];

const prefsStore = new LocalStoragePanelStateStore({ namespace: "meridian.email-prefs" });

interface EmailPrefs {
  greeting: string;
  signOff: string;
}

async function loadPrefs(userId: string): Promise<EmailPrefs> {
  try {
    const raw = await prefsStore.load(userId);
    if (raw === null) return { greeting: "", signOff: "" };
    const parsed = JSON.parse(raw) as Partial<EmailPrefs> | null;
    return {
      greeting: typeof parsed?.greeting === "string" ? parsed.greeting : "",
      signOff: typeof parsed?.signOff === "string" ? parsed.signOff : "",
    };
  } catch {
    return { greeting: "", signOff: "" };
  }
}

async function savePrefs(userId: string, prefs: EmailPrefs): Promise<void> {
  await prefsStore.save(userId, JSON.stringify(prefs));
}

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
  /** Posts a note through the page's instant-first mutate. */
  onPostNote(body: string, kind: NoteKindWire): Promise<void>;
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

  // Step 4: client email.
  const [prefs, setPrefs] = useState<EmailPrefs>({ greeting: "", signOff: "" });
  useEffect(() => {
    if (step !== "email") return;
    void loadPrefs(userId).then(setPrefs);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step]);

  const recipientName = contactName ?? clientName;
  const salutation = (prefs.greeting.trim() !== "" ? prefs.greeting : "Hi {name},").replace(
    "{name}",
    recipientName ?? "there",
  );
  const signature = prefs.signOff.trim() !== "" ? prefs.signOff : "Thank you,";
  const [emailBody, setEmailBody] = useState("");
  useEffect(() => {
    if (step !== "email") return;
    setEmailBody(`${salutation}\n\n${finalBody}\n\n${signature}`);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step]);

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
    return (
      <div
        className="fixed inset-0 z-50 flex items-center justify-center p-4"
        style={{ background: "var(--m-scrim)" }}
        role="dialog"
        aria-label="Close validation complete"
        data-testid="close-flow"
      >
        <div
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
              Optionally email the client the summary. Greeting and sign-off come from your preferences
              (editable here); {recipientName !== null ? `writing to ${recipientName}` : "no client/contact name on this ticket"}.
              The note posts client-visible and the outbound email rides the backend mail path.
            </p>
            <div className="grid gap-1">
              <Label htmlFor="close-email-greeting">Greeting</Label>
              <Textarea
                id="close-email-greeting"
                rows={2}
                value={salutation}
                onChange={(e) => setPrefs({ ...prefs, greeting: e.target.value })}
                data-testid="close-email-greeting"
              />
            </div>
            <div className="grid gap-1">
              <Label htmlFor="close-email-body">Email body</Label>
              <Textarea
                id="close-email-body"
                rows={7}
                value={emailBody}
                onChange={(e) => setEmailBody(e.target.value)}
                data-testid="close-email-body"
              />
            </div>
            <div className="grid gap-1">
              <Label htmlFor="close-email-signoff">Sign-off</Label>
              <Textarea
                id="close-email-signoff"
                rows={2}
                value={signature}
                onChange={(e) => setPrefs({ ...prefs, signOff: e.target.value })}
                data-testid="close-email-signoff"
              />
            </div>
            <p className="flex items-center gap-1 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              <PaperclipIcon /> Preferences save to your account on this device.
            </p>
            <div className="flex justify-between gap-2">
              <Button size="sm" variant="ghost" onClick={onCloseDialog} disabled={busy}>
                Skip email
              </Button>
              <div className="flex gap-2">
                <Button
                  size="sm"
                  disabled={busy}
                  onClick={() => {
                    void savePrefs(userId, prefs);
                    setStep("done");
                  }}
                  data-testid="close-email-skip"
                >
                  Save prefs, finish
                </Button>
                <Button
                  size="sm"
                  variant="primary"
                  disabled={busy || emailBody.trim() === ""}
                  data-testid="close-email-send"
                  onClick={() =>
                    void run(async () => {
                      await onPostNote(emailBody, "client_visible");
                      await savePrefs(userId, prefs);
                      setStep("done");
                    })
                  }
                >
                  Send &amp; finish
                </Button>
              </div>
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
