// /welcome — the organization setup wizard. Six steps, one card at a time.
//
// Purpose: get a new organization to the point where it can actually run, in
// six steps a person can finish in ten minutes or abandon at step one and come
// back to. The pack's state 09 is the shape: a quiet indigo header band with
// the wordmark pill, a step rail on the left showing every step's true state,
// and ONE card in front with two faint cards behind it for what remains.
//
// THE CARD STACK IS AN HONEST DEPTH CUE, NOT DECORATION. The card in front is
// the step you are on. The two faint ones behind are the steps after it. A
// stack of six would be a lie about how much is left.
//
// THE TWO "LATER" BUTTONS ARE NOT THE SAME BUTTON, AND THE CODE IS THE PROOF.
// `Skip for now` calls skipOrgStep, which posts `{never:false}` and persists
// nothing: the step stays pending on the server, keeps its place in the rail,
// and can be reopened from this page, the Today banner, or Settings. `Never
// show this setup again` is not on this screen at all — it lives in the guide
// menu in the header, because it is a different weight of decision, and it
// opens a dialog that spells out its consequences. There is no third option
// that does neither, because a "later" that records a choice is not a later.
//
// EVERY STEP'S OWN ACTION IS THE PRIMARY, AND IT IS ALWAYS WIRED. There is no
// "Next" that only moves focus: that would be indistinguishable from "Skip for
// now" and would be a dead button, which this repo forbids. A step whose real
// action is elsewhere (Imports, the AI switch) says so on the card and links
// there.
//
// THE AI STEP IS OFF UNTIL YOU DECIDE, and it does not nudge. The pack's
// principle: the feature waits for the person, with no sales pitch and no
// "recommended" label. If they skip it, nothing is switched on for them and
// nothing is waiting for an answer.
"use client";

import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import type { components } from "@marula-core/sdk";
import { Spinner } from "@marula-core/ui";
import { useRequireAuth } from "../../lib/api";
import { api } from "../../lib/api";
import { friendlyMutationText } from "../../lib/error-copy";
import { inviteSetupToken } from "./wizard-links";
import { InviteMemberFlow, type InviteMemberBody, type InviteSendResult } from "../../components/onboarding/invite-member-flow";
import {
  ORG_STEP_LABELS,
  ORG_STEP_ORDER,
  completeOrgStep,
  dismissOrgStep,
  fetchOnboardingSummary,
  skipOrgStep,
  type OnboardingSummary,
  type OnboardingStepView,
} from "../../lib/onboarding";
import {
  BackButton,
  EntryFooter,
  EntryHeader,
  EntryNote,
  GuideMenu,
  NeverShowDialog,
  SkipButton,
  SkipExplain,
  WizardRail,
  WizardStack,
  type StepState,
} from "../../components/auth/entry";

/** The last step, named, so the "nothing left to focus" case is not an index
 *  into a tuple that TypeScript has to treat as possibly empty. */
const ORG_LAST_STEP = ORG_STEP_ORDER[ORG_STEP_ORDER.length - 1] as string;

/** The rail's own reading of a step, from what the server knows. */
/** The label for a step, with a total fallback: an unknown key must not
 *  crash the wizard or leave a heading blank. */
function labelFor(key: string): {
  short: string;
  title: string;
  blurb: string;
  cta: string;
} {
  const found: { short: string; title: string; blurb: string; cta: string } | undefined =
    ORG_STEP_LABELS[key];
  return (
    found ?? {
      short: key,
      title: key,
      blurb: "This step is not set up for this organization.",
      cta: "Mark done",
    }
  );
}

function railState(view: OnboardingStepView | undefined, skipped: boolean): StepState {
  if (view === undefined) return "todo";
  if (view.dismissed) return "skipped";
  if (skipped) return "skipped";
  if (view.resolved) return "done";
  return "todo";
}

