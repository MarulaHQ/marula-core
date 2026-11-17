"use client";

// AiSetupStep — the org wizard's AI setup step (Wave 72, p72-onboarding-ai-setup).
//
// Purpose: one screen, one job — connect AI. Providers first, keys inline
// with their provider, defaults last:
//   1. The org master switch (ai.features_enabled) — stays OFF unless the
//      operator turns it on, per ADR 0008's default-off posture.
//   2. Chat provider cards (the lib/ai-providers.ts list): OpenAI, Anthropic,
//      Ollama and a custom OpenAI-compatible endpoint are live; Gemini and
//      OpenRouter render visible-but-disabled with an honest note, because
//      the seal surface does not accept their ids yet (backend gap, flagged
//      in the W72 lane report — never invented client-side).
//   3. The decision engine: Jev (provider `typesafe`) takes its credential
//      through the AI credential slot; the other decision options the brief
//      names are visible-but-disabled for the same honest reason.
//   4. Defaults: two radio rows (chat default on general_chat, decision
//      default pinned per decision surface) listing catalog pairs whose
//      provider is actually configured. A default that points at an
//      unconfigured provider is not selectable.
//
// Keys are sealed at rest and NEVER render back — presence only (the
// ByokLegacyCard contract). The draft lives in component state only, never
// in localStorage, and nothing logs it. The inline test-connection runs
// after a key saves: testing → connected → the honest failure message
// verbatim from the server.
//
// "Not now" is a first-class, guilt-free exit: it persists nothing and the
// step stays resumable. "Save and continue" reports honestly what saved and
// what did not, then hands the advance to the wizard.
//
// Gating: the seal/test/credential/pick endpoints are owner/admin standing
// (byok.rs require_admin), so the step self-gates on `special:manage_users`
// — the established /api/v1/me proxy for owner/admin (lib/dashboards.ts
// isManagerPermissions). A technician sees one honest sentence and the exit.
//
// Hooks all run on every render — the gate, error and loading branches sit
// AFTER them, so the hook order never changes shape.
//
// Client boundary: the step owns key drafts, save state and inline
// test-connection results, and every interaction below is a browser one —
// nothing here is server-rendered content.