export default function WelcomeWizardPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const [summary, setSummary] = useState<OnboardingSummary | null>(null);
  const [focus, setFocus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Local, deliberately: "Skip for now" persists nothing, so the only way to
  // show a step as skipped for the length of this visit is to remember it here.
  const [skipped, setSkipped] = useState<readonly string[]>([]);
  const [neverShow, setNeverShow] = useState(false);
  /** True once somebody has come back to this page, which is what the resumed
   *  list in the pack's state 14 is for. */
  const [returning, setReturning] = useState(false);

  const [clientName, setClientName] = useState("");
  const [boardName, setBoardName] = useState("");
  const [ticketTitle, setTicketTitle] = useState("");
  const [inviteLink, setInviteLink] = useState<string | null>(null);

  const loadSummary = useCallback(async (): Promise<OnboardingSummary | null> => {
    const s = await fetchOnboardingSummary();
    if (s) setSummary(s);
    return s;
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const s = await loadSummary();
      const first = s?.org.steps.find((st) => !st.resolved)?.step ?? null;
      setFocus(first ?? ORG_LAST_STEP);
      // Somebody with progress behind them is returning, not starting.
      if (s !== null && s.org.steps.some((st) => st.resolved)) setReturning(true);
    })();
  }, [auth.status, loadSummary]);

  const steps = summary?.org.steps ?? [];
  const stepView = (key: string) => steps.find((s) => s.step === key);
  const resolvedCount = steps.filter((s) => s.resolved).length;
  const allResolved = steps.length > 0 && resolvedCount === steps.length;

  const advance = (step: string): string => {
    const idx = ORG_STEP_ORDER.indexOf(step as (typeof ORG_STEP_ORDER)[number]);
    const next = ORG_STEP_ORDER[Math.min(idx + 1, ORG_STEP_ORDER.length - 1)];
    return typeof next === "string" ? next : ORG_STEP_ORDER[0];
  };

  /** The previous step in the rail. Navigation only: nothing is recorded, so
   *  walking back and forth leaves the same state as never moving. */
  const stepBack = (step: string): void => {
    const idx = ORG_STEP_ORDER.indexOf(step as (typeof ORG_STEP_ORDER)[number]);
    if (idx <= 0) return;
    const previous = ORG_STEP_ORDER[idx - 1];
    if (typeof previous === "string") setFocus(previous);
  };

  const refresh = async (next?: string) => {
    await loadSummary();
    if (next !== undefined) setFocus(next);
  };

  const markComplete = async (step: string) => {
    setError(null);
    setBusy(true);
    try {
      const { error: apiError } = await completeOrgStep(step);
      if (apiError !== undefined) {
        setError(friendlyMutationText(apiError ?? null));
        return;
      }
      await refresh(advance(step));
    } finally {
      setBusy(false);
    }
  };

  /** `never=false` records nothing. `never=true` is the heavier choice and is
   *  only reachable from the guide menu, never from a step's action row. */
  const recordSkip = async (step: string, never: boolean) => {
    setError(null);
    setBusy(true);
    try {
      const res = never ? await dismissOrgStep(step) : await skipOrgStep(step);
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      if (!never) {
        setSkipped((current) => (current.includes(step) ? current : [...current, step]));
      }
      await refresh(advance(step));
    } finally {
      setBusy(false);
    }
  };

  const createClient = async () => {
    setError(null);
    setBusy(true);
    try {
      const { error: apiError } = await api.api.POST("/api/v1/clients", {
        body: { name: clientName.trim() } as never,
      });
      if (apiError !== undefined) {
        setError(friendlyMutationText(apiError ?? null));
        return;
      }
      setClientName("");
      // The gate is server-side: it only flips once the artifact exists.
      const res = await completeOrgStep("clients");
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      await refresh("board");
    } finally {
      setBusy(false);
    }
  };

  const createBoard = async () => {
    setError(null);
    setBusy(true);
    try {
      const { error: apiError } = await api.api.POST("/api/v1/boards", {
        body: { name: boardName.trim(), is_default: true },
      });
      if (apiError !== undefined) {
        setError(friendlyMutationText(apiError ?? null));
        return;
      }
      setBoardName("");
      const res = await completeOrgStep("board");
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      await refresh("tickets");
    } finally {
      setBusy(false);
    }
  };

  const createTicket = async () => {
    setError(null);
    setBusy(true);
    try {
      const { error: apiError } = await api.api.POST("/api/v1/tickets", {
        body: { title: ticketTitle.trim(), priority: "p3" } as never,
      });
      if (apiError !== undefined) {
        setError(friendlyMutationText(apiError ?? null));
        return;
      }
      setTicketTitle("");
      const res = await completeOrgStep("tickets");
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      await refresh("invites");
    } finally {
      setBusy(false);
    }
  };

  /** The wizard's invite send: the POST shape is the flow's (idempotency key
   *  included); success wiring is this step's own — complete the org step,
   *  hold the one-time link for the note, move the rail on. Kept in the
   *  page because completeOrgStep("invites") belongs to the wizard. */
  const sendInvite = async (body: InviteMemberBody): Promise<InviteSendResult> => {
    const { data, error: apiError } = await api.api.POST("/api/v1/org/users", { body });
    if (apiError !== undefined || data === undefined) {
      return { ok: false, message: friendlyMutationText(apiError ?? null) };
    }
    return { ok: true, data };
  };

  const inviteSent = async (data: components["schemas"]["OrgUserMutationResponse"]) => {
    const setup = inviteSetupToken(data);
    if (setup) setInviteLink(`${window.location.origin}/accept-invite?token=${setup}`);
    const res = await completeOrgStep("invites");
    if (res.error !== undefined) {
      setError(friendlyMutationText(res.error));
      return;
    }
    await refresh("feeder");
  };

  const enableAi = async () => {
    setError(null);
    setBusy(true);
    try {
      const { error: apiError } = await api.api.PUT("/api/v1/settings/org", {
        body: { key: "ai.features_enabled", value: true },
      });
      if (apiError !== undefined) {
        setError(friendlyMutationText(apiError ?? null));
        return;
      }
      const res = await completeOrgStep("ai");
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  if (auth.status !== "authed") {
    return (
      <div className="entry-root">
        <main className="entry-main">
          <Spinner label="Opening your setup guide" />
        </main>
      </div>
    );
  }

  if (focus === null) {
    return (
      <div className="entry-root">
        <EntryHeader background="band" />
        <main className="entry-main">
          <Spinner label="Opening your setup guide" />
        </main>
        <EntryFooter />
      </div>
    );
  }

  const meta = labelFor(focus);
  const stepIndex = ORG_STEP_ORDER.indexOf(focus as (typeof ORG_STEP_ORDER)[number]);
  // The pack's org rail is NAMED, not numbered: "Setting up Brightline IT"
  // (auth/neutral/states/09-welcome-invite-people.png). The signed-in user's
  // org carries no name on `Me`, so the honest fallback is the generic
  // sentence — a name is never invented. Recorded in the parity ledger.
  const orgRailHeading = "Setting up your organization";
  const states = ORG_STEP_ORDER.map(
    (key) => railState(stepView(key), skipped.includes(key)),
  );
  const skipCount = skipped.length;

  // Set during the resumed list's render: the first row that is not done
  // carries the dark primary, so the screen names a starting point.
  let nextKeyFound = false;

  // The pack's state 14: a person who comes back gets the list of what is done
  // and what is left, with a way straight back in. It is the same page, so the
  // rail is the same rail — the list just leads.
  if (returning) {
    return (
      <div className="entry-root">
        <EntryHeader
          background="band"
          right={<GuideMenu label="Setup guide" onNever={() => setNeverShow(true)} />}
        />
        <main className="entry-main">
          <div className="entry-resume" data-testid="wizard-resumed">
            <div className="entry-resume__hd">
              {/* SANS here, not the serif the entry screens wear. The pattern
                 and the fruit are still there, but this is a list of facts
                 rather than an invitation, and the serif would keep claiming
                 otherwise. (auth/neutral/states/13-skipped-and-resumed.png) */}
              <h1 className="entry-resume__title">Welcome back</h1>
              <p>
                Your setup is {resolvedCount} of {ORG_STEP_ORDER.length} done
                {skipCount > 0
                  ? `. ${skipCount === 1 ? "One step is" : `${skipCount} steps are`} skipped, and ${skipCount === 1 ? "it is" : "they are"} exactly where you left ${skipCount === 1 ? "it" : "them"}.`
                  : "."}
              </p>
              {/* The progress bar belongs on this card, not only in the rail:
                  this screen has no rail, so the count above it would otherwise
                  be the only measure of how far along somebody is. */}
              <div className="entry-progress" aria-hidden>
                <i style={{ width: `${Math.round((resolvedCount / ORG_STEP_ORDER.length) * 100)}%` }} />
              </div>
            </div>
            {ORG_STEP_ORDER.map((key, i) => {
              // The first step still to do carries the dark primary, so the
              // screen says where to start instead of presenting six equal
              // buttons. The pack picks one ("Pick up here") and leaves the rest
              // outlined.
              const isNext = states[i] !== "done" && !nextKeyFound;
              if (isNext) nextKeyFound = true;
              return (
                <div
                  className={`entry-rrow${states[i] === "done" ? " entry-rrow--done" : ""}`}
                  key={key}
                >
                  <span className="entry-step__n" style={{ pointerEvents: "none" }}>
                    {states[i] === "done" ? "✓" : i + 1}
                  </span>
                  <span className="entry-rrow__t">
                    <b>{labelFor(key).short}</b>
                    <span>
                      {states[i] === "done"
                        ? "Done"
                        : states[i] === "skipped"
                          ? "Skipped. Nothing was recorded, and it is still here."
                          : "Not started"}
                    </span>
                  </span>
                  {states[i] === "done" ? null : (
                    <button
                      type="button"
                      className={`entry-btn entry-btn--small${isNext ? "" : " entry-btn--secondary"}`}
                      onClick={() => {
                        setFocus(key);
                        setReturning(false);
                      }}
                      data-testid={`wizard-resume-${key}`}
                    >
                      {isNext ? "Pick up here" : states[i] === "skipped" ? "Do it now" : "Start"}
                    </button>
                  )}
                </div>
              );
            })}
            <div className="entry-resume__ft">
              {/* The pack puts the Never-show-again choice HERE as well as in
                  the header, because this is the screen where somebody is
                  deciding whether they still want the list at all. */}
              <p className="entry-explain" style={{ flex: 1, margin: 0 }}>
                <b>Skipped</b> steps stay on this list until you do them. <b>Never show this setup
                again</b> removes the whole list, and can be undone from Settings.
              </p>
              <button
                type="button"
                className="entry-btn entry-btn--secondary"
                onClick={() => setNeverShow(true)}
                data-testid="wizard-never-show-again"
              >
                Never show again
              </button>
            </div>
          </div>
        </main>
        <EntryFooter />
        {neverShow && (
          <NeverShowDialog
            skippedCount={skipCount}
            busy={busy}
            onKeep={() => setNeverShow(false)}
            onConfirm={() => {
              setNeverShow(false);
              for (const key of ORG_STEP_ORDER) {
                if (!stepView(key)?.resolved) void recordSkip(key, true);
              }
              router.replace("/today");
            }}
          />
        )}
      </div>
    );
  }

  return (
    <div className="entry-root">
      <EntryHeader
        background="band"
        right={<GuideMenu label="Setup guide" onNever={() => setNeverShow(true)} />}
      />

      <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
        <WizardRail
          heading={orgRailHeading}
          steps={ORG_STEP_ORDER.map((key) => labelFor(key).short)}
          states={states}
          current={meta.short}
          onSelect={(label) => {
            const key = ORG_STEP_ORDER[ORG_STEP_ORDER.findIndex((k) => labelFor(k).short === label)];
            if (key !== undefined) setFocus(key);
          }}
          doneCount={resolvedCount}
          total={ORG_STEP_ORDER.length}
          skipped={skipCount}
          variant="org"
        />

        <WizardStack>
          {summary?.org.disabled && (
            <div className="entry-note" data-testid="onboarding-disabled-note">
              <span>
                Onboarding is switched off in Settings &rarr; Onboarding. Your progress is kept
                either way.
              </span>
            </div>
          )}

          <div>
            <div className="entry-kicker">
              Step {stepIndex + 1} of {ORG_STEP_ORDER.length}
            </div>
            <h1>{meta.title}</h1>
            <p className="entry-sub">{meta.blurb}</p>
          </div>

          {error !== null && (
            <p className="entry-error" role="alert" data-testid="wizard-error">
              {error}
            </p>
          )}

          {focus === "clients" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
              <input
                className="entry-input"
                value={clientName}
                onChange={(e) => setClientName(e.target.value)}
                placeholder="Client name"
                aria-label="Client name"
                data-testid="wizard-client-name"
              />
              {stepView("clients")?.satisfied && !stepView("clients")?.done && (
                <EntryNote>
                  This organization already has clients, so you can mark the step done instead of
                  adding another.
                </EntryNote>
              )}
              <div className="entry-actions">
                <BackButton onClick={() => stepBack(focus)} disabled={stepIndex === 0} />
                <span className="entry-actions__spacer" />
                <SkipButton onClick={() => void recordSkip("clients", false)} busy={busy} />
                {stepView("clients")?.satisfied ? (
                  <button
                    type="button"
                    className="entry-btn"
                    onClick={() => void markComplete("clients")}
                    disabled={busy}
                  >
                    Mark done
                  </button>
                ) : (
                  <button
                    type="button"
                    className="entry-btn"
                    onClick={() => void createClient()}
                    disabled={busy || clientName.trim() === ""}
                    data-testid="wizard-clients-create"
                  >
                    {busy ? "Creating…" : meta.cta}
                  </button>
                )}
              </div>
            </div>
          )}

          {focus === "board" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
              <input
                className="entry-input"
                value={boardName}
                onChange={(e) => setBoardName(e.target.value)}
                placeholder="Board name, for example Service desk"
                aria-label="Board name"
                data-testid="wizard-board-name"
              />
              {stepView("board")?.satisfied && !stepView("board")?.done && (
                <EntryNote>
                  Boards already exist here, so you can mark the step done instead.
                </EntryNote>
              )}
              <div className="entry-actions">
                <BackButton onClick={() => stepBack(focus)} disabled={stepIndex === 0} />
                <span className="entry-actions__spacer" />
                <SkipButton onClick={() => void recordSkip("board", false)} busy={busy} />
                {stepView("board")?.satisfied ? (
                  <button
                    type="button"
                    className="entry-btn"
                    onClick={() => void markComplete("board")}
                    disabled={busy}
                  >
                    Mark done
                  </button>
                ) : (
                  <button
                    type="button"
                    className="entry-btn"
                    onClick={() => void createBoard()}
                    disabled={busy || boardName.trim() === ""}
                    data-testid="wizard-board-create"
                  >
                    {busy ? "Creating…" : meta.cta}
                  </button>
                )}
              </div>
            </div>
          )}

          {focus === "tickets" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
              <input
                className="entry-input"
                value={ticketTitle}
                onChange={(e) => setTicketTitle(e.target.value)}
                placeholder="What is the first thing you need help with?"
                aria-label="Ticket title"
                data-testid="wizard-ticket-title"
              />
              {stepView("tickets")?.satisfied && !stepView("tickets")?.done && (
                <EntryNote>Tickets already exist, so you can mark the step done.</EntryNote>
              )}
              <div className="entry-actions">
                <BackButton onClick={() => stepBack(focus)} disabled={stepIndex === 0} />
                <span className="entry-actions__spacer" />
                <SkipButton onClick={() => void recordSkip("tickets", false)} busy={busy} />
                {stepView("tickets")?.satisfied ? (
                  <button
                    type="button"
                    className="entry-btn"
                    onClick={() => void markComplete("tickets")}
                    disabled={busy}
                  >
                    Mark done
                  </button>
                ) : (
                  <button
                    type="button"
                    className="entry-btn"
                    onClick={() => void createTicket()}
                    disabled={busy || ticketTitle.trim() === ""}
                    data-testid="wizard-ticket-create"
                  >
                    {busy ? "Creating…" : meta.cta}
                  </button>
                )}
              </div>
            </div>
          )}

          {focus === "invites" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
              {/* The same guided flow the team settings dialog renders, so
                  inviting reads one way everywhere. The card's own actions
                  (rail Back, Skip, Mark done) stay in the row below; the
                  flow owns its internal steps and the send button. */}
              <InviteMemberFlow
                send={sendInvite}
                onSent={inviteSent}
                testIds={{
                  intent: "wizard-invite-intent",
                  role: "wizard-invite-role",
                  name: "wizard-invite-name",
                  email: "wizard-invite-email",
                  send: "wizard-invite-send",
                }}
              />
              {inviteLink !== null && (
                <div className="entry-note" data-testid="wizard-invite-link">
                  <span>
                    <b>One-time setup link. Copy it now</b> &mdash; it is shown once and nobody can
                    see it again: <br />
                    <code style={{ fontFamily: "var(--m-font-mono)", fontSize: 12, wordBreak: "break-all" }}>
                      {inviteLink}
                    </code>
                  </span>
                </div>
              )}
              {stepView("invites")?.satisfied && !stepView("invites")?.done && (
                <EntryNote>Invites are already out, so you can mark the step done.</EntryNote>
              )}
              <div className="entry-actions">
                <BackButton onClick={() => stepBack(focus)} disabled={stepIndex === 0} />
                <span className="entry-actions__spacer" />
                <SkipButton onClick={() => void recordSkip("invites", false)} busy={busy} />
                {stepView("invites")?.satisfied ? (
                  <button
                    type="button"
                    className="entry-btn"
                    onClick={() => void markComplete("invites")}
                    disabled={busy}
                  >
                    Mark done
                  </button>
                ) : null}
              </div>
            </div>
          )}

          {focus === "feeder" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
              <p className="entry-sub" style={{ margin: 0 }}>
                Feeder connectors bring in configurations and devices from NinjaOne, IT Glue or
                Auvix. You set them up in Imports with a machine API key, and this step finishes
                itself once a feed has actually written records.
              </p>
              <div className="entry-actions">
                <BackButton onClick={() => stepBack(focus)} disabled={stepIndex === 0} />
                <span className="entry-actions__spacer" />
                <SkipButton onClick={() => void recordSkip("feeder", false)} busy={busy} />
                <button
                  type="button"
                  className="entry-btn entry-btn--secondary"
                  onClick={() => router.push("/imports")}
                >
                  Go to Imports
                </button>
                {stepView("feeder")?.satisfied && (
                  <button
                    type="button"
                    className="entry-btn"
                    onClick={() => void markComplete("feeder")}
                    disabled={busy}
                  >
                    Mark done
                  </button>
                )}
              </div>
            </div>
          )}

          {focus === "ai" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
              <p className="entry-sub" style={{ margin: 0 }}>
                Booking, drafting and merge-assist surfaces stay off until you turn them on.
                Nothing calls a model until you configure a provider, and there is nothing waiting
                for an answer here.
              </p>
              <div className="entry-actions">
                <BackButton onClick={() => stepBack(focus)} disabled={stepIndex === 0} />
                <span className="entry-actions__spacer" />
                <SkipButton onClick={() => void recordSkip("ai", false)} busy={busy} />
                <button
                  type="button"
                  className="entry-btn"
                  onClick={() => void enableAi()}
                  disabled={busy}
                  data-testid="wizard-ai-enable"
                >
                  {busy ? "Turning on…" : meta.cta}
                </button>
              </div>
            </div>
          )}

          <SkipExplain />

          {allResolved && (
            <div className="entry-note" data-testid="wizard-complete">
              <span>
                Setup is resolved. Anything you skipped is still here: this page, the banner on
                Today, and Settings &rarr; Onboarding all reopen it.
              </span>
              <button
                type="button"
                className="entry-btn entry-btn--small"
                onClick={() => router.replace("/today")}
                style={{ flexShrink: 0 }}
              >
                Enter Marula
              </button>
            </div>
          )}
        </WizardStack>
      </div>

      <EntryFooter />
      {neverShow && (
        <NeverShowDialog
          skippedCount={skipCount}
          busy={busy}
          onKeep={() => setNeverShow(false)}
          onConfirm={() => {
            setNeverShow(false);
            for (const key of ORG_STEP_ORDER) {
              if (!stepView(key)?.resolved) void recordSkip(key, true);
            }
            router.replace("/today");
          }}
        />
      )}
    </div>
  );
}