import { useCallback, useEffect, useMemo, useState, type KeyboardEvent } from "react";
import { Spinner } from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import { api, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { rejectionText, resultErrorText } from "../../lib/error-copy";
import {
  CHAT_PROVIDER_META,
  CHAT_SURFACE_KEY,
  DECISION_CREDENTIAL_PROVIDER,
  DECISION_PROVIDER_META,
  KEY_WHY_SENTENCE,
} from "../../lib/ai-providers";

type ByokOverviewResponse = components["schemas"]["ByokOverviewResponse"];
type AiMatrixResponse = components["schemas"]["AiMatrixResponse"];
type DecisionOverviewResponse = components["schemas"]["DecisionOverviewResponse"];
type TestConnectionResponse = components["schemas"]["TestConnectionResponse"];

/** One save outcome, in plain words — the report is the honest exit. */
interface SaveOutcome {
  ok: boolean;
  line: string;
}

interface AiSetupStepProps {
  /** Wizard Back. Navigation only; nothing is recorded. */
  onBack: () => void;
  backDisabled: boolean;
  /** "Not now" — the skip. Records nothing; the wizard owns the recording. */
  onSkip: () => void;
  /** "Done" after the honest report — the wizard completes the step. */
  onDone: () => void;
}

/** A catalog pair's capabilities, as the wire's `unknown` in practice. */
function capabilitiesOf(entry: { capabilities: unknown }): string[] {
  return Array.isArray(entry.capabilities) ? (entry.capabilities as string[]) : [];
}

export function AiSetupStep({ onBack, backDisabled, onSkip, onDone }: AiSetupStepProps) {
  const auth = useRequireAuth();
  const isAdmin = auth.me?.permissions.includes("special:manage_users") ?? false;

  // ---- the three reads -------------------------------------------------------
  const [overview, setOverview] = useState<ByokOverviewResponse | null>(null);
  const [matrix, setMatrix] = useState<AiMatrixResponse | null>(null);
  const [decision, setDecision] = useState<DecisionOverviewResponse | null>(null);
  /** The chat default as it stands on the server (org or user pick). */
  const [chatPicked, setChatPicked] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoadError(null);
    const [status, mat, dec, mine] = await Promise.all([
      api.api.GET("/api/v1/settings/ai/status"),
      api.api.GET("/api/v1/settings/ai/matrix"),
      api.api.GET("/api/v1/settings/ai/decision"),
      api.api.GET("/api/v1/ai/me/surfaces"),
    ]);
    if (status.data !== undefined && status.error === undefined) setOverview(status.data);
    if (mat.data !== undefined && mat.error === undefined) setMatrix(mat.data);
    if (dec.data !== undefined && dec.error === undefined) setDecision(dec.data);
    // The current chat default is preselected so the row reads as a fact,
    // not a blank.
    const chatRow = mine.data?.surfaces.find((s) => s.surface_key === CHAT_SURFACE_KEY);
    if (chatRow?.picked != null) {
      setChatPicked(`${chatRow.picked.provider}/${chatRow.picked.model}`);
    }
    if (status.error !== undefined && mat.error !== undefined && dec.error !== undefined) {
      setLoadError("The AI settings could not load. Check your connection and try again.");
    }
  }, []);

  useEffect(() => {
    if (auth.status === "authed" && isAdmin) void load();
  }, [auth.status, isAdmin, load]);

  const configuredByok = useMemo(
    () => new Set(overview?.configured_providers ?? []),
    [overview],
  );
  const orgCredentialProviders = useMemo(
    () =>
      new Set(
        (matrix?.credentials ?? [])
          .filter((c) => c.owner_user_id === null && c.status === "active")
          .map((c) => c.provider),
      ),
    [matrix],
  );
  const decisionFeaturesEnabled = decision?.features_enabled ?? false;

  // ---- master switch ---------------------------------------------------------
  const [aiOn, setAiOn] = useState<boolean | null>(null);

  // ---- chat provider selection + key ----------------------------------------
  const [pickedChat, setPickedChat] = useState<string | null>(null);
  const [keyDraft, setKeyDraft] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [keyBusy, setKeyBusy] = useState(false);
  const [keyError, setKeyError] = useState<string | null>(null);
  const [testing, setTesting] = useState<string | null>(null);
  const [testResults, setTestResults] = useState<Record<string, TestConnectionResponse>>({});

  // ---- decision selection + credential --------------------------------------
  const [pickedDecision, setPickedDecision] = useState<string | null>(null);
  const [decisionKeyDraft, setDecisionKeyDraft] = useState("");
  const [showDecisionKey, setShowDecisionKey] = useState(false);
  const [decisionKeyBusy, setDecisionKeyBusy] = useState(false);
  const [decisionKeyError, setDecisionKeyError] = useState<string | null>(null);

  // ---- defaults ---------------------------------------------------------------
  const [chatDefaultKey, setChatDefaultKey] = useState<string | null>(null);
  const [decisionDefaultKey, setDecisionDefaultKey] = useState<string | null>(null);

  // ---- save -------------------------------------------------------------------
  const [saving, setSaving] = useState(false);
  const [reported, setReported] = useState(false);
  const [outcomes, setOutcomes] = useState<SaveOutcome[] | null>(null);

  const pairKey = (provider: string, model: string) => `${provider}/${model}`;

  /** Chat-eligible pairs: enabled completion entries whose provider is
   *  actually configured (Ollama needs no key at all). */
  const chatPairs = useMemo(() => {
    return (matrix?.catalog ?? []).filter(
      (e) =>
        e.enabled &&
        capabilitiesOf(e).includes("completion") &&
        (e.provider === "ollama" || configuredByok.has(e.provider)),
    );
  }, [matrix, configuredByok]);

  /** Decision-eligible pairs: enabled decision entries whose provider holds
   *  an org credential row — the decision harness's own vocabulary. */
  const decisionPairs = useMemo(() => {
    return (matrix?.catalog ?? []).filter(
      (e) =>
        e.enabled &&
        capabilitiesOf(e).includes("decision") &&
        orgCredentialProviders.has(e.provider),
    );
  }, [matrix, orgCredentialProviders]);

  const testProvider = async (provider: string) => {
    setTesting(provider);
    const { data, error } = await api.api.POST("/api/v1/settings/ai/test", {
      body: { provider },
    });
    setTesting(null);
    if (error !== undefined || data === undefined) {
      setTestResults((prev) => ({
        ...prev,
        [provider]: {
          provider,
          ok: false,
          message: "The connection test could not run. Nothing was changed.",
          latency_ms: null,
        },
      }));
      return;
    }
    setTestResults((prev) => ({ ...prev, [provider]: data }));
  };

  const saveChatKey = async (): Promise<SaveOutcome | null> => {
    const meta = CHAT_PROVIDER_META.find((p) => p.id === pickedChat);
    if (meta === undefined || overview === null) return null;
    if (meta.need !== "none" && keyDraft.trim() === "") {
      setKeyError("Paste the key first, or pick a different provider.");
      return null;
    }
    if (meta.need === "none") return null; // Ollama: nothing to paste, nothing to seal
    setKeyError(null);
    setKeyBusy(true);
    const previous = overview;
    const wasSealed = configuredByok.has(meta.id);
    // Optimistic presence flip — the key itself never renders back.
    if (!wasSealed) {
      setOverview({
        ...overview,
        configured_providers: [...overview.configured_providers, meta.id],
      });
    }
    const result = await runEnvelopeMutation<
      components["schemas"]["PutByokSecretRequest"],
      components["schemas"]["ByokSecretMutationResponse"]
    >({
      describe: `Save ${meta.name} key`,
      entityId: `byok:${meta.id}`,
      optimistic: () => {},
      revert: () => setOverview(previous),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        provider: meta.id,
        api_key: keyDraft.trim(),
      }),
      request: (client, body) => client.PUT("/api/v1/settings/ai/byok", { body }),
    });
    setKeyBusy(false);
    if (!result.ok) {
      const message =
        result.friendly ??
        rejectionText(result, `Saving the ${meta.name} key was rejected. Nothing was changed.`);
      setKeyError(message);
      return { ok: false, line: message };
    }
    setKeyDraft("");
    void testProvider(meta.id);
    return {
      ok: true,
      line: wasSealed
        ? `${meta.name} key replaced — sealed, never shown again.`
        : `${meta.name} key saved — sealed, never shown again.`,
    };
  };

  const saveDecisionCredential = async (): Promise<SaveOutcome | null> => {
    if (pickedDecision !== DECISION_CREDENTIAL_PROVIDER) return null;
    if (decisionKeyDraft.trim() === "") {
      setDecisionKeyError("Paste the credential first, or pick Jev later in Settings.");
      return null;
    }
    setDecisionKeyError(null);
    setDecisionKeyBusy(true);
    const wasSealed = orgCredentialProviders.has(DECISION_CREDENTIAL_PROVIDER);
    const { error } = await api.api.PUT("/api/v1/settings/ai/credentials", {
      body: {
        provider: DECISION_CREDENTIAL_PROVIDER,
        kind: "api_key",
        secret: decisionKeyDraft.trim(),
        idempotency_key: `aicred:${DECISION_CREDENTIAL_PROVIDER}:${Date.now()}`,
      } as never,
    });
    setDecisionKeyBusy(false);
    if (error !== undefined) {
      const message = resultErrorText(error, "The Jev credential was rejected. Nothing was changed.");
      setDecisionKeyError(message);
      return { ok: false, line: message };
    }
    setDecisionKeyDraft("");
    return {
      ok: true,
      line: wasSealed
        ? "Jev credential replaced — sealed, never shown again."
        : "Jev credential saved — sealed, never shown again.",
    };
  };

  /** The honest exit: run every pending save, collect what happened. */
  const saveAll = async () => {
    setSaving(true);
    const lines: SaveOutcome[] = [];

    // 1. The master switch, only when it actually changed.
    if (aiOn !== null && aiOn !== decisionFeaturesEnabled) {
      const { error } = await api.api.PUT("/api/v1/settings/org", {
        body: { key: "ai.features_enabled", value: aiOn },
      });
      lines.push(
        error === undefined
          ? { ok: true, line: `AI turned ${aiOn ? "on" : "off"} for the organization.` }
          : { ok: false, line: "The AI switch did not save. Flip it later in Settings → AI." },
      );
    }

    // 2. The chat key.
    const chatMeta = CHAT_PROVIDER_META.find((p) => p.id === pickedChat);
    if (chatMeta !== undefined && chatMeta.need !== "none") {
      if (keyDraft.trim() === "") {
        lines.push({
          ok: false,
          line: `${chatMeta.name} was picked but no key was pasted, so nothing was saved for it.`,
        });
      } else {
        const outcome = await saveChatKey();
        if (outcome !== null) lines.push(outcome);
      }
    }

    // 3. The decision credential.
    if (pickedDecision === DECISION_CREDENTIAL_PROVIDER) {
      if (decisionKeyDraft.trim() === "") {
        lines.push({
          ok: false,
          line: "Jev was picked but no credential was pasted, so nothing was saved for it.",
        });
      } else {
        const outcome = await saveDecisionCredential();
        if (outcome !== null) lines.push(outcome);
      }
    }

    // 4. The chat default pick (org default on the chat surface).
    if (chatDefaultKey !== null && chatDefaultKey !== chatPicked) {
      const [provider = "", ...rest] = chatDefaultKey.split("/");
      const model = rest.join("/");
      const { error } = await api.api.PUT("/api/v1/settings/ai/model-picks/{surface_key}", {
        params: { path: { surface_key: CHAT_SURFACE_KEY } },
        body: { provider, model, idempotency_key: `orgpick:${CHAT_SURFACE_KEY}:${Date.now()}` },
      });
      lines.push(
        error === undefined
          ? { ok: true, line: `Chat default set to ${provider} / ${model}.` }
          : {
              ok: false,
              line: `The ${provider} / ${model} chat default was refused — set it later in Settings → AI.`,
            },
      );
      if (error === undefined) setChatPicked(chatDefaultKey);
    }

    // 5. The decision default, pinned on every decision surface that has no
    //    fork of its own. A forked surface keeps its own pinned model.
    if (decision !== null && decisionDefaultKey !== null && decisionDefaultKey !== pairKeyOf(decision)) {
      const [provider = "", ...rest] = decisionDefaultKey.split("/");
      const model = rest.join("/");
      const forked = decision.surfaces.filter((s) => s.customized).length;
      let pinned = 0;
      let refused = 0;
      for (const s of decision.surfaces.filter((x) => !x.customized)) {
        const { error } = await api.api.PUT("/api/v1/settings/ai/decision/{surface_key}/config", {
          params: { path: { surface_key: s.surface_key } },
          body: {
            model_provider: provider,
            model_id: model,
            idempotency_key: `dconf:${s.surface_key}:${Date.now()}`,
          } as never,
        });
        if (error === undefined) pinned += 1;
        else refused += 1;
      }
      if (pinned > 0) {
        lines.push({
          ok: true,
          line: `Decision default ${provider} / ${model} pinned on ${pinned} surface${pinned === 1 ? "" : "s"}.`,
        });
      }
      if (refused > 0) {
        lines.push({
          ok: false,
          line: `${refused} decision surface${refused === 1 ? "" : "s"} refused the pin — finish them in Settings → AI.`,
        });
      }
      if (forked > 0) {
        lines.push({
          ok: true,
          line: `${forked} forked decision surface${forked === 1 ? " keeps" : "s keep"} their own pinned model, untouched.`,
        });
      }
    }

    if (lines.length === 0) {
      lines.push({ ok: true, line: "Nothing was picked, so nothing was saved." });
    }
    setOutcomes(lines);
    setReported(true);
    setSaving(false);
    void load();
  };

  /** The org's de facto decision default: one pair agreed on by every
   *  non-forked decision surface, else null. */
  function pairKeyOf(d: DecisionOverviewResponse): string | null {
    const pinned = d.surfaces.filter(
      (s) => !s.customized && s.model_provider != null && s.model_id != null,
    );
    if (pinned.length === 0) return null;
    const first = pinned[0];
    if (first === undefined) return null;
    const same = pinned.every(
      (s) => s.model_provider === first.model_provider && s.model_id === first.model_id,
    );
    return same ? pairKey(first.model_provider as string, first.model_id as string) : null;
  }

  const finish = () => {
    setOutcomes(null);
    setReported(false);
    onDone();
  };

  // ---- the branch shape: gate → error → loading → body ----------------------
  if (auth.status !== "authed") return null;

  if (!isAdmin) {
    return (
      <div className="entry-mstep" data-testid="ai-setup-gate">
        <p className="entry-hint" style={{ margin: 0 }}>
          AI setup is owner/admin work. Your administrator can do it in Settings &rarr; AI &mdash;
          nothing here is holding the rest of setup up.
        </p>
        <div className="entry-actions">
          <button
            type="button"
            className="entry-btn entry-btn--secondary"
            onClick={onSkip}
            data-testid="ai-setup-notnow"
          >
            Not now
          </button>
        </div>
      </div>
    );
  }

  if (loadError !== null) {
    return (
      <div className="entry-mstep" data-testid="ai-setup">
        <div className="entry-field">
          <p className="entry-error" role="alert">
            <Icon name="alert" size={16} />
            {loadError}
          </p>
        </div>
        <div className="entry-actions">
          <button type="button" className="entry-btn entry-btn--secondary" onClick={() => void load()}>
            Try again
          </button>
          <span className="entry-actions__spacer" />
          <button type="button" className="entry-btn entry-btn--secondary" onClick={onSkip}>
            Not now
          </button>
        </div>
      </div>
    );
  }

  if (overview === null || matrix === null || decision === null) {
    return (
      <div
        className="entry-mstep"
        style={{ display: "flex", justifyContent: "center", padding: "18px 0" }}
      >
        <Spinner label="Loading AI settings" />
      </div>
    );
  }

  const currentChatMeta = CHAT_PROVIDER_META.find((p) => p.id === pickedChat) ?? null;
  const currentDecisionMeta = DECISION_PROVIDER_META.find((p) => p.id === pickedDecision) ?? null;
  const decisionConfigured = orgCredentialProviders.has(DECISION_CREDENTIAL_PROVIDER);
  const decisionPin = pairKeyOf(decision);
  const chatCatalogPairs = matrix.catalog.filter((e) => capabilitiesOf(e).includes("completion"));
  const decisionCatalogPairs = matrix.catalog.filter((e) => capabilitiesOf(e).includes("decision"));

  const chatRadioTabIndex = (id: string) =>
    pickedChat === id || (pickedChat === null && id === CHAT_PROVIDER_META[0]?.id) ? 0 : -1;

  const chatRadiogroupKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const enabled = CHAT_PROVIDER_META.filter((p) => p.backed);
    const current = enabled.findIndex((p) => p.id === pickedChat);
    const next =
      e.key === "ArrowDown"
        ? enabled[(current + 1 + enabled.length) % enabled.length]
        : enabled[(current - 1 + enabled.length) % enabled.length];
    if (next !== undefined) {
      setPickedChat(next.id);
      setKeyDraft("");
      setKeyError(null);
    }
  };

  return (
    <div
      className="entry-mstep"
      data-testid="ai-setup"
      style={{ display: "flex", flexDirection: "column", gap: 16 }}
    >
      <p className="entry-hint" style={{ margin: 0 }}>
        {KEY_WHY_SENTENCE} Keys are sealed at rest and never shown again &mdash; presence only.
      </p>

      {/* ---- 1 · the master switch ------------------------------------ */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 12,
          border: "1px solid var(--m-border)",
          borderRadius: 12,
          padding: "12px 14px",
        }}
        data-testid="ai-setup-master"
      >
        <div style={{ flex: 1 }}>
          <p style={{ margin: 0, fontSize: 14, fontWeight: 600 }}>AI in this organization</p>
          <p className="entry-hint" style={{ margin: 0 }}>
            Nothing calls a model until this is on and a provider is configured.
          </p>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={aiOn ?? decisionFeaturesEnabled}
          aria-label="AI in this organization"
          onClick={() => setAiOn((v) => !(v ?? decisionFeaturesEnabled))}
          data-testid="ai-setup-master-switch"
          style={{
            width: 44,
            height: 28,
            flexShrink: 0,
            borderRadius: 999,
            border: "1px solid var(--m-input)",
            background: (aiOn ?? decisionFeaturesEnabled) ? "var(--m-primary)" : "var(--m-surface-2)",
            position: "relative",
            cursor: "pointer",
            transition: "background 120ms ease",
          }}
        >
          <span
            style={{
              position: "absolute",
              top: 3,
              left: (aiOn ?? decisionFeaturesEnabled) ? 19 : 3,
              width: 20,
              height: 20,
              borderRadius: 999,
              background: (aiOn ?? decisionFeaturesEnabled) ? "var(--m-on-primary)" : "var(--m-text-secondary)",
              transition: "left 120ms ease",
            }}
          />
        </button>
      </div>

      {/* ---- 2 · chat providers ---------------------------------------- */}
      <div>
        <p style={{ margin: 0, fontSize: 14, fontWeight: 600 }}>Chat providers</p>
        <div
          role="radiogroup"
          aria-label="Chat provider"
          onKeyDown={chatRadiogroupKeyDown}
          data-testid="ai-setup-chat-cards"
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fill, minmax(220px, 1fr))",
            gap: 8,
            marginTop: 8,
          }}
        >
          {CHAT_PROVIDER_META.map((meta, i) => {
            const selected = pickedChat === meta.id;
            const sealed = configuredByok.has(meta.id);
            return (
              <button
                key={meta.id}
                type="button"
                className="entry-aicard"
                disabled={!meta.backed}
                style={{
                  animationDelay: `${i * 40}ms`,
                  display: "flex",
                  alignItems: "flex-start",
                  gap: 10,
                  minHeight: 44,
                  textAlign: "left",
                  borderRadius: 12,
                  border: `1px solid ${selected ? "var(--m-primary)" : "var(--m-border)"}`,
                  background: selected ? "var(--m-accent-tint)" : "var(--m-surface)",
                  padding: "10px 12px",
                  cursor: meta.backed ? "pointer" : "not-allowed",
                  color: meta.backed ? "var(--m-text)" : "var(--m-text-muted)",
                }}
                onClick={
                  meta.backed
                    ? () => {
                        setPickedChat(selected ? null : meta.id);
                        setKeyDraft("");
                        setKeyError(null);
                      }
                    : undefined
                }
                aria-disabled={!meta.backed}
                aria-checked={meta.backed ? selected : undefined}
                role={meta.backed ? "radio" : undefined}
                tabIndex={meta.backed ? chatRadioTabIndex(meta.id) : undefined}
                data-testid={`ai-setup-provider-${meta.id}`}
              >
                <span
                  aria-hidden
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    justifyContent: "center",
                    width: 32,
                    height: 32,
                    flexShrink: 0,
                    borderRadius: 8,
                    background: "var(--m-surface-2)",
                    color: "var(--m-text-secondary)",
                    fontSize: 12,
                    fontWeight: 700,
                    letterSpacing: "0.02em",
                  }}
                >
                  {meta.monogram}
                </span>
                <span style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 0 }}>
                  <span style={{ fontSize: 14, fontWeight: 600 }}>{meta.name}</span>
                  <span
                    style={{
                      fontSize: 12,
                      color: meta.backed ? "var(--m-text-secondary)" : "var(--m-text-muted)",
                    }}
                  >
                    {meta.backed ? meta.blurb : "Not available yet — the key store doesn't take it"}
                  </span>
                </span>
                {meta.backed ? (
                  selected ? (
                    <span
                      style={{ marginLeft: "auto", flexShrink: 0, display: "flex", color: "var(--m-text)" }}
                    >
                      <Icon name="check" size={16} />
                    </span>
                  ) : sealed ? (
                    <span
                      style={{
                        marginLeft: "auto",
                        flexShrink: 0,
                        fontSize: 11,
                        fontWeight: 600,
                        color: "var(--m-text-secondary)",
                        whiteSpace: "nowrap",
                      }}
                    >
                      Key sealed
                    </span>
                  ) : null
                ) : (
                  <span style={{ marginLeft: "auto", flexShrink: 0, display: "flex" }}>
                    <Icon name="lock" size={14} />
                  </span>
                )}
              </button>
            );
          })}
        </div>

        {/* The key lives with its provider: pick a card, its field appears. */}
        {currentChatMeta !== null && currentChatMeta.need !== "none" && (
          <div className="entry-field" style={{ marginTop: 10 }} data-testid="ai-setup-chat-key-field">
            <label className="entry-label" htmlFor="ai-setup-key">
              {currentChatMeta.name} API key
            </label>
            <div style={{ position: "relative" }}>
              <input
                id="ai-setup-key"
                className="entry-input"
                type={showKey ? "text" : "password"}
                autoComplete="off"
                spellCheck={false}
                value={keyDraft}
                onChange={(e) => {
                  setKeyDraft(e.target.value);
                  if (keyError !== null) setKeyError(null);
                }}
                placeholder={
                  configuredByok.has(currentChatMeta.id)
                    ? "Paste to replace the sealed key"
                    : "Paste the API key"
                }
                data-testid="ai-setup-key-input"
                aria-describedby={keyError !== null ? "ai-setup-key-error" : undefined}
                aria-invalid={keyError !== null ? true : undefined}
              />
              <button
                type="button"
                aria-label={showKey ? "Hide the key" : "Show the key"}
                aria-pressed={showKey}
                onClick={() => setShowKey((v) => !v)}
                data-testid="ai-setup-key-toggle"
                style={{
                  position: "absolute",
                  top: 0,
                  right: 0,
                  display: "flex",
                  width: 44,
                  height: 48,
                  alignItems: "center",
                  justifyContent: "center",
                  border: "none",
                  background: "transparent",
                  color: "var(--m-text-secondary)",
                  cursor: "pointer",
                }}
              >
                <Icon name="eye" size={16} />
              </button>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <button
                type="button"
                className="entry-btn entry-btn--small"
                onClick={() => void saveChatKey()}
                disabled={keyBusy || keyDraft.trim() === ""}
                data-testid="ai-setup-key-save"
              >
                {keyBusy ? <Spinner /> : "Save key"}
              </button>
              {configuredByok.has(currentChatMeta.id) && keyDraft.trim() === "" && (
                <span className="entry-hint">A key is already sealed for this provider.</span>
              )}
            </div>
            {keyError !== null && (
              <p
                className="entry-error"
                role="alert"
                id="ai-setup-key-error"
                data-testid="ai-setup-key-error"
              >
                <Icon name="alert" size={16} />
                {keyError}
              </p>
            )}
            {/* The test result, only ever after the key was saved. */}
            {(() => {
              const chatTest = testResults[currentChatMeta.id];
              if (testing === currentChatMeta.id) {
                return (
                  <p role="status" className="entry-hint" data-testid={`ai-setup-test-${currentChatMeta.id}`}>
                    Testing the connection…
                  </p>
                );
              }
              if (chatTest === undefined) return null;
              return (
                <p
                  role="status"
                  className="entry-hint"
                  data-testid={`ai-setup-test-${currentChatMeta.id}`}
                  style={{
                    color: chatTest.ok ? "var(--m-status-success-fg)" : "var(--m-status-danger-fg)",
                    fontWeight: 600,
                  }}
                >
                  {chatTest.ok ? "Connected" : "Failed"}: {chatTest.message}
                  {chatTest.latency_ms != null ? ` (${chatTest.latency_ms} ms)` : ""}
                </p>
              );
            })()}
          </div>
        )}
        {currentChatMeta !== null && currentChatMeta.need === "none" && (
          <p
            className="entry-hint"
            style={{ margin: "10px 0 0" }}
            data-testid="ai-setup-provider-ollama-note"
          >
            Runs on this machine&apos;s Ollama. Nothing to paste here &mdash; pick a model with
            Ollama as its provider under Defaults to route chat through it.
          </p>
        )}
      </div>

      {/* ---- 3 · the decision engine ----------------------------------- */}
      <div>
        <p style={{ margin: 0, fontSize: 14, fontWeight: 600 }}>Decision engine</p>
        <p className="entry-hint" style={{ margin: "2px 0 0" }}>
          Jev runs the decision surfaces &mdash; triage, dispatch, suggestions. It takes its own
          credential, sealed like a key.
        </p>
        <div
          role="radiogroup"
          aria-label="Decision provider"
          data-testid="ai-setup-decision-cards"
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fill, minmax(220px, 1fr))",
            gap: 8,
            marginTop: 8,
          }}
        >
          {DECISION_PROVIDER_META.map((meta) => {
            const selected = pickedDecision === meta.id;
            return (
              <button
                key={meta.id}
                type="button"
                disabled={!meta.backed}
                aria-checked={meta.backed ? selected : undefined}
                role={meta.backed ? "radio" : undefined}
                tabIndex={meta.backed ? (selected ? 0 : -1) : undefined}
                style={{
                  display: "flex",
                  alignItems: "flex-start",
                  gap: 10,
                  minHeight: 44,
                  textAlign: "left",
                  borderRadius: 12,
                  border: `1px solid ${selected ? "var(--m-primary)" : "var(--m-border)"}`,
                  background: selected ? "var(--m-accent-tint)" : "var(--m-surface)",
                  padding: "10px 12px",
                  cursor: meta.backed ? "pointer" : "not-allowed",
                  color: meta.backed ? "var(--m-text)" : "var(--m-text-muted)",
                }}
                onClick={
                  meta.backed
                    ? () => {
                        setPickedDecision(selected ? null : meta.id);
                        setDecisionKeyDraft("");
                        setDecisionKeyError(null);
                      }
                    : undefined
                }
                data-testid={`ai-setup-decision-${meta.id}`}
              >
                <span
                  aria-hidden
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    justifyContent: "center",
                    width: 32,
                    height: 32,
                    flexShrink: 0,
                    borderRadius: 8,
                    background: "var(--m-surface-2)",
                    color: "var(--m-text-secondary)",
                    fontSize: 12,
                    fontWeight: 700,
                    letterSpacing: "0.02em",
                  }}
                >
                  {meta.monogram}
                </span>
                <span style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 0 }}>
                  <span style={{ fontSize: 14, fontWeight: 600 }}>{meta.name}</span>
                  <span
                    style={{
                      fontSize: 12,
                      color: meta.backed ? "var(--m-text-secondary)" : "var(--m-text-muted)",
                    }}
                  >
                    {meta.backed ? meta.blurb : "Not available yet — decisions ride Jev today"}
                  </span>
                </span>
                {meta.backed && decisionConfigured && (
                  <span
                    style={{
                      marginLeft: "auto",
                      flexShrink: 0,
                      fontSize: 11,
                      fontWeight: 600,
                      color: "var(--m-text-secondary)",
                      whiteSpace: "nowrap",
                    }}
                  >
                    Credential sealed
                  </span>
                )}
              </button>
            );
          })}
        </div>
        {currentDecisionMeta !== null && (
          <div className="entry-field" style={{ marginTop: 10 }} data-testid="ai-setup-decision-key-field">
            <label className="entry-label" htmlFor="ai-setup-decision-key">
              Jev credential
            </label>
            <div style={{ position: "relative" }}>
              <input
                id="ai-setup-decision-key"
                className="entry-input"
                type={showDecisionKey ? "text" : "password"}
                autoComplete="off"
                spellCheck={false}
                value={decisionKeyDraft}
                onChange={(e) => {
                  setDecisionKeyDraft(e.target.value);
                  if (decisionKeyError !== null) setDecisionKeyError(null);
                }}
                placeholder={
                  decisionConfigured ? "Paste to replace the sealed credential" : "Paste the Jev credential"
                }
                data-testid="ai-setup-decision-key-input"
                aria-describedby={decisionKeyError !== null ? "ai-setup-decision-key-error" : undefined}
                aria-invalid={decisionKeyError !== null ? true : undefined}
              />
              <button
                type="button"
                aria-label={showDecisionKey ? "Hide the credential" : "Show the credential"}
                aria-pressed={showDecisionKey}
                onClick={() => setShowDecisionKey((v) => !v)}
                data-testid="ai-setup-decision-key-toggle"
                style={{
                  position: "absolute",
                  top: 0,
                  right: 0,
                  display: "flex",
                  width: 44,
                  height: 48,
                  alignItems: "center",
                  justifyContent: "center",
                  border: "none",
                  background: "transparent",
                  color: "var(--m-text-secondary)",
                  cursor: "pointer",
                }}
              >
                <Icon name="eye" size={16} />
              </button>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <button
                type="button"
                className="entry-btn entry-btn--small"
                onClick={() => void saveDecisionCredential()}
                disabled={decisionKeyBusy || decisionKeyDraft.trim() === ""}
                data-testid="ai-setup-decision-key-save"
              >
                {decisionKeyBusy ? <Spinner /> : "Save credential"}
              </button>
            </div>
            {decisionKeyError !== null && (
              <p
                className="entry-error"
                role="alert"
                id="ai-setup-decision-key-error"
                data-testid="ai-setup-decision-key-error"
              >
                <Icon name="alert" size={16} />
                {decisionKeyError}
              </p>
            )}
          </div>
        )}
      </div>

      {/* ---- 4 · defaults ------------------------------------------------ */}
      <div>
        <p style={{ margin: 0, fontSize: 14, fontWeight: 600 }}>Defaults</p>
        <p className="entry-hint" style={{ margin: "2px 0 0" }}>
          Which model answers when nobody picks one. Only providers you configured above are
          selectable.
        </p>

        <p
          style={{ margin: "10px 0 2px", fontSize: 13, fontWeight: 600, color: "var(--m-text-secondary)" }}
        >
          Chat default
        </p>
        <p className="entry-hint" style={{ margin: "0 0 4px" }} data-testid="ai-setup-chat-current">
          {chatPicked !== null
            ? `Right now: ${chatPicked}`
            : "No default yet — chat has none until you pick one."}
        </p>
        <div
          role="radiogroup"
          aria-label="Chat default model"
          data-testid="ai-setup-chat-defaults"
          style={{ display: "flex", flexDirection: "column", gap: 4 }}
        >
          {chatCatalogPairs.length === 0 ? (
            <div className="entry-note" data-testid="ai-setup-chat-defaults-empty">
              <Icon name="info" size={16} />
              <span>
                No models are cataloged yet.{" "}
                <a href="/settings/ai" style={{ color: "inherit", fontWeight: 600 }}>
                  Catalog one in Settings → AI
                </a>{" "}
                and it will show up here.
              </span>
            </div>
          ) : (
            chatCatalogPairs.map((e) => {
              const eligible = chatPairs.some((p) => p.id === e.id);
              const key = pairKey(e.provider, e.model);
              const selected = chatDefaultKey === key;
              return (
                <button
                  key={e.id}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  tabIndex={selected ? 0 : -1}
                  disabled={!eligible}
                  onClick={() => setChatDefaultKey(selected ? null : key)}
                  data-testid={`ai-setup-chat-default-${e.provider}-${e.model}`}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 10,
                    minHeight: 44,
                    textAlign: "left",
                    borderRadius: 10,
                    border: `1px solid ${selected ? "var(--m-primary)" : "var(--m-border)"}`,
                    background: selected ? "var(--m-accent-tint)" : "var(--m-surface)",
                    padding: "8px 12px",
                    cursor: eligible ? "pointer" : "not-allowed",
                    color: eligible ? "var(--m-text)" : "var(--m-text-muted)",
                  }}
                >
                  <span style={{ flexShrink: 0, display: "flex" }}>
                    <Icon name={selected ? "check" : "info"} size={16} />
                  </span>
                  <span style={{ fontFamily: "var(--m-font-mono)", fontSize: 13 }}>
                    {e.provider}/{e.model}
                  </span>
                  {e.label !== `${e.provider}/${e.model}` && (
                    <span className="entry-hint" style={{ fontSize: 12 }}>
                      {e.label}
                    </span>
                  )}
                  {!eligible && (
                    <span className="entry-hint" style={{ marginLeft: "auto", fontSize: 12 }}>
                      provider not configured
                    </span>
                  )}
                </button>
              );
            })
          )}
        </div>

        <p
          style={{ margin: "12px 0 2px", fontSize: 13, fontWeight: 600, color: "var(--m-text-secondary)" }}
        >
          Decision default
        </p>
        <p className="entry-hint" style={{ margin: "0 0 4px" }} data-testid="ai-setup-decision-current">
          {decisionPin !== null
            ? `Right now: ${decisionPin}`
            : "No default yet — decision surfaces stay quietly off."}
        </p>
        <div
          role="radiogroup"
          aria-label="Decision default model"
          data-testid="ai-setup-decision-defaults"
          style={{ display: "flex", flexDirection: "column", gap: 4 }}
        >
          {decisionCatalogPairs.length === 0 ? (
            <div className="entry-note" data-testid="ai-setup-decision-defaults-empty">
              <Icon name="info" size={16} />
              <span>
                No decision models are cataloged yet.{" "}
                <a href="/settings/ai" style={{ color: "inherit", fontWeight: 600 }}>
                  Catalog one in Settings → AI
                </a>{" "}
                and it will show up here.
              </span>
            </div>
          ) : (
            decisionCatalogPairs.map((e) => {
              const eligible = decisionPairs.some((p) => p.id === e.id);
              const key = pairKey(e.provider, e.model);
              const selected = decisionDefaultKey === key;
              return (
                <button
                  key={e.id}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  tabIndex={selected ? 0 : -1}
                  disabled={!eligible}
                  onClick={() => setDecisionDefaultKey(selected ? null : key)}
                  data-testid={`ai-setup-decision-default-${e.provider}-${e.model}`}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 10,
                    minHeight: 44,
                    textAlign: "left",
                    borderRadius: 10,
                    border: `1px solid ${selected ? "var(--m-primary)" : "var(--m-border)"}`,
                    background: selected ? "var(--m-accent-tint)" : "var(--m-surface)",
                    padding: "8px 12px",
                    cursor: eligible ? "pointer" : "not-allowed",
                    color: eligible ? "var(--m-text)" : "var(--m-text-muted)",
                  }}
                >
                  <span style={{ flexShrink: 0, display: "flex" }}>
                    <Icon name={selected ? "check" : "info"} size={16} />
                  </span>
                  <span style={{ fontFamily: "var(--m-font-mono)", fontSize: 13 }}>
                    {e.provider}/{e.model}
                  </span>
                  {!eligible && (
                    <span className="entry-hint" style={{ marginLeft: "auto", fontSize: 12 }}>
                      {e.provider === DECISION_CREDENTIAL_PROVIDER
                        ? "no credential yet"
                        : "provider not configured"}
                    </span>
                  )}
                </button>
              );
            })
          )}
        </div>
      </div>

      {/* ---- the honest report ----------------------------------------- */}
      {outcomes !== null && (
        <div
          role="status"
          data-testid="ai-setup-report"
          style={{
            display: "flex",
            flexDirection: "column",
            gap: 4,
            border: "1px solid var(--m-border)",
            borderRadius: 12,
            background: "var(--m-surface-2)",
            padding: "12px 14px",
          }}
        >
          <p style={{ margin: 0, fontSize: 13, fontWeight: 600 }}>What saved</p>
          {outcomes.map((o, i) => (
            <p
              key={i}
              className="entry-hint"
              style={{
                margin: 0,
                color: o.ok ? "var(--m-text-secondary)" : "var(--m-status-danger-fg)",
                fontWeight: o.ok ? 400 : 600,
              }}
            >
              {o.line}
            </p>
          ))}
        </div>
      )}

      {/* ---- actions: the exit is first-class -------------------------- */}
      <div className="entry-actions" style={{ marginTop: 4 }}>
        <button
          type="button"
          className="entry-btn entry-btn--secondary entry-btn--small"
          onClick={onBack}
          disabled={backDisabled}
          data-testid="ai-setup-back"
        >
          Back
        </button>
        <span className="entry-actions__spacer" />
        <button
          type="button"
          className="entry-btn entry-btn--secondary entry-btn--small"
          onClick={onSkip}
          data-testid="ai-setup-notnow"
        >
          Not now
        </button>
        <button
          type="button"
          className="entry-btn"
          onClick={() => void (reported ? finish() : saveAll())}
          disabled={saving}
          data-testid={reported ? "ai-setup-done" : "ai-setup-save"}
        >
          {saving ? (
            <>
              <Spinner /> Saving…
            </>
          ) : reported ? (
            "Done"
          ) : (
            "Save and continue"
          )}
        </button>
      </div>
      <p className="entry-hint" style={{ textAlign: "center", margin: 0 }} data-testid="ai-setup-later-hint">
        Not now is fine &mdash; everything here can be set up later in Settings &rarr; AI.
      </p>
    </div>
  );
}
